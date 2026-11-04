//! Forgejo or Gitea through the REST API v1, with the token read from `target.token_env` at call time (ADR-2611031914 §1, §7). `base_url` is `https://<host>` in use and a local test server in tests; a missing or empty token variable is `Unauthorized` naming the variable.

use hexa_core::ports::forge::{ForgeError, ForgeTarget, IssueDraft, IssueRef, PrDraft, PrRef};

use crate::ports::Forge;

/// Forgejo or Gitea through the REST API v1, with the token read from `target.token_env` at call time (ADR-2611031914 §1, §7). `base_url` is `https://<host>` in use and a local test server in tests; a missing or empty token variable is `Unauthorized` naming the variable.
pub struct ForgeForgejo {
    _private: (),
}

impl ForgeForgejo {
    pub fn new(target: &ForgeTarget, base_url: &str) -> Result<ForgeForgejo, ForgeError> {
        let _ = (target, base_url);
        todo!("ADR-2611031914")
    }
}

impl Forge for ForgeForgejo {
    fn find_issue(&self, key: &str) -> Result<Option<IssueRef>, ForgeError> {
        let _ = key;
        todo!("ADR-2611031914")
    }
    fn open_issue(&self, d: &IssueDraft) -> Result<IssueRef, ForgeError> {
        let _ = d;
        todo!("ADR-2611031914")
    }
    fn find_pr(&self, head: &str) -> Result<Option<PrRef>, ForgeError> {
        let _ = head;
        todo!("ADR-2611031914")
    }
    fn open_pr(&self, d: &PrDraft) -> Result<PrRef, ForgeError> {
        let _ = d;
        todo!("ADR-2611031914")
    }
}
