use std::path::Path;
use std::process::Command;

#[derive(Debug)]
pub struct BootstrapReport {
    pub ready: bool,
    pub service_checks: Vec<(String, bool)>,
    pub model_checks: Vec<(String, bool)>,
    /// A logged-in `claude` CLI is on PATH.
    pub frontier: bool,
    /// Which path to a model is open, in words.
    pub path: &'static str,
    pub config_exists: bool,
}

impl BootstrapReport {
    pub fn format_success(&self) -> String {
        let mut output = String::from("✓ Bootstrap complete\n\n");
        output.push_str("╭─ Validation Report ───────────────────╮\n");
        for (service, ok) in &self.service_checks {
            output.push_str(&format!("│ {} {}\n", if *ok { "✓" } else { "○" }, service));
        }
        for (model, ready) in &self.model_checks {
            output.push_str(&format!("│ {} {}{}\n", if *ready { "✓" } else { "○" }, model, if *ready { " (loaded)" } else { " (not pulled)" }));
        }
        output.push_str(&format!("│ {} frontier (claude)\n", if self.frontier { "✓" } else { "○" }));
        output.push_str(&format!("│ path: {}\n", self.path));
        output.push_str("╰───────────────────────────────────────╯");
        output
    }

    pub fn format_warning(&self) -> String {
        let provider = hexa_infer::local_provider();
        let mut output = String::from("✗ No path to a model\n\n");
        output.push_str("╭─ Validation Report ───────────────────╮\n");
        for (service, ok) in &self.service_checks {
            output.push_str(&format!("│ {} {}\n", if *ok { "✓" } else { "✗" }, service));
        }
        for (model, ready) in &self.model_checks {
            output.push_str(&format!("│ {} {}{}\n", if *ready { "✓" } else { "✗" }, model, if *ready { "" } else { " (not pulled)" }));
        }
        output.push_str(&format!("│ {} frontier (claude)\n", if self.frontier { "✓" } else { "✗" }));
        if !self.config_exists {
            output.push_str("│ ✗ .hexa/project.json\n");
        }
        output.push_str("╰───────────────────────────────────────╯\n");
        output.push_str(&format!(
            "Either start {} ({}) and pull the models the project configures, or log in to the claude CLI.",
            provider.display_name,
            provider.install_hint()
        ));
        output
    }
}

pub struct BootstrapValidator;

impl BootstrapValidator {
    pub fn new() -> Self {
        Self
    }

    pub async fn validate_all(&self) -> anyhow::Result<BootstrapReport> {
        let mut service_checks = vec![];
        let mut model_checks = vec![];

        // Check services
        // SpacetimeDB (3033) and hexa-nexus (5555) were checked here too. Both
        // are deleted (ADR-2608241500); an inference backend is the only
        // service hexa needs.
        let provider = hexa_infer::local_provider();
        service_checks.push((
            provider.display_name.to_string(),
            self.check_service_health(provider.default_port).await,
        ));

        // Check the models this project configures — not a hardcoded list.
        //
        // The list here was three literal ids and had drifted: it checked
        // `gemma4:latest` and `qwen2.5-coder:32b` while `.hexa/project.json`
        // declared `gemma4-12b` and `devstral-small-2:24b`. Bootstrap
        // therefore validated three models the project does not use and said
        // nothing about the three it does.
        for (label, _key, model) in hexa_infer::configured_tiers() {
            let present = self.model_exists(&model);
            model_checks.push((format!("{model} ({label})"), present));
        }

        // Check config
        let config_exists = Path::new(".hexa/project.json").exists();

        // Two paths to a model: the local server with the models the
        // project configures, or a logged-in `claude` CLI. Either is enough
        // to run, and a machine set up for the frontier path has no local
        // server on purpose. Only no path at all fails.
        let local_up = service_checks.iter().all(|(_, ok)| *ok);
        let local_ok = local_up && model_checks.iter().all(|(_, ok)| *ok);
        let frontier = Command::new("which").arg("claude").output().map(|o| o.status.success()).unwrap_or(false);
        let path = match (local_ok, frontier) {
            (true, true) => "local server and frontier",
            (true, false) => "local server only",
            (false, true) => "frontier only",
            (false, false) => "none",
        };
        let ready = config_exists && (local_ok || frontier);

        Ok(BootstrapReport {
            ready,
            service_checks,
            model_checks,
            config_exists,
            frontier,
            path,
        })
    }

    async fn check_service_health(&self, port: u16) -> bool {
        match tokio::net::TcpStream::connect(hexa_infer::local_provider().socket_addr()).await {
            Ok(_) => true,
            Err(_) => false,
        }
    }

    fn model_exists(&self, model_name: &str) -> bool {
        if let Ok(output) = Command::new(hexa_infer::local_provider().binary)
            .arg("show")
            .arg(model_name)
            .output()
        {
            output.status.success()
        } else {
            false
        }
    }
}
