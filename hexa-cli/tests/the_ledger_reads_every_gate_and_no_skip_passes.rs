//! The ledger reads every gate it records, and a skipped case is not a pass
//! (ADR-2610221540 §9, phase 1).
//!
//! On 2026-10-22 `hexa adr gates` reported 20 ADRs without a gate. Twelve of
//! them record one as `**Gate:**`, which it did not read. The gate of the
//! generated-adapter decision passed on the forge with its TypeScript, Go and
//! Rust cases printing `skipping:` and returning. These pin both.
//!
//! Ids are assembled at run time: the citation checker scans source, and a
//! literal id here would claim a decision exists.

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
    let out = Command::new(hexa())
        .args(args)
        .current_dir(root)
        .env_remove("HEXA_NO_SKIP")
        .output()
        .expect("run hexa");
    (
        out.status.code().unwrap_or(-1),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

/// A project whose ledger holds one ADR with `body` after the title.
fn ledger(body: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().expect("tempdir");
    let f = format!("ADR-{}", "2701010000");
    std::fs::create_dir_all(d.path().join("docs/adrs")).unwrap();
    std::fs::write(
        d.path().join("docs/adrs").join(format!("{f}-a-decision.md")),
        format!(
            "---\nid: {f}\nstatus: accepted\ndate: 2027-01-01\n---\n# {f}: A decision\n\n\
             **Status:** Accepted\n\n## Decision\n\n1. **x**\n\n{body}\n\n## References\n\nnone\n"
        ),
    )
    .unwrap();
    d
}

#[test]
fn a_bold_gate_line_is_a_gate() {
    let d = ledger("## Implementation\n\n**Gate:** `true` — the house format since September.");
    let (code, out) = run(d.path(), &["adr", "gates"]);
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("records no gate"), "`**Gate:**` records a gate:\n{out}");
    assert!(out.contains("1 passed"), "{out}");
}

#[test]
fn a_commit_id_before_the_command_is_not_the_command() {
    // ADR-2609221700's shape: the first backquoted span is the commit the
    // gate failed against, and the commands follow as a list.
    let d = ledger("## Implementation\n\n**Gates**, both written first and shown failing against `19b3487`:\n\n- `true` — one.\n- `true` — two.");
    let (code, out) = run(d.path(), &["adr", "gates"]);
    assert_eq!(code, 0, "a commit id is not a program:\n{out}");
    assert!(out.contains("1 passed"), "{out}");
}

#[test]
fn every_listed_gate_runs_not_only_the_first() {
    let d = ledger("## Implementation\n\n**Gates**, written first:\n\n- `true` — passes.\n- `false` — fails.");
    let (code, out) = run(d.path(), &["adr", "gates"]);
    assert_ne!(code, 0, "running half of a gate and calling it green is the narrowing this forbids:\n{out}");
    assert!(out.contains("1 failed"), "{out}");
}

#[test]
fn prose_after_the_command_is_not_part_of_it() {
    // ADR-2609211200's shape: more backquoted text follows in the sentence.
    let d = ledger("## Implementation\n\n**Gate:** `true` — store in one project, `false` in another, and read back.");
    let (code, out) = run(d.path(), &["adr", "gates"]);
    assert_eq!(code, 0, "only the leading command is the gate:\n{out}");
    assert!(out.contains("1 passed"), "and it is found and run:\n{out}");
}

#[test]
fn an_appended_gate_section_replaces_the_one_before_it() {
    // An accepted body is append-only (ADR-2609151930): a gate that went
    // stale is re-recorded below, never rewritten in place.
    let d = ledger("## Gate\n\n`false`\n\n## Gate (re-recorded 2027-01-02)\n\n`true`: the original polled a window that moved.");
    let (code, out) = run(d.path(), &["adr", "gates"]);
    assert_eq!(code, 0, "the last gate section is the current one:\n{out}");
}

#[test]
fn a_skip_is_not_proven_here_and_fails_under_ci() {
    // A gate whose test refuses to skip when the runner forbids it, which is
    // what the repository's skip sites now do.
    let d = ledger(
        "## Gate\n\n`sh -c 'if [ -n \"$HEXA_NO_SKIP\" ]; then echo \"not proven here: no frobnicator on PATH\" >&2; exit 101; fi'`",
    );
    let (code, out) = run(d.path(), &["adr", "gates"]);
    assert_eq!(code, 0, "without --ci it is reported, not failed:\n{out}");
    assert!(out.contains("not proven here") && out.contains("frobnicator"), "and says why:\n{out}");
    assert!(!out.contains("1 passed"), "a skip is not a pass:\n{out}");
    let (code, out) = run(d.path(), &["adr", "gates", "--ci"]);
    assert_ne!(code, 0, "under --ci a case nobody ran fails the command:\n{out}");
}

#[test]
fn a_dry_run_lists_without_running() {
    let d = ledger("## Gate\n\n`false`");
    let (code, out) = run(d.path(), &["adr", "gates", "--dry-run"]);
    assert_eq!(code, 0, "nothing ran, so nothing failed:\n{out}");
    assert!(out.contains("false") && out.contains("1 with a gate"), "{out}");
}

/// ADRs here that record no gate the runner can read. It may fall as they
/// are backfilled; it may never rise. Every one accepted from ADR-2610221540
/// on must record a gate.
const WITHOUT_A_GATE_CEILING: usize = 8;

#[test]
fn the_ledger_without_gates_only_shrinks() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let (code, out) = run(root, &["adr", "gates", "--dry-run"]);
    assert_eq!(code, 0, "{out}");
    let without = out
        .lines()
        .find_map(|l| {
            let head = l.split(" without a gate").next()?;
            (head != l).then(|| head.rsplit(' ').next()?.parse::<usize>().ok()).flatten()
        })
        .unwrap_or_else(|| panic!("a summary with the count without a gate:\n{out}"));
    assert!(without <= WITHOUT_A_GATE_CEILING, "{without} ADRs without a gate > ceiling {WITHOUT_A_GATE_CEILING}:\n{out}");
}

/// Every test in this repository that can skip refuses to under
/// `HEXA_NO_SKIP`, so CI and `adr gates` see a skip as a case nobody ran.
#[test]
fn every_skip_site_refuses_under_no_skip() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut sites = 0;
    for krate in std::fs::read_dir(root).unwrap().flatten() {
        let tests = krate.path().join("tests");
        let Ok(entries) = std::fs::read_dir(&tests) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.file_name().is_some_and(|n| n == "the_ledger_reads_every_gate_and_no_skip_passes.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&p).unwrap_or_default();
            for (i, line) in text.lines().enumerate() {
                if line.contains("eprintln!(\"skipping:") {
                    panic!(
                        "{}:{} skips with a bare eprintln; route it through a skip that refuses under HEXA_NO_SKIP",
                        p.display(),
                        i + 1
                    );
                }
            }
            if text.contains("HEXA_NO_SKIP") {
                sites += 1;
            }
        }
    }
    assert!(sites >= 2, "the two files that skip on a missing toolchain must honour HEXA_NO_SKIP; found {sites}");
}
