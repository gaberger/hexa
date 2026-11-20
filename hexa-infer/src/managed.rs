//! Managed mode (ADR-2610281500): hexa driven by a harness that owns every model call.
//!
//! `HEXA_MANAGED=1` enters it. The settings come from the environment, the token from an
//! inherited file descriptor, and every completion goes to the harness's URL, routed by the
//! model's `api` in the tier map. Nothing else hexa knows about a provider applies.

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use hexa_core::ports::inference::{IInferencePort, InferenceError, InferenceRequest, InferenceResponse};

use crate::adapters::secondary::{AnthropicAdapter, OpenAiCompatAdapter};

/// The waits between retries of a connection error or a 5xx (§5).
pub const RETRY_DELAYS: [Duration; 5] = [
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(16),
    Duration::from_secs(32),
];

/// The largest token the fd may carry (§1).
const MAX_TOKEN_BYTES: usize = 4096;

/// The five tiers the map must name, no more and no fewer.
const TIERS: [&str; 5] = ["t1", "t2", "t2.5", "t3", "react"];

/// Which wire shape a model speaks at the managed URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Api {
    Anthropic,
    OpenAi,
}

/// One tier's entry in `HEXA_MANAGED_TIERS`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TierEntry {
    pub model: String,
    pub api: Api,
}

/// A missing or malformed managed setting: the variable, and what is wrong with it.
/// It never carries the token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedError {
    pub var: String,
    pub reason: String,
}

impl fmt::Display for ManagedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.var, self.reason)
    }
}

impl std::error::Error for ManagedError {}

fn bad(var: &str, reason: impl Into<String>) -> ManagedError {
    ManagedError { var: var.to_string(), reason: reason.into() }
}

/// The managed-mode settings.
pub struct Managed {
    /// The harness's URL, without a trailing `/`.
    pub url: String,
    pub token: String,
    pub state_dir: PathBuf,
    /// Unix seconds.
    pub deadline: Option<u64>,
    tiers: BTreeMap<String, TierEntry>,
    delays: Vec<Duration>,
}

impl fmt::Debug for Managed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Managed")
            .field("url", &self.url)
            .field("token", &"<redacted>")
            .field("state_dir", &self.state_dir)
            .field("deadline", &self.deadline)
            .field("tiers", &self.tiers)
            .field("delays", &self.delays)
            .finish()
    }
}

impl Managed {
    /// The settings, read through `read` (an environment) and `read_fd` (the token's fd).
    ///
    /// `Ok(None)` unless `HEXA_MANAGED` is exactly `1`. The token is never read from a variable.
    pub fn from_env(
        read: &dyn Fn(&str) -> Option<String>,
        read_fd: &dyn Fn(i32) -> std::io::Result<Vec<u8>>,
    ) -> Result<Option<Managed>, ManagedError> {
        if read("HEXA_MANAGED").as_deref() != Some("1") {
            return Ok(None);
        }
        if read("HEXA_MANAGED_DELEGATE").is_some() {
            return Err(bad("HEXA_MANAGED_DELEGATE", "managed mode never runs the claude delegate; unset it"));
        }
        let url = parse_url(read("HEXA_MANAGED_URL"))?;
        let token = read_token(read("HEXA_MANAGED_TOKEN_FD"), read_fd)?;
        let tiers = parse_tiers(read("HEXA_MANAGED_TIERS"))?;
        let state_dir = match read("HEXA_STATE_DIR") {
            None => return Err(bad("HEXA_STATE_DIR", "not set")),
            Some(s) if s.is_empty() => return Err(bad("HEXA_STATE_DIR", "empty")),
            Some(s) => {
                let p = PathBuf::from(s);
                if !p.is_absolute() {
                    return Err(bad("HEXA_STATE_DIR", "not an absolute path"));
                }
                p
            }
        };
        let deadline = match read("HEXA_MANAGED_DEADLINE") {
            None => None,
            Some(s) => Some(
                s.parse::<u64>()
                    .map_err(|_| bad("HEXA_MANAGED_DEADLINE", format!("'{s}' is not Unix seconds")))?,
            ),
        };
        Ok(Some(Managed { url, token, state_dir, deadline, tiers, delays: RETRY_DELAYS.to_vec() }))
    }

