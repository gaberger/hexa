//! Inference diagnostics for `hexa doctor`.
//!
//! hexa has two ways to reach a model: a local inference server, and the
//! frontier path through a logged-in `claude` CLI. Either one is enough to
//! run. This module checks both, lists the tiers the project configures, and
//! says which path is open. It names the local provider through
//! `hexa_infer::local_provider()`, the one place a provider is named.

use colored::Colorize;
use std::time::Duration;

/// One check: pass with a detail, warn with a detail, or fail with a detail.
#[derive(Debug, Clone)]
pub enum CheckStatus {
    Pass(String),
    Warn(String),
    Fail(String),
}

/// What the inference checks found.
#[derive(Debug, Clone)]
pub struct InferenceStatus {
    pub local_server: CheckStatus,
    pub frontier: CheckStatus,
    /// `(label, tier key, model id)` from `.hexa/project.json`.
    pub tiers: Vec<(&'static str, &'static str, String)>,
}

impl InferenceStatus {
    /// At least one path to a model is open.
    pub fn has_any_inference(&self) -> bool {
        matches!(self.local_server, CheckStatus::Pass(_)) || matches!(self.frontier, CheckStatus::Pass(_))
    }

    /// Which path is open, in words.
    pub fn path(&self) -> &'static str {
        match (
            matches!(self.local_server, CheckStatus::Pass(_)),
            matches!(self.frontier, CheckStatus::Pass(_)),
        ) {
            (true, true) => "local server and frontier",
            (true, false) => "local server only",
            (false, true) => "frontier only",
            (false, false) => "none",
        }
    }
}

/// Run the inference checks without printing.
pub async fn run_composition_check_quiet() -> InferenceStatus {
    InferenceStatus {
        local_server: check_local_server().await,
        frontier: check_claude_binary().await,
        tiers: hexa_infer::configured_tiers(),
    }
}

/// Run the inference checks and print the section.
pub async fn run_composition_check() -> InferenceStatus {
    let status = run_composition_check_quiet().await;
    println!("  {}", "Inference:".bold());
    print_status("local server", &status.local_server);
    if status.tiers.is_empty() {
        println!("    {} tiers          none configured in .hexa/project.json", "!".yellow());
    } else {
        for (label, _key, model) in &status.tiers {
            println!("      tier {:<5} {}", label, model);
        }
    }
    print_status("frontier (claude)", &status.frontier);
    println!("    path:            {}", status.path().bold());
    status
}

fn print_status(label: &str, status: &CheckStatus) {
    match status {
        CheckStatus::Pass(detail) => println!("    {} {} ({})", "\u{2713}".green(), label, detail),
        CheckStatus::Warn(detail) => println!("    {} {} ({})", "!".yellow(), label, detail),
        CheckStatus::Fail(detail) => println!("    {} {} ({})", "\u{2717}".red(), label, detail),
    }
}

async fn check_local_server() -> CheckStatus {
    let provider = hexa_infer::local_provider();
    let host = format!("{} at {}", provider.display_name, provider.base_url());
    let url = provider.base_url();
    let client = match reqwest::Client::builder().timeout(Duration::from_secs(2)).build() {
        Ok(c) => c,
        Err(_) => return CheckStatus::Fail("cannot build HTTP client".to_string()),
    };
    match client.get(format!("{}/api/tags", url)).send().await {
        Ok(resp) if resp.status().is_success() => {
            let models = resp
                .json::<serde_json::Value>()
                .await
                .ok()
                .and_then(|v| v["models"].as_array().map(|a| a.len()));
            match models {
                Some(n) => CheckStatus::Pass(format!("{host}, {n} model{}", if n == 1 { "" } else { "s" })),
                None => CheckStatus::Pass(format!("{host}, reachable")),
            }
        }
        Ok(resp) => CheckStatus::Fail(format!("{host}, HTTP {}", resp.status())),
        Err(e) => {
            let reason = if e.is_connect() {
                "not running"
            } else if e.is_timeout() {
                "timeout (2s)"
            } else {
                "unreachable"
            };
            CheckStatus::Fail(format!("{host}, {reason}"))
        }
    }
}

async fn check_claude_binary() -> CheckStatus {
    let which = tokio::process::Command::new("which").arg("claude").output().await;
    match which {
        Ok(output) if output.status.success() => {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let version = tokio::process::Command::new("claude")
                .arg("--version")
                .output()
                .await
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
            match version {
                Some(v) if !v.is_empty() => CheckStatus::Pass(format!("{path}, {v}")),
                _ => CheckStatus::Pass(path),
            }
        }
        _ => CheckStatus::Fail("not on PATH".to_string()),
    }
}
