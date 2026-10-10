//! The contract, proven against a running server (ADR-2610092329).
//!
//! Every operation is sent once, in an order that makes its own inputs:
//! creates first, so the id a create returns is the id a read then asks
//! for, and deletes last, so the run removes what it made. Each answer is
//! judged against the contract. A declared error proves nothing about the
//! success shape, so it is `Unproven`, not a pass, and a run that proved
//! nothing at all exits as vacuous.

use std::collections::HashMap;

use serde_json::{json, Map, Value};

use super::domain::{
    ApiBody, ApiContract, ApiOperation, ApiParam, ApiSchema, ConformanceReport, OperationVerdict, ParamLocation,
    ProbeRequest, TypeRef, Verdict,
};
use super::ports::HttpProbe;

const SAMPLE: &str = "hexa-contract-test";

/// Send every operation of `contract` through `probe` and judge each answer.
/// `examples` maps a wire name to the value to send for it.
pub async fn run(contract: &ApiContract, probe: &dyn HttpProbe, examples: &Map<String, Value>) -> ConformanceReport {
    let mut ops: Vec<&ApiOperation> = contract.operations.iter().collect();
    ops.sort_by_key(|op| (rank(op), op.file.clone(), op.line));
    let mut learned: Learned = HashMap::new();
    let mut report = ConformanceReport::default();
    for op in ops {
        let route = format!("{} {}", op.http_method, op.path);
        let request = match request_for(op, contract, examples, &learned) {
            Ok(r) => r,
            Err(why) => {
                report.operations.push(OperationVerdict { route, verdict: Verdict::Unproven, status: None, details: vec![why] });
                continue;
            }
        };
        let verdict = match probe.send(&request).await {
            Err(e) => OperationVerdict { route, verdict: Verdict::Unreachable, status: None, details: vec![e] },
            Ok(resp) => {
                let (verdict, details) = judge(op, contract, resp.status, &resp.body);
                if resp.status == op.success && rank(op) == 0 {
                    if let Ok(v) = serde_json::from_str::<Value>(&resp.body) {
                        learn(&v, &op.path, &mut learned);
                    }
                }
                OperationVerdict { route, verdict, status: Some(resp.status), details }
            }
        };
        report.operations.push(verdict);
    }
    report
}

/// Creates, then collection reads, then reads by id, then updates, then deletes.
fn rank(op: &ApiOperation) -> u8 {
    let has_path = op.params.iter().any(|p| p.location == ParamLocation::Path);
    match (op.http_method.as_str(), has_path) {
        ("POST" | "PUT" | "PATCH", false) => 0,
        ("GET" | "HEAD", false) => 1,
        ("GET" | "HEAD", true) => 2,
        ("POST" | "PUT" | "PATCH", true) => 3,
        ("DELETE", _) => 4,
        _ => 5,
    }
}

/// Values seen in success answers, keyed by the resource path that returned
/// them and the wire name, so `/users` and `/posts` each keep their own `id`.
type Learned = HashMap<(String, String), Value>;

/// The resource a parameter belongs to: the path before its `{name}`
/// placeholder, or the whole path for a query parameter.
fn scope_of(op: &ApiOperation, p: &ApiParam) -> String {
    match op.path.find(&format!("{{{}}}", p.name)) {
        Some(i) if p.location == ParamLocation::Path => op.path[..i].trim_end_matches('/').to_string(),
        _ => op.path.clone(),
    }
}

/// Remember every scalar field of a create's answer by its wire name. Only
/// what this run made is learned: a list or a read shows records that were
/// already there, and an id taken from one would let a later update or
/// delete touch data the run did not create.
fn learn(v: &Value, scope: &str, learned: &mut Learned) {
    if let Value::Object(m) = v {
        for (k, val) in m {
            if matches!(val, Value::String(_) | Value::Number(_) | Value::Bool(_)) {
                learned.entry((scope.trim_end_matches('/').to_string(), k.clone())).or_insert_with(|| val.clone());
            }
        }
    }
}

