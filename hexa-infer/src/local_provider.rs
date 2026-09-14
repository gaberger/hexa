//! Who serves inference on this machine, named in exactly one place.
//!
//! Founding goal G1's test is that **zero non-test files outside this crate
//! name a provider or a model**. `hexa bootstrap` and `hexa doctor composition`
//! both broke it, and not trivially: between them they hardcoded the
//! provider's display name, its CLI binary, its process name, its port, two
//! install commands, and a list of three model ids.
//!
//! The model list was the expensive part. It had drifted: `hexa bootstrap
//! validate` checked for `gemma4:latest` and `qwen2.5-coder:32b` while
//! `.hexa/project.json` configured `gemma4-12b` and `devstral-small-2:24b`. So
//! bootstrap reported a healthy install after verifying three models the
//! project does not use, and said nothing about the three it does. A check
//! that validates the wrong subject passes for the wrong reason, which looks
//! exactly like passing.
//!
//! [`configured_tiers`] fixes that by reading the same config the dispatcher
//! reads. There is no hardcoded model list here either — a default model in
//! this file would be a model the operator never chose.

/// The local inference server hexa bootstraps against.
///
/// One value, because this is a single-binary tool for one machine. When hexa
/// needs to support a second local server, this becomes an enum and every
/// caller below keeps compiling — which is the point of naming it here rather
/// than spelling it out in six files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalProvider {
    /// For display: "Ollama".
    pub display_name: &'static str,
    /// The executable on `PATH`, and the process name to match.
    pub binary: &'static str,
    /// The port it listens on by default.
    pub default_port: u16,
    /// The subcommand that starts it in the foreground.
    pub serve_arg: &'static str,
    /// How to install it, per platform.
    pub install_macos: &'static str,
    pub install_linux: &'static str,
    /// The environment variable that overrides where it listens.
    pub host_env: &'static str,
}

/// The local provider hexa targets.
pub const fn local_provider() -> LocalProvider {
    LocalProvider {
        display_name: "Ollama",
        binary: "ollama",
        default_port: 11434,
        serve_arg: "serve",
        install_macos: "brew install ollama",
        install_linux: "curl https://ollama.ai/install.sh | sh",
        // The hexa-specific spelling, tried before the plain `OLLAMA_HOST`.
        //
        // This was `"OLLAMA_HOST"`, which made both halves of every
        // `[host_env, "OLLAMA_HOST"]` pair read the same variable and the
        // fallback dead code. `HEXA_OLLAMA_HOST` was therefore ignored here
        // while `hexa config inference` printed it as the way to point at a
        // server, and `openai_compat` read it — so an operator following
        // hexa's own advice still got a probe against localhost.
        host_env: "HEXA_OLLAMA_HOST",
    }
}

impl LocalProvider {
    /// `http://127.0.0.1:<port>` — the base URL for a default install.
    pub fn default_base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.default_port)
    }

    /// Where the server actually is: the host environment variable if set,
    /// otherwise the default.
    ///
    /// **This exists because the same variable was read three ways.**
    /// `resource_governor.rs` treated it as `host:port` and prepended a
    /// scheme; `doctor/composition.rs` treated it as a full URL and defaulted
    /// to `http://localhost:11434`; `complete.rs` defaulted to
    /// `http://127.0.0.1:11434`. Set `OLLAMA_HOST=127.0.0.1:11434`, as the
    /// server's own documentation tells you to, and one of those three built
    /// `http://http://…`. Normalising once is the only way that stays fixed.
    pub fn base_url(&self) -> String {
        base_url_with(self, &|k| std::env::var(k).ok())
    }

    /// `host:port` for a TCP reachability probe, from the same resolved URL.
    ///
    /// Callers used to build `format!("127.0.0.1:{port}")` themselves, which
    /// meant a probe against localhost even when the operator had pointed the
    /// server at another machine — reporting "not running" for a server that
    /// was running fine.
    pub fn socket_addr(&self) -> String {
        socket_addr_with(self, &|k| std::env::var(k).ok())
    }

    /// The install hint for the platform this binary was built for.
    pub fn install_hint(&self) -> &'static str {
        if cfg!(target_os = "macos") {
            self.install_macos
        } else {
            self.install_linux
        }
    }
}

/// `base_url`, with the environment reader injected. The local server's
/// address comes from `host_env` (`HEXA_OLLAMA_HOST`), then `OLLAMA_HOST`,
/// then the default port; a bare `host:port` gets `http://`.
pub fn base_url_with(p: &LocalProvider, env: &dyn Fn(&str) -> Option<String>) -> String {
    // Trim before deciding: a blank override falls back to the default,
    // and "  " once became "http://".
    let raw = env(p.host_env)
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .filter(|v| !v.is_empty())
        .or_else(|| env("OLLAMA_HOST").map(|v| v.trim().trim_end_matches('/').to_string()).filter(|v| !v.is_empty()));
    match raw {
        Some(v) => {
            if v.starts_with("http://") || v.starts_with("https://") { v } else { format!("http://{v}") }
        }
        None => p.default_base_url(),
    }
}

