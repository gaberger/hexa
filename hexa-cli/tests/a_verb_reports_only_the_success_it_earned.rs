//! A verb reports only the success it earned.
//!
//! An audit ran each of these verbs and watched it print a green it had not
//! earned: `verify` confirmed a false claim, `analyze --json` awarded A+ to a
//! tree it never read, `gate coverage` counted untested Rust as covered,
//! `spec verify` passed on nothing, `plan create` saved nothing and exited 0,
//! `agent report` recorded done with no checks, and `adr accept` printed a
//! rationale it threw away. A gate that degrades silently is worse than no
//! gate (CLAUDE.md), so each case here runs the built binary in a temp repo.
//!
//! Ids are assembled at run time: the citation checker scans source, and a
//! literal id here would claim a decision exists.

use std::path::{Path, PathBuf};
use std::process::Command;

fn hexa_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hexa"))
}

struct Run {
    code: i32,
    out: String,
}

fn run_env(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Run {
    let home = dir.join(".test-home");
    std::fs::create_dir_all(&home).unwrap();
    let mut cmd = Command::new(hexa_bin());
    cmd.args(args)
        .current_dir(dir)
        .env("HOME", &home)
        .env("HEXA_HOME", home.join(".hexa"))
        .env("CLAUDE_PROJECT_DIR", dir)
        .env_remove("HEXA_TASK")
        .env_remove("HEXA_INTERNAL")
        .env("NO_COLOR", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let o = cmd.output().expect("run hexa");
    Run {
        code: o.status.code().unwrap_or(-1),
        out: format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)),
    }
}

fn run(dir: &Path, args: &[&str]) -> Run {
    run_env(dir, args, &[])
}

fn git(dir: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@x", "-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body).unwrap();
}

/// A temp dir holding a git repository with `files`, all committed.
fn repo(files: &[(&str, &str)]) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    for (rel, body) in files {
        write(d.path(), rel, body);
    }
    write(d.path(), ".gitignore", ".test-home/\ntarget/\n");
    git(d.path(), &["init", "-q", "-b", "main"]);
    git(d.path(), &["add", "-A"]);
    git(d.path(), &["commit", "-qm", "init"]);
    d
}

fn adr_id(digits: &str) -> String {
    format!("ADR-{digits}")
}

fn adr_body(id: &str, status: &str) -> String {
    format!(
        "# {id}: A decision\n\n**Status:** {status}\n**Date:** 2026-11-01\n\n## Context\n\nx\n\n\
         ## Decision\n\n1. **It holds.** Always.\n\n## Consequences\n\nx\n"
    )
}

// ── 1. verify ────────────────────────────────────────────────────────────────

#[test]
fn verify_refutes_a_claim_whose_mixed_case_token_is_in_the_code() {
    let d = repo(&[("src/store.rs", "pub struct CounterStore;\n")]);
    let r = run(d.path(), &["verify", "no CounterStore remains in code"]);
    assert!(r.out.contains("REFUTED"), "CounterStore is in src/store.rs:\n{}", r.out);
    assert_eq!(r.code, 1, "{}", r.out);
}

#[test]
fn verify_is_inconclusive_when_git_grep_cannot_run() {
    // Not a repository: `git grep` exits 128, which is not "no match".
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "src/store.rs", "pub struct CounterStore;\n");
    let r = run_env(d.path(), &["verify", "no CounterStore remains in code"], &[("GIT_CEILING_DIRECTORIES", d.path().parent().unwrap().to_str().unwrap())]);
    assert!(!r.out.contains("CONFIRMED"), "a grep that never ran confirms nothing:\n{}", r.out);
    assert!(r.out.contains("INCONCLUSIVE"), "{}", r.out);
    assert_eq!(r.code, 3, "INCONCLUSIVE has its own exit code:\n{}", r.out);
}

