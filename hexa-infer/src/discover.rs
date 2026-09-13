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
    /// The models this path is known to serve. Empty means *not
    /// enumerated*, never *none* (ADR-2609131617 §1).
    pub models: Vec<String>,
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
fn discover_with(
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
    // A local runtime serves whatever has been pulled into it; doctor
    // enumerates it (ADR-2609131617 §2), so it starts unenumerated.
    out.push(Found { kind: "local", name: p.display_name.to_string(), detail: url.clone(), via, reachable: probe(&url), models: Vec::new() });

    // API providers, by the presence of their key or URL.
    if env("ANTHROPIC_API_KEY").is_some() {
        let base = env("ANTHROPIC_BASE_URL").unwrap_or_else(|| "api.anthropic.com".to_string());
        out.push(Found { kind: "api", name: "anthropic".to_string(), detail: base, via: "ANTHROPIC_API_KEY".to_string(), reachable: None, models: Vec::new() });
    }
    if let Some(url) = env("HEXA_INFERENCE_URL") {
        let model = env("HEXA_INFERENCE_MODEL").map(|m| format!(" · {m}")).unwrap_or_default();
        out.push(Found {
            kind: "api",
            name: "openai-compatible".to_string(),
            detail: format!("{url}{model}"),
            via: "HEXA_INFERENCE_URL".to_string(),
            reachable: probe(&url),
            models: env("HEXA_INFERENCE_MODEL").into_iter().collect(),
        });
    }
    if let Some(host) = env("HEXA_VLLM_HOST") {
        let model = env("HEXA_VLLM_MODEL").map(|m| format!(" · {m}")).unwrap_or_default();
        out.push(Found {
            kind: "api",
            name: "vllm".to_string(),
            detail: format!("{host}{model}"),
            via: "HEXA_VLLM_HOST".to_string(),
            reachable: probe(&host),
            models: env("HEXA_VLLM_MODEL").into_iter().collect(),
        });
    }

    // Endpoints registered with `hexa config inference add`.
    for e in endpoints {
        out.push(Found {
            kind: "registered",
            name: e.id.clone(),
            detail: format!("{} · {} · {}", e.url, e.provider, e.model),
            via: "~/.hexa/inference-servers.json".to_string(),
            reachable: probe(&e.url),
            models: {
                let mut m = e.models.clone();
                if !e.model.is_empty() && !m.contains(&e.model) {
                    m.push(e.model.clone());
                }
                m
            },
        });
    }

    // The frontier path: a logged-in claude CLI.
    if let Some(path) = claude_path {
        out.push(Found { kind: "frontier", name: "claude".to_string(), detail: path.to_string(), via: "PATH".to_string(), reachable: Some(true), models: Vec::new() });
    }
    out
}

/// What a configured tier's model resolves to among the discovered paths
/// (ADR-2609131617 §3).
#[derive(Debug, Clone, PartialEq)]
pub enum Coverage {
    /// A reachable path lists it; the name is that path's.
    Served(String),
    /// Every reachable path was enumerated and none lists it.
    NotServed,
    /// A reachable path could not be enumerated, so nothing can be said.
    /// The name is that path's.
    Unverified(String),
}

/// Does a frontier CLI answer for this model id?
fn frontier_serves(model: &str) -> bool {
    let m = model.to_ascii_lowercase();
    m.starts_with("claude") || m.starts_with("anthropic/") || m.starts_with("us.anthropic.")
}

/// Resolve `model` against the discovered paths. Never a substring match:
/// `registry::serving` makes the same point about routing, and a diagnosis
/// that guesses is the thing this replaces.
pub fn serves(found: &[Found], model: &str) -> Coverage {
    let reachable = || found.iter().filter(|f| f.reachable == Some(true));
    for f in reachable() {
        if f.models.iter().any(|m| m == model) {
            return Coverage::Served(f.name.clone());
        }
        if f.kind == "frontier" && frontier_serves(model) {
            return Coverage::Served(f.name.clone());
        }
    }
    // Nothing listed it. Only say so when every reachable path was asked.
    match reachable().find(|f| f.models.is_empty() && f.kind != "frontier") {
        Some(f) => Coverage::Unverified(f.name.clone()),
        None => Coverage::NotServed,
    }
}

