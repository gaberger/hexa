//! Filing an issue and opening a pull request, over the ports only
//! (ADR-2611031914 §4, §5). The forge is the ledger; memory is a cache.
//!
//! - "Filed?" is answered by the marker on the forge. `issue:<key>` only saves
//!   a lookup, so losing it costs one search, never a duplicate.
//! - A create happens only after a listing proved the marker absent.
//! - A create whose outcome is unknown is never retried: hexa looks again.

use hexa_core::ports::forge::{ForgeError, IssueDraft, PrDraft};

use crate::ports::{Forge, MemoryScope, MemoryStore};

/// The memory key an issue's URL is recorded under.
pub fn link_key(key: &str) -> String {
    format!("issue:{key}")
}

/// What filing one key did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Filed {
    /// The forge already had it; nothing was opened.
    Found(String),
    /// It was opened now.
    Opened(String),
}

impl Filed {
    pub fn url(&self) -> &str {
        match self {
            Filed::Found(u) | Filed::Opened(u) => u,
        }
    }
}

/// Why filing one key did not finish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileError {
    /// The forge said no, or could not be asked. Nothing was opened.
    Forge(ForgeError),
    /// A create was sent and its outcome is unknown; a second look did not
    /// find it. Nothing was confirmed created, so rerunning is safe.
    Unconfirmed(ForgeError),
    /// The issue is on the forge, but its link was not recorded.
    NotRecorded { filed: Filed, why: String },
}

/// Find `d.key`'s issue, open it only when absent, and record its URL under
/// `issue:<key>` in project memory.
pub fn file_issue(forge: &dyn Forge, store: &dyn MemoryStore, d: &IssueDraft) -> Result<Filed, FileError> {
    let filed = match forge.find_issue(&d.key).map_err(FileError::Forge)? {
        Some(found) => Filed::Found(found.url),
        None => match forge.open_issue(d) {
            Ok(opened) => Filed::Opened(opened.url),
            Err(e @ ForgeError::Unreachable(_)) => match forge.find_issue(&d.key) {
                Ok(Some(found)) => Filed::Opened(found.url),
                _ => return Err(FileError::Unconfirmed(e)),
            },
            Err(e) => return Err(FileError::Forge(e)),
        },
    };
    match store.put(MemoryScope::Project, &link_key(&d.key), filed.url()) {
        Ok(()) => Ok(filed),
        Err(why) => Err(FileError::NotRecorded { filed, why }),
    }
}

/// The keys `hexa issue sync` would file, from one snapshot of `entries`:
/// those under `prefix` (default `bug:` and `gap:`), valid, and with no link.
pub fn unfiled_keys(entries: &[(String, String)], prefix: Option<&str>) -> Vec<String> {
    let has = |k: &str| entries.iter().any(|(e, _)| e == k);
    let mut keys: Vec<String> = entries
        .iter()
        .map(|(k, _)| k.clone())
        .filter(|k| !k.starts_with("issue:"))
        .filter(|k| match prefix {
            Some(p) => k.starts_with(p),
            None => k.starts_with("bug:") || k.starts_with("gap:"),
        })
        .filter(|k| hexa_core::ports::forge::valid_key(k))
        .filter(|k| !has(&link_key(k)))
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

/// What opening a pull request did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opened {
    /// One from this branch was open already.
    Existing(String),
    Created(String),
}

/// Open `d` unless the forge already has an open pull request from `d.head`.
/// A create refused as a duplicate, or lost, is settled by looking again.
pub fn open_pr(forge: &dyn Forge, d: &PrDraft) -> Result<Opened, ForgeError> {
    if let Some(found) = forge.find_pr(&d.head)? {
        return Ok(Opened::Existing(found.url));
    }
    match forge.open_pr(d) {
        Ok(pr) => Ok(Opened::Created(pr.url)),
        Err(e @ (ForgeError::Refused(_) | ForgeError::Unreachable(_))) => match forge.find_pr(&d.head) {
            Ok(Some(found)) => Ok(Opened::Existing(found.url)),
            _ => Err(e),
        },
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unfiled_keys_skip_links_other_prefixes_and_bad_keys() {
        let e: Vec<(String, String)> = [
            ("bug:a", "x"),
            ("bug:b", "x"),
            ("issue:bug:b", "u"),
            ("gap:c", "x"),
            ("note:d", "x"),
            ("bug:bad--key", "x"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        assert_eq!(unfiled_keys(&e, None), vec!["bug:a", "gap:c"]);
        assert_eq!(unfiled_keys(&e, Some("gap:")), vec!["gap:c"]);
    }
}