    /// The entry for a tier (`t1`, `t2`, `t2.5`, `t3`, `react`).
    pub fn tier(&self, name: &str) -> Option<&TierEntry> {
        self.tiers.get(name)
    }

    /// The API of a model any tier lists.
    pub fn api_of(&self, model: &str) -> Option<Api> {
        self.tiers.values().find(|t| t.model == model).map(|t| t.api)
    }

    /// Replace the retry waits (tests use short ones).
    pub fn with_retry_delays(mut self, d: Vec<Duration>) -> Managed {
        self.delays = d;
        self
    }

    /// Send one request to the managed URL, routed by its model's API (§3), retried on a
    /// connection error or a 5xx and never on a 401 or 402 (§5).
    pub async fn complete(&self, request: InferenceRequest) -> Result<InferenceResponse, InferenceError> {
        let Some(api) = self.api_of(&request.model) else {
            return Err(InferenceError::UnknownProvider(format!(
                "model '{}' is not in HEXA_MANAGED_TIERS; managed mode calls no other model",
                request.model
            )));
        };
        let adapter: Box<dyn IInferencePort> = match api {
            Api::Anthropic => Box::new(
                AnthropicAdapter::new(self.token.clone(), request.model.clone()).with_base_url(self.url.clone()),
            ),
            Api::OpenAi => Box::new(OpenAiCompatAdapter::new(
                self.token.clone(),
                format!("{}/v1", self.url),
                request.model.clone(),
            )),
        };
        let mut attempt = 0;
        loop {
            match adapter.complete(request.clone()).await {
                Err(e) if retryable(&e) && attempt < self.delays.len() => {
                    tracing::warn!(attempt = attempt + 1, error = %e, "managed completion failed; retrying");
                    tokio::time::sleep(self.delays[attempt]).await;
                    attempt += 1;
                }
                other => return other,
            }
        }
    }
}

/// A connection failure or a server error: worth another try.
fn retryable(e: &InferenceError) -> bool {
    match e {
        InferenceError::Network(_) | InferenceError::ProviderUnavailable(_) => true,
        InferenceError::ApiError { status, .. } => *status >= 500,
        _ => false,
    }
}

/// The result code of a failed managed completion (§5, §9).
pub fn code_for(e: &InferenceError) -> &'static str {
    match e {
        InferenceError::ApiError { status: 401, .. } => "auth",
        InferenceError::ApiError { status: 402, .. } => "budget",
        _ => "model_error",
    }
}

fn parse_url(v: Option<String>) -> Result<String, ManagedError> {
    const VAR: &str = "HEXA_MANAGED_URL";
    let Some(raw) = v else { return Err(bad(VAR, "not set")) };
    let Some((scheme, rest)) = raw.split_once("://") else {
        return Err(bad(VAR, "not a URL"));
    };
    if scheme != "http" && scheme != "https" {
        return Err(bad(VAR, "the scheme must be http or https"));
    }
    if rest.contains('?') || rest.contains('#') {
        return Err(bad(VAR, "a query or fragment is not allowed"));
    }
    let host = rest.strip_suffix('/').unwrap_or(rest);
    if host.is_empty() {
        return Err(bad(VAR, "no host"));
    }
    if host.contains('/') {
        return Err(bad(VAR, "a path beyond / is not allowed"));
    }
    Ok(format!("{scheme}://{host}"))
}

fn read_token(
    v: Option<String>,
    read_fd: &dyn Fn(i32) -> std::io::Result<Vec<u8>>,
) -> Result<String, ManagedError> {
    const VAR: &str = "HEXA_MANAGED_TOKEN_FD";
    let Some(raw) = v else { return Err(bad(VAR, "not set; the token is read only from an inherited fd")) };
    let fd = match raw.parse::<i32>() {
        Ok(fd) if fd >= 3 && raw.bytes().all(|b| b.is_ascii_digit()) => fd,
        _ => return Err(bad(VAR, format!("'{raw}' is not a decimal fd of at least 3"))),
    };
    let bytes = read_fd(fd).map_err(|e| bad(VAR, format!("cannot read fd {fd}: {e}")))?;
    let bytes = bytes.strip_suffix(b"\n").unwrap_or(&bytes);
    if bytes.is_empty() {
        return Err(bad(VAR, "the token is empty"));
    }
    if bytes.len() > MAX_TOKEN_BYTES {
        return Err(bad(VAR, "the token is over 4 KiB"));
    }
    String::from_utf8(bytes.to_vec()).map_err(|_| bad(VAR, "the token is not UTF-8"))
}