fn request_for(
    op: &ApiOperation,
    contract: &ApiContract,
    examples: &Map<String, Value>,
    learned: &Learned,
) -> Result<ProbeRequest, String> {
    let mut path = op.path.clone();
    let mut query: Vec<String> = Vec::new();
    for p in &op.params {
        let made = learned.get(&(scope_of(op, p), p.name.clone()));
        // A write to a record by id may only reach one this run created, never
        // one named in --examples.
        let writes = p.location == ParamLocation::Path && !matches!(op.http_method.as_str(), "GET" | "HEAD");
        let given = if writes { made.cloned() } else { examples.get(&p.name).or(made).cloned() };
        match p.location {
            ParamLocation::Path => {
                let Some(v) = given else {
                    return Err(format!(
                        "no value for `{{{}}}`: an earlier create returns one, or give it in --examples",
                        p.name
                    ));
                };
                path = path.replace(&format!("{{{}}}", p.name), &encode(&scalar_text(&v)));
            }
            ParamLocation::Query => {
                let v = match given {
                    Some(v) => v,
                    None if p.required => synthesize(&p.name, &p.ty, contract, examples, 0),
                    None => continue,
                };
                match v {
                    Value::Array(items) => {
                        query.extend(items.iter().map(|i| format!("{}={}", encode(&p.name), encode(&scalar_text(i)))))
                    }
                    other => query.push(format!("{}={}", encode(&p.name), encode(&scalar_text(&other)))),
                }
            }
        }
    }
    let body = op.body.as_ref().map(|b| match b {
        ApiBody::Whole(t) => {
            let t = match t {
                TypeRef::Optional(inner) => inner,
                other => other,
            };
            synthesize("body", t, contract, examples, 0)
        }
        ApiBody::Fields(fields) => {
            let mut m = Map::new();
            for (name, t, required) in fields {
                if let Some(v) = examples.get(name) {
                    m.insert(name.clone(), v.clone());
                } else if *required {
                    m.insert(name.clone(), synthesize(name, t, contract, examples, 0));
                }
            }
            Value::Object(m)
        }
    });
    if !query.is_empty() {
        path = format!("{path}?{}", query.join("&"));
    }
    Ok(ProbeRequest { method: op.http_method.clone(), path_and_query: path, body: body.map(|b| b.to_string()) })
}

/// A value of type `ty` for a field called `name`: what a cooperative
/// client would plausibly send.
fn synthesize(name: &str, ty: &TypeRef, contract: &ApiContract, examples: &Map<String, Value>, depth: usize) -> Value {
    if depth > 8 {
        return Value::Null;
    }
    match ty {
        TypeRef::String => {
            let n = name.to_lowercase();
            if ["url", "uri", "link", "href"].iter().any(|k| n.contains(k)) {
                json!(format!("https://example.com/{SAMPLE}"))
            } else if n.contains("email") {
                json!(format!("{SAMPLE}@example.com"))
            } else {
                json!(SAMPLE)
            }
        }
        TypeRef::Integer => json!(1),
        TypeRef::Number => json!(1.5),
        TypeRef::Boolean => json!(true),
        TypeRef::Array(e) => json!([synthesize(name, e, contract, examples, depth + 1)]),
        TypeRef::Map(_) => json!({}),
        TypeRef::Optional(t) => synthesize(name, t, contract, examples, depth + 1),
        TypeRef::Named(n) => match contract.schemas.get(n) {
            Some(ApiSchema::Object(fields)) => {
                let mut m = Map::new();
                for (field, t, required) in fields {
                    if let Some(v) = examples.get(field) {
                        m.insert(field.clone(), v.clone());
                    } else if *required {
                        m.insert(field.clone(), synthesize(field, t, contract, examples, depth + 1));
                    }
                }
                Value::Object(m)
            }
            Some(ApiSchema::StringEnum(names)) => names.first().map_or(Value::Null, |s| json!(s)),
            None => Value::Null,
        },
        TypeRef::Unit | TypeRef::Unsupported(_) => Value::Null,
    }
}

fn scalar_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Percent-encode everything outside the unreserved set.
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            other => format!("%{other:02X}"),
        })
        .collect()
}

