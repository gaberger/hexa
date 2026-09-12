//! The project config, on bootstrap.
//!
//! An existing `.hexa/project.json` is left exactly as it is. This step once
//! overwrote it with three hardcoded model names: a project's name,
//! `analyze.exclude` and budget were gone, and validation then reported the
//! injected models as missing. Model names belong in `hexa-infer` and in the
//! project's own config, never in this file.

use std::fs;
use std::path::Path;

pub struct ConfigSetup {
    dry_run: bool,
}

/// What the step did, for the caller to print.
pub enum ConfigOutcome {
    Kept,
    Created,
    WouldCreate,
}

impl ConfigSetup {
    pub fn new(config: super::BootstrapConfig) -> Self {
        Self { dry_run: config.dry_run }
    }

    pub async fn setup(&self) -> anyhow::Result<ConfigOutcome> {
        let config_path = Path::new(".hexa").join("project.json");
        if config_path.exists() {
            return Ok(ConfigOutcome::Kept);
        }
        if self.dry_run {
            return Ok(ConfigOutcome::WouldCreate);
        }
        fs::create_dir_all(".hexa")?;
        // No models. A project configures its tiers with `hexa config`; a
        // default here would be a choice made for the user in silence.
        let body = serde_json::json!({ "inference": { "tier_models": {} } });
        fs::write(&config_path, serde_json::to_string_pretty(&body)? + "\n")?;
        Ok(ConfigOutcome::Created)
    }
}
