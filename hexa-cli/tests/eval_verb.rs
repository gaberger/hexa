//! `hexa eval` (ADR-2611160910): the dry run, scoring a saved transcript, the
//! run path with a stand-in `claude`, and the report. No test here runs a
//! model or spends anything: a real run needs a person to start it.
//!
//! Every test runs from a fresh repository of its own, and checks that the
//! only thing `hexa eval` left in it is `.hexa/evals/`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn scenarios() -> PathBuf {
    workspace().join("evals/hexa-usage")
}

fn fixture(name: &str) -> PathBuf {
    scenarios().join("_fixtures").join(name)
}

/// A repository to run `hexa eval` from, and a HEXA_HOME of its own.
struct Here {
    repo: tempfile::TempDir,
    home: tempfile::TempDir,
}

fn here() -> Here {
    let repo = tempfile::tempdir().unwrap();
    let ok = Command::new("git").args(["init", "-q"]).current_dir(repo.path()).status().unwrap();
    assert!(ok.success());
    std::fs::write(repo.path().join("README.md"), "mine\n").unwrap();
    for args in [&["add", "-A"][..], &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "x"]] {
        assert!(Command::new("git").args(args).current_dir(repo.path()).status().unwrap().success());
    }
    Here { repo, home: tempfile::tempdir().unwrap() }
}

impl Here {
    fn hexa(&self, args: &[&str]) -> (i32, String) {
        self.hexa_with(args, &[])
    }

    fn hexa_with(&self, args: &[&str], env: &[(&str, String)]) -> (i32, String) {
        let mut c = Command::new(env!("CARGO_BIN_EXE_hexa"));
        c.args(args)
            .current_dir(self.repo.path())
            .env("HEXA_HOME", self.home.path())
            .env("HEXA_NO_CLAUDE_MODS", "1")
            .env("NO_COLOR", "1");
        for (k, v) in env {
            c.env(k, v);
        }
        let out = c.output().unwrap();
        (
            out.status.code().unwrap_or(-1),
            format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)),
        )
    }

    /// `git status --porcelain`, without `.hexa/evals/`.
    fn changes_outside_evals(&self) -> Vec<String> {
        let out = Command::new("git")
            .args(["status", "--porcelain", "--untracked-files=all"])
            .current_dir(self.repo.path())
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| !l[3..].starts_with(".hexa/evals/"))
            .map(str::to_string)
            .collect()
    }
}

#[test]
fn a_dry_run_builds_the_repository_prints_the_command_and_spends_nothing() {
    let h = here();
    let dir = scenarios();
    let (code, out) = h.hexa(&["eval", "run", "--dry-run", "--dir", dir.to_str().unwrap(), "--scenario", "small-fix"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("✓ small-fix"), "{out}");
    assert!(out.contains("claude '-p' 'The page count is wrong"), "{out}");
    assert!(out.contains("'--output-format' 'stream-json' '--verbose' '--max-turns' '15'"), "{out}");
    assert!(out.contains("'--max-budget-usd' '1'"), "{out}");
    assert!(out.contains("nothing was spent"), "{out}");
    assert!(h.changes_outside_evals().is_empty(), "{:?}", h.changes_outside_evals());
    assert!(!h.repo.path().join(".hexa/evals").exists(), "a dry run writes no result");
}

#[test]
fn a_dry_run_refuses_a_scenario_that_is_already_solved() {
    let h = here();
    let d = tempfile::tempdir().unwrap();
    let s = d.path().join("solved");
    std::fs::create_dir_all(s.join("fixture")).unwrap();
    std::fs::write(s.join("fixture/README.md"), "x\n").unwrap();
    std::fs::write(
        s.join("scenario.toml"),
        "prompt = 'p'\noutcome = 'true'\noutcome_fails_at_start = true\n[[expect_verbs]]\npattern = 'hexa'\n",
    )
    .unwrap();
    let (code, out) = h.hexa(&["eval", "run", "--dry-run", "--dir", d.path().to_str().unwrap()]);
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("measures nothing"), "{out}");
}

fn score_json(h: &Here, scenario: &str, transcript: &str, outcome: Option<&str>) -> serde_json::Value {
    let dir = scenarios();
    let t = fixture(transcript);
    let mut args = vec!["eval", "run", "--dir", dir.to_str().unwrap(), "--scenario", scenario, "--score", t.to_str().unwrap(), "--json"];
    if let Some(o) = outcome {
        args.extend(["--outcome", o]);
    }
    let (code, out) = h.hexa(&args);
    assert_eq!(code, 0, "{out}");
    serde_json::from_str(&out).unwrap_or_else(|e| panic!("{e}:\n{out}"))
}

