//! What inference is reachable from here, discovered from the environment.
//!
//! hexa looked at one address, the local server's default port, and called
//! everything else absent. A machine can hold an API key, point
//! `OLLAMA_HOST` at another box, register endpoints with `hexa config
//! inference add`, or have a logged-in `claude`. This reads all of that and
//! reports each path it finds. Keys are reported present or absent, never
//! printed.

use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use crate::endpoint::Endpoint;

/// One place a model can be reached.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    /// `local`, `api`, `registered` or `frontier`.
    pub kind: &'static str,
    pub name: String,
    /// The address, path or model; never a secret.
    pub detail: String,
    /// What told us: an environment variable, a file, or PATH.
    pub via: String,
    /// `Some(true)` answered a probe, `Some(false)` did not, `None` not probed.
    pub reachable: Option<bool>,
}

impl Found {
    /// A path counts unless a probe said no.
    pub fn open(&self) -> bool {
        self.reachable != Some(false)
    }
}

/// Discover from the real environment, registry and PATH.
pub fn discover() -> Vec<Found> {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let endpoints = crate::registry::load();
    let claude = std::process::Command::new("which")
        .arg("claude")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    discover_with(&env, &endpoints, claude.as_deref(), &probe)
}

/// The same, with every input injected. What the tests drive.
pub fn discover_with(
    env: &dyn Fn(&str) -> Option<String>,
    endpoints: &[Endpoint],
    claude_path: Option<&str>,
    probe: &dyn Fn(&str) -> Option<bool>,
) -> Vec<Found> {
    let mut out = Vec::new();

    // The local server, wherever the environment points it.
    let p = crate::local_provider();
    let via = [p.host_env, "OLLAMA_HOST"]
        .iter()
        .find(|k| env(k).is_some())
        .map(|k| k.to_string())
        .unwrap_or_else(|| "default address".to_string());
    let url = crate::local_provider::base_url_with(&p, env);
    out.push(Found { kind: "local", name: p.display_name.to_string(), detail: url.clone(), via, reachable: probe(&url) });

    // API providers, by the presence of their key or URL.
    if env("ANTHROPIC_API_KEY").is_some() {
        let base = env("ANTHROPIC_BASE_URL").unwrap_or_else(|| "api.anthropic.com".to_string());
        out.push(Found { kind: "api", name: "anthropic".to_string(), detail: base, via: "ANTHROPIC_API_KEY".to_string(), reachable: None });
    }
    if let Some(url) = env("HEXA_INFERENCE_URL") {
        let model = env("HEXA_INFERENCE_MODEL").map(|m| format!(" · {m}")).unwrap_or_default();
        out.push(Found { kind: "api", name: "openai-compatible".to_string(), detail: format!("{url}{model}"), via: "HEXA_INFERENCE_URL".to_string(), reachable: probe(&url) });
    }
    if let Some(host) = env("HEXA_VLLM_HOST") {
        let model = env("HEXA_VLLM_MODEL").map(|m| format!(" · {m}")).unwrap_or_default();
        out.push(Found { kind: "api", name: "vllm".to_string(), detail: format!("{host}{model}"), via: "HEXA_VLLM_HOST".to_string(), reachable: probe(&host) });
    }

    // Endpoints registered with `hexa config inference add`.
    for e in endpoints {
        out.push(Found {
            kind: "registered",
            name: e.id.clone(),
            detail: format!("{} · {} · {}", e.url, e.provider, e.model),
            via: "~/.hexa/inference-servers.json".to_string(),
            reachable: probe(&e.url),
        });
    }

    // The frontier path: a logged-in claude CLI.
    if let Some(path) = claude_path {
        out.push(Found { kind: "frontier", name: "claude".to_string(), detail: path.to_string(), via: "PATH".to_string(), reachable: Some(true) });
    }
    out
}

/// Is there any path to a model?
pub fn any_path(found: &[Found]) -> bool {
    found.iter().any(Found::open)
}

/// The open paths, in words: "local server, anthropic, claude".
pub fn path_words(found: &[Found]) -> String {
    let names: Vec<String> = found
        .iter()
        .filter(|f| f.open())
        .map(|f| match f.kind {
            "local" => "local server".to_string(),
            "frontier" => "claude".to_string(),
            _ => f.name.clone(),
        })
        .collect();
    if names.is_empty() { "none".to_string() } else { names.join(", ") }
}

/// A TCP connect with a short timeout; `None` when the URL has no host.
fn probe(url: &str) -> Option<bool> {
    let hostport = url.trim_start_matches("http://").trim_start_matches("https://");
    let hostport = hostport.split('/').next().unwrap_or("");
    let hostport = if hostport.contains(':') {
        hostport.to_string()
    } else if url.starts_with("https://") {
        format!("{hostport}:443")
    } else {
        format!("{hostport}:80")
    };
    let addr = hostport.to_socket_addrs().ok()?.next()?;
    Some(TcpStream::connect_timeout(&addr, Duration::from_millis(700)).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of<'p>(pairs: &'p [(&'p str, &'p str)]) -> impl Fn(&str) -> Option<String> + 'p {
        move |k: &str| pairs.iter().find(|(kk, _)| *kk == k).map(|(_, v)| v.to_string())
    }

    #[test]
    fn a_key_in_the_environment_is_a_path_and_is_never_printed() {
        let env = env_of(&[("ANTHROPIC_API_KEY", "sk-secret-value")]);
        let found = discover_with(&env, &[], None, &|_| Some(false));
        let api = found.iter().find(|f| f.kind == "api").expect("anthropic found");
        assert_eq!(api.via, "ANTHROPIC_API_KEY");
        assert!(!format!("{found:?}").contains("sk-secret"), "the key leaked");
        assert!(any_path(&found));
        assert_eq!(path_words(&found), "anthropic");
    }

    #[test]
    fn the_local_server_follows_the_environment_and_a_failed_probe_closes_it() {
        let env = env_of(&[("OLLAMA_HOST", "10.0.0.7:11434")]);
        let found = discover_with(&env, &[], None, &|_| Some(false));
        let local = &found[0];
        assert_eq!(local.kind, "local");
        assert!(local.detail.contains("10.0.0.7:11434"), "{}", local.detail);
        assert_eq!(local.via, "OLLAMA_HOST");
        assert!(!local.open());
        assert!(!any_path(&found));
        assert_eq!(path_words(&found), "none");
    }

    #[test]
    fn a_registered_endpoint_and_the_claude_cli_are_paths() {
        let env = env_of(&[]);
        let ep = Endpoint { id: "box".into(), url: "http://box:8000".into(), provider: "openai_compat".into(), model: "m".into(), models: vec![], status: "healthy".into(), ..Default::default() };
        let found = discover_with(&env, &[ep], Some("/usr/local/bin/claude"), &|u| Some(u.contains("box")));
        assert_eq!(path_words(&found), "box, claude");
    }
}