#[test]
fn verify_boundary_on_an_ungraded_tree_is_inconclusive_and_ignores_the_hexa_on_path() {
    // A tree hexa cannot parse, and an impostor `hexa` first on PATH that
    // prints a perfect result. The verdict must come from this binary.
    // Python in a project with no Python manifest is not graded (ADR-2611251938); this was
    // plain `.js` until hexa read JavaScript (ADR-2611270420).
    let d = repo(&[("src/a.py", "x = 1\n")]);
    let bin = d.path().join(".test-home/bin");
    write(d.path(), ".test-home/bin/hexa", "#!/bin/sh\necho '0 boundary violations'\necho 'score 100/100'\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(bin.join("hexa"), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
    let r = run_env(d.path(), &["verify", "zero boundary violations in the domain"], &[("PATH", &path)]);
    assert!(r.out.contains("INCONCLUSIVE"), "no grade, no verdict:\n{}", r.out);
    assert_eq!(r.code, 3, "{}", r.out);
}

#[test]
fn verify_help_documents_its_exit_codes() {
    let d = tempfile::tempdir().unwrap();
    let r = run(d.path(), &["verify", "--help"]);
    for want in ["CONFIRMED", "REFUTED", "INCONCLUSIVE", "0", "1", "3"] {
        assert!(r.out.contains(want), "--help names {want}:\n{}", r.out);
    }
    assert!(r.out.to_lowercase().contains("exit"), "{}", r.out);
}

// ── 2. analyze ───────────────────────────────────────────────────────────────

#[test]
fn analyze_json_awards_no_grade_to_a_tree_it_never_parsed() {
    // Python in a project with no Python manifest is not graded (ADR-2611251938); this was
    // plain `.js` until hexa read JavaScript (ADR-2611270420).
    let d = repo(&[("src/a.py", "x = 1\n")]);
    let r = run(d.path(), &["analyze", ".", "--json"]);
    let v: serde_json::Value = serde_json::from_str(r.out.trim_start_matches(|c| c != '{'))
        .unwrap_or_else(|e| panic!("not JSON ({e}):\n{}", r.out));
    assert_eq!(v["graded"], serde_json::json!(false), "{}", r.out);
    assert!(v["grade"].is_null(), "no A+ on nothing parsed: {}", v["grade"]);
}

#[test]
fn analyze_exit_code_fails_a_tree_it_never_parsed() {
    // Python in a project with no Python manifest is not graded (ADR-2611251938); this was
    // plain `.js` until hexa read JavaScript (ADR-2611270420).
    let d = repo(&[("src/a.py", "x = 1\n")]);
    let r = run(d.path(), &["analyze", ".", "--exit-code"]);
    assert_ne!(r.code, 0, "--exit-code passed a tree it never read:\n{}", r.out);
}

#[test]
fn a_composition_root_rs_file_is_in_the_composition_root_layer() {
    let d = repo(&[
        ("Cargo.toml", "[package]\nname = \"cr\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
        ("src/lib.rs", "pub mod domain;\npub mod composition_root;\n"),
        ("src/domain/mod.rs", "pub mod thing;\n"),
        ("src/domain/thing.rs", "pub fn t() -> u8 { 1 }\n"),
        ("src/composition_root.rs", "pub fn wire() -> u8 { crate::domain::thing::t() }\n"),
    ]);
    let r = run(d.path(), &["analyze", "."]);
    assert!(!r.out.contains("no layer      src/composition_root.rs"), "{}", r.out);
    assert!(r.out.contains("coverage 4/4"), "every file is in a layer:\n{}", r.out);
}

#[test]
fn analyze_help_describes_every_flag() {
    let d = tempfile::tempdir().unwrap();
    for args in [&["analyze", "--help"][..], &["dev", "analyze", "--help"][..]] {
        let r = run(d.path(), args);
        for flag in ["--strict", "--json", "--file", "--quiet"] {
            let line = r.out.lines().find(|l| l.trim_start().starts_with(flag)).unwrap_or_else(|| panic!("{flag} in:\n{}", r.out));
            let rest = line.trim_start().trim_start_matches(flag).trim_start_matches(" <PATH>").trim();
            let next = r.out.lines().skip_while(|l| *l != line).nth(1).unwrap_or("").trim().to_string();
            assert!(!rest.is_empty() || (!next.is_empty() && !next.starts_with('-')), "{args:?}: {flag} has no help text:\n{}", r.out);
        }
    }
}

// ── 3. gate coverage ─────────────────────────────────────────────────────────

#[test]
fn gate_coverage_refuses_rust_rather_than_counting_a_compile_error_as_coverage() {
    let d = repo(&[
        ("Cargo.toml", "[package]\nname = \"gc\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
        ("src/lib.rs", "pub mod util;\n#[cfg(test)]\nmod t { #[test] fn ok() {} }\n"),
        ("src/util.rs", "pub fn untested() -> u8 { 7 }\n"),
    ]);
    let r = run(d.path(), &["gate", "coverage", ".", "--gate", "cargo test"]);
    assert_ne!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("not measurable by deletion for rust"), "{}", r.out);
    assert!(!r.out.contains("the gate needs these modules"), "no coverage is reported:\n{}", r.out);
}

#[test]
fn gate_coverage_finds_the_composition_root_from_the_repo_root() {
    let d = repo(&[
        ("src/main.ts", "import { f } from './util.js';\nf();\n"),
        ("src/util.ts", "export function f() { return 1; }\n"),
    ]);
    let r = run(d.path(), &["gate", "coverage", "src", "--gate", "true", "--uncounted", "--composition-root", "src/main.ts"]);
    assert!(r.out.contains("util.ts"), "{}", r.out);
    assert!(!r.out.contains("main.ts"), "the composition root is wiring, never deleted:\n{}", r.out);
    let r = run(d.path(), &["gate", "coverage", "src", "--gate", "true", "--uncounted"]);
    assert!(!r.out.contains("main.ts"), "and it is found without the flag:\n{}", r.out);
    assert!(d.path().join("src/main.ts").exists() && d.path().join("src/util.ts").exists());
}

// ── 4. spec ──────────────────────────────────────────────────────────────────

const SPEC_DIGITS: &str = "2701020000";

fn spec_repo(cargo: bool, scenarios_with_tests: bool) -> tempfile::TempDir {
    let id = adr_id(SPEC_DIGITS);
    let mut files: Vec<(String, String)> = vec![(format!("docs/adrs/{id}-a-decision.md"), adr_body(&id, "Accepted"))];
    if cargo {
        files.push(("Cargo.toml".into(), "[package]\nname = \"fx\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[workspace]\n".into()));
        files.push(("src/lib.rs".into(), String::new()));
        files.push(("tests/inv.rs".into(), "#[test]\nfn passes() {}\n".into()));
    }
    let test = |t: &str| if scenarios_with_tests { format!(",\"test\":\"{t}\"") } else { String::new() };
    let sc = |sid: &str, kind: &str| {
        format!(
            "{{\"id\":\"{sid}\",\"decision\":[\"§1\"],\"kind\":\"{kind}\",\"description\":\"d\",\"given\":\"g\",\"when\":\"w\",\"then\":\"t\"{}}}",
            test("tests/inv.rs::passes")
        )
    };
    files.push((
        format!("docs/specs/{id}.json"),
        format!("{{\"adr\":\"{id}\",\"feature\":\"f\",\"specs\":[{},{}]}}", sc("S1", "example"), sc("S2", "negative")),
    ));
    let refs: Vec<(&str, &str)> = files.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    repo(&refs)
}

#[test]
fn spec_verify_fails_when_nothing_passed_and_scenarios_are_pending() {
    let d = spec_repo(true, false);
    let id = adr_id(SPEC_DIGITS);
    let r = run(d.path(), &["spec", "accept", &id, "--by", "a reviewer"]);
    assert_eq!(r.code, 0, "{}", r.out);
    git(d.path(), &["commit", "-qam", "accept"]);
    let r = run(d.path(), &["spec", "verify", &id]);
    assert!(r.out.contains("0 passed"), "{}", r.out);
    assert_ne!(r.code, 0, "0 passed and 2 pending is not a pass:\n{}", r.out);
}

#[test]
fn spec_draft_refuses_a_project_with_no_cargo_toml_before_the_paid_call() {
    let d = spec_repo(false, false);
    let id = adr_id(SPEC_DIGITS);
    std::fs::remove_file(d.path().join(format!("docs/specs/{id}.json"))).unwrap();
    let r = run(d.path(), &["spec", "draft", &id]);
    assert_ne!(r.code, 0, "{}", r.out);
    // The free check for a reachable model may refuse first; either way the
    // paid call is never made and nothing is drafted.
    assert!(
        (r.out.contains("Cargo.toml") && r.out.contains("Rust")) || r.out.contains("no completion model is reachable"),
        "draft says why before any model call:\n{}",
        r.out
    );
    assert!(!d.path().join(format!("docs/specs/{id}.json")).exists(), "nothing drafted");
}

/// The spec file of `spec_repo`, relative to its root.
fn spec_rel() -> String {
    format!("docs/specs/{}.json", adr_id(SPEC_DIGITS))
}

#[test]
fn spec_accept_refuses_an_untracked_spec_and_names_the_commit_to_make() {
    let d = spec_repo(true, false);
    let id = adr_id(SPEC_DIGITS);
    git(d.path(), &["rm", "-q", "--cached", &spec_rel()]);
    git(d.path(), &["commit", "-qm", "untrack the spec"]);
    let before = std::fs::read_to_string(d.path().join(spec_rel())).unwrap();
    let r = run(d.path(), &["spec", "accept", &id, "--by", "a reviewer"]);
    assert_ne!(r.code, 0, "an acceptance of text no commit holds:\n{}", r.out);
    assert!(r.out.contains("untracked") && r.out.contains("git add") && r.out.contains("git commit"), "{}", r.out);
    assert_eq!(std::fs::read_to_string(d.path().join(spec_rel())).unwrap(), before, "nothing is recorded");
}

#[test]
fn spec_accept_refuses_a_modified_spec_that_head_holds_no_acceptance_of() {
    let d = spec_repo(true, false);
    let id = adr_id(SPEC_DIGITS);
    let p = d.path().join(spec_rel());
    let edited = std::fs::read_to_string(&p).unwrap().replacen("\"then\":\"t\"", "\"then\":\"t, edited\"", 1);
    assert!(edited.contains("t, edited"), "the edit landed:\n{edited}");
    std::fs::write(&p, &edited).unwrap();
    let r = run(d.path(), &["spec", "accept", &id, "--by", "a reviewer"]);
    assert_ne!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("modified") && r.out.contains("git commit"), "{}", r.out);
    assert_eq!(std::fs::read_to_string(&p).unwrap(), edited, "nothing is recorded");
}

#[test]
fn spec_accept_takes_an_edited_spec_whose_acceptance_head_holds() {
    let d = spec_repo(true, false);
    let id = adr_id(SPEC_DIGITS);
    let r = run(d.path(), &["spec", "accept", &id, "--by", "a reviewer"]);
    assert_eq!(r.code, 0, "a committed spec is accepted:\n{}", r.out);
    git(d.path(), &["commit", "-qam", "accept"]);
    let p = d.path().join(spec_rel());
    let edited = std::fs::read_to_string(&p).unwrap().replacen("\"then\": \"t\"", "\"then\": \"t, edited\"", 1);
    assert!(edited.contains("t, edited"), "the edit landed:\n{edited}");
    std::fs::write(&p, edited).unwrap();
    let r = run(d.path(), &["spec", "accept", &id, "--by", "a reviewer"]);
    assert_eq!(r.code, 0, "a re-acceptance's predecessor is anchored in HEAD:\n{}", r.out);
}

#[test]
fn spec_accept_refuses_a_project_with_no_cargo_toml() {
    let d = spec_repo(false, false);
    let id = adr_id(SPEC_DIGITS);
    let before = std::fs::read_to_string(d.path().join(spec_rel())).unwrap();
    let r = run(d.path(), &["spec", "accept", &id, "--by", "a reviewer"]);
    assert_ne!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("Cargo.toml") && r.out.contains("Rust"), "accept says why, as draft does:\n{}", r.out);
    assert_eq!(std::fs::read_to_string(d.path().join(spec_rel())).unwrap(), before, "nothing is recorded");
}

