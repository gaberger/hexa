//! The repository's forge (ADR-2611031914 §7). The values and pure functions
//! are the domain's (`domain::forge`); the `Forge` trait that reaches a forge
//! is in `hexa-exec`'s ports, beside the other driven ports.
pub use crate::domain::forge::{
    forge_base_url, forge_for_remote, issue_from_memory, key_marker, looks_secret, marker_is_last_line, pr_body, remote_host, remote_password, valid_env_name, valid_key, ForgeError, ForgeHost, ForgeKind,
    ForgeTarget, IssueDraft, IssueRef, PrDraft, PrRef,
};