/// The verdict on one answer.
fn judge(op: &ApiOperation, contract: &ApiContract, status: u16, body: &str) -> (Verdict, Vec<String>) {
    if status == op.success {
        let Some(t) = &op.response else {
            return if body.trim().is_empty() {
                (Verdict::Proven, Vec::new())
            } else {
                (Verdict::Violation, vec![format!("{status} declares no body, and the server sent one")])
            };
        };
        let Ok(v) = serde_json::from_str::<Value>(body) else {
            return (Verdict::Violation, vec![format!("{status}: the body is not JSON")]);
        };
        let mut errors = Vec::new();
        validate(&v, t, "$", contract, &mut errors);
        if !errors.is_empty() {
            return (Verdict::Violation, errors);
        }
        // An empty list or map, or a null for an optional, fits any inner
        // schema, so it checked nothing the contract says: valid, and proof
        // of nothing.
        if is_vacuous(&v, t) {
            return (
                Verdict::Unproven,
                vec![format!(
                    "answered {status} with an empty list: nothing to check its items against. An earlier create should have made one; check it, or give inputs in --examples"
                )],
            );
        }
        return (Verdict::Proven, Vec::new());
    }
    if op.errors.contains(&status) {
        return (
            Verdict::Unproven,
            vec![format!(
                "answered {status}, a declared error: nothing wrong, nothing proven. Give inputs the server accepts in --examples"
            )],
        );
    }
    let mut declared: Vec<String> = vec![op.success.to_string()];
    declared.extend(op.errors.iter().map(u16::to_string));
    (Verdict::Violation, vec![format!("answered {status}, which is not declared (declared: {})", declared.join(", "))])
}

/// Whether `v` satisfies `ty` without any value having been checked.
fn is_vacuous(v: &Value, ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Optional(inner) => v.is_null() || is_vacuous(v, inner),
        TypeRef::Array(_) => v.as_array().is_some_and(Vec::is_empty),
        TypeRef::Map(_) => v.as_object().is_some_and(Map::is_empty),
        _ => false,
    }
}

