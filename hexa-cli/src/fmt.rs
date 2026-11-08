//! Shared CLI table formatting (ADR-2026-03-24-1226).
//!
//! ONE function for all hexa CLI table output:
//!   `pretty_table(&["Col1", "Col2"], &[vec!["a", "b"], vec!["c", "d"]])`
//!
//! Plus helpers: status_badge, score_badge, truncate, progress, telemetry_badge.

use colored::Colorize;
use std::collections::HashMap;
use tabled::settings::Style;
use tabled::{Table, Tabled};

// ── pretty_table — the ONE function ─────────────────────────────────────

// ── HexTable — derive-based wrapper ─────────────────────────────────────

/// For commands that use `#[derive(Tabled)]` structs.
/// Wraps `tabled::Table` with consistent hexa styling.
pub struct HexTable;

impl HexTable {
    /// Rounded-border table from Tabled-derived rows.
    pub fn render<T: Tabled>(rows: &[T]) -> String {
        if rows.is_empty() {
            return "  (no results)".dimmed().to_string();
        }
        Table::new(rows).with(Style::rounded()).to_string()
    }

    /// Borderless table from Tabled-derived rows.
    pub fn compact<T: Tabled>(rows: &[T]) -> String {
        if rows.is_empty() {
            return String::new();
        }
        Table::new(rows).with(Style::blank()).to_string()
    }
}

// ── Status Badges ───────────────────────────────────────────────────────

/// Colored status badge for ADR/task/plan status.
pub fn status_badge(status: &str) -> String {
    match status.to_lowercase().as_str() {
        "accepted" | "done" | "completed" | "pass" | "passed" => {
            status.green().bold().to_string()
        }
        "proposed" | "pending" | "planned" => status.yellow().to_string(),
        "in_progress" | "active" | "running" => status.cyan().bold().to_string(),
        "deprecated" | "superseded" | "abandoned" | "stale" => status.red().to_string(),
        "fail" | "failed" | "error" => status.red().bold().to_string(),
        _ => status.to_string(),
    }
}

// ── Telemetry (ADR-2611050000) ──────────────────────────────────────────

/// How one telemetry key is shown: `{icon} {label}: {value}`.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct TelemetryEntry {
    pub icon: String,
    pub label: String,
}

/// The telemetry asset, keyed by telemetry key.
#[derive(Debug, Default)]
pub struct TelemetryAsset {
    entries: HashMap<String, TelemetryEntry>,
}

impl TelemetryAsset {
    /// Parse the text of `telemetry.json`. Never fails: malformed text gives an
    /// empty asset and a warning on stderr, so the binary starts regardless.
    pub fn parse(_text: &str) -> TelemetryAsset {
        unimplemented!("ADR-2611050000 §1")
    }

    pub fn get(&self, _key: &str) -> Option<&TelemetryEntry> {
        unimplemented!("ADR-2611050000 §1")
    }
}

/// One `{icon} {label}: {value}` per key the asset knows, in key order, joined
/// by a single space. A key the asset does not know is left out. Plain text:
/// the caller styles it. Empty when nothing is shown.
pub fn telemetry_badge(_telemetry: &HashMap<String, String>, _asset: &TelemetryAsset) -> String {
    unimplemented!("ADR-2611050000 §2")
}

/// `line`, then a space and `badge`; `line` unchanged when `badge` is empty, so
/// a quiet loop gains no trailing whitespace.
pub fn append_telemetry(_line: &str, _badge: &str) -> String {
    unimplemented!("ADR-2611050000 §3")
}

// ── Text Helpers ────────────────────────────────────────────────────────

/// Truncate a string to `max_len` characters, appending "…" if truncated.
pub fn truncate(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else if max_len <= 1 {
        "…".to_string()
    } else {
        let truncated: String = s.chars().take(max_len - 1).collect();
        format!("{}…", truncated)
    }
}

/// Format a count as "N/M" with coloring based on completion.
pub fn progress(done: u32, total: u32) -> String {
    let text = format!("{}/{}", done, total);
    if done >= total {
        text.green().bold().to_string()
    } else if done > 0 {
        text.yellow().to_string()
    } else {
        text.dimmed().to_string()
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_works() {
        assert_eq!(truncate("hello world", 5), "hell…");
        assert_eq!(truncate("hi", 5), "hi");
        assert_eq!(truncate("hello", 5), "hello");
    }

    #[test]
    fn truncate_utf8_safe() {
        // Should not panic on multi-byte chars
        let s = "hello — world";
        let t = truncate(s, 8);
        assert_eq!(t.chars().count(), 8);
    }

    #[test]
    fn status_badges_colored() {
        assert!(!status_badge("accepted").is_empty());
        assert!(!status_badge("proposed").is_empty());
        assert!(!status_badge("deprecated").is_empty());
        assert!(!status_badge("unknown").is_empty());
    }

    #[test]
    fn progress_formatting() {
        let p = progress(3, 5);
        assert!(p.contains("3/5"));
    }
}
