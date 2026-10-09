//! The API contract, built from what every file says (ADR-2610092245).
//!
//! The parser hands over, per file, the tagged ports and the declared types
//! with their signatures already lowered out of the language. This resolves
//! names across files, applies the parameter-binding rules, and collects
//! every problem with the file and line it is written at. It never guesses:
//! a type it cannot reach is an error, not an empty schema, because a schema
//! that accepts anything is a gate that degraded without saying so.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

use super::domain::{
    lower_camel, ApiArg, ApiBody, ArgSource, ApiContract, ApiDiagnostic, ApiFacts, ApiFindings, ApiMethodDecl, ApiOperation,
    ApiParam, ApiPortDecl, ApiSchema, ErrorChannel, HexLayer, Language, ParamLocation, TypeBody, TypeDecl,
    TypeRef,
};
use super::layer_classifier::LayerMap;
use super::ports::{AnalysisError, AstPort};

/// What one file says, and the layer it is in.
#[derive(Debug, Clone)]
pub struct FileApi {
    pub file: String,
    pub layer: HexLayer,
    pub facts: ApiFacts,
}

/// The contract, and everything wrong with it. A contract with errors is not
/// written anywhere.
#[derive(Debug, Clone, Default)]
pub struct ApiBuild {
    pub contract: ApiContract,
    pub errors: Vec<ApiDiagnostic>,
    /// The tagged ports that made it into the contract.
    pub ports: Vec<String>,
}

const BODY_METHODS: &[&str] = &["POST", "PUT", "PATCH"];
const QUERY_METHODS: &[&str] = &["GET", "DELETE", "HEAD"];

/// Read `files` (project-relative, the set the grade reads) for what each
/// says about the API.
fn read_project(root: &Path, files: &[String], ast: &dyn AstPort) -> Result<Vec<FileApi>, AnalysisError> {
    let layers = LayerMap::from_project(root).map_err(AnalysisError::Other)?;
    let mut out = Vec::new();
    for rel in files {
        let rel = rel.clone();
        let lang = Language::from_path(&rel);
        let Ok(source) = std::fs::read_to_string(root.join(&rel)) else { continue };
        let facts = ast.extract_api(Path::new(&rel), &source, lang)?;
        if facts == ApiFacts::default() {
            continue;
        }
        out.push(FileApi { layer: layers.classify(&rel), file: rel, facts });
    }
    Ok(out)
}

/// The contract for the project at `root`, over `files`.
pub fn from_project(root: &Path, files: &[String], ast: &dyn AstPort) -> Result<ApiBuild, AnalysisError> {
    Ok(build(&read_project(root, files, ast)?))
}

