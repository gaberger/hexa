use std::process::Command;

const RUSTUP: &str = "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh";

#[derive(Debug, Clone)]
pub struct PrereqStatus {
    pub name: String,
    pub installed: bool,
    pub install_cmd: Option<String>,
}

#[derive(Debug)]
pub struct PrereqReport {
    pub statuses: Vec<PrereqStatus>,
}

impl PrereqReport {
    pub fn all_ok(&self) -> bool {
        self.statuses.iter().all(|s| s.installed)
    }

    pub fn format(&self) -> String {
        let mut output = String::from("⚠ Prerequisites check failed:\n");
        for status in &self.statuses {
            if !status.installed {
                output.push_str(&format!("  ✗ {} (not found)\n", status.name));
                if let Some(cmd) = &status.install_cmd {
                    output.push_str(&format!("    Install: {}\n", cmd));
                }
            }
        }
        output
    }
}

/// The inference prerequisite from what discovery found: met by any open
/// path, or by an installed local server.
fn inference_status(found: &[hexa_infer::ports::Found], local_installed: bool) -> PrereqStatus {
    let installed = local_installed || hexa_infer::reach::any_path(found);
    let name = if installed && !local_installed {
        format!("Inference ({})", hexa_infer::reach::path_words(found))
    } else {
        "Inference".to_string()
    };
    let hint = format!(
        "any one of: install the local server ({}), set an API key or HEXA_INFERENCE_URL, register an endpoint with `hexa config inference add <template> <url>`, or set HEXA_COMPLETION_COMMAND",
        hexa_infer::ports::local_provider().install_hint()
    );
    PrereqStatus { name, installed, install_cmd: (!installed).then_some(hint) }
}

pub struct PrereqChecker;

impl PrereqChecker {
    pub fn new() -> Self {
        Self
    }

    pub async fn check_all(&self) -> anyhow::Result<PrereqReport> {
        // The inference server, whoever it is. hexa-infer names it; this file
        // must not (founding goal G1).
        let mut statuses = vec![self.check_inference_server()];

        // Check Rust
        statuses.push(self.check_rust());

        // Check Bun
        statuses.push(self.check_bun());

        // Check Cargo
        statuses.push(self.check_cargo());

        Ok(PrereqReport { statuses })
    }

    /// Is any inference reachable? hexa cannot know every model a machine
    /// offers, so it reads the environment (`hexa_infer::discovery()`): a
    /// local server, an API key or inference URL, a registered endpoint, a
    /// completion command, a logged-in frontier CLI. Any of them will do.
    /// An installed local server counts even when stopped. This file must not
    /// name a server or provider (founding goal G1).
    fn check_inference_server(&self) -> PrereqStatus {
        use hexa_infer::ports::Discovery;
        let found = hexa_infer::discovery().discover();
        let local_installed = self.command_exists(hexa_infer::ports::local_provider().binary);
        inference_status(&found, local_installed)
    }

    fn check_rust(&self) -> PrereqStatus {
        self.status("Rust", "rustc", RUSTUP)
    }

    fn check_bun(&self) -> PrereqStatus {
        self.status("Bun", "bun", "curl -fsSL https://bun.sh/install | bash")
    }

    fn check_cargo(&self) -> PrereqStatus {
        self.status("Cargo", "cargo", RUSTUP)
    }

    fn status(&self, name: &str, binary: &str, install: &str) -> PrereqStatus {
        let installed = self.command_exists(binary);
        PrereqStatus {
            name: name.to_string(),
            installed,
            install_cmd: (!installed).then(|| install.to_string()),
        }
    }

    fn command_exists(&self, cmd: &str) -> bool {
        Command::new("which")
            .arg(cmd)
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hexa_infer::ports::Found;

    fn found(kind: &'static str, name: &str, reachable: Option<bool>) -> Found {
        Found { kind, name: name.into(), detail: String::new(), via: String::new(), reachable, models: vec![] }
    }

    /// Any inference path the environment offers satisfies bootstrap; no one
    /// server is required.
    #[test]
    fn any_discovered_path_is_enough_inference() {
        let api = [found("local", "local", Some(false)), found("api", "anthropic", None)];
        assert!(inference_status(&api, false).installed, "an API key is a path");
        let cli = [found("local", "local", Some(false)), found("frontier", "claude", Some(true))];
        assert!(inference_status(&cli, false).installed, "a logged-in frontier CLI is a path");
        let up = [found("local", "local", Some(true))];
        assert!(inference_status(&up, false).installed, "a running local server is a path");
    }

    /// An installed local server that is not running still counts: it can be started.
    #[test]
    fn an_installed_local_server_counts_even_when_stopped() {
        let down = [found("local", "local", Some(false))];
        assert!(inference_status(&down, true).installed);
    }

    /// Nothing found: the check fails and the hint names every way in, not one product.
    #[test]
    fn no_path_fails_and_names_the_ways_in() {
        let down = [found("local", "local", Some(false))];
        let s = inference_status(&down, false);
        assert!(!s.installed);
        let hint = s.install_cmd.expect("a hint");
        assert!(hint.contains("hexa config inference add"), "{hint}");
        assert!(hint.contains("API key"), "{hint}");
    }
}
