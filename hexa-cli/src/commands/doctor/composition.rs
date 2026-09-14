//! Inference diagnostics for `hexa doctor`.
//!
//! Every path to a model the machine holds, discovered by `hexa_infer::discover`
//! from the environment (`OLLAMA_HOST`, `ANTHROPIC_API_KEY`,
//! `HEXA_INFERENCE_URL`, `HEXA_VLLM_HOST`), the endpoint registry, and a
//! logged-in `claude` on PATH. Any one of them is enough to run.

use colored::Colorize;


/// What the inference checks found.
#[derive(Debug, Clone)]
pub struct InferenceStatus {
    /// Every path to a model the environment, the registry and PATH hold.
    pub found: Vec<hexa_infer::Found>,
    /// `(label, tier key, model id)` from `.hexa/project.json`.
    pub tiers: Vec<(&'static str, &'static str, String)>,
}

/// A configured tier and what its model resolved to (ADR-2609131617 §3).
pub struct TierCoverage {
    pub label: &'static str,
    pub model: String,
    pub coverage: hexa_infer::Coverage,
}

impl InferenceStatus {
    /// At least one path to a model is open.
    pub fn has_any_inference(&self) -> bool {
        hexa_infer::discover::any_path(&self.found)
    }

    /// Every configured tier, resolved against the reachable paths.
    pub fn tier_coverage(&self) -> Vec<TierCoverage> {
        self.tiers
            .iter()
            .map(|(label, _key, model)| TierCoverage {
                label,
                model: model.clone(),
                coverage: hexa_infer::serves(&self.found, model),
            })
            .collect()
    }

    /// The tiers naming a model nothing reachable can serve. A run with any
    /// of these is not a passing run (ADR-2609131617 §4).
    pub fn unserved_tiers(&self) -> Vec<TierCoverage> {
        self.tier_coverage()
            .into_iter()
            .filter(|t| t.coverage == hexa_infer::Coverage::NotServed)
            .collect()
    }

    /// What the reachable paths do serve, for the line that says so.
    pub fn served_models(&self) -> Vec<String> {
        hexa_infer::served_models(&self.found)
    }

    /// The open paths, in words.
    pub fn path(&self) -> String {
        hexa_infer::discover::path_words(&self.found)
    }
}

/// Run the inference checks without printing.
///
/// A reachable runtime that carries no model list is asked for one, so a
/// tier can be answered rather than shrugged at (ADR-2609131617 §2).
pub async fn run_composition_check_quiet() -> InferenceStatus {
    let mut found = hexa_infer::discover();
    for f in found.iter_mut() {
        if f.reachable == Some(true) && f.models.is_empty() && f.kind != "frontier" {
            let url = f.detail.split(' ').next().unwrap_or(&f.detail).to_string();
            if let Some(models) = hexa_infer::enumerate_models(&url).await {
                f.models = models;
            }
        }
    }
    InferenceStatus { found, tiers: hexa_infer::configured_tiers() }
}

/// Run the inference checks and print the section.
pub async fn run_composition_check() -> InferenceStatus {
    let status = run_composition_check_quiet().await;
    println!("  {}", "Inference:".bold());
    for f in &status.found {
        let mark = match f.reachable {
            Some(true) => "\u{2713}".green(),
            Some(false) => "\u{2717}".red(),
            None => "\u{25cb}".dimmed(),
        };
        let state = match f.reachable {
            Some(true) => "reachable",
            Some(false) => "not reachable",
            None => "key present, not probed",
        };
        println!("    {} {:<10} {} ({}; via {})", mark, f.kind, f.name, state, f.via);
        println!("      {}", f.detail.dimmed());
    }
    if status.tiers.is_empty() {
        println!("    {} tiers          none configured in .hexa/project.json", "!".yellow());
    } else {
        // A tier is printed with what answers for it, never bare: printing
        // the configuration back is not a check (ADR-2609131617).
        for t in status.tier_coverage() {
            let (mark, note) = match &t.coverage {
                hexa_infer::Coverage::Served(by) => ("\u{2713}".green(), format!("served by {by}").dimmed().to_string()),
                hexa_infer::Coverage::NotServed => ("\u{2717}".red(), "no reachable path serves it".red().to_string()),
                hexa_infer::Coverage::Unverified(by) => ("\u{25cb}".dimmed(), format!("unverified: {by} did not list its models").yellow().to_string()),
            };
            println!("    {} tier {:<5} {} — {}", mark, t.label, t.model, note);
        }
        let unserved = status.unserved_tiers();
        if !unserved.is_empty() {
            let served = status.served_models();
            let offer = if served.is_empty() { "nothing".to_string() } else { served.join(", ") };
            println!("      {} reachable paths serve: {}", "→".dimmed(), offer);
        }
    }
    println!("    path:            {}", status.path().bold());
    status
}