/// What `hexa analyze` reports: the contract's errors, its size, and the
/// tagged ports that no primary adapter names (declared, not served).
pub fn findings(root: &Path, files: &[String], ast: &dyn AstPort) -> Result<ApiFindings, AnalysisError> {
    let built = from_project(root, files, ast)?;
    if built.ports.is_empty() {
        return Ok(ApiFindings { operations: 0, errors: built.errors, unserved: Vec::new() });
    }
    let layers = LayerMap::from_project(root).map_err(AnalysisError::Other)?;
    let mut named: HashSet<String> = HashSet::new();
    for rel in files {
        if layers.classify(rel) != HexLayer::AdaptersPrimary {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(root.join(rel)) else { continue };
        let refs = ast.extract_references(Path::new(rel), &source, Language::from_path(rel))?;
        named.extend(refs.into_keys());
    }
    let mut unserved: Vec<String> = built.ports.iter().filter(|p| !named.contains(*p)).cloned().collect();
    unserved.sort();
    unserved.dedup();
    Ok(ApiFindings { operations: built.contract.operations.len(), errors: built.errors, unserved })
}

/// Build the contract from what the files say.
pub fn build(files: &[FileApi]) -> ApiBuild {
    let mut types: HashMap<&str, Vec<(&str, &TypeDecl)>> = HashMap::new();
    for f in files {
        for t in &f.facts.types {
            types.entry(t.name.as_str()).or_default().push((f.file.as_str(), t));
        }
    }
    let mut b = Builder { types, schemas: BTreeMap::new(), errors: Vec::new() };
    let mut operations: Vec<ApiOperation> = Vec::new();
    let mut ports = Vec::new();
    let mut services: BTreeSet<String> = BTreeSet::new();
    let mut version: Option<String> = None;

    for f in files {
        for &line in &f.facts.stray_tags {
            b.error(
                &f.file,
                line,
                "`@hexa:api` tags a driving port (a trait or interface in ports/) or one of its methods, and nothing else"
                    .into(),
            );
        }
        for port in &f.facts.ports {
            let Some(header) = b.port_header(f, port) else { continue };
            services.insert(header.service.clone());
            if version.is_none() {
                version = header.version.clone();
            }
            ports.push(port.name.clone());
            for m in port.methods.iter().filter(|m| m.tag.is_some()) {
                if let Some(op) = b.operation(f, port, &header, m) {
                    operations.push(op);
                }
            }
        }
    }

    let mut seen_routes: HashMap<(String, String), (String, usize)> = HashMap::new();
    let mut seen_ids: HashMap<String, (String, usize)> = HashMap::new();
    for op in &operations {
        let route = (op.http_method.clone(), op.path.clone());
        if let Some((file, line)) = seen_routes.get(&route) {
            b.error(&op.file, op.line, format!("`{} {}` is already declared at {file}:{line}", op.http_method, op.path));
        } else {
            seen_routes.insert(route, (op.file.clone(), op.line));
        }
        if let Some((file, line)) = seen_ids.get(&op.operation_id) {
            b.error(&op.file, op.line, format!("operation id `{}` is already used at {file}:{line}", op.operation_id));
        } else {
            seen_ids.insert(op.operation_id.clone(), (op.file.clone(), op.line));
        }
    }

    operations.sort_by(|a, c| (&a.file, a.line).cmp(&(&c.file, c.line)));
    let mut errors = b.errors;
    errors.sort();
    errors.dedup();
    ApiBuild {
        contract: ApiContract {
            title: services.into_iter().collect::<Vec<_>>().join(", "),
            version: version.unwrap_or_else(|| "0.0.0".into()),
            operations,
            schemas: b.schemas,
        },
        errors,
        ports,
    }
}

struct PortHeader {
    service: String,
    version: Option<String>,
    statuses: BTreeSet<u16>,
}

struct Builder<'a> {
    types: HashMap<&'a str, Vec<(&'a str, &'a TypeDecl)>>,
    schemas: BTreeMap<String, ApiSchema>,
    errors: Vec<ApiDiagnostic>,
}

impl<'a> Builder<'a> {
    fn error(&mut self, file: &str, line: usize, message: String) {
        self.errors.push(ApiDiagnostic { file: file.to_string(), line, message });
    }

    fn port_header(&mut self, f: &FileApi, port: &ApiPortDecl) -> Option<PortHeader> {
        if f.layer != HexLayer::Ports {
            self.error(
                &f.file,
                port.line,
                format!(
                    "`{}` carries `@hexa:api` but is in {}; the API contract is a driving port in ports/",
                    port.name, f.layer
                ),
            );
            return None;
        }
        let Some(tag) = &port.tag else {
            self.error(
                &f.file,
                port.line,
                format!("methods of `{0}` are tagged but `{0}` is not; tag the interface with `@hexa:api`", port.name),
            );
            return None;
        };
        let mut header = PortHeader { service: port.name.clone(), version: None, statuses: BTreeSet::new() };
        for token in tag.split_whitespace() {
            match token.split_once('=') {
                Some(("service", v)) if !v.is_empty() => header.service = v.to_string(),
                Some(("version", v)) if !v.is_empty() => header.version = Some(v.to_string()),
                _ => self.error(
                    &f.file,
                    port.line,
                    format!("`{token}` in `@hexa:api {tag}`: a port takes `service=<name>` and `version=<v>`"),
                ),
            }
        }
        for s in &port.statuses {
            if let Some(code) = self.status(&f.file, port.line, s) {
                header.statuses.insert(code);
            }
        }
        Some(header)
    }

    fn status(&mut self, file: &str, line: usize, text: &str) -> Option<u16> {
        match text.split_whitespace().next().and_then(|t| t.parse::<u16>().ok()) {
            Some(code) if (400..=599).contains(&code) => Some(code),
            _ => {
                self.error(file, line, format!("`@hexa:status {text}`: an error status is a number from 400 to 599"));
                None
            }
        }
    }

