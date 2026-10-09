//! The contract as an OpenAPI 3.1 document (ADR-2610092245 §5).
//!
//! A pure function of the contract. Every language reaches it through the
//! same [`ApiContract`], so a port written in Rust, Go and TypeScript renders
//! one document — which is the cross-language claim, made testable.

use serde_json::{json, Map, Value};

use super::domain::{ApiBody, ApiContract, ApiOperation, ApiSchema, ParamLocation, TypeRef};

/// The document for `contract`. `generator` goes in `info.x-generated-by`,
/// which comparisons leave out: it names what wrote the file, not what the
/// file says.
pub fn render(contract: &ApiContract, generator: &str) -> Value {
    let mut paths: Map<String, Value> = Map::new();
    for op in &contract.operations {
        let entry = paths.entry(op.path.clone()).or_insert_with(|| json!({}));
        entry[op.http_method.to_lowercase()] = operation(op);
    }
    let schemas: Map<String, Value> = contract.schemas.iter().map(|(name, s)| (name.clone(), schema_of(s))).collect();
    json!({
        "openapi": "3.1.0",
        "info": {
            "title": contract.title,
            "version": contract.version,
            "x-generated-by": generator,
        },
        "paths": paths,
        "components": { "schemas": schemas },
    })
}

/// The document with its generator stamp removed: what it says, for comparing.
pub fn content_of(mut doc: Value) -> Value {
    if let Some(info) = doc.get_mut("info").and_then(Value::as_object_mut) {
        info.remove("x-generated-by");
    }
    doc
}

fn operation(op: &ApiOperation) -> Value {
    let mut o = json!({
        "operationId": op.operation_id,
        "tags": [op.service],
    });
    if !op.description.is_empty() {
        o["description"] = json!(op.description);
    }
    if !op.params.is_empty() {
        o["parameters"] = Value::Array(
            op.params
                .iter()
                .map(|p| {
                    json!({
                        "name": p.name,
                        "in": match p.location { ParamLocation::Path => "path", ParamLocation::Query => "query" },
                        "required": p.required,
                        "schema": type_schema(&p.ty),
                    })
                })
                .collect(),
        );
    }
    if let Some(body) = &op.body {
        let (schema, required) = match body {
            ApiBody::Whole(TypeRef::Optional(t)) => (type_schema(t), false),
            ApiBody::Whole(t) => (type_schema(t), true),
            ApiBody::Fields(fields) => (object(fields), true),
        };
        o["requestBody"] = json!({
            "required": required,
            "content": { "application/json": { "schema": schema } },
        });
    }
    let mut responses = Map::new();
    let mut ok = json!({ "description": reason(op.success) });
    if let Some(t) = &op.response {
        ok["content"] = json!({ "application/json": { "schema": type_schema(t) } });
    }
    responses.insert(op.success.to_string(), ok);
    for code in &op.errors {
        responses.insert(code.to_string(), json!({ "description": reason(*code) }));
    }
    o["responses"] = Value::Object(responses);
    o
}

fn schema_of(s: &ApiSchema) -> Value {
    match s {
        ApiSchema::Object(fields) => object(fields),
        ApiSchema::StringEnum(names) => json!({ "type": "string", "enum": names }),
    }
}

fn object(fields: &[(String, TypeRef, bool)]) -> Value {
    let properties: Map<String, Value> = fields.iter().map(|(n, t, _)| (n.clone(), type_schema(t))).collect();
    let required: Vec<&str> = fields.iter().filter(|(_, _, r)| *r).map(|(n, _, _)| n.as_str()).collect();
    let mut o = json!({ "type": "object", "properties": properties });
    if !required.is_empty() {
        o["required"] = json!(required);
    }
    o
}

fn type_schema(t: &TypeRef) -> Value {
    match t {
        TypeRef::String => json!({ "type": "string" }),
        TypeRef::Integer => json!({ "type": "integer" }),
        TypeRef::Number => json!({ "type": "number" }),
        TypeRef::Boolean => json!({ "type": "boolean" }),
        TypeRef::Array(e) => json!({ "type": "array", "items": type_schema(e) }),
        TypeRef::Map(v) => json!({ "type": "object", "additionalProperties": type_schema(v) }),
        TypeRef::Optional(inner) => type_schema(inner),
        TypeRef::Named(n) => json!({ "$ref": format!("#/components/schemas/{n}") }),
        // The builder resolves these away or refuses the contract; a `{}`
        // here would accept anything, so neither ever reaches the document.
        TypeRef::Unit | TypeRef::Unsupported(_) => json!({ "not": {} }),
    }
}

fn reason(code: u16) -> &'static str {
    match code {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        410 => "Gone",
        412 => "Precondition Failed",
        415 => "Unsupported Media Type",
        422 => "Unprocessable Content",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        c if c < 300 => "Success",
        c if c < 500 => "Client Error",
        _ => "Server Error",
    }
}
