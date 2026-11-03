//! The shapes that go over the wire.
//!
//! These live here, not in the domain. JSON field names are a transport
//! detail: a domain type carrying serde attributes means renaming a field
//! silently changes what every client receives.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct CreateRequest {
    pub url: String,
    pub title: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    /// Optional on purpose. A missing tag gets this adapter's own 400, not
    /// axum's rejection, so the client always sees the same error shape.
    pub tag: Option<String>,
    pub limit: Option<u32>,
}