    fn operation(
        &mut self,
        f: &FileApi,
        port: &ApiPortDecl,
        header: &PortHeader,
        m: &ApiMethodDecl,
    ) -> Option<ApiOperation> {
        let file = f.file.as_str();
        let tag = m.tag.as_deref().unwrap_or("");
        let (http_method, path, success) = match parse_operation_tag(tag) {
            Ok(t) => t,
            Err(why) => {
                self.error(file, m.line, format!("`@hexa:api {tag}`: {why}"));
                return None;
            }
        };
        let route = format!("{http_method} {path}");
        let mut ok = true;

        // Path parameters, in path order.
        let mut remaining: Vec<&(String, TypeRef)> = m.params.iter().collect();
        let mut params = Vec::new();
        // Where each argument's value comes from, by its name in the signature.
        let mut sources: HashMap<String, (ArgSource, TypeRef, bool)> = HashMap::new();
        for seg in path_segments(&path) {
            let found = remaining
                .iter()
                .position(|(n, _)| n == &seg || lower_camel(n) == lower_camel(&seg));
            let Some(i) = found else {
                self.error(file, m.line, format!("`{route}`: `{{{seg}}}` names no parameter of `{}`", m.name));
                ok = false;
                continue;
            };
            let (name, ty) = remaining.remove(i);
            match self.resolve(ty, file, m.line) {
                Some(t) if is_scalar(&t) => {
                    sources.insert(name.clone(), (ArgSource::Path(seg.clone()), t.clone(), true));
                    params.push(ApiParam { name: seg.clone(), location: ParamLocation::Path, ty: t, required: true })
                }
                Some(_) => {
                    self.error(file, m.line, format!("`{route}`: path parameter `{name}` must be a scalar"));
                    ok = false;
                }
                None => ok = false,
            }
        }

        for (name, _) in &remaining {
            if name.is_empty() {
                self.error(file, m.line, format!("`{route}`: every parameter of `{}` needs a name", m.name));
                return None;
            }
        }

        // What is left is the query or the body.
        let mut body = None;
        if QUERY_METHODS.contains(&http_method.as_str()) {
            for (name, ty) in remaining {
                let Some(t) = self.resolve(ty, file, m.line) else {
                    ok = false;
                    continue;
                };
                let (inner, required) = unwrap_optional(t);
                let queryable = is_scalar(&inner) || matches!(&inner, TypeRef::Array(e) if is_scalar(e));
                if !queryable {
                    self.error(
                        file,
                        m.line,
                        format!("`{route}`: a {http_method} cannot take a body, and `{name}` is not a scalar or a list of scalars"),
                    );
                    ok = false;
                    continue;
                }
                sources.insert(name.clone(), (ArgSource::Query(lower_camel(name)), inner.clone(), required));
                params.push(ApiParam { name: lower_camel(name), location: ParamLocation::Query, ty: inner, required });
            }
        } else {
            let mut fields = Vec::new();
            let mut names = Vec::new();
            for (name, ty) in &remaining {
                match self.resolve(ty, file, m.line) {
                    Some(t) => {
                        fields.push((lower_camel(name), t));
                        names.push(name.clone());
                    }
                    None => ok = false,
                }
            }
            let whole = matches!(fields.as_slice(), [(_, t)] if matches!(unwrap_optional(t.clone()).0, TypeRef::Named(_)));
            for (name, (wire, t)) in names.iter().zip(&fields) {
                let (inner, required) = unwrap_optional(t.clone());
                let source = if whole { ArgSource::Body } else { ArgSource::BodyField(wire.clone()) };
                sources.insert(name.clone(), (source, inner, required));
            }
            body = match fields.as_slice() {
                [] => None,
                [(_, t)] if whole => Some(ApiBody::Whole(t.clone())),
                _ => Some(ApiBody::Fields(
                    fields
                        .into_iter()
                        .map(|(n, t)| {
                            let (inner, required) = unwrap_optional(t);
                            (n, inner, required)
                        })
                        .collect(),
                )),
            };
        }

        let response = match self.resolve(&m.returns, file, m.line) {
            Some(TypeRef::Unit) => None,
            Some(t) => Some(t),
            None => {
                ok = false;
                None
            }
        };
        let success = success.unwrap_or(if response.is_none() { 204 } else { 200 });

        let mut errors = header.statuses.clone();
        for s in &m.statuses {
            if let Some(code) = self.status(file, m.line, s) {
                errors.insert(code);
            }
        }
        let mut error_type = None;
        let mut error_variants = Vec::new();
        if let ErrorChannel::Typed(TypeRef::Named(e)) = &m.error {
            let variants = self.variant_statuses(e);
            if !variants.is_empty() {
                error_type = Some(e.clone());
            }
            for (vfile, line, ident, text) in variants {
                let code = match text {
                    Some(t) => self.status(&vfile, line, &t).unwrap_or(500),
                    None => 500,
                };
                errors.insert(code);
                error_variants.push((ident, code));
            }
        }
        if m.error != ErrorChannel::None {
            errors.insert(500);
        }

        ok.then(|| ApiOperation {
            service: header.service.clone(),
            port: port.name.clone(),
            method_name: m.name.clone(),
            operation_id: lower_camel(&m.name),
            http_method,
            path,
            description: m.doc.clone(),
            params,
            body,
            success,
            response,
            errors: errors.into_iter().collect(),
            file: file.to_string(),
            line: m.line,
            args: m
                .written
                .iter()
                .map(|w| {
                    let (source, ty, required) = if w.context {
                        (ArgSource::Context, TypeRef::Unit, true)
                    } else {
                        sources.get(&w.name).cloned().unwrap_or((ArgSource::Body, TypeRef::Unit, true))
                    };
                    ApiArg { name: w.name.clone(), written: w.written.clone(), source, ty, required }
                })
                .collect(),
            is_async: m.is_async,
            fails: m.error != ErrorChannel::None,
            error_type,
            error_variants,
        })
    }

