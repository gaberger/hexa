//! What hexa-exec's agent loop needs from the outside, as contracts.
//!
//! - [`Tool`] — one capability the model can call. Each tool in `crate::tools`
//!   is an adapter behind it.
//! - [`Worktrees`] — the git worktrees an isolated run works in.
//!
//! The loop holds these, never a concrete tool or git itself; which ones it gets
//! is wiring, at the crate root (`default_tools`, `default_deps`).

use std::path::{Path, PathBuf};

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

/// Which store a memory call reads or writes.
///
/// `Project` is the default everywhere. `Shared` is the old per-user file, kept
/// for the entries that really are cross-project — model calibration, a lesson
/// about the machine — and for reaching what a pre-scoping install already
/// wrote. Nothing falls back from one to the other: an implicit fallback is the
/// cross-project bleed this split exists to stop.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MemoryScope {
    #[default]
    Project,
    Shared,
}

/// Where lessons and keyed notes are kept, by scope (ADR-2609211200).
pub trait MemoryStore: Send + Sync {
    /// The file a scope's entries live in.
    fn path(&self, scope: MemoryScope) -> PathBuf;
    fn put(&self, scope: MemoryScope, key: &str, value: &str) -> Result<(), String>;
    fn get(&self, scope: MemoryScope, key: &str) -> Option<String>;
    /// The most recent `limit` entries.
    fn entries(&self, scope: MemoryScope, limit: usize) -> Vec<(String, String)>;
    fn search(&self, scope: MemoryScope, query: &str) -> Vec<(String, String)>;
    /// Whether the key was there to delete.
    fn delete(&self, scope: MemoryScope, key: &str) -> Result<bool, String>;
}
