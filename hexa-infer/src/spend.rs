//! Token and dollar spend, appended locally, and the budget that reads it.
//!
//! Every inference hexa makes lands here as one JSON line in
//! `~/.hexa/inference-log.jsonl`: the model, input and output tokens, the
//! cost in dollars when it is known, and the source (`react`, `harden`,
//! `complete`). Local models cost nothing and record no dollar figure; a
//! `claude -p` call reports its own `total_cost_usd`, which is recorded as
//! given. `hexa spend` reads the file; the frontier path asks `budget_check`
//! before it spends.
//!
//! Best-effort on the write side: a failed append must never fail an
//! inference that succeeded.

use std::fs::{create_dir_all, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// `~/.hexa`, or `$HEXA_HOME`.
pub fn home() -> PathBuf {
    std::env::var("HEXA_HOME").map(PathBuf::from).unwrap_or_else(|_| {
        PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".hexa")
    })
}

fn log_path() -> PathBuf {
    home().join("inference-log.jsonl")
}

/// Append one call's usage with no dollar figure. Local and API completions.
pub fn record(model: &str, input_tokens: u64, output_tokens: u64) {
    record_with(model, input_tokens, output_tokens, None, "complete");
}

/// Append one call's usage. `cost_usd` is recorded only when the provider
/// reported it; a made-up number would be worse than none.
pub fn record_with(model: &str, input_tokens: u64, output_tokens: u64, cost_usd: Option<f64>, source: &str) {
    let path = log_path();
    if let Some(dir) = path.parent() {
        if create_dir_all(dir).is_err() {
            return;
        }
    }
    let mut row = serde_json::json!({
        "model": model,
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
        "source": source,
        "ts": chrono::Utc::now().to_rfc3339(),
    });
    if let Some(c) = cost_usd {
        row["cost_usd"] = serde_json::json!(c);
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = f.write_all(format!("{row}\n").as_bytes());
    }
}

/// The model that served a frontier (`claude -p`) call, read out of its JSON
/// answer.
///
/// The spend log must name a model, not a code path. `claude-code` is the name
/// of the process hexa shells out to; recording it means `hexa spend` can
/// report dollars it cannot attribute to anything that ran, and a spend log
/// that cannot name what served a call cannot support any cost claim. When the
/// answer names nothing, say so — `"unknown (frontier default)"` is an honest
/// gap, a code-path name is a false attribution.
///
/// Resolution order: the top-level `model` field, then the keys of
/// `modelUsage` (sorted, joined with `+`, since one answer may be served by
/// more than one model), then the unknown marker.
pub fn model_from_frontier_json(v: &Value) -> String {
    if let Some(m) = v.get("model").and_then(Value::as_str).filter(|m| !m.is_empty()) {
        return m.to_string();
    }
    if let Some(map) = v.get("modelUsage").and_then(Value::as_object) {
        if !map.is_empty() {
            let mut names: Vec<&str> = map.keys().map(String::as_str).collect();
            names.sort_unstable();
            return names.join("+");
        }
    }
    "unknown (frontier default)".to_string()
}

/// Every recorded call, oldest first.
pub fn entries() -> Vec<Value> {
    entries_in(&log_path())
}

fn entries_in(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .map(|s| s.lines().filter_map(|l| serde_json::from_str(l).ok()).collect())
        .unwrap_or_default()
}

/// `inference.budget_usd_per_day` from `.hexa/project.json`, if declared.
pub fn budget_usd_per_day() -> Option<f64> {
    budget_in(&crate::tiers::project_root())
}