#[test]
fn spec_tests_keeps_the_red_tests_by_default_and_has_no_keep_red_flag() {
    let d = repo(&[("README.md", "x\n")]);
    let r = run(d.path(), &["spec", "tests", "--help"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(!r.out.contains("--keep-red"), "keeping the red tests is the default, not an opt-in:\n{}", r.out);
    assert!(r.out.contains("guard"), "the help says passing tests are set aside as guards:\n{}", r.out);
}

// ── 5. plan ──────────────────────────────────────────────────────────────────

#[test]
fn plan_create_makes_its_directory_and_keeps_the_adr() {
    let id = adr_id("2701030000");
    let d = repo(&[(&format!("docs/adrs/{id}-a-decision.md"), &adr_body(&id, "Accepted"))]);
    let r = run(d.path(), &["plan", "create", "add a store", "--adr", &id]);
    assert_eq!(r.code, 0, "{}", r.out);
    let dir = d.path().join("docs/workplans");
    let saved: Vec<_> = std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{e}:\n{}", r.out)).flatten().collect();
    assert_eq!(saved.len(), 1, "{}", r.out);
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(saved[0].path()).unwrap()).unwrap();
    assert_eq!(v["adr"], serde_json::json!(id), "the validated --adr is persisted");
}

#[test]
fn plan_create_matches_the_adr_id_exactly() {
    let id = adr_id("2701030000");
    let d = repo(&[(&format!("docs/adrs/{id}-a-decision.md"), &adr_body(&id, "Accepted"))]);
    let r = run(d.path(), &["plan", "create", "add a store", "--adr", &adr_id("27")]);
    assert_ne!(r.code, 0, "a prefix is not an id:\n{}", r.out);
    assert!(!d.path().join("docs/workplans").exists() || std::fs::read_dir(d.path().join("docs/workplans")).unwrap().next().is_none());
}

#[test]
fn plan_ready_exits_non_zero_when_a_gate_fails() {
    let d = repo(&[
        ("Cargo.toml", "[package]\nname = \"pr\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n"),
        ("src/lib.rs", "pub fn broken() -> u8 { \"not a u8\" }\n"),
    ]);
    let r = run(d.path(), &["plan", "ready", "--no-tests"]);
    assert!(r.out.contains("failing"), "{}", r.out);
    assert_ne!(r.code, 0, "a failing gate is not readiness:\n{}", r.out);
}

#[test]
fn plan_lint_all_finds_every_workplan() {
    let plan = |id: &str| format!("{{\"id\":\"{id}\",\"title\":\"t\",\"status\":\"planned\",\"phases\":[]}}");
    let d = repo(&[("docs/workplans/feat-one.json", &plan("one")), ("docs/workplans/feat-two.json", &plan("two"))]);
    let r = run(d.path(), &["plan", "lint", "--all"]);
    assert!(!r.out.contains("no workplans found"), "two are present:\n{}", r.out);
    assert!(r.out.contains('2'), "{}", r.out);
}

// ── 6. agent report ──────────────────────────────────────────────────────────

fn register(dir: &Path, gate: Option<&str>) -> String {
    let mut args = vec!["agent", "register", "--task", "t", "--lease", "src/a/", "--json"];
    if let Some(g) = gate {
        args.extend(["--gate", g]);
    }
    let r = run(dir, &args);
    let v: serde_json::Value = serde_json::from_str(&r.out).unwrap_or_else(|e| panic!("{e}: {}", r.out));
    v["id"].as_str().unwrap().to_string()
}

fn report(dir: &Path, id: &str, commit: &str) -> Run {
    run(dir, &["agent", "report", "--id", id, "--outcome", "passed", "--commit", commit, "--json"])
}

const GREEN: &str = "echo 'test result: ok. 1 passed; 0 failed; 0 ignored'";
const VACUOUS: &str = "echo 'test result: ok. 0 passed; 0 failed; 0 ignored'";

#[test]
fn agent_report_refuses_passed_for_a_commit_that_does_not_exist() {
    let d = repo(&[("src/a/x.rs", "fn a() {}\n")]);
    let id = register(d.path(), Some(GREEN));
    let r = report(d.path(), &id, "deadbeef");
    assert_ne!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("deadbeef"), "{}", r.out);
    let list = run(d.path(), &["agent", "list", "--json"]);
    assert!(!list.out.contains("\"done\""), "nothing recorded done:\n{}", list.out);
}