    /// The declaration a name means: a real declaration before another name
    /// for one, so `type Bookmark = domain.Bookmark` resolves to the struct.
    fn lookup(&self, name: &str) -> Option<(&'a str, &'a TypeDecl)> {
        let found = self.types.get(name)?;
        let real = found.iter().find(|(_, d)| !matches!(d.body, TypeBody::Alias(_)));
        let alias = found
            .iter()
            .find(|(_, d)| !matches!(&d.body, TypeBody::Alias(TypeRef::Named(t)) if t == name));
        real.or(alias).copied()
    }

    /// Each variant of an error enum: where the enum is, the variant's
    /// identifier, and its `@hexa:status` text if it has one.
    fn variant_statuses(&self, name: &str) -> Vec<(String, usize, String, Option<String>)> {
        let mut name = name.to_string();
        for _ in 0..8 {
            match self.lookup(&name) {
                Some((_, TypeDecl { body: TypeBody::Alias(TypeRef::Named(t)), .. })) => name = t.clone(),
                Some((file, TypeDecl { body: TypeBody::Enum(vs), line, .. })) => {
                    return vs.iter().map(|v| (file.to_string(), *line, v.ident.clone(), v.status.clone())).collect()
                }
                _ => break,
            }
        }
        Vec::new()
    }

    /// Resolve a type to one the renderer can write: scalars, sequences,
    /// maps, optionals, and `Named` for a schema now in `self.schemas`.
    /// Errors name `file:line`, the place the type is used.
    fn resolve(&mut self, ty: &TypeRef, file: &str, line: usize) -> Option<TypeRef> {
        self.resolve_depth(ty, file, line, 0)
    }

    fn resolve_depth(&mut self, ty: &TypeRef, file: &str, line: usize, depth: usize) -> Option<TypeRef> {
        if depth > 32 {
            self.error(file, line, "type aliases form a cycle".into());
            return None;
        }
        let wrap = |b: fn(Box<TypeRef>) -> TypeRef, t: Option<TypeRef>| t.map(|t| b(Box::new(t)));
        match ty {
            TypeRef::String | TypeRef::Integer | TypeRef::Number | TypeRef::Boolean | TypeRef::Unit => Some(ty.clone()),
            TypeRef::Array(t) => wrap(TypeRef::Array, self.resolve_depth(t, file, line, depth + 1)),
            TypeRef::Optional(t) => wrap(TypeRef::Optional, self.resolve_depth(t, file, line, depth + 1)),
            TypeRef::Map(t) => wrap(TypeRef::Map, self.resolve_depth(t, file, line, depth + 1)),
            TypeRef::Unsupported(text) => {
                self.error(
                    file,
                    line,
                    format!("`{text}` has no schema hexa can derive; use a declared type, a scalar, a list, a map or an optional"),
                );
                None
            }
            TypeRef::Named(name) => {
                let Some((decl_file, decl)) = self.lookup(name) else {
                    self.error(file, line, format!("`{name}` is not declared in this project, so it has no schema"));
                    return None;
                };
                match &decl.body {
                    TypeBody::Alias(t) | TypeBody::Newtype(t) => self.resolve_depth(t, decl_file, decl.line, depth + 1),
                    TypeBody::Struct(fields) => {
                        if self.schemas.contains_key(&decl.name) {
                            return Some(TypeRef::Named(decl.name.clone()));
                        }
                        // Claimed before its fields, so a type that contains
                        // itself refers to itself instead of recursing.
                        self.schemas.insert(decl.name.clone(), ApiSchema::Object(Vec::new()));
                        let mut props = Vec::new();
                        let mut ok = true;
                        for field in fields {
                            match self.resolve_depth(&field.ty, decl_file, field.line, depth + 1) {
                                Some(t) => {
                                    let (inner, required) = unwrap_optional(t);
                                    props.push((field.wire_name.clone(), inner, required && !field.optional));
                                }
                                None => ok = false,
                            }
                        }
                        if !ok {
                            self.schemas.remove(&decl.name);
                            return None;
                        }
                        self.schemas.insert(decl.name.clone(), ApiSchema::Object(props));
                        Some(TypeRef::Named(decl.name.clone()))
                    }
                    TypeBody::Enum(variants) => {
                        if variants.iter().any(|v| !v.unit) {
                            self.error(
                                decl_file,
                                decl.line,
                                format!("enum `{}` carries data, which has no schema hexa can derive", decl.name),
                            );
                            return None;
                        }
                        let names = variants.iter().map(|v| v.name.clone()).collect();
                        self.schemas.insert(decl.name.clone(), ApiSchema::StringEnum(names));
                        Some(TypeRef::Named(decl.name.clone()))
                    }
                }
            }
        }
    }
}

