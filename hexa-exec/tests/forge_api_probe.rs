//! ADR-2611031914 §7: every public item with its exact signature, written
//! from the ADR text, not from the code. It only has to compile: a
//! signature that drifts from the ADR stops the build here.

#![allow(dead_code, unused_variables)]

use std::path::Path;

use hexa_core::ports::forge::{
    forge_for_remote, issue_from_memory, key_marker, looks_secret, pr_body, ForgeError, ForgeHost, ForgeKind,
    ForgeTarget, IssueDraft, IssueRef, PrDraft, PrRef,
};
use hexa_exec::forge_forgejo::ForgeForgejo;
use hexa_exec::forge_github::ForgeGithub;
use hexa_exec::ports::Forge;

fn domain() {
    let _: fn(&str, &[ForgeHost]) -> Option<ForgeTarget> = forge_for_remote;
    let _: fn(&str) -> String = key_marker;
    let _: fn(&str, &str, &str) -> IssueDraft = issue_from_memory;
    let _: fn(&[String], Option<&str>, Option<&str>, Option<&str>) -> String = pr_body;
    let _: fn(&str, Option<&str>) -> Option<usize> = looks_secret;
    let _ = |k: ForgeKind| match k {
        ForgeKind::Github | ForgeKind::Forgejo => {}
    };
    let _ = |h: ForgeHost| -> (String, ForgeKind, Option<String>) { (h.host, h.kind, h.token_env) };
    let _ = |t: ForgeTarget| -> (ForgeKind, String, String, String, Option<String>) {
        (t.kind, t.host, t.owner, t.repo, t.token_env)
    };
    let _ = |d: IssueDraft| -> (String, String, String) { (d.key, d.title, d.body) };
    let _ = |r: IssueRef| -> (u64, String, bool) { (r.number, r.url, r.open) };
    let _ = |d: PrDraft| -> (String, String, String, String, bool) { (d.head, d.base, d.title, d.body, d.draft) };
    let _ = |r: PrRef| -> (u64, String) { (r.number, r.url) };
    let _ = |e: ForgeError| match e {
        ForgeError::Unreachable(s) | ForgeError::Unauthorized(s) | ForgeError::NotFound(s) | ForgeError::Refused(s) => s,
    };
}

fn port<F: Forge>(f: &F, d: &IssueDraft, p: &PrDraft) {
    let _: Result<Option<IssueRef>, ForgeError> = f.find_issue("bug:x");
    let _: Result<IssueRef, ForgeError> = f.open_issue(d);
    let _: Result<Option<PrRef>, ForgeError> = f.find_pr("feat/x");
    let _: Result<PrRef, ForgeError> = f.open_pr(p);
}

fn adapters(t: &ForgeTarget) {
    let _: Result<ForgeForgejo, ForgeError> = ForgeForgejo::new(t, "http://127.0.0.1:1");
    let _: ForgeGithub = ForgeGithub::new(t, Path::new("/usr/bin/false"));
    fn is_forge<T: Forge>() {}
    is_forge::<ForgeForgejo>();
    is_forge::<ForgeGithub>();
}

#[test]
fn adr_2611031914_section_7_items_exist_with_their_signatures() {
    // Compiling is the check; the functions above are never called.
    let _ = (domain as fn(), adapters as fn(&ForgeTarget));
}