/// The same hand-computed scores as hexa-exec/tests/usage_eval_scores.rs,
/// through the verb.
#[test]
fn score_reads_each_fixture_to_its_hand_computed_score() {
    let h = here();
    let cases = [
        ("small-fix", "good-run.jsonl", "pass", true, 1.0, 0, 0, 7),
        ("small-fix", "sed-bypass.jsonl", "pass", true, 0.0, 2, 1, 6),
        ("small-fix", "gate-after-edit.jsonl", "pass", true, 0.5, 1, 0, 8),
        ("feature-with-gate", "outcome-fails.jsonl", "fail", false, 0.75, 0, 1, 9),
    ];
    for (scenario, t, o, outcome, process, bypasses, refusals, turns) in cases {
        let v = score_json(&h, scenario, t, Some(o));
        assert_eq!(v["outcome"], outcome, "{t}: {v}");
        assert!((v["process"].as_f64().unwrap() - process).abs() < 1e-9, "{t}: {v}");
        assert_eq!(v["bypasses"], bypasses, "{t}: {v}");
        assert_eq!(v["refusals"], refusals, "{t}: {v}");
        assert_eq!(v["turns"], turns, "{t}: {v}");
    }
    assert!(h.changes_outside_evals().is_empty());
    assert!(!h.repo.path().join(".hexa/evals").exists(), "scoring writes nothing");
}

#[test]
fn score_without_an_outcome_reports_it_as_not_run() {
    let h = here();
    let v = score_json(&h, "small-fix", "good-run.jsonl", None);
    assert!(v["outcome"].is_null(), "{v}");
}

#[test]
fn score_takes_exactly_one_scenario() {
    let h = here();
    let t = fixture("good-run.jsonl");
    let dir = scenarios();
    let (code, out) = h.hexa(&[
        "eval", "run", "--dir", dir.to_str().unwrap(), "--score", t.to_str().unwrap(),
        "--scenario", "small-fix", "--scenario", "feature-with-gate",
    ]);
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("one --scenario"), "{out}");
}

#[test]
fn a_run_without_claude_on_path_stops_before_building_anything() {
    let h = here();
    let dir = scenarios();
    // A PATH that holds git and nothing else. `/usr/bin:/bin` is not "no claude":
    // a machine with a system-wide claude installed there has one.
    let bin = tempfile::tempdir().unwrap();
    let git = std::env::var_os("PATH")
        .and_then(|p| std::env::split_paths(&p).map(|d| d.join("git")).find(|g| g.is_file()))
        .expect("git on PATH");
    std::os::unix::fs::symlink(&git, bin.path().join("git")).unwrap();
    let (code, out) = h.hexa_with(
        &["eval", "run", "--dir", dir.to_str().unwrap(), "--scenario", "small-fix"],
        &[("PATH", bin.path().display().to_string())],
    );
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("`claude` is not on PATH"), "{out}");
    assert!(!h.repo.path().join(".hexa/evals").exists());
}

/// A stand-in `claude` that replays a saved transcript, so the run path is
/// exercised end to end without a model.
fn fake_claude(transcript: &Path) -> (tempfile::TempDir, String) {
    let bin = tempfile::tempdir().unwrap();
    let p = bin.path().join("claude");
    std::fs::write(
        &p,
        format!("#!/bin/sh\n[ \"$1\" = --version ] && {{ echo fake; exit 0; }}\ncat '{}'\n", transcript.display()),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = format!("{}:{}", bin.path().display(), std::env::var("PATH").unwrap_or_default());
    (bin, path)
}

#[test]
fn a_run_writes_one_row_per_scenario_under_hexa_evals_and_nothing_else() {
    let h = here();
    let dir = scenarios();
    let (_bin, path) = fake_claude(&fixture("good-run.jsonl"));
    let (code, out) = h.hexa_with(
        &["eval", "run", "--dir", dir.to_str().unwrap(), "--scenario", "small-fix", "--model", "stand-in"],
        &[("PATH", path)],
    );
    assert_eq!(code, 0, "{out}");
    let evals = h.repo.path().join(".hexa/evals");
    let files: Vec<PathBuf> = std::fs::read_dir(&evals)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    assert_eq!(files.len(), 1, "{files:?}");
    assert!(files[0].file_name().unwrap().to_string_lossy().ends_with("-stand-in.json"));
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&files[0]).unwrap()).unwrap();
    let rows = v["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    // The stand-in changed nothing, so the bug is still there: the outcome
    // fails, while the transcript's process is full marks.
    assert_eq!(rows[0]["scenario"], "small-fix");
    assert_eq!(rows[0]["outcome"], false, "{v}");
    assert!((rows[0]["process"].as_f64().unwrap() - 1.0).abs() < 1e-9);
    assert_eq!(rows[0]["turns"], 7);
    let saved = evals.join(rows[0]["transcript"].as_str().unwrap());
    assert!(saved.is_file(), "the transcript is kept beside the result");
    assert!(h.changes_outside_evals().is_empty(), "{:?}", h.changes_outside_evals());
}