#[cfg(test)]
mod tier_coverage {
    use super::*;
    use hexa_infer::{Coverage, Found};

    fn path(kind: &'static str, name: &str, reachable: Option<bool>, models: &[&str]) -> Found {
        Found {
            kind,
            name: name.to_string(),
            detail: String::new(),
            via: String::new(),
            reachable,
            models: models.iter().map(|m| m.to_string()).collect(),
        }
    }

    fn status(found: Vec<Found>, tiers: &[(&'static str, &'static str)]) -> InferenceStatus {
        InferenceStatus {
            found,
            tiers: tiers.iter().map(|(l, m)| (*l, "k", m.to_string())).collect(),
        }
    }

    /// ADR-2609131617 §4: a tier nothing can serve is a failure, and the
    /// message can name what is actually on offer. This is the machine the
    /// ADR was written from: Ollama down, tt-gptoss up, three tiers pointing
    /// at models only Ollama would have had.
    #[test]
    fn a_tier_nothing_serves_is_a_failure_and_the_served_models_are_nameable() {
        let s = status(
            vec![
                path("local", "Ollama", Some(false), &[]),
                path("registered", "tt-gptoss", Some(true), &["openai/gpt-oss-120b"]),
                path("frontier", "claude", Some(true), &[]),
            ],
            &[("T1", "qwen3:4b"), ("T2", "gemma4-12b"), ("T2.5", "devstral-small-2:24b")],
        );
        let unserved = s.unserved_tiers();
        assert_eq!(unserved.len(), 3, "every tier names a model only the down runtime had");
        assert_eq!(unserved[0].label, "T1");
        assert_eq!(s.served_models(), vec!["openai/gpt-oss-120b".to_string()]);
        assert!(s.has_any_inference(), "paths are open; it is the tier map that is broken");
    }

    /// The same machine with the runtime up and the models pulled: green,
    /// on evidence, and each tier says what answers for it.
    #[test]
    fn a_tier_a_reachable_path_lists_is_served_by_it() {
        let s = status(
            vec![
                path("local", "Ollama", Some(true), &["qwen3:4b", "gemma4-12b"]),
                path("registered", "tt-gptoss", Some(true), &["openai/gpt-oss-120b"]),
            ],
            &[("T1", "qwen3:4b"), ("T2", "openai/gpt-oss-120b")],
        );
        assert!(s.unserved_tiers().is_empty());
        let c = s.tier_coverage();
        assert_eq!(c[0].coverage, Coverage::Served("Ollama".into()));
        assert_eq!(c[1].coverage, Coverage::Served("tt-gptoss".into()));
    }

    /// A runtime that would not list its models leaves the tier unverified,
    /// which warns and does not fail: doctor may say it could not check.
    #[test]
    fn an_unverified_tier_warns_and_does_not_fail() {
        let s = status(
            vec![path("local", "Ollama", Some(true), &[])],
            &[("T1", "qwen3:4b")],
        );
        assert!(s.unserved_tiers().is_empty(), "unverified is not a failure");
        assert_eq!(s.tier_coverage()[0].coverage, Coverage::Unverified("Ollama".into()));
    }

    /// No tiers configured is the pre-existing warning, not a coverage failure.
    #[test]
    fn no_tiers_configured_is_not_an_unserved_tier() {
        let s = status(vec![path("frontier", "claude", Some(true), &[])], &[]);
        assert!(s.unserved_tiers().is_empty());
        assert!(s.tier_coverage().is_empty());
    }
}
