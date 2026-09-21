//! An error-severity rule violation is a violation, and the grade says so.
//!
//! ADR-2609211430 §1. `--exit-code` failed on a rule error while `--grade A`
//! passed and the tool printed A+ on the same tree, because
//! `compute_health_score` had no term for the rules file. Every error-severity
//! rule in every project was invisible to the letter hexa prints and to the
//! floor `hexa scaffold` enforces.
//!
//! The gate drives the binary, because the disagreement was between two
//! surfaces of the binary — a unit test on the scoring function alone would
//! have kept passing throughout.

use std::path::Path;
use std::process::Command;

fn hexa() -> std::path::PathBuf {
    let mut p = std::env::current_exe().expect("test exe");
    p.pop();
    p.pop();
    p.push("hexa");
    p
}

/// A minimal Rust tree with one domain file and one adapter, plus a rules
/// file at `severity`. `pattern` is what the rule looks for.
fn tree(severity: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().expect("tempdir");
    let r = d.path();
    std::fs::create_dir_all(r.join(".hexa")).unwrap();
    std::fs::create_dir_all(r.join("src/domain")).unwrap();
    std::fs::create_dir_all(r.join("src/adapters/secondary")).unwrap();
    std::fs::write(
        r.join("Cargo.toml"),
        "[package]\nname=\"rt\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        r.join(".hexa/ADR-rules.toml"),
        format!(
            "[[adr_rules]]\n\
             adr = \"ADR-0000\"\n\
             id = \"domain-imports-no-runtime\"\n\
             message = \"The domain pulled in an infrastructure crate.\"\n\
             severity = \"{severity}\"\n\
             file_patterns = [\".rs\"]\n\
             exclude_patterns = [\"/adapters/\", \"/ports/\", \"/usecases/\", \"main.rs\", \"test\"]\n\
             violation_patterns = [\"use sqlx\"]\n"
        ),
    )
    .unwrap();
    std::fs::write(r.join("src/domain/order.rs"), "use sqlx::PgPool;\npub struct Order;\n").unwrap();
    std::fs::write(
        r.join("src/adapters/secondary/pg.rs"),
        "use sqlx::PgPool;\npub struct PgStore;\n",
    )
    .unwrap();
    d
}

