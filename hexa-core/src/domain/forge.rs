//! The repository's forge: where an issue is filed and a pull request is
//! opened (ADR-2611031914). Pure: the mapping from a remote to a forge, the
//! texts hexa sends, and the scan that refuses to send a secret. The forges
//! themselves are reached through the `Forge` port in `hexa-exec`.

use serde::{Deserialize, Serialize};

/// Which forge API a host speaks (§1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ForgeKind {
    /// github.com, through the `gh` CLI and its authentication.
    Github,
    /// Forgejo or Gitea, through REST API v1 and a token variable.
    Forgejo,
}

/// A host listed in `$HEXA_HOME/forges.json` (§2): its kind and the name of
/// the variable that holds its token, never the token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForgeHost {
    pub host: String,
    pub kind: ForgeKind,
    pub token_env: Option<String>,
}

/// The forge and repository a remote maps to (§2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForgeTarget {
    pub kind: ForgeKind,
    pub host: String,
    pub owner: String,
    pub repo: String,
    pub token_env: Option<String>,
}

/// An issue to file, keyed by the memory key or gap it came from (§4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueDraft {
    pub key: String,
    pub title: String,
    pub body: String,
}

/// An issue on the forge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueRef {
    pub number: u64,
    pub url: String,
    pub open: bool,
}

/// A pull request to open from a pushed branch (§5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrDraft {
    pub head: String,
    pub base: String,
    pub title: String,
    pub body: String,
    pub draft: bool,
}

/// A pull request on the forge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrRef {
    pub number: u64,
    pub url: String,
}

/// Why a forge call did not do what was asked. A message never holds a
/// token's value (§6).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ForgeError {
    #[error("the forge did not answer: {0}")]
    Unreachable(String),
    #[error("the forge refused the credential: {0}")]
    Unauthorized(String),
    #[error("not found on the forge: {0}")]
    NotFound(String),
    #[error("refused: {0}")]
    Refused(String),
}

/// The forge an `origin` URL (https or ssh form) maps to (§2): `github.com`
/// is GitHub, a host in `hosts` is its recorded kind, anything else `None`.
pub fn forge_for_remote(remote_url: &str, hosts: &[ForgeHost]) -> Option<ForgeTarget> {
    let _ = (remote_url, hosts);
    todo!("ADR-2611031914")
}

/// The marker that makes a recorded key's issue findable (§4).
pub fn key_marker(key: &str) -> String {
    let _ = key;
    todo!("ADR-2611031914")
}

/// The issue for a memory entry (§7): title the value's first sentence, at
/// most 80 characters; body the value, the project and the marker.
pub fn issue_from_memory(key: &str, value: &str, project: &str) -> IssueDraft {
    let _ = (key, value, project);
    todo!("ADR-2611031914")
}

/// A pull request body from the commit subjects and the loop's evidence (§5).
pub fn pr_body(commits: &[String], adr: Option<&str>, gate: Option<&str>, last_result: Option<&str>) -> String {
    let _ = (commits, adr, gate, last_result);
    todo!("ADR-2611031914")
}

/// The 1-based line of the first credential shape in `text`, or `None` (§6).
pub fn looks_secret(text: &str, token: Option<&str>) -> Option<usize> {
    let _ = (text, token);
    todo!("ADR-2611031914")
}