fn budget_in(root: &Path) -> Option<f64> {
    let text = std::fs::read_to_string(root.join(".hexa").join("project.json")).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    v.get("inference")?.get("budget_usd_per_day")?.as_f64().filter(|b| *b > 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spend_report::{since, totals};

    fn row(model: &str, i: u64, o: u64, cost: Option<f64>, ts: &str) -> Value {
        let mut r = serde_json::json!({ "model": model, "input_tokens": i, "output_tokens": o, "source": "t", "ts": ts });
        if let Some(c) = cost {
            r["cost_usd"] = serde_json::json!(c);
        }
        r
    }

    #[test]
    fn totals_sum_tokens_and_only_reported_costs() {
        let rows = vec![
            row("local", 100, 50, None, "2026-09-12T10:00:00+00:00"),
            row("claude-code", 2000, 400, Some(0.031), "2026-09-12T11:00:00+00:00"),
            row("claude-code", 1000, 100, Some(0.010), "2026-09-11T11:00:00+00:00"),
        ];
        let t = totals(&rows);
        assert_eq!((t.calls, t.input_tokens, t.output_tokens, t.priced_calls), (3, 3100, 550, 2));
        assert!((t.cost_usd - 0.041).abs() < 1e-9);
        let today = since(&rows, "2026-09-12T00:00:00+00:00");
        assert_eq!(today.len(), 2);
    }

    #[test]
    fn a_budget_is_read_from_the_project_and_zero_means_none() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join(".hexa")).unwrap();
        let p = d.path().join(".hexa/project.json");
        std::fs::write(&p, r#"{ "inference": { "budget_usd_per_day": 5.0 } }"#).unwrap();
        assert_eq!(budget_in(d.path()), Some(5.0));
        std::fs::write(&p, r#"{ "inference": { "budget_usd_per_day": 0 } }"#).unwrap();
        assert_eq!(budget_in(d.path()), None);
        std::fs::write(&p, r#"{ "inference": {} }"#).unwrap();
        assert_eq!(budget_in(d.path()), None);
    }

    #[test]
    fn entries_read_back_what_record_wrote() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("inference-log.jsonl");
        std::fs::write(&path, format!("{}\n{}\n", row("a", 1, 2, None, "2026-01-01T00:00:00+00:00"), row("b", 3, 4, Some(0.5), "2026-01-02T00:00:00+00:00"))).unwrap();
        let rows = entries_in(&path);
        assert_eq!(rows.len(), 2);
        assert_eq!(totals(&rows).cost_usd, 0.5);
    }
}

#[cfg(test)]
mod spend_names_a_model {
    //! ADR-2609160300 §4: the spend log names a model, not a code path.
    //! `record_with("claude-code", ...)` wrote the name of the process hexa
    //! shelled out to, so `hexa spend` could report $107 without being able to
    //! say what served any of it.
    use super::model_from_frontier_json;
    use serde_json::json;

    #[test]
    fn the_models_own_id_wins_when_the_answer_carries_one() {
        let v = json!({"result": "ok", "model": "claude-opus-5", "usage": {"input_tokens": 1}});
        assert_eq!(model_from_frontier_json(&v), "claude-opus-5");
    }

    #[test]
    fn a_per_model_usage_map_is_read_when_there_is_no_top_level_model() {
        let v = json!({"result": "ok", "modelUsage": {"claude-opus-5": {"inputTokens": 12}}});
        assert_eq!(model_from_frontier_json(&v), "claude-opus-5");
    }

    #[test]
    fn several_models_in_one_answer_are_all_named() {
        let v = json!({"modelUsage": {"claude-haiku-4-5": {"inputTokens": 3}, "claude-opus-5": {"inputTokens": 9}}});
        let got = model_from_frontier_json(&v);
        assert!(got.contains("claude-opus-5") && got.contains("claude-haiku-4-5"), "got {got}");
    }

    #[test]
    fn an_answer_that_names_nothing_says_so_rather_than_naming_the_code_path() {
        let v = json!({"result": "ok", "usage": {"input_tokens": 1}});
        let got = model_from_frontier_json(&v);
        assert_ne!(got, "claude-code", "a code path is not a model");
        assert!(got.contains("unknown"), "an unnamed model must be recorded as unknown, got {got}");
    }
}
