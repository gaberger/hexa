//! `hexa adr accept` (and the other transition verbs) update both places an
//! ADR states its status, in the style each already uses.
//!
//! Observed 2026-10-16 on ADR-2610160756: `hexa adr accept` rewrote the
//! frontmatter's `status: proposed` as `Status: Accepted` — the key's case
//! changed, and the value's case no longer matched the other 18 ADRs — and it
//! left the body's `**Status:** Proposed` as it was. The ledger then
//! disagreed with itself, and it was corrected by hand.

use std::path::{Path, PathBuf};
use std::process::Command;

const ID: &str = "ADR-2610160001";

struct Ledger {
    _tmp: tempfile::TempDir,
    root: PathBuf,
}

impl Ledger {
    fn with(adr: &str) -> Ledger {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        std::fs::create_dir_all(root.join("docs/adrs")).unwrap();
        std::fs::write(root.join(format!("docs/adrs/{ID}-a-decision.md")), adr).unwrap();
        // The replacement a supersede points at.
        std::fs::write(
            root.join("docs/adrs/ADR-2610160002-the-replacement.md"),
            "---\nid: ADR-2610160002\nstatus: accepted\ndate: 2026-10-16\n---\n# ADR-2610160002: The replacement\n\n**Status:** Accepted\n",
        )
        .unwrap();
        Ledger { _tmp: tmp, root }
    }

    fn hexa(&self, args: &[&str]) -> String {
        let out = Command::new(env!("CARGO_BIN_EXE_hexa"))
            .args(args)
            .current_dir(&self.root)
            .env("HOME", self.root.join("home"))
            .output()
            .expect("run hexa");
        let text = String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "hexa {args:?} failed: {text}");
        text
    }

    fn adr(&self) -> String {
        std::fs::read_to_string(adr_path(&self.root)).unwrap()
    }
}

fn adr_path(root: &Path) -> PathBuf {
    root.join(format!("docs/adrs/{ID}-a-decision.md"))
}

/// The shape every ADR in this ledger has: lowercase frontmatter, bold body.
const PROPOSED: &str = "---\nid: ADR-2610160001\nstatus: proposed\ndate: 2026-10-16\n---\n# ADR-2610160001: A decision\n\n**Status:** Proposed\n**Date:** 2026-10-16\n\n## Context\n\nThe status of the thing is unclear.\n";

fn lines(text: &str) -> Vec<&str> {
    text.lines().collect()
}

#[test]
fn accept_rewrites_the_frontmatter_and_the_body_in_their_own_styles() {
    let l = Ledger::with(PROPOSED);
    l.hexa(&["adr", "accept", ID]);
    let adr = l.adr();

    assert!(lines(&adr).contains(&"status: accepted"), "frontmatter keeps its lowercase key and value:\n{adr}");
    assert!(!lines(&adr).contains(&"Status: Accepted"), "the frontmatter key is not recased:\n{adr}");
    assert!(lines(&adr).contains(&"**Status:** Accepted"), "the body line moves too:\n{adr}");
    assert!(!adr.contains("Proposed") && !adr.contains("proposed"), "no line still says proposed:\n{adr}");
    assert!(adr.contains("The status of the thing is unclear."), "prose is left alone:\n{adr}");
    assert_eq!(adr.lines().count(), PROPOSED.lines().count(), "no line added or lost:\n{adr}");
}

#[test]
fn complete_rewrites_both_lines() {
    let l = Ledger::with(&PROPOSED.replace("status: proposed", "status: accepted").replace("**Status:** Proposed", "**Status:** Accepted"));
    l.hexa(&["adr", "complete", ID, "--force"]);
    let adr = l.adr();

    assert!(lines(&adr).contains(&"status: completed"), "{adr}");
    assert!(lines(&adr).contains(&"**Status:** Completed"), "{adr}");
}

#[test]
fn supersede_rewrites_both_lines_and_links_the_replacement_once() {
    let l = Ledger::with(&PROPOSED.replace("status: proposed", "status: accepted").replace("**Status:** Proposed", "**Status:** Accepted"));
    l.hexa(&["adr", "supersede", ID, "--by", "ADR-2610160002"]);
    let adr = l.adr();

    assert!(lines(&adr).contains(&"status: superseded"), "{adr}");
    assert!(lines(&adr).contains(&"**Status:** Superseded"), "{adr}");
    assert_eq!(adr.matches("ADR-2610160002").count(), 1, "one backlink:\n{adr}");
}

#[test]
fn a_capitalised_frontmatter_keeps_its_case() {
    let adr = PROPOSED.replace("status: proposed", "Status: Proposed");
    let l = Ledger::with(&adr);
    l.hexa(&["adr", "accept", ID]);
    let adr = l.adr();

    assert!(lines(&adr).contains(&"Status: Accepted"), "{adr}");
    assert!(lines(&adr).contains(&"**Status:** Accepted"), "{adr}");
}

#[test]
fn a_body_only_adr_still_transitions() {
    let adr = "# ADR-2610160001: A decision\n\n**Status:** Proposed\n**Date:** 2026-10-16\n";
    let l = Ledger::with(adr);
    l.hexa(&["adr", "accept", ID]);
    let adr = l.adr();

    assert!(lines(&adr).contains(&"**Status:** Accepted"), "{adr}");
    assert!(!adr.contains("---"), "no frontmatter is invented:\n{adr}");
}
