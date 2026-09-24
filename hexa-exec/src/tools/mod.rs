//! Typed tool library (ADR-2026-05-08-2500).
//!
//! Provides typed primitives the LLM can compose deterministically via
//! Anthropic function-calling. Each tool wraps an existing hexa capability
//! (cargo, ripgrep, ADR write, inbox) behind a typed schema. The
//! `ToolRegistry` exports the schema set the inference path attaches to
//! Phase 3 REASON calls.
//!
//! Add a new tool: implement `Tool` (a port, `crate::ports`) + register it in
//! `hexa_exec::default_tools()` at the crate root.


pub mod adr_draft;
pub mod adr_status_set;
pub mod cargo_check;
pub mod code_patch;
pub mod cost_meter;
pub mod dep_audit;
pub mod escalate_to_operator;
pub mod memory_search;
pub mod repo_grep;
pub mod repo_read;
pub mod secret_scan;
pub mod spec_draft;
pub mod typescript_check;
pub mod web_search;
pub mod workplan_emit;
pub mod workspace_boundary_check;

pub use crate::ports::{Tool, ToolResult};

#[cfg(test)]
mod tests {
    #[test]
    fn registry_has_first_wave() {
        let r = crate::default_tools();
        let names = r.names();
        assert!(names.contains(&"cargo_check"), "cargo_check missing");
        assert!(names.contains(&"repo_grep"), "repo_grep missing");
        assert!(names.contains(&"repo_read"), "repo_read missing");
        assert!(names.contains(&"web_search"), "web_search missing");
        assert!(names.contains(&"adr_draft"), "adr_draft missing");
        assert!(names.contains(&"spec_draft"), "spec_draft missing");
        assert!(names.contains(&"escalate_to_operator"), "escalate_to_operator missing");
        assert!(names.contains(&"memory_search"), "memory_search missing");
    }

    #[test]
    fn anthropic_schema_shape() {
        let r = crate::default_tools();
        let s = r.anthropic_schema();
        let arr = s.as_array().expect("must be array");
        assert!(!arr.is_empty(), "schema array empty");
        for entry in arr {
            assert!(entry.get("name").is_some(), "missing name");
            assert!(entry.get("description").is_some(), "missing description");
            let schema = entry.get("input_schema").expect("missing input_schema");
            assert_eq!(
                schema.get("type").and_then(|v| v.as_str()),
                Some("object"),
                "input_schema must be object type"
            );
        }
    }
}
