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
