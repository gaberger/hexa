//! What hexa-exec's agent loop needs from the outside, as contracts.
//!
//! - [`Tool`] — one capability the model can call. Each tool in `crate::tools`
//!   is an adapter behind it.
//! - [`Worktrees`] — the git worktrees an isolated run works in.
//!
//! The loop holds these, never a concrete tool or git itself; which ones it gets
//! is wiring, at the crate root (`default_tools`, `default_deps`).

use std::path::Path;

use async_trait::async_trait;
use serde_json::Value;

/// Output envelope for every tool call. JSON shape preserved across all
/// tools so the SOP executor can handle errors uniformly without per-tool
/// downcasting.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolResult {
    pub ok: bool,
    pub output: Value,
    pub error: Option<String>,
    pub elapsed_ms: u64,
    /// True when output was truncated due to size cap.
    pub truncated: bool,
}

impl ToolResult {
    pub fn ok(output: Value, elapsed_ms: u64) -> Self {
        Self { ok: true, output, error: None, elapsed_ms, truncated: false }
    }
    pub fn ok_truncated(output: Value, elapsed_ms: u64) -> Self {
        Self { ok: true, output, error: None, elapsed_ms, truncated: true }
    }
    pub fn err(error: impl Into<String>, elapsed_ms: u64) -> Self {
        Self {
            ok: false,
            output: Value::Null,
            error: Some(error.into()),
            elapsed_ms,
            truncated: false,
        }
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    /// JSON schema for `input_schema` field of an Anthropic function-calling tool.
    /// Must be a JSON object with `type: "object"`, `properties: {...}`, `required: [...]`.
    fn input_schema(&self) -> Value;
    async fn execute(&self, input: Value) -> ToolResult;
}

/// The git worktrees an isolated run is confined to (ADR-2606071323).
pub trait Worktrees: Send + Sync {
    /// Create a worktree at `path` on `branch` (created from HEAD if absent).
    fn create(&self, repo: &Path, branch: &str, path: &Path) -> Result<(), String>;

    /// Remove the worktree at `path` and delete its branch.
    fn remove(&self, repo: &Path, path: &Path) -> Result<(), String>;
}
