//! Integration tests for ADR-2610301500: a dead end gives a guide, and the
//! guide records the gap.

use hexa_core::domain::guide::{check_commands, command_words, fix_path, guides, Guide};
use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

/// `hexa` with HOME at `home`, no provider in the environment, and the local
/// server pointed at a port nothing listens on.
fn hexa(dir: &Path, home: &Path, args: &[&str]) -> (bool, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_hexa"));
    cmd.args(args).current_dir(dir).env("HOME", home).env("NO_COLOR", "1");
    for k in ["HEXA_HOME", "ANTHROPIC_API_KEY", "ANTHROPIC_BASE_URL", "HEXA_INFERENCE_URL", "HEXA_INFERENCE_MODEL", "OPENAI_API_KEY", "OPENROUTER_API_KEY"] {
        cmd.env_remove(k);
    }
    cmd.env("OLLAMA_HOST", "127.0.0.1:9").env("HEXA_OLLAMA_HOST", "127.0.0.1:9");
    let out = cmd.output().expect("run hexa");
    (out.status.success(), String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr))
}

/// A project named `name` with one ADR.
fn project(root: &Path, name: &str) -> PathBuf {
    let p = root.join(name);
    std::fs::create_dir_all(p.join("docs/adrs")).unwrap();
    std::fs::create_dir_all(p.join(".hexa")).unwrap();
    std::fs::write(
        p.join("docs/adrs/ADR-2610301501-x.md"),
        "---\nid: ADR-2610301501\nstatus: proposed\n---\n# ADR-2610301501: x\n\n## Decision\n\n1. **One.** It does one thing.\n",
    )
    .unwrap();
    p
}

const DRAFT: &[&str] = &["spec", "draft", "ADR-2610301501", "--model", "some-local-model"];

fn gaps_lines(home: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(home.join(".hexa/gaps.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

/// Every file under `dir`, with its bytes.
fn tree(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push((p.clone(), std::fs::read(&p).unwrap()));
            }
        }
    }
    out.sort();
    out
}

fn guide(id: &str) -> Guide {
    guides().into_iter().find(|g| g.id == id).unwrap_or_else(|| panic!("no guide {id}"))
}

/// Whether `hexa <words> --help` succeeds.
fn runs(words: &[String]) -> bool {
    Command::new(env!("CARGO_BIN_EXE_hexa")).args(words).arg("--help").output().is_ok_and(|o| o.status.success())
}

// ── §1 ──────────────────────────────────────────────────

/// S01: every guide in the table has an id, a what, a step around, and a fix naming a hexa path.
#[test]
fn s01_every_guide_is_whole() {
    let all = guides();
    assert!(!all.is_empty());
    let mut ids: Vec<&str> = all.iter().map(|g| g.id.as_str()).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), all.len(), "ids are unique");
    for g in &all {
        assert!(!g.id.is_empty() && g.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'), "kebab-case: {}", g.id);
        assert!(!g.what.trim().is_empty(), "{}", g.id);
        assert!(!g.around.is_empty(), "{}", g.id);
        assert!(!g.fix.is_empty(), "{}", g.id);
        for f in &g.fix {
            assert!(workspace().join(fix_path(f)).exists(), "{}: fix path {} exists", g.id, fix_path(f));
        }
        g.validate().unwrap();
    }
}

/// S02: an entry without a route around or a fix is refused.
#[test]
fn s02_a_guide_without_a_route_or_fix_is_refused() {
    let base = Guide { id: "x".into(), what: "w".into(), around: vec!["a".into()], fix: vec!["hexa-cli/src/main.rs".into()] };
    let no_around = Guide { around: vec![], ..base.clone() };
    let no_fix = Guide { fix: vec![], ..base.clone() };
    let e = no_around.validate().unwrap_err();
    assert!(e.contains("around"), "{e}");
    let e = no_fix.validate().unwrap_err();
    assert!(e.contains("fix"), "{e}");
    base.validate().unwrap();
}

// ── §2 ──────────────────────────────────────────────────

/// S03: `hexa spec draft` with no completion model reachable prints the no-completion-model guide.
#[test]
fn s03_draft_with_no_completion_model_prints_the_guide() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let p = project(root.path(), "a");
    let (ok, out) = hexa(&p, &home, DRAFT);
    assert!(!ok, "{out}");
    let said = out.find("no completion model is reachable").unwrap_or_else(|| panic!("{out}"));
    if let Some(t) = out.find("transport error") {
        assert!(said < t, "the cause before the transport: {out}");
    }
    for (i, step) in guide("no-completion-model").around.iter().enumerate() {
        assert!(out.contains(&format!("{}. {}", i + 1, step)), "step {}: {out}", i + 1);
    }
    assert!(out.contains("hexa gap pr no-completion-model"), "{out}");
}

/// S04: a guide never turns a failure into a success, and an ordinary error prints no guide.
#[test]
fn s04_a_guide_keeps_the_failure_and_ordinary_errors_have_none() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let p = project(root.path(), "a");
    let (ok, out) = hexa(&p, &home, DRAFT);
    assert!(!ok && out.contains("hexa gap pr"), "{out}");
    let (ok, out) = hexa(&p, &home, &["spec", "draft", "ADR-9999", "--model", "some-local-model"]);
    assert!(!ok, "{out}");
    assert!(!out.contains("hexa gap pr"), "{out}");
}