/// Every model the reachable paths list, for the line that says what is
/// actually on offer.
pub fn served_models(found: &[Found]) -> Vec<String> {
    let mut out: Vec<String> = found
        .iter()
        .filter(|f| f.reachable == Some(true))
        .flat_map(|f| f.models.iter().cloned())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The names in an Ollama `/api/tags` body.
fn tags_models(v: &serde_json::Value) -> Vec<String> {
    v.get("models")
        .and_then(|m| m.as_array())
        .map(|a| a.iter().filter_map(|m| m.get("name").and_then(|n| n.as_str()).map(String::from)).collect())
        .unwrap_or_default()
}

/// The ids in an OpenAI-compatible `/v1/models` body.
fn openai_models(v: &serde_json::Value) -> Vec<String> {
    v.get("data")
        .and_then(|m| m.as_array())
        .map(|a| a.iter().filter_map(|m| m.get("id").and_then(|n| n.as_str()).map(String::from)).collect())
        .unwrap_or_default()
}

/// Ask a runtime what it serves: Ollama's `/api/tags`, then an
/// OpenAI-compatible `/v1/models`. `None` when neither answers, which is
/// what makes the tier unverified rather than unserved (ADR-2609131617 §2).
pub async fn enumerate_models(base_url: &str) -> Option<Vec<String>> {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(3)).build().ok()?;
    let base = base_url.trim_end_matches('/');
    for (path, pick) in [("/api/tags", tags_models as fn(&serde_json::Value) -> Vec<String>), ("/v1/models", openai_models)] {
        let Ok(resp) = client.get(format!("{base}{path}")).send().await else { continue };
        if !resp.status().is_success() {
            continue;
        }
        let Ok(json) = resp.json::<serde_json::Value>().await else { continue };
        let models = pick(&json);
        if !models.is_empty() {
            return Some(models);
        }
    }
    None
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

/// Does this authority already carry a port?
///
/// A bare `contains(':')` says yes for every IPv6 address, whose own colons
/// are not a port separator, so `http://[::1]` got no default appended and
/// `to_socket_addrs` then refused it — a reachable server reported
/// unreachable. In the bracket form the port, if any, follows the `]`.
/// Found by `hexa harden` on this file (ADR-2609131907).
fn has_port(hostport: &str) -> bool {
    match hostport.rfind(']') {
        Some(close) => hostport[close + 1..].starts_with(':'),
        None => hostport.contains(':'),
    }
}

/// A TCP connect with a short timeout; `None` when the URL has no host.
fn probe(url: &str) -> Option<bool> {
    let hostport = url.trim_start_matches("http://").trim_start_matches("https://");
    let hostport = hostport.split('/').next().unwrap_or("");
    let hostport = if has_port(hostport) {
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
mod has_port_tests {
    use super::has_port;

    /// An IPv6 address's own colons are not a port separator. `[::1]` with
    /// no port must take the default like any other host.
    #[test]
    fn an_ipv6_address_without_a_port_has_no_port() {
        assert!(!has_port("[::1]"), "the colons are the address");
        assert!(!has_port("[2001:db8::1]"));
        assert!(has_port("[::1]:11434"), "the port follows the bracket");
        assert!(has_port("[2001:db8::1]:443"));
    }

    #[test]
    fn an_ordinary_host_is_judged_by_its_colon() {
        assert!(!has_port("127.0.0.1"));
        assert!(!has_port("localhost"));
        assert!(has_port("127.0.0.1:7000"));
        assert!(has_port("localhost:11434"));
    }
}

#[cfg(test)]
mod serves_tests {
    use super::*;

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

    /// ADR-2609131617 §3: served names the backend; unreachable does not
    /// serve; nothing listing it, with everything enumerated, is not served.
    #[test]
    fn serves_is_answered_only_from_reachable_enumerated_paths() {
        let tt = path("registered", "tt-gptoss", Some(true), &["openai/gpt-oss-120b"]);
        assert_eq!(serves(std::slice::from_ref(&tt), "openai/gpt-oss-120b"), Coverage::Served("tt-gptoss".into()));
        assert_eq!(serves(std::slice::from_ref(&tt), "qwen3:4b"), Coverage::NotServed);

        let down = path("local", "Ollama", Some(false), &[]);
        assert_eq!(serves(&[down.clone(), tt.clone()], "qwen3:4b"), Coverage::NotServed, "an unreachable path serves nothing");
        assert_eq!(serves(&[path("registered", "x", Some(false), &["m"])], "m"), Coverage::NotServed);

        // Never a substring: a diagnosis that guesses is what this replaces.
        assert_eq!(serves(&[path("registered", "x", Some(true), &["meta/llama-3.3-70b"])], "llama-3"), Coverage::NotServed);
    }

    /// A reachable path nobody could enumerate makes the answer unverified,
    /// never "not served" and never "fine".
    #[test]
    fn an_unenumerated_reachable_path_is_unverified_not_either_answer() {
        let mystery = path("local", "Ollama", Some(true), &[]);
        let tt = path("registered", "tt-gptoss", Some(true), &["openai/gpt-oss-120b"]);
        assert_eq!(serves(&[mystery.clone(), tt.clone()], "qwen3:4b"), Coverage::Unverified("Ollama".into()));
        // It still does not mask a path that does list the model.
        assert_eq!(serves(&[mystery, tt], "openai/gpt-oss-120b"), Coverage::Served("tt-gptoss".into()));
    }

    /// A frontier CLI answers for its own family without being enumerated.
    #[test]
    fn a_claude_model_is_served_by_a_reachable_frontier() {
        let claude = path("frontier", "claude", Some(true), &[]);
        assert_eq!(serves(std::slice::from_ref(&claude), "claude-opus-5"), Coverage::Served("claude".into()));
        assert_eq!(serves(std::slice::from_ref(&claude), "anthropic/claude-sonnet-5"), Coverage::Served("claude".into()));
        assert_eq!(serves(&[claude], "qwen3:4b"), Coverage::NotServed, "a frontier does not answer for a local model");
    }

    /// Both model endpoints are parsed from the shape each really returns.
    #[test]
    fn both_model_endpoints_are_parsed() {
        let tags = serde_json::json!({"models":[{"name":"qwen3:4b"},{"name":"gemma4-12b"}]});
        assert_eq!(tags_models(&tags), vec!["qwen3:4b".to_string(), "gemma4-12b".to_string()]);
        let oai = serde_json::json!({"object":"list","data":[{"id":"openai/gpt-oss-120b","object":"model"}]});
        assert_eq!(openai_models(&oai), vec!["openai/gpt-oss-120b".to_string()]);
        // A body of the other shape yields nothing rather than a wrong list.
        assert!(tags_models(&oai).is_empty());
        assert!(openai_models(&tags).is_empty());
    }

    #[test]
    fn served_models_lists_what_the_reachable_paths_offer() {
        let found = vec![
            path("registered", "tt", Some(true), &["openai/gpt-oss-120b"]),
            path("local", "Ollama", Some(false), &["qwen3:4b"]),
            path("api", "vllm", Some(true), &["mistral-7b"]),
        ];
        assert_eq!(served_models(&found), vec!["mistral-7b".to_string(), "openai/gpt-oss-120b".to_string()]);
    }
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