fn is_scalar(t: &TypeRef) -> bool {
    matches!(t, TypeRef::String | TypeRef::Integer | TypeRef::Number | TypeRef::Boolean)
}

/// (`T`, required) for `T`, (`T`, not required) for an optional `T`.
fn unwrap_optional(t: TypeRef) -> (TypeRef, bool) {
    match t {
        TypeRef::Optional(inner) => (*inner, false),
        other => (other, true),
    }
}

/// `{id}` segments of a path, in order.
fn path_segments(path: &str) -> Vec<String> {
    path.split('/')
        .filter_map(|s| s.strip_prefix('{').and_then(|s| s.strip_suffix('}')))
        .map(str::to_string)
        .collect()
}

/// `<METHOD> <path> [<status>]`.
fn parse_operation_tag(tag: &str) -> Result<(String, String, Option<u16>), String> {
    let mut parts = tag.split_whitespace();
    let method = parts.next().ok_or("an operation is `@hexa:api <METHOD> <path> [<status>]`")?.to_uppercase();
    if !BODY_METHODS.contains(&method.as_str()) && !QUERY_METHODS.contains(&method.as_str()) {
        return Err(format!("`{method}` is not one of GET, POST, PUT, PATCH, DELETE, HEAD"));
    }
    let path = parts.next().ok_or("the operation has no path")?.to_string();
    if !path.starts_with('/') {
        return Err(format!("the path `{path}` must begin with `/`"));
    }
    for seg in path.split('/') {
        let opens = seg.matches('{').count();
        let closes = seg.matches('}').count();
        let whole = seg.starts_with('{') && seg.ends_with('}') && seg.len() > 2;
        if opens + closes > 0 && !(opens == 1 && closes == 1 && whole) {
            return Err(format!("`{seg}`: a path parameter is a whole segment, `{{name}}`"));
        }
    }
    let status = match parts.next() {
        None => None,
        Some(s) => match s.parse::<u16>() {
            Ok(code) if (200..=299).contains(&code) => Some(code),
            _ => return Err(format!("`{s}`: the success status is a number from 200 to 299")),
        },
    };
    if let Some(extra) = parts.next() {
        return Err(format!("unexpected `{extra}`"));
    }
    Ok((method, path, status))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_operation_tag_is_method_path_and_status() {
        assert_eq!(parse_operation_tag("post /a/{id} 201"), Ok(("POST".into(), "/a/{id}".into(), Some(201))));
        assert_eq!(parse_operation_tag("GET /a"), Ok(("GET".into(), "/a".into(), None)));
        assert!(parse_operation_tag("FETCH /a").is_err());
        assert!(parse_operation_tag("GET a").is_err());
        assert!(parse_operation_tag("GET /a/{id").is_err());
        assert!(parse_operation_tag("GET /a/x{id}").is_err());
        assert!(parse_operation_tag("GET /a 404").is_err());
        assert!(parse_operation_tag("GET /a 200 x").is_err());
        assert!(parse_operation_tag("").is_err());
    }

    #[test]
    fn segments_are_read_in_order() {
        assert_eq!(path_segments("/a/{x}/b/{y}"), vec!["x", "y"]);
        assert!(path_segments("/a").is_empty());
    }
}