/// `socket_addr`, with the environment reader injected, for the same reason
/// `base_url_with` exists: a test that reaches for the process environment is
/// racing every other test in the binary.
fn socket_addr_with(p: &LocalProvider, env: &dyn Fn(&str) -> Option<String>) -> String {
    let url = base_url_with(p, env);
    let rest = url.strip_prefix("http://").or_else(|| url.strip_prefix("https://")).unwrap_or(&url);
    let host_port = rest.split('/').next().unwrap_or(rest);
    if host_port.contains(':') {
        host_port.to_string()
    } else {
        format!("{host_port}:{}", p.default_port)
    }
}

/// The tiers this project configures, as `(label, tier key, model id)`.
///
/// Empty when `.hexa/project.json` declares none — which is a real answer, and
/// a caller should say so rather than substitute a guess.
pub fn configured_tiers() -> Vec<(&'static str, &'static str, String)> {
    [("T1", "t1"), ("T2", "t2"), ("T2.5", "t2.5")]
        .into_iter()
        .filter_map(|(label, key)| crate::tiers::tier_model(key).map(|m| (label, key, m)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_base_url_is_built_from_the_declared_port() {
        let p = local_provider();
        assert_eq!(p.default_base_url(), format!("http://127.0.0.1:{}", p.default_port));
    }

    /// A reader that answers for one variable and nothing else.
    ///
    /// These tests used to `set_var` and restore. Two of them did it to the
    /// same variable, and `cargo test` runs them in parallel, so one read the
    /// other's value and failed intermittently with
    /// `OLLAMA_HOST="  " left: "http://gpu-box:9999"`. The provider already
    /// takes an injected reader; the tests simply were not using it.
    fn only(key: &'static str, value: &'static str) -> impl Fn(&str) -> Option<String> {
        move |k: &str| (k == key).then(|| value.to_string())
    }

    /// Both spellings of the override must produce one well-formed URL.
    #[test]
    fn the_host_override_is_normalised_either_way() {
        let p = local_provider();
        for (set, want) in [
            ("127.0.0.1:9999", "http://127.0.0.1:9999"),
            ("http://box:9999", "http://box:9999"),
            ("  ", ""), // blank falls back to the default
        ] {
            let got = base_url_with(&p, &only(p.host_env, set));
            let expected = if want.is_empty() { p.default_base_url() } else { want.to_string() };
            assert_eq!(got, expected, "{}={set:?}", p.host_env);
            assert!(!got.contains("http://http"), "double scheme from {set:?}");
        }
    }

    /// `OLLAMA_HOST` is read only when the hexa-specific variable is absent.
    #[test]
    fn the_hexa_variable_wins_over_the_plain_one() {
        let p = local_provider();
        let both = |k: &str| match k {
            "OLLAMA_HOST" => Some("plain:1111".to_string()),
            k if k == p.host_env => Some("hexa:2222".to_string()),
            _ => None,
        };
        assert_eq!(base_url_with(&p, &both), "http://hexa:2222");
        assert_eq!(base_url_with(&p, &only("OLLAMA_HOST", "plain:1111")), "http://plain:1111");
    }

    #[test]
    fn the_probe_target_follows_the_host_override() {
        let p = local_provider();
        assert_eq!(socket_addr_with(&p, &only(p.host_env, "http://gpu-box:9999/")), "gpu-box:9999");
        assert_eq!(
            socket_addr_with(&p, &only(p.host_env, "gpu-box")),
            format!("gpu-box:{}", p.default_port)
        );
    }

    #[test]
    fn the_install_hint_is_never_empty() {
        assert!(!local_provider().install_hint().is_empty());
    }

    /// `configured_tiers` must return what the project declares and nothing
    /// else. A hardcoded fallback here would put a model id back into the
    /// code path this module exists to clear.
    #[test]
    fn no_config_means_no_tiers_rather_than_a_default() {
        let dir = tempfile::tempdir().expect("tempdir");
        // tier_model resolves through HEXA_PROJECT_ROOT; point it at an empty
        // directory and the answer must be "none", not a guess.
        let prev = std::env::var("HEXA_PROJECT_ROOT").ok();
        std::env::set_var("HEXA_PROJECT_ROOT", dir.path());
        let got = configured_tiers();
        match prev {
            Some(p) => std::env::set_var("HEXA_PROJECT_ROOT", p),
            None => std::env::remove_var("HEXA_PROJECT_ROOT"),
        }
        assert!(got.is_empty(), "an unconfigured project produced tiers: {got:?}");
    }
}