// ── §3 ──────────────────────────────────────────────────

/// S05: printing a guide appends one hit; `hexa gap list` ranks by hits.
#[test]
fn s05_hits_are_recorded_and_ranked() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let a = project(root.path(), "proj-a");
    let b = project(root.path(), "proj-b");
    hexa(&a, &home, DRAFT);
    hexa(&a, &home, DRAFT);
    hexa(&b, &home, DRAFT);
    let hits: Vec<_> = gaps_lines(&home).into_iter().filter(|v| v["id"] == "no-completion-model").collect();
    assert_eq!(hits.len(), 3, "{hits:?}");
    let (ok, out) = hexa(&a, &home, &["gap", "list"]);
    assert!(ok, "{out}");
    let row = out.lines().find(|l| l.contains("no-completion-model")).unwrap_or_else(|| panic!("{out}"));
    assert!(row.contains('3') && row.contains("proj-b"), "{row}");
}

/// S06: recording a gap reaches no network and writes nothing in the project.
#[test]
fn s06_recording_writes_nothing_in_the_project() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let a = project(root.path(), "proj-a");
    let b = project(root.path(), "proj-b");
    let (ta, tb) = (tree(&a), tree(&b));
    let run = |p: &Path| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_hexa"));
        cmd.args(DRAFT).current_dir(p).env("HOME", &home).env_remove("HEXA_HOME");
        for k in ["ANTHROPIC_API_KEY", "HEXA_INFERENCE_URL", "OPENAI_API_KEY", "OPENROUTER_API_KEY"] {
            cmd.env_remove(k);
        }
        cmd.env("OLLAMA_HOST", "127.0.0.1:9").env("HEXA_OLLAMA_HOST", "127.0.0.1:9");
        cmd.env("HTTPS_PROXY", "http://127.0.0.1:9").env("HTTP_PROXY", "http://127.0.0.1:9");
        let out = cmd.output().unwrap();
        String::from_utf8_lossy(&out.stdout).to_string()
    };
    let out = run(&a);
    run(&b);
    assert!(out.contains("hexa gap pr no-completion-model"), "the same with the network down: {out}");
    assert_eq!(tree(&a), ta, "nothing changed under proj-a");
    assert_eq!(tree(&b), tb, "nothing changed under proj-b");
    assert_eq!(gaps_lines(&home).len(), 2);
}

// ── §4 ──────────────────────────────────────────────────

/// S07: `hexa gap add` records an unlisted guide that `hexa gap list` shows.
#[test]
fn s07_gap_add_records_an_unlisted_guide() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let p = project(root.path(), "a");
    let (ok, out) = hexa(&p, &home, &["gap", "add", "guard-x", "--what", "w", "--around", "step one", "--fix", "hexa-cli/src/commands/spec_ledger.rs"]);
    assert!(ok, "{out}");
    let (ok, out) = hexa(&p, &home, &["gap", "list"]);
    assert!(ok, "{out}");
    let row = out.lines().find(|l| l.contains("guard-x")).unwrap_or_else(|| panic!("{out}"));
    assert!(row.contains("unlisted"), "{row}");
    assert!(out.contains('w'), "{out}");
}

/// S08: an unlisted guide needs a route and a fix, and may not reuse a listed id.
#[test]
fn s08_gap_add_refuses_an_incomplete_or_taken_guide() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let p = project(root.path(), "a");
    let fix = "hexa-cli/src/commands/spec_ledger.rs";
    let cases: [(&[&str], &str); 3] = [
        (&["gap", "add", "g1", "--what", "w", "--fix", fix], "around"),
        (&["gap", "add", "g2", "--what", "w", "--around", "s"], "fix"),
        (&["gap", "add", "no-completion-model", "--what", "w", "--around", "s", "--fix", fix], "no-completion-model"),
    ];
    for (args, why) in cases {
        let (ok, out) = hexa(&p, &home, args);
        assert!(!ok, "{args:?}: {out}");
        assert!(out.contains(why), "{args:?} names {why}: {out}");
    }
    assert!(gaps_lines(&home).is_empty(), "nothing was recorded");
}

/// S09: the CLAUDE.md template tells an agent to add a guide when it routes around hexa.
#[test]
fn s09_the_template_says_to_add_a_guide() {
    let t = std::fs::read_to_string(workspace().join("hexa-cli/assets/templates/claude-md-hexa-section.md")).unwrap();
    let verb = t.find("build the verb").expect("build the verb");
    let gap = t.find("hexa gap add").expect("hexa gap add in the template");
    let section_of = |at: usize| t[..at].rfind("\n## ").or_else(|| t[..at].rfind("\n### "));
    assert_eq!(section_of(verb), section_of(gap), "in the same section as \"build the verb\"");
}

// ── §5 ──────────────────────────────────────────────────