#[test]
fn a_run_stops_when_the_daily_budget_is_spent() {
    let h = here();
    std::fs::create_dir_all(h.repo.path().join(".hexa")).unwrap();
    std::fs::write(h.repo.path().join(".hexa/project.json"), r#"{ "inference": { "budget_usd_per_day": 1.0 } }"#).unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    std::fs::write(
        h.home.path().join("inference-log.jsonl"),
        format!("{{\"model\":\"m\",\"input_tokens\":1,\"output_tokens\":1,\"cost_usd\":1.5,\"source\":\"harden\",\"ts\":\"{now}\"}}\n"),
    )
    .unwrap();
    let dir = scenarios();
    let (_bin, path) = fake_claude(&fixture("good-run.jsonl"));
    let (_, out) = h.hexa_with(&["eval", "run", "--dir", dir.to_str().unwrap(), "--scenario", "small-fix"], &[("PATH", path)]);
    assert!(out.contains("stopping before small-fix"), "{out}");
}

#[test]
fn report_prints_the_newest_result_and_how_it_moved() {
    let h = here();
    let evals = h.repo.path().join(".hexa/evals");
    std::fs::create_dir_all(&evals).unwrap();
    let row = |outcome: bool, process: f64, bypasses: u32| {
        serde_json::json!({ "scenario": "small-fix", "outcome": outcome, "process": process, "bypasses": bypasses,
                            "refusals": 0, "turns": 5, "cost_usd": 0.1, "stop": "success" })
    };
    let file = |rows: serde_json::Value| serde_json::json!({ "schema": 1, "model": "m", "hexa": "h", "started": "t", "rows": rows }).to_string();
    std::fs::write(evals.join("20261101T000000Z-m.json"), file(serde_json::json!([row(false, 0.0, 2)]))).unwrap();
    std::fs::write(evals.join("20261102T000000Z-m.json"), file(serde_json::json!([row(true, 0.5, 1)]))).unwrap();
    let (code, out) = h.hexa(&["eval", "report"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("20261102T000000Z-m.json"), "{out}");
    assert!(out.contains("compared with") && out.contains("20261101T000000Z-m.json"), "{out}");
    assert!(out.contains("pass (was fail)"), "{out}");
    assert!(out.contains("0.50 (+0.50)"), "{out}");
    assert!(out.contains("1 (-1)"), "{out}");
}

/// Run inside a Claude Code session, `hexa eval` must not hand that
/// session's id to the scenario: setup's `hexa loop` records would be bound
/// to the caller, and the model would see them as another session's work.
/// The first baseline run hit exactly this.
#[test]
fn a_scenario_does_not_inherit_the_callers_claude_session() {
    let h = here();
    let dump = h.home.path().join("env.txt");
    let bin = tempfile::tempdir().unwrap();
    let p = bin.path().join("claude");
    std::fs::write(
        &p,
        format!(
            "#!/bin/sh\n[ \"$1\" = --version ] && {{ echo fake; exit 0; }}\nenv > '{}'\ncat '{}'\n",
            dump.display(),
            fixture("good-run.jsonl").display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = format!("{}:{}", bin.path().display(), std::env::var("PATH").unwrap_or_default());
    let dir = scenarios();
    let (code, out) = h.hexa_with(
        &["eval", "run", "--dir", dir.to_str().unwrap(), "--scenario", "small-fix"],
        &[
            ("PATH", path),
            ("CLAUDE_CODE_SESSION_ID", "the-callers-session".into()),
            ("CLAUDE_SESSION_ID", "the-callers-session".into()),
            ("CLAUDECODE", "1".into()),
            ("HEXA_TASK", "the-callers-task".into()),
            ("CLAUDE_CONFIG_DIR", "/kept/for/login".into()),
        ],
    );
    assert_eq!(code, 0, "{out}");
    let env = std::fs::read_to_string(&dump).unwrap();
    assert!(!env.contains("the-callers-session"), "{env}");
    assert!(!env.contains("the-callers-task"), "{env}");
    assert!(!env.lines().any(|l| l.starts_with("CLAUDECODE=")), "{env}");
    assert!(env.contains("CLAUDE_CONFIG_DIR=/kept/for/login"), "claude's login must still be found:\n{env}");
}
