//! GitHub through the `gh` CLI and its own authentication (ADR-2611031914 §1, §7). `gh` is the CLI's path, a recording stub in tests.

use std::path::Path;

use hexa_core::ports::forge::{ForgeError, ForgeTarget, IssueDraft, IssueRef, PrDraft, PrRef};

use crate::ports::Forge;

/// GitHub through the `gh` CLI and its own authentication (ADR-2611031914 §1, §7). `gh` is the CLI's path, a recording stub in tests.
pub struct ForgeGithub {
    _private: (),
}

impl ForgeGithub {
    pub fn new(target: &ForgeTarget, gh: &Path) -> ForgeGithub {
        let _ = (target, gh);
        todo!("ADR-2611031914")
    }
}

impl Forge for ForgeGithub {
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