fn three_hits(root: &Path) -> (PathBuf, PathBuf) {
    let home = root.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let a = project(root, "proj-a");
    let b = project(root, "proj-b");
    hexa(&a, &home, DRAFT);
    hexa(&a, &home, DRAFT);
    hexa(&b, &home, DRAFT);
    (home, a)
}

/// S10: `hexa gap pr` prints a title and a body with the hits, the route and the files.
#[test]
fn s10_gap_pr_writes_title_and_body() {
    let root = tempfile::tempdir().unwrap();
    let (home, a) = three_hits(root.path());
    let (ok, out) = hexa(&a, &home, &["gap", "pr", "no-completion-model"]);
    assert!(ok, "{out}");
    let g = guide("no-completion-model");
    assert!(!out.lines().next().unwrap_or("").trim().is_empty(), "a title first: {out}");
    assert!(out.contains(&g.what), "{out}");
    assert!(out.contains('3') && out.contains("proj-a") && out.contains("proj-b"), "{out}");
    for s in &g.around {
        assert!(out.contains(s.as_str()), "{s}: {out}");
    }
    for f in &g.fix {
        assert!(out.contains(fix_path(f)), "{f}: {out}");
    }
}

/// S11: `hexa gap pr` opens nothing and changes nothing; an unknown id fails.
#[test]
fn s11_gap_pr_changes_nothing_and_refuses_unknown() {
    let root = tempfile::tempdir().unwrap();
    let (home, a) = three_hits(root.path());
    let git = |args: &[&str]| Command::new("git").args(args).current_dir(&a).output().unwrap();
    git(&["init", "-q"]);
    git(&["-c", "user.email=t@t", "-c", "user.name=t", "add", "-A"]);
    git(&["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "x"]);
    let status = || String::from_utf8_lossy(&git(&["status", "--porcelain"]).stdout).to_string();
    let (before, gaps) = (status(), std::fs::read(home.join(".hexa/gaps.jsonl")).unwrap());
    let (ok, out) = hexa(&a, &home, &["gap", "pr", "no-completion-model"]);
    assert!(ok, "{out}");
    assert_eq!(status(), before);
    assert_eq!(std::fs::read(home.join(".hexa/gaps.jsonl")).unwrap(), gaps);
    let (ok, out) = hexa(&a, &home, &["gap", "pr", "nope"]);
    assert!(!ok && out.contains("nope"), "{out}");
}

// ── §6 ──────────────────────────────────────────────────

/// S12: every `hexa ` step in every guide is a real command.
#[test]
fn s12_every_hexa_step_is_a_real_command() {
    let (checked, errors) = check_commands(&guides(), &runs);
    assert!(checked >= 1, "the check counted no step, so it proved nothing");
    assert!(errors.is_empty(), "{errors:?}");
}

/// S16: the no-completion-model guide's `hexa spec accept` step passes the check.
#[test]
fn s16_the_accept_step_passes() {
    let g = guide("no-completion-model");
    let step = g.around.iter().find(|s| s.starts_with("hexa spec accept")).expect("a hexa spec accept step");
    assert_eq!(command_words(step), Some(vec!["spec".to_string(), "accept".to_string()]));
    assert!(runs(&command_words(step).unwrap()));
    let (checked, errors) = check_commands(&[g], &runs);
    assert!(checked >= 1 && errors.is_empty(), "{errors:?}");
}

/// S13: a guide naming a verb hexa lacks fails the check.
#[test]
fn s13_a_missing_verb_fails_the_check() {
    let g = Guide {
        id: "bad-guide".into(),
        what: "w".into(),
        around: vec!["hexa no-such-verb <x> — does nothing".into()],
        fix: vec!["hexa-cli/src/main.rs".into()],
    };
    let (checked, errors) = check_commands(&[g], &runs);
    assert_eq!(checked, 1);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("bad-guide") && errors[0].contains("no-such-verb"), "{errors:?}");
}

// ── §7 ──────────────────────────────────────────────────

/// S14: the table starts with the three guides found building ADR-2610301330.
#[test]
fn s14_the_first_three_guides() {
    let ids: Vec<String> = guides().into_iter().map(|g| g.id).collect();
    for id in ["no-completion-model", "link-hand-written-tests", "guard-test-is-a-mirror"] {
        assert!(ids.iter().any(|i| i == id), "{id} in {ids:?}");
    }
    let link = guide("link-hand-written-tests");
    assert!(link.around.last().unwrap().starts_with("hexa spec seal"), "{:?}", link.around);
}

/// S15: the red commit is set only after the tests are committed failing.
#[test]
fn s15_red_is_set_after_the_failing_commit() {
    let steps = guide("link-hand-written-tests").around;
    let at = |needle: &str| steps.iter().position(|s| s.contains(needle)).unwrap_or_else(|| panic!("{needle} in {steps:?}"));
    let run = at("fail");
    let commit = at("git commit");
    let red = at("`red`");
    let seal = at("hexa spec seal");
    assert!(run <= commit && commit < red && red < seal, "run {run}, commit {commit}, red {red}, seal {seal}: {steps:?}");
    assert!(steps[red].contains("git rev-parse"), "red is the commit just made: {}", steps[red]);
}
