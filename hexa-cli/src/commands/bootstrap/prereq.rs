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

pub struct PrereqChecker;

impl PrereqChecker {
    pub fn new() -> Self {
        Self
    }

    pub async fn check_all(&self) -> anyhow::Result<PrereqReport> {
        let mut statuses = vec![];

        // The inference server, whoever it is. hexa-infer names it; this file
        // must not (founding goal G1).
        statuses.push(self.check_inference_server());

        // Check Rust
        statuses.push(self.check_rust());

        // Check Bun
        statuses.push(self.check_bun());

        // Check Cargo
        statuses.push(self.check_cargo());

        Ok(PrereqReport { statuses })
    }

    /// Is the local inference server installed?
    ///
    /// Every detail, the display name, the executable and the install command
    /// per platform, comes from `hexa_infer::local_provider()`. This file must
    /// not name the server (founding goal G1).
    fn check_inference_server(&self) -> PrereqStatus {
        let provider = hexa_infer::local_provider();
        let hint = provider.install_hint();
        self.status(provider.display_name, provider.binary, &hint)
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
