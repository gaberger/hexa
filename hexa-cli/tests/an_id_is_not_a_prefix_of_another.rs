//! The decision checker runs in CI, and resolves the id shapes it claims to.
//!
//! `main` went registry-inconsistent under green CI because nothing ran
//! `hexa adr doctor`. A checker nothing runs is documentation.
//!
//! Every id here is assembled at run time. Written whole, they would read as
//! citations to this file — the checker scans source, and a test that writes
//! `ADR-9999` is claiming a decision exists. That trap caught this session four
//! times, in two docs and two source comments, which is why it is called out
//! rather than worked around quietly.

use std::path::Path;
use std::process::Command;

fn hexa() -> std::path::PathBuf {
    let mut p = std::env::current_exe().expect("test exe");
    p.pop();
    p.pop();
    p.push("hexa");
    p
}

fn run(root: &Path, args: &[&str]) -> (i32, String) {
    let out = Command::new(hexa()).args(args).current_dir(root).output().expect("run hexa");
    (
        out.status.code().unwrap_or(-1),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

/// `ADR-` plus the digits given, built at run time (see the module note).
fn id(digits: &str) -> String {
    format!("ADR-{digits}")
}

/// A project whose `docs/adrs/` holds one ADR with `file_digits`, and whose
/// source cites `cite_digits`.
fn project(file_digits: Option<&str>, cite_digits: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().expect("tempdir");
    let r = d.path();
    std::fs::create_dir_all(r.join("docs/adrs")).unwrap();
    std::fs::create_dir_all(r.join("src")).unwrap();
    if let Some(fd) = file_digits {
        let f = id(fd);
        std::fs::write(
            r.join("docs/adrs").join(format!("{f}-a-decision.md")),
            format!("---\nid: {f}\nstatus: accepted\ndate: 2026-01-01\n---\n# {f}: a decision\n"),
        )
        .unwrap();
    }
    std::fs::write(
        r.join("src/lib.rs"),
        format!("// see {} for why\npub fn f() {{}}\n", id(cite_digits)),
    )
    .unwrap();
    d
}

// ── The shapes the checker claims to resolve ─────────────────────────────────

#[test]
fn a_three_digit_id_resolves() {
    let d = project(Some("001"), "001");
    let (code, out) = run(d.path(), &["adr", "doctor"]);
    assert!(!out.contains("DanglingCitation"), "{out}");
    assert_eq!(code, 0, "{out}");
}

#[test]
fn a_timestamp_id_resolves() {
    let d = project(Some("2609121400"), "2609121400");
    let (_, out) = run(d.path(), &["adr", "doctor"]);
    assert!(!out.contains("DanglingCitation"), "{out}");
}

#[test]
fn a_dated_id_resolves() {
    let d = project(Some("2026-04-15-0100"), "2026-04-15-0100");
    let (_, out) = run(d.path(), &["adr", "doctor"]);
    assert!(!out.contains("DanglingCitation"), "{out}");
}

#[test]
fn a_genuinely_missing_id_is_reported() {
    // The control that stops the three above passing by finding nothing at all.
    let d = project(Some("001"), "2699999999");
    let (code, out) = run(d.path(), &["adr", "doctor"]);
    assert!(out.contains("DanglingCitation"), "{out}");
    assert_ne!(code, 0, "a dangling citation must fail the build:\n{out}");
}

// ── The blind spot (ADR-2609221830) ──────────────────────────────────────────

#[test]
fn a_four_digit_dangling_citation_is_reported() {
    // The headline case. It matched on its first three digits, the guard saw
    // the fourth and dropped the whole match, so the checker said "registry is
    // consistent" about a file it never read.
    let d = project(Some("001"), "0042");
    let (code, out) = run(d.path(), &["adr", "doctor"]);
    assert!(out.contains("DanglingCitation"), "a four-digit id that dangles is a finding:\n{out}");
    assert_ne!(code, 0, "{out}");
}

#[test]
fn the_finding_names_the_id_that_was_written() {
    let d = project(Some("001"), "0042");
    let (_, out) = run(d.path(), &["adr", "doctor"]);
    assert!(out.contains(&id("0042")), "not a prefix of it:\n{out}");
}

#[test]
fn a_four_digit_id_with_a_file_resolves() {
    let d = project(Some("0001"), "0001");
    let (code, out) = run(d.path(), &["adr", "doctor"]);
    assert!(!out.contains("DanglingCitation"), "{out}");
    assert_eq!(code, 0, "{out}");
}

#[test]
fn a_placeholder_is_not_a_citation() {
    // Digits never filled in, letters in their place. Matching the digit run
    // and stopping at the letter invents a citation to a decision nobody wrote
    // — which is what the first attempt at this fix did, seven times.
    let d = tempfile::tempdir().expect("tempdir");
    let r = d.path();
    std::fs::create_dir_all(r.join("docs/adrs")).unwrap();
    std::fs::create_dir_all(r.join("src")).unwrap();
    let real = id("001");
    std::fs::write(
        r.join("docs/adrs").join(format!("{real}-a-decision.md")),
        format!("---\nid: {real}\nstatus: accepted\ndate: 2026-01-01\n---\n# {real}: a decision\n"),
    )
    .unwrap();
    // `ADR-2606071` followed by three letters, assembled so this file does not
    // itself cite anything.
    let placeholder = format!("{}{}", id("2606071"), "X".repeat(3));
    std::fs::write(r.join("src/lib.rs"), format!("//! see {placeholder}\npub fn f() {{}}\n")).unwrap();
    let (code, out) = run(r, &["adr", "doctor"]);
    assert!(!out.contains("DanglingCitation"), "a placeholder is not a citation:\n{out}");
    assert_eq!(code, 0, "{out}");
}

#[test]
fn a_fixture_in_test_code_is_not_a_citation() {
    // A unit test writing a short id is data for an assertion, not a claim
    // that a decision exists. Without this the checker reports 51 findings
    // about its own test data, and gets switched off.
    let d = tempfile::tempdir().expect("tempdir");
    let r = d.path();
    std::fs::create_dir_all(r.join("docs/adrs")).unwrap();
    std::fs::create_dir_all(r.join("src")).unwrap();
    std::fs::create_dir_all(r.join("tests")).unwrap();
    let real = id("001");
    std::fs::write(
        r.join("docs/adrs").join(format!("{real}-a-decision.md")),
        format!("---\nid: {real}\nstatus: accepted\ndate: 2026-01-01\n---\n# {real}: a decision\n"),
    )
    .unwrap();
    // One in an integration test, one in a unit-test module beside the code.
    std::fs::write(r.join("tests/it.rs"), format!("// fixture {}\n", id("7777"))).unwrap();
    std::fs::write(
        r.join("src/lib.rs"),
        format!(
            "pub fn f() {{}}\n\n#[cfg(test)]\nmod tests {{\n    // fixture {}\n    #[test]\n    fn t() {{}}\n}}\n",
            id("8888")
        ),
    )
    .unwrap();
    let (code, out) = run(r, &["adr", "doctor"]);
    assert!(!out.contains("DanglingCitation"), "test code is not scanned for citations:\n{out}");
    assert_eq!(code, 0, "{out}");
}

#[test]
fn an_id_in_a_fenced_code_block_is_not_a_citation() {
    // Sample input, sample output, a configuration illustration. Three real
    // sites were exactly this, two of them records that must not be edited to
    // suit a checker: an example inside an accepted ADR, which is append-only,
    // and a quoted terminal transcript in a case study.
    let d = tempfile::tempdir().expect("tempdir");
    let r = d.path();
    std::fs::create_dir_all(r.join("docs/adrs")).unwrap();
    let real = id("001");
    std::fs::write(
        r.join("docs/adrs").join(format!("{real}-a-decision.md")),
        format!("---\nid: {real}\nstatus: accepted\ndate: 2026-01-01\n---\n# {real}: a decision\n"),
    )
    .unwrap();
    std::fs::write(
        r.join("docs/guide.md"),
        format!("# Guide\n\n```toml\nadr = \"{}\"\n```\n", id("0000")),
    )
    .unwrap();
    let (code, out) = run(r, &["adr", "doctor"]);
    assert!(!out.contains("DanglingCitation"), "a fence is an example:\n{out}");
    assert_eq!(code, 0, "{out}");
}

#[test]
fn an_id_in_ordinary_prose_is_still_a_citation() {
    // The control for the fence rule: outside a fence, an id is a claim.
    let d = tempfile::tempdir().expect("tempdir");
    let r = d.path();
    std::fs::create_dir_all(r.join("docs/adrs")).unwrap();
    let real = id("001");
    std::fs::write(
        r.join("docs/adrs").join(format!("{real}-a-decision.md")),
        format!("---\nid: {real}\nstatus: accepted\ndate: 2026-01-01\n---\n# {real}: a decision\n"),
    )
    .unwrap();
    std::fs::write(r.join("docs/guide.md"), format!("# Guide\n\nSee {} for why.\n", id("0000")))
        .unwrap();
    let (_, out) = run(r, &["adr", "doctor"]);
    assert!(out.contains("DanglingCitation"), "prose outside a fence is read:\n{out}");
}

#[test]
fn a_citation_in_shipped_code_is_still_scanned() {
    // The control for the exclusion above: excluding test code must not stop
    // the checker reading the code that ships.
    let d = project(Some("001"), "2699999999");
    let (_, out) = run(d.path(), &["adr", "doctor"]);
    assert!(out.contains("DanglingCitation"), "src/ is still read:\n{out}");
}

// ── The checker has to actually run ──────────────────────────────────────────

#[test]
fn ci_runs_the_decision_checker() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("workspace root");
    let ci = std::fs::read_to_string(root.join(".github/workflows/ci.yml")).expect("read ci.yml");
    assert!(
        ci.contains("adr doctor"),
        "CI must run the decision checker, or the registry rots while CI stays green"
    );
}

#[test]
fn the_repository_itself_is_consistent() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("workspace root");
    let (code, out) = run(root, &["adr", "doctor"]);
    assert_eq!(code, 0, "hexa's own registry must pass the check CI now runs:\n{out}");
}
