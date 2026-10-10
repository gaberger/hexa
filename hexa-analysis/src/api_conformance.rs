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
    ApiBody, ApiContract, ApiOperation, ApiSchema, ConformanceReport, OperationVerdict, ParamLocation,
    ProbeRequest, TypeRef, Verdict,
};
use super::ports::HttpProbe;

const SAMPLE: &str = "hexa-contract-test";

/// Send every operation of `contract` through `probe` and judge each answer.
/// `examples` maps a wire name to the value to send for it.
pub async fn run(contract: &ApiContract, probe: &dyn HttpProbe, examples: &Map<String, Value>) -> ConformanceReport {
    let mut ops: Vec<&ApiOperation> = contract.operations.iter().collect();
    ops.sort_by_key(|op| (rank(op), op.file.clone(), op.line));
    let mut learned: HashMap<String, Value> = HashMap::new();
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
                if resp.status == op.success {
                    if let Ok(v) = serde_json::from_str::<Value>(&resp.body) {
                        learn(&v, &mut learned);
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

/// Remember every scalar field of a success answer by its wire name, from
/// an object or the first element of a list.
fn learn(v: &Value, learned: &mut HashMap<String, Value>) {
    let object = match v {
        Value::Array(items) => items.first(),
        other => Some(other),
    };
    if let Some(Value::Object(m)) = object {
        for (k, val) in m {
            if matches!(val, Value::String(_) | Value::Number(_) | Value::Bool(_)) {
                learned.insert(k.clone(), val.clone());
            }
        }
    }
}

fn request_for(
    op: &ApiOperation,
    contract: &ApiContract,
    examples: &Map<String, Value>,
    learned: &HashMap<String, Value>,
) -> Result<ProbeRequest, String> {
    let mut path = op.path.clone();
    let mut query: Vec<String> = Vec::new();
    for p in &op.params {
        let given = examples.get(&p.name).or_else(|| learned.get(&p.name)).cloned();
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
        // An empty list fits any item schema, so it checked nothing the
        // contract says about the items: valid, and proof of nothing.
        if matches!(t, TypeRef::Array(_)) && v.as_array().is_some_and(Vec::is_empty) {
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
        let op = ApiOperation {
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
            response: Some(TypeRef::Array(Box::new(TypeRef::Named("B".into())))),
            errors: vec![500],
            file: "f".into(),
            line: 1,
            args: vec![],
            is_async: false,
            fails: true,
            error_type: None,
            error_variants: vec![],
        };
        assert_eq!(judge(&op, &c, 200, "[]").0, Verdict::Unproven);
        assert_eq!(judge(&op, &c, 200, r#"[{"id":"x","tags":[]}]"#).0, Verdict::Proven);
        assert_eq!(judge(&op, &c, 200, r#"[{"tags":[]}]"#).0, Verdict::Violation);
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
}