fn kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Every way `v` fails `ty`, each at its JSON path.
fn validate(v: &Value, ty: &TypeRef, at: &str, contract: &ApiContract, out: &mut Vec<String>) {
    let expected = |want: &str, out: &mut Vec<String>| out.push(format!("{at}: expected {want}, got {}", kind(v)));
    match ty {
        TypeRef::String if !v.is_string() => expected("string", out),
        TypeRef::Integer if !(v.is_i64() || v.is_u64()) => expected("integer", out),
        TypeRef::Number if !v.is_number() => expected("number", out),
        TypeRef::Boolean if !v.is_boolean() => expected("boolean", out),
        TypeRef::Array(e) => match v.as_array() {
            Some(items) => {
                for (i, item) in items.iter().enumerate() {
                    validate(item, e, &format!("{at}[{i}]"), contract, out);
                }
            }
            None => expected("array", out),
        },
        TypeRef::Map(t) => match v.as_object() {
            Some(m) => {
                for (k, item) in m {
                    validate(item, t, &format!("{at}.{k}"), contract, out);
                }
            }
            None => expected("object", out),
        },
        TypeRef::Optional(t) => {
            if !v.is_null() {
                validate(v, t, at, contract, out);
            }
        }
        TypeRef::Named(n) => match contract.schemas.get(n) {
            Some(ApiSchema::Object(fields)) => {
                let Some(m) = v.as_object() else {
                    return expected(&format!("object {n}"), out);
                };
                for (field, t, required) in fields {
                    match m.get(field) {
                        Some(Value::Null) if !required => {}
                        Some(item) => validate(item, t, &format!("{at}.{field}"), contract, out),
                        None if *required => out.push(format!("{at}.{field}: required by {n}, missing")),
                        None => {}
                    }
                }
                for key in m.keys().filter(|k| !fields.iter().any(|(f, _, _)| f == *k)) {
                    out.push(format!("{at}.{key}: not in the schema of {n}"));
                }
            }
            Some(ApiSchema::StringEnum(names)) => {
                if !v.as_str().is_some_and(|s| names.iter().any(|n| n == s)) {
                    out.push(format!("{at}: expected one of {}, got {v}", names.join(", ")));
                }
            }
            None => out.push(format!("{at}: schema {n} is missing from the contract")),
        },
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ProbeResponse;

    fn list_op(response: TypeRef) -> ApiOperation {
        ApiOperation {
            service: "s".into(),
            port: "P".into(),
            method_name: "list".into(),
            operation_id: "list".into(),
            http_method: "GET".into(),
            path: "/b".into(),
            description: String::new(),
            params: vec![],
            body: None,
            success: 200,
            response: Some(response),
            errors: vec![500],
            file: "f".into(),
            line: 1,
            args: vec![],
            is_async: false,
            fails: true,
            error_type: None,
            error_variants: vec![],
        }
    }

    fn contract() -> ApiContract {
        let mut c = ApiContract::default();
        c.schemas.insert(
            "B".into(),
            ApiSchema::Object(vec![
                ("id".into(), TypeRef::String, true),
                ("tags".into(), TypeRef::Array(Box::new(TypeRef::String)), true),
                ("note".into(), TypeRef::String, false),
            ]),
        );
        c
    }

    #[test]
    fn validation_names_each_failure_at_its_path() {
        let c = contract();
        let mut out = Vec::new();
        validate(&json!({"tags": ["a", 2], "extra": 1}), &TypeRef::Named("B".into()), "$", &c, &mut out);
        out.sort();
        assert_eq!(
            out,
            vec![
                "$.extra: not in the schema of B",
                "$.id: required by B, missing",
                "$.tags[1]: expected string, got integer",
            ]
        );
    }

    #[test]
    fn an_optional_field_may_be_absent_or_null() {
        let c = contract();
        let mut out = Vec::new();
        validate(&json!({"id": "x", "tags": [], "note": null}), &TypeRef::Named("B".into()), "$", &c, &mut out);
        assert!(out.is_empty(), "{out:?}");
    }

    #[test]
    fn a_synthesized_value_follows_the_name_and_the_type() {
        let c = contract();
        let e = Map::new();
        assert_eq!(synthesize("url", &TypeRef::String, &c, &e, 0), json!("https://example.com/hexa-contract-test"));
        assert_eq!(synthesize("b", &TypeRef::Named("B".into()), &c, &e, 0), json!({"id": SAMPLE, "tags": [SAMPLE]}));
    }

    #[test]
    fn encoding_keeps_unreserved_characters() {
        assert_eq!(encode("a b/c-d"), "a%20b%2Fc-d");
    }

    #[test]
    fn an_empty_list_is_unproven_and_a_full_one_is_judged() {
        let c = contract();
        let op = list_op(TypeRef::Array(Box::new(TypeRef::Named("B".into()))));
        assert_eq!(judge(&op, &c, 200, "[]").0, Verdict::Unproven);
        assert_eq!(judge(&op, &c, 200, r#"[{"id":"x","tags":[]}]"#).0, Verdict::Proven);
        assert_eq!(judge(&op, &c, 200, r#"[{"tags":[]}]"#).0, Verdict::Violation);
    }

    #[test]
    fn an_empty_answer_under_optional_or_map_is_unproven() {
        let c = ApiContract::default();
        let item = || Box::new(TypeRef::String);
        let cases = [
            (TypeRef::Optional(Box::new(TypeRef::Array(item()))), "[]"),
            (TypeRef::Optional(Box::new(TypeRef::Array(item()))), "null"),
            (TypeRef::Map(item()), "{}"),
            (TypeRef::Optional(Box::new(TypeRef::String)), "null"),
        ];
        for (ty, body) in cases {
            let op = list_op(ty.clone());
            assert_eq!(judge(&op, &c, 200, body).0, Verdict::Unproven, "{ty:?} {body}");
        }
        let op = list_op(TypeRef::Optional(Box::new(TypeRef::Array(item()))));
        assert_eq!(judge(&op, &c, 200, r#"["a"]"#).0, Verdict::Proven);
    }

    #[test]
    fn a_report_that_proved_nothing_is_vacuous() {
        let v = |verdict| OperationVerdict { route: "GET /".into(), verdict, status: None, details: vec![] };
        let r = |vs: Vec<Verdict>| ConformanceReport { operations: vs.into_iter().map(v).collect() };
        assert_eq!(r(vec![Verdict::Proven, Verdict::Proven]).exit_code(), 0);
        assert_eq!(r(vec![Verdict::Proven, Verdict::Unproven]).exit_code(), 1);
        assert_eq!(r(vec![Verdict::Unproven, Verdict::Unreachable]).exit_code(), 2);
        assert_eq!(r(vec![]).exit_code(), 2);
    }

    fn op(method: &str, path: &str, success: u16, response: Option<TypeRef>, line: usize) -> ApiOperation {
        let params = path
            .split('/')
            .filter_map(|s| s.strip_prefix('{').and_then(|s| s.strip_suffix('}')))
            .map(|n| ApiParam { name: n.into(), location: ParamLocation::Path, ty: TypeRef::String, required: true })
            .collect();
        ApiOperation {
            service: "s".into(),
            port: "P".into(),
            method_name: format!("m{line}"),
            operation_id: format!("m{line}"),
            http_method: method.into(),
            path: path.into(),
            description: String::new(),
            params,
            body: None,
            success,
            response,
            errors: vec![404],
            file: "f".into(),
            line,
            args: vec![],
            is_async: false,
            fails: true,
            error_type: None,
            error_variants: vec![],
        }
    }

    /// Creates return `u1` and `p1`; `/users` already holds a record `old`.
    struct Server;

    #[async_trait::async_trait]
    impl HttpProbe for Server {
        async fn send(&self, r: &ProbeRequest) -> Result<ProbeResponse, String> {
            let (status, body) = match (r.method.as_str(), r.path_and_query.as_str()) {
                ("POST", "/users") => (201, r#"{"id":"u1"}"#),
                ("POST", "/posts") => (201, r#"{"id":"p1"}"#),
                ("GET", "/users") => (200, r#"[{"id":"old"},{"id":"u1"}]"#),
                ("GET" | "DELETE", "/users/u1") | ("GET" | "DELETE", "/posts/p1") => (200, r#"{"id":"x"}"#),
                _ => (404, ""),
            };
            Ok(ProbeResponse { status, body: body.into() })
        }
    }

    #[tokio::test]
    async fn an_id_is_the_one_its_own_resource_created_and_a_read_never_replaces_it() {
        let item = Some(TypeRef::Unsupported("any".into()));
        let mut c = ApiContract::default();
        c.operations = vec![
            op("POST", "/users", 201, item.clone(), 1),
            op("POST", "/posts", 201, item.clone(), 2),
            op("GET", "/users", 200, item.clone(), 3),
            op("GET", "/users/{id}", 200, item.clone(), 4),
            op("DELETE", "/users/{id}", 200, item.clone(), 5),
            op("GET", "/posts/{id}", 200, item.clone(), 6),
            op("DELETE", "/posts/{id}", 200, item, 7),
        ];
        let report = run(&c, &Server, &Map::new()).await;
        let unproven: Vec<_> = report.operations.iter().filter(|o| o.verdict != Verdict::Proven).collect();
        assert!(unproven.is_empty(), "{unproven:?}");
    }

    /// Records every request; `/users` holds only a record the run did not make.
    struct Recorder(std::sync::Mutex<Vec<String>>);

    #[async_trait::async_trait]
    impl HttpProbe for Recorder {
        async fn send(&self, r: &ProbeRequest) -> Result<ProbeResponse, String> {
            self.0.lock().unwrap().push(format!("{} {}", r.method, r.path_and_query));
            let (status, body) = match r.method.as_str() {
                "POST" => (500, ""),
                "GET" if r.path_and_query == "/users" => (200, r#"[{"id":"old"}]"#),
                _ => (200, r#"{"id":"old"}"#),
            };
            Ok(ProbeResponse { status, body: body.into() })
        }
    }

    #[tokio::test]
    async fn a_delete_never_reaches_a_record_the_run_did_not_create() {
        let item = Some(TypeRef::Unsupported("any".into()));
        let mut c = ApiContract::default();
        c.operations = vec![
            op("POST", "/users", 201, item.clone(), 1),
            op("GET", "/users", 200, item.clone(), 2),
            op("PUT", "/users/{id}", 200, item.clone(), 3),
            op("DELETE", "/users/{id}", 200, item, 4),
        ];
        let mut examples = Map::new();
        examples.insert("id".into(), json!("mine"));
        let probe = Recorder(Default::default());
        let report = run(&c, &probe, &examples).await;
        let sent = probe.0.lock().unwrap().clone();
        assert!(!sent.iter().any(|s| s.starts_with("DELETE") || s.starts_with("PUT")), "{sent:?}");
        assert!(report.operations.iter().any(|o| o.route == "DELETE /users/{id}" && o.verdict == Verdict::Unproven));
    }
}