#[test]
fn agent_report_refuses_passed_when_the_gate_fails_is_vacuous_or_absent() {
    let d = repo(&[("src/a/x.rs", "fn a() {}\n")]);
    let head = git(d.path(), &["rev-parse", "--short", "HEAD"]);
    for gate in [Some("false"), Some(VACUOUS), None] {
        let id = register(d.path(), gate);
        let r = report(d.path(), &id, &head);
        assert_ne!(r.code, 0, "gate {gate:?}:\n{}", r.out);
        run(d.path(), &["agent", "release", "--id", &id, "--json"]);
    }
}

#[test]
fn agent_report_records_passed_when_the_commit_resolves_and_the_gate_passes() {
    let d = repo(&[("src/a/x.rs", "fn a() {}\n")]);
    let head = git(d.path(), &["rev-parse", "--short", "HEAD"]);
    let id = register(d.path(), Some(GREEN));
    let r = report(d.path(), &id, &head);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("\"done\""), "{}", r.out);
}

// ── 7. adr accept / supersede ────────────────────────────────────────────────

fn adr_repo() -> (tempfile::TempDir, String, String) {
    let a = adr_id("2701040000");
    let b = adr_id("2701050000");
    let d = repo(&[
        (&format!("docs/adrs/{a}-first.md"), &adr_body(&a, "Proposed")),
        (&format!("docs/adrs/{b}-second.md"), &adr_body(&b, "Proposed")),
    ]);
    (d, a, b)
}

