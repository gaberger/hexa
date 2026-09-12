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

impl InferenceStatus {
    /// At least one path to a model is open.
    pub fn has_any_inference(&self) -> bool {
        hexa_infer::discover::any_path(&self.found)
    }

    /// The open paths, in words.
    pub fn path(&self) -> String {
        hexa_infer::discover::path_words(&self.found)
    }
}

/// Run the inference checks without printing.
pub async fn run_composition_check_quiet() -> InferenceStatus {
    InferenceStatus { found: hexa_infer::discover(), tiers: hexa_infer::configured_tiers() }
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
        for (label, _key, model) in &status.tiers {
            println!("      tier {:<5} {}", label, model);
        }
    }
    println!("    path:            {}", status.path().bold());
    status
}