fn parse_tiers(v: Option<String>) -> Result<BTreeMap<String, TierEntry>, ManagedError> {
    const VAR: &str = "HEXA_MANAGED_TIERS";
    let Some(raw) = v else { return Err(bad(VAR, "not set")) };
    let value: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| bad(VAR, format!("not JSON: {e}")))?;
    let Some(obj) = value.as_object() else { return Err(bad(VAR, "not a JSON object")) };
    if let Some(k) = obj.keys().find(|k| !TIERS.contains(&k.as_str())) {
        return Err(bad(VAR, format!("unknown tier '{k}'")));
    }
    let mut out = BTreeMap::new();
    for tier in TIERS {
        let Some(entry) = obj.get(tier) else { return Err(bad(VAR, format!("tier '{tier}' is missing"))) };
        let Some(entry) = entry.as_object() else {
            return Err(bad(VAR, format!("tier '{tier}' is not an object")));
        };
        if let Some(k) = entry.keys().find(|k| *k != "model" && *k != "api") {
            return Err(bad(VAR, format!("tier '{tier}' has an unknown key '{k}'")));
        }
        let model = match entry.get("model").and_then(|m| m.as_str()) {
            Some(m) if !m.is_empty() => m.to_string(),
            _ => return Err(bad(VAR, format!("tier '{tier}' needs a non-empty \"model\""))),
        };
        let api = match entry.get("api").and_then(|a| a.as_str()) {
            Some("anthropic") => Api::Anthropic,
            Some("openai") => Api::OpenAi,
            _ => return Err(bad(VAR, format!("tier '{tier}' needs \"api\": \"anthropic\" or \"openai\""))),
        };
        out.insert(tier.to_string(), TierEntry { model, api });
    }
    Ok(out)
}

static ACTIVE: OnceLock<Result<Option<Managed>, ManagedError>> = OnceLock::new();

/// The process's managed settings, read once from the real environment and token fd.
/// `None` outside managed mode, and when the settings are bad (see [`active_or_error`]).
pub fn active() -> Option<&'static Managed> {
    active_or_error().ok().flatten()
}

/// [`active`], with a bad setting kept as the error it is.
pub fn active_or_error() -> Result<Option<&'static Managed>, &'static ManagedError> {
    ACTIVE
        .get_or_init(|| {
            let parsed = Managed::from_env(&|k: &str| std::env::var(k).ok(), &read_inherited_fd);
            if let Ok(Some(_)) = parsed {
                forbid_dumping();
            }
            parsed
        })
        .as_ref()
        .map(Option::as_ref)
}

unsafe extern "C" {
    fn fcntl(fd: i32, cmd: i32, ...) -> i32;
    fn prctl(option: i32, ...) -> i32;
}

/// Read an inherited fd to EOF, and close it.
fn read_inherited_fd(fd: i32) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    use std::os::fd::FromRawFd;
    const F_GETFD: i32 = 1;
    // SAFETY: F_GETFD only queries the descriptor's flags; it tells us the fd is open before we
    // take ownership of it.
    if unsafe { fcntl(fd, F_GETFD) } == -1 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: the fd is open and was handed to this process for the token alone; nothing else
    // owns it. The File closes it when dropped.
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    let mut out = Vec::new();
    file.by_ref().take(MAX_TOKEN_BYTES as u64 + 2).read_to_end(&mut out)?;
    Ok(out)
}

/// PR_SET_DUMPABLE=0: no core dump, and no ptrace or /proc/self/mem by the same user (§7).
fn forbid_dumping() {
    const PR_SET_DUMPABLE: i32 = 4;
    // SAFETY: prctl(PR_SET_DUMPABLE, 0) changes only this process's dumpable flag.
    unsafe {
        prctl(PR_SET_DUMPABLE, 0u64);
    }
}