fn adr_text(d: &Path, id: &str) -> String {
    let dir = d.join("docs/adrs");
    let f = std::fs::read_dir(&dir).unwrap().flatten().find(|e| e.file_name().to_string_lossy().starts_with(id)).unwrap();
    std::fs::read_to_string(f.path()).unwrap()
}

#[test]
fn adr_accept_records_its_rationale_and_leaves_no_temp_file() {
    let (d, a, _) = adr_repo();
    let r = run(d.path(), &["adr", "accept", &a, "--rationale", "the review signed off"]);
    assert_eq!(r.code, 0, "{}", r.out);
    let text = adr_text(d.path(), &a);
    assert!(text.contains("**Status:** Accepted"), "{text}");
    let line = text.lines().find(|l| l.contains("the review signed off")).unwrap_or_else(|| panic!("rationale recorded:\n{text}"));
    assert!(line.contains("20"), "the rationale line is dated: {line}");
    let names: Vec<String> = std::fs::read_dir(d.path().join("docs/adrs")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
    assert_eq!(names.len(), 2, "no temp file left: {names:?}");
}

#[test]
fn adr_accept_twice_says_already_accepted_and_changes_nothing() {
    let (d, a, _) = adr_repo();
    assert_eq!(run(d.path(), &["adr", "accept", &a, "--rationale", "first"]).code, 0);
    let before = adr_text(d.path(), &a);
    let r = run(d.path(), &["adr", "accept", &a, "--rationale", "second"]);
    assert!(r.out.contains("already Accepted"), "{}", r.out);
    assert_eq!(adr_text(d.path(), &a), before, "nothing changed");
}

#[test]
fn adr_supersede_refuses_itself_and_unknown_or_prefix_ids() {
    let (d, a, _) = adr_repo();
    let before = adr_text(d.path(), &a);
    for by in [a.clone(), adr_id("2799999999"), adr_id("2701")] {
        let r = run(d.path(), &["adr", "supersede", &a, "--by", &by]);
        assert_ne!(r.code, 0, "--by {by}:\n{}", r.out);
    }
    assert_eq!(adr_text(d.path(), &a), before, "nothing changed");
}

#[test]
fn adr_supersede_by_another_records_the_backlink() {
    let (d, a, b) = adr_repo();
    let r = run(d.path(), &["adr", "supersede", &a, "--by", &b, "--rationale", "b replaces a"]);
    assert_eq!(r.code, 0, "{}", r.out);
    let text = adr_text(d.path(), &a);
    assert!(text.contains("Superseded") && text.contains(&b) && text.contains("b replaces a"), "{text}");
}