/// A tree with the same files and no rules file at all — the baseline the
/// penalty is measured against.
fn clean_tree() -> tempfile::TempDir {
    let d = tree("error");
    std::fs::remove_file(d.path().join(".hexa/ADR-rules.toml")).unwrap();
    d
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

/// stdout alone. `--json` puts the document on stdout and its progress
/// lines on stderr, so a helper that merges the two hands `serde_json` a
/// trailing "Loaded 1 rule(s)" and fails for a reason that is not the subject.
fn stdout_of(root: &Path, args: &[&str]) -> String {
    let out = Command::new(hexa()).args(args).current_dir(root).output().expect("run hexa");
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// The score hexa reports for a tree, read from `--json` so no printing
/// detail is being asserted.
fn score(root: &Path) -> u64 {
    let out = stdout_of(root, &["analyze", ".", "--json"]);
    let v: serde_json::Value = serde_json::from_str(out.trim()).unwrap_or_else(|e| {
        panic!("analyze --json did not produce JSON ({e}):\n{out}");
    });
    v["score"].as_u64().unwrap_or_else(|| panic!("no score in:\n{out}"))
}

#[test]
fn an_error_severity_rule_violation_costs_ten_points() {
    let base = clean_tree();
    let bad = tree("error");
    let baseline = score(base.path());
    let with_error = score(bad.path());
    assert_eq!(
        with_error,
        baseline.saturating_sub(10),
        "one rule error must cost the same 10 points a boundary violation costs \
         (baseline {baseline}, with the error {with_error})"
    );
}

#[test]
fn the_grade_floor_fails_on_a_tree_that_exit_code_already_failed() {
    let d = tree("error");
    let (exit_code_status, _) = run(d.path(), &["analyze", ".", "--exit-code"]);
    assert_eq!(exit_code_status, 1, "--exit-code fails on a rule error, as it always did");

    let (grade_status, out) = run(d.path(), &["analyze", ".", "--grade", "A+"]);
    assert_eq!(
        grade_status, 1,
        "--grade must agree with --exit-code about the same tree, got:\n{out}"
    );
}

#[test]
fn a_warning_leaves_the_grade_alone_and_strict_still_catches_it() {
    let base = clean_tree();
    let warn = tree("warning");
    assert_eq!(
        score(warn.path()),
        score(base.path()),
        "a warning is advisory — that is what --strict is for"
    );

    let (graded, _) = run(warn.path(), &["analyze", ".", "--grade", "A+"]);
    assert_eq!(graded, 0, "a warning must not move the letter");

    let (strict, _) = run(warn.path(), &["analyze", ".", "--strict"]);
    assert_eq!(strict, 1, "--strict still promotes warnings to failures");
}

#[test]
fn the_rule_is_scoped_and_an_adapter_import_is_not_a_finding() {
    // Pins the fixture itself: if the rule started flagging the adapter too,
    // the penalty above would be 20 and the first test would fail for a
    // reason that has nothing to do with scoring.
    let d = tree("error");
    let (_, out) = run(d.path(), &["analyze", "."]);
    assert!(
        out.contains("1 error(s)"),
        "expected exactly one error — the domain file, not the adapter:\n{out}"
    );
}

#[test]
fn the_printed_formula_names_the_term_that_moved_the_score() {
    // A score a reader cannot reconstruct is a number with a story attached.
    let d = tree("error");
    let out = stdout_of(d.path(), &["analyze", ".", "--json"]);
    let v: serde_json::Value = serde_json::from_str(out.trim()).expect("json");
    let formula = v["explain"]["score"]["formula"].as_str().unwrap_or("");
    assert!(
        formula.contains("rule_errors"),
        "the formula must name the rule-error term, got: {formula:?}"
    );

    // And the components must add up to the score. A reader who sums them
    // and lands 10 points away from the printed number has been handed a
    // story, which is the failure this ADR is about.
    let c = &v["score_components"];
    let sum = 100u64
        - 10 * (c["violations"].as_u64().unwrap() + c["rule_errors"].as_u64().unwrap())
        - 15 * c["circular_deps"].as_u64().unwrap()
        - c["dead_exports"].as_u64().unwrap().min(20)
        - c["unused_ports"].as_u64().unwrap().min(10);
    assert_eq!(
        sum,
        v["score"].as_u64().unwrap(),
        "the components must reconstruct the score: {c}"
    );
}

/// ADR-2609211430 §4: a rule scopes itself positively.
///
/// Before `path_patterns`, reaching one layer meant naming every other layer
/// in `exclude_patterns` — so a directory nobody listed fell into scope
/// silently, which is the same shape of bug as the grade that did not move.
#[test]
fn path_patterns_scope_a_rule_to_a_layer_without_naming_the_others() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    std::fs::create_dir_all(r.join(".hexa")).unwrap();
    std::fs::create_dir_all(r.join("src/domain")).unwrap();
    std::fs::create_dir_all(r.join("src/adapters/secondary")).unwrap();
    // A layer no exclude list would have thought to name.
    std::fs::create_dir_all(r.join("src/workers")).unwrap();
    std::fs::write(
        r.join("Cargo.toml"),
        "[package]\nname=\"rt\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        r.join(".hexa/ADR-rules.toml"),
        "[[adr_rules]]\n         adr = \"ADR-0000\"\n         id = \"domain-imports-no-runtime\"\n         message = \"The domain pulled in an infrastructure crate.\"\n         severity = \"error\"\n         file_patterns = [\".rs\"]\n         path_patterns = [\"/domain/\"]\n         violation_patterns = [\"use sqlx\"]\n",
    )
    .unwrap();
    std::fs::write(r.join("src/domain/order.rs"), "use sqlx::PgPool;\npub struct Order;\n").unwrap();
    std::fs::write(
        r.join("src/adapters/secondary/pg.rs"),
        "use sqlx::PgPool;\npub struct PgStore;\n",
    )
    .unwrap();
    std::fs::write(r.join("src/workers/sync.rs"), "use sqlx::PgPool;\npub struct Sync;\n").unwrap();

    let (_, out) = run(r, &["analyze", "."]);
    assert!(
        out.contains("1 error(s)"),
        "only the domain file is in scope — not the adapter, and not a layer \
         no exclude list names:\n{out}"
    );
    assert!(
        out.contains("src/domain/order.rs"),
        "and it must be the domain file that was reported:\n{out}"
    );
}

/// A rules file written before `path_patterns` existed keeps working.
#[test]
fn a_rule_without_path_patterns_is_unrestricted() {
    let d = tree("error");
    let (_, out) = run(d.path(), &["analyze", "."]);
    assert!(out.contains("1 error(s)"), "exclude-based scoping still works:\n{out}");
}
