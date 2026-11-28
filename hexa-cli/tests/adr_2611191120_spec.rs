//! A test names its runner, and a project names where its tests run.
//!
//! Written from the accepted scenarios of `docs/specs/<this ADR>.json` and the
//! signatures in `hexa-core/src/domain/test_runner.rs`, before the code. The
//! end-to-end cases run the built binary in a temp repository; a remote
//! runner is a local script that clones the pushed commit, as a build server
//! would.
//!
//! Fixture ids are assembled at run time: the citation checker scans source,
//! and a literal id here would claim a decision exists.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use hexa_core::domain::evidence::{is_single_script, read_gate_evidence, Evidence};
use hexa_core::domain::test_runner::{
    batch_script, check_remote_template, fill, is_test_file, judge_counted, judge_script, judge_vitest, parse_test_ref,
    remote_command, runner_for, split_batch, Runner, Verdict,
};

// ── harness ──────────────────────────────────────────────────────────────────

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
        .env("NO_COLOR", "1")
        // HOME is the fixture's, which holds no git identity: a verb that
        // commits (`do gate`) needs one.
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@x")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@x");
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

fn read(root: &Path, rel: &str) -> String {
    std::fs::read_to_string(root.join(rel)).unwrap_or_default()
}

fn executable(p: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn bash(script: &str) -> (bool, String) {
    let o = Command::new("bash").arg("-c").arg(script).output().unwrap();
    (o.status.success(), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
}

fn the_id() -> String {
    format!("ADR-{}", "2701190000")
}

fn adr_body(id: &str) -> String {
    format!(
        "# {id}: A decision\n\n**Status:** Accepted\n**Date:** 2026-11-01\n\n## Context\n\nx\n\n\
         ## Decision\n\n1. **It holds.** Always.\n\n## Consequences\n\nx\n"
    )
}

fn spec_body(id: &str, s1: Option<&str>, s2: Option<&str>) -> String {
    let sc = |sid: &str, kind: &str, t: Option<&str>| {
        let mut v = serde_json::json!({
            "id": sid, "decision": ["§1"], "kind": kind, "description": format!("{sid} holds"),
            "given": "a thing", "when": "it runs", "then": "it holds"
        });
        if let Some(t) = t {
            v["test"] = serde_json::json!(t);
        }
        v
    };
    let spec = serde_json::json!({
        "adr": id, "feature": "a-feature",
        "specs": [sc("S1", "example", s1), sc("S2", "negative", s2)]
    });
    serde_json::to_string_pretty(&spec).unwrap() + "\n"
}

fn spec_rel() -> String {
    format!("docs/specs/{}.json", the_id())
}

/// A repository holding `files`, an ADR with one decision, and its spec with
/// S1 and S2 bound to `s1` and `s2`, committed. With `accept`, the spec is
/// accepted, sealed and committed too.
fn spec_repo(files: &[(&str, &str)], s1: Option<&str>, s2: Option<&str>, accept: bool) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let root = d.path();
    let id = the_id();
    for (rel, body) in files {
        write(root, rel, body);
    }
    write(root, ".gitignore", ".test-home/\ntarget/\n");
    write(root, &format!("docs/adrs/{id}-a-decision.md"), &adr_body(&id));
    write(root, &spec_rel(), &spec_body(&id, s1, s2));
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["add", "-A"]);
    git(root, &["commit", "-qm", "init"]);
    if accept {
        let r = run(root, &["spec", "accept", &id, "--by", "a reviewer"]);
        assert_eq!(r.code, 0, "the fixture's spec is accepted:\n{}", r.out);
        if s1.is_some() || s2.is_some() {
            let r = run(root, &["spec", "seal", &id]);
            assert_eq!(r.code, 0, "the fixture's tests are sealed:\n{}", r.out);
        }
        git(root, &["add", "-A"]);
        git(root, &["commit", "-qm", "accept"]);
    }
    d
}

/// A bare origin for `root`, pushed, and a runner script beside it that clones
/// the commit it is given, prints the id it checked out, runs the command
/// there, and appends one line per invocation to `log`. With `silent`, it
/// logs and exits 0 without running or printing anything.
struct Remote {
    _dir: tempfile::TempDir,
    log: PathBuf,
    template: String,
}

fn remote_for(root: &Path, silent: bool) -> Remote {
    let dir = tempfile::tempdir().unwrap();
    let origin = dir.path().join("origin.git");
    git(dir.path(), &["init", "-q", "--bare", origin.to_str().unwrap()]);
    git(root, &["remote", "add", "origin", origin.to_str().unwrap()]);
    let log = dir.path().join("runner.log");
    let runner = dir.path().join("runner.sh");
    let body = if silent {
        format!("#!/bin/bash\necho \"$1\" >> '{}'\nexit 0\n", log.display())
    } else {
        format!(
            "#!/bin/bash\necho \"$1\" >> '{log}'\nd=$(mktemp -d)\n\
             git clone -q '{origin}' \"$d/c\" && cd \"$d/c\" && git checkout -q \"$1\" || exit 2\n\
             echo \"runner: testing $(git rev-parse HEAD)\"\nbash -c \"$2\"; rc=$?\ncd /; rm -rf \"$d\"; exit $rc\n",
            log = log.display(),
            origin = origin.display()
        )
    };
    std::fs::write(&runner, body).unwrap();
    executable(&runner);
    Remote { template: format!("bash {} {{commit}} {{cmd}}", runner.display()), log, _dir: dir }
}

fn invocations(r: &Remote) -> usize {
    std::fs::read_to_string(&r.log).map(|s| s.lines().count()).unwrap_or(0)
}

/// Set `test.remote` in the project, commit it, and push `main`.
fn use_remote(root: &Path, r: &Remote) {
    let pj = serde_json::json!({ "test": { "remote": r.template } });
    write(root, ".hexa/project.json", &serde_json::to_string_pretty(&pj).unwrap());
    git(root, &["add", "-A"]);
    git(root, &["commit", "-qm", "remote"]);
    git(root, &["push", "-q", "origin", "main"]);
}

const PASSES: &str = "#!/bin/bash\ntouch ran-here-$(basename \"$0\" .sh)\nexit 0\n";
const FAILS: &str = "#!/bin/bash\ntouch ran-here-$(basename \"$0\" .sh)\nexit 1\n";

fn vitest_json(results: &[(&str, &str)]) -> String {
    let passed = results.iter().filter(|(_, s)| *s == "passed").count();
    let failed = results.iter().filter(|(_, s)| *s == "failed").count();
    let assertions: Vec<serde_json::Value> = results
        .iter()
        .map(|(name, status)| {
            serde_json::json!({
                "ancestorTitles": [], "fullName": name, "title": name, "status": status,
                "failureMessages": if *status == "failed" { vec!["expected 1 to be 2"] } else { vec![] }
            })
        })
        .collect();
    serde_json::json!({
        "numTotalTests": results.len(), "numPassedTests": passed, "numFailedTests": failed,
        "numPendingTests": results.len() - passed - failed, "success": failed == 0,
        "testResults": [{ "name": "/x/web/src/a.test.ts", "status": if failed == 0 { "passed" } else { "failed" },
                          "assertionResults": assertions }]
    })
    .to_string()
}

/// An `npx` first on PATH that logs where it ran and its arguments, and prints `json`.
fn fake_npx(root: &Path, json: &str) -> (String, PathBuf) {
    let bin = root.join(".test-home/bin");
    let log = root.join(".test-home/npx.log");
    write(root, ".test-home/report.json", json);
    write(
        root,
        ".test-home/bin/npx",
        &format!("#!/bin/sh\necho \"$PWD|$*\" >> '{}'\ncat '{}'\n", log.display(), root.join(".test-home/report.json").display()),
    );
    executable(&bin.join("npx"));
    (format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()), log)
}

// ── §1 a test reference names its runner ─────────────────────────────────────

/// S01: A test reference names its runner by its extension
#[test]
fn s01_a_test_reference_names_its_runner_by_its_extension() {
    let none = BTreeMap::new();
    let cases = [
        ("tests/a.rs::f", "tests/a.rs", Some("f"), Runner::Cargo),
        ("console/src/a.test.ts::adds two", "console/src/a.test.ts", Some("adds two"), Runner::Vitest),
        ("ci/check.sh", "ci/check.sh", None, Runner::Bash),
        ("ci/check.sh::fast", "ci/check.sh", Some("fast"), Runner::Bash),
    ];
    for (r, file, name, runner) in cases {
        let t = parse_test_ref(r).unwrap_or_else(|| panic!("{r} parses"));
        assert_eq!(t.file, file, "{r}");
        assert_eq!(t.name.as_deref(), name, "{r}");
        assert_eq!(runner_for(t.ext(), &none), Some(runner), "{r}");
    }
    let t = parse_test_ref("src/x.rs::tests::f").unwrap();
    assert_eq!((t.file.as_str(), t.name.as_deref()), ("src/x.rs", Some("tests::f")), "split at the first `::`");
}

/// S02: A ref with no extension, or a non-shell file with no name, is not a test reference; an unknown extension has no runner
#[test]
fn s02_an_unknown_ref_or_extension_is_refused() {
    for r in ["tests/a.ts", "noext::x", "::x"] {
        assert_eq!(parse_test_ref(r), None, "{r} is not a test reference");
    }
    let t = parse_test_ref("tests/a.rb::t").expect("parses");
    assert_eq!(runner_for(t.ext(), &BTreeMap::new()), None, "no runner for .rb");

    let d = spec_repo(
        &[("package.json", "{}\n"), ("tests/a.rb", "def t; end\n"), ("tests/ok.sh", PASSES)],
        Some("tests/a.rb::t"),
        Some("tests/ok.sh"),
        true,
    );
    let r = run(d.path(), &["spec", "verify", &the_id()]);
    assert_ne!(r.code, 0, "{}", r.out);
    assert!(r.out.contains(".rb") && r.out.contains("test.runners"), "names the extension and the setting:\n{}", r.out);
}

/// S03: A vitest scenario runs vitest from the package directory and passes when exactly the named test ran and passed
#[test]
fn s03_a_vitest_scenario_runs_in_its_package_and_passes() {
    let d = spec_repo(
        &[
            ("package.json", "{}\n"),
            ("web/package.json", "{\"name\":\"web\"}\n"),
            ("web/src/a.test.ts", "test('adds two', () => {})\n"),
            ("tests/ok.sh", PASSES),
        ],
        Some("web/src/a.test.ts::adds two"),
        Some("tests/ok.sh"),
        true,
    );
    let (path, log) = fake_npx(d.path(), &vitest_json(&[("adds two", "passed")]));
    let r = run_env(d.path(), &["spec", "verify", &the_id()], &[("PATH", &path)]);
    assert_eq!(r.code, 0, "{}", r.out);
    let line = std::fs::read_to_string(&log).unwrap_or_default();
    let (dir, args) = line.lines().next().and_then(|l| l.split_once('|')).unwrap_or_else(|| panic!("npx ran:\n{}", r.out));
    assert!(dir.ends_with("/web"), "vitest runs in the package directory, not {dir}");
    assert!(args.starts_with("vitest run src/a.test.ts -t "), "{args}");
    assert!(args.ends_with("--reporter=json"), "{args}");
}

/// S04: A vitest run that matched nothing, matched a skipped test, or ran more than one test is not a pass
#[test]
fn s04_a_vacuous_or_ambiguous_vitest_run_is_not_a_pass() {
    let n = "adds two";
    assert_eq!(judge_vitest(&vitest_json(&[]), n), Verdict::Vacuous);
    assert_eq!(judge_vitest(&vitest_json(&[(n, "skipped")]), n), Verdict::Vacuous);
    assert_eq!(judge_vitest(&vitest_json(&[(n, "passed"), ("adds two more", "passed")]), n), Verdict::Ambiguous(2));
    assert!(matches!(judge_vitest(&vitest_json(&[(n, "failed")]), n), Verdict::Failed(_)));
    assert!(matches!(judge_vitest("RUN v2.1.0\n no tests\n", n), Verdict::Unreadable(_)));
    assert_eq!(judge_vitest(&format!("npm warn exec\n{}\n", vitest_json(&[(n, "passed")])), n), Verdict::Passed);

    let d = spec_repo(
        &[("package.json", "{}\n"), ("web/src/a.test.ts", "test.skip('adds two', () => {})\n"), ("tests/ok.sh", PASSES)],
        Some("web/src/a.test.ts::adds two"),
        Some("tests/ok.sh"),
        true,
    );
    let (path, _) = fake_npx(d.path(), &vitest_json(&[(n, "skipped")]));
    let r = run_env(d.path(), &["spec", "verify", &the_id()], &[("PATH", &path)]);
    assert_ne!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("ran 0 tests"), "a skipped test is vacuous:\n{}", r.out);
}

/// S05: A bash test script passes on exit 0 and receives its argument
#[test]
fn s05_a_bash_script_passes_on_exit_zero_with_its_argument() {
    let d = spec_repo(
        &[("package.json", "{}\n"), ("tests/gate.sh", "#!/bin/bash\n[ \"$1\" = fast ]\n"), ("tests/ok.sh", PASSES)],
        Some("tests/gate.sh::fast"),
        Some("tests/ok.sh"),
        true,
    );
    let r = run(d.path(), &["spec", "verify", &the_id()]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("2 passed"), "{}", r.out);
}

/// S06: A bash script that fails, or that reports zero tests from a runner hexa reads, does not pass
#[test]
fn s06_a_failing_or_vacuous_bash_script_does_not_pass() {
    assert!(matches!(judge_script(false, "boom\n"), Verdict::Failed(_)));
    assert_eq!(judge_script(true, "running 0 tests\ntest result: ok. 0 passed; 0 failed\n"), Verdict::Vacuous);
    assert_eq!(judge_script(true, "all fine\n"), Verdict::Passed);

    let d = spec_repo(
        &[("package.json", "{}\n"), ("tests/bad.sh", FAILS), ("tests/ok.sh", PASSES)],
        Some("tests/bad.sh"),
        Some("tests/ok.sh"),
        true,
    );
    let r = run(d.path(), &["spec", "verify", &the_id()]);
    assert_ne!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("1 failed"), "{}", r.out);
}

// ── §2 configured runners ────────────────────────────────────────────────────

/// S07: A configured runner runs its template with the file and name quoted, and passes on exactly one counted test
#[test]
fn s07_a_configured_runner_runs_its_template() {
    let pj = serde_json::json!({ "test": { "runners": { "py": "bash run-py.sh {file} {name}" } } }).to_string();
    let d = spec_repo(
        &[
            ("package.json", "{}\n"),
            (".hexa/project.json", &pj),
            ("run-py.sh", "#!/bin/bash\nprintf '%s\\n' \"$#\" \"$1\" \"$2\" > .test-home/args\necho 'test result: ok. 1 passed; 0 failed'\n"),
            ("tests/t.py", "def a(): pass\n"),
            ("tests/ok.sh", PASSES),
        ],
        Some("tests/t.py::a name"),
        Some("tests/ok.sh"),
        true,
    );
    let r = run(d.path(), &["spec", "verify", &the_id()]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert_eq!(read(d.path(), ".test-home/args"), "2\ntests/t.py\na name\n", "file and name are two quoted arguments");
}

/// S08: A configured runner's unrecognised or many-test output is not a pass, and a configured runner overrides a built-in one
#[test]
fn s08_a_configured_runner_is_judged_by_the_evidence_reader_and_overrides() {
    assert!(matches!(judge_counted(true, "all good\n"), Verdict::Unreadable(_)));
    assert_eq!(judge_counted(true, "test result: ok. 2 passed; 0 failed\n"), Verdict::Ambiguous(2));
    assert_eq!(judge_counted(true, "test result: ok. 1 passed; 0 failed\n"), Verdict::Passed);
    assert!(matches!(judge_counted(false, "test result: FAILED. 0 passed; 1 failed\n"), Verdict::Failed(_)));
    let mut configured = BTreeMap::new();
    configured.insert("ts".to_string(), "npx jest {file} -t {name}".to_string());
    assert_eq!(runner_for("ts", &configured), Some(Runner::Configured("npx jest {file} -t {name}".to_string())));
    assert_eq!(runner_for("tsx", &configured), Some(Runner::Vitest), "only the configured extension is overridden");
}

// ── §3 the remote template ───────────────────────────────────────────────────

const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

/// S09: A remote template is filled with the full commit and the command as one quoted word
#[test]
fn s09_a_remote_template_is_filled() {
    let t = "bash ci/remote-gate.sh --commit {commit} --no-gate --extra {cmd}";
    let got = remote_command(t, COMMIT, "cargo test --test a -- f --exact").unwrap();
    assert_eq!(got, format!("bash ci/remote-gate.sh --commit '{COMMIT}' --no-gate --extra 'cargo test --test a -- f --exact'"));
}

/// S10: A template that lacks a placeholder, or a commit that is not a full id, is refused
#[test]
fn s10_a_template_without_a_placeholder_or_a_short_commit_is_refused() {
    let e = check_remote_template("run {commit}").unwrap_err();
    assert!(e.contains("{cmd}"), "{e}");
    let e = check_remote_template("run {cmd}").unwrap_err();
    assert!(e.contains("{commit}"), "{e}");
    assert!(check_remote_template("run {commit} {cmd}").is_ok());
    let e = remote_command("run {commit} {cmd}", "abc1234", "true").unwrap_err();
    assert!(e.contains("full commit"), "{e}");
    assert!(remote_command("run {cmd}", COMMIT, "true").is_err(), "the template is checked when it is used too");
}

/// S11: Any command passes through a remote template unchanged
#[test]
fn s11_any_command_passes_through_unchanged() {
    // A deterministic generator over the characters a shell treats specially.
    let alphabet: Vec<char> = "ab Z09 '\"$`\\!*?;&|<>(){}[]#~%^=\n\t-_./:é".chars().collect();
    let mut seed: u64 = 0x9e3779b97f4a7c15;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut cases: Vec<String> = vec![String::new(), "'".into(), "''".into(), "a'b".into(), "{cmd}".into(), "{commit}".into()];
    for _ in 0..120 {
        let len = (next() % 24) as usize;
        cases.push((0..len).map(|_| alphabet[(next() % alphabet.len() as u64) as usize]).collect());
    }
    for s in cases {
        let script = fill("printf %s {cmd}", &[("cmd", &s)]);
        let o = Command::new("bash").arg("-c").arg(&script).output().unwrap();
        assert!(o.status.success(), "{script:?}");
        assert_eq!(String::from_utf8_lossy(&o.stdout), s, "through {script:?}");
    }
}

// ── §4 one remote run per commit ─────────────────────────────────────────────

/// S12: Every scenario of a commit goes to the remote runner in one run, and its results are the scenarios' results
#[test]
fn s12_every_scenario_goes_to_the_remote_in_one_run() {
    let d = spec_repo(
        &[("package.json", "{}\n"), ("tests/a.sh", PASSES), ("tests/b.sh", PASSES)],
        Some("tests/a.sh"),
        Some("tests/b.sh"),
        true,
    );
    let remote = remote_for(d.path(), false);
    use_remote(d.path(), &remote);
    let r = run(d.path(), &["spec", "verify", &the_id()]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("2 passed"), "{}", r.out);
    assert_eq!(invocations(&remote), 1, "one remote run for the commit:\n{}", r.out);
    assert!(!d.path().join("ran-here-a").exists() && !d.path().join("ran-here-b").exists(), "nothing ran here");
}

/// S13: A test that fails on the remote fails the scenario, and nothing falls back to running it here
#[test]
fn s13_a_remote_failure_fails_and_nothing_runs_here() {
    let d = spec_repo(
        &[("package.json", "{}\n"), ("tests/a.sh", PASSES), ("tests/b.sh", FAILS)],
        Some("tests/a.sh"),
        Some("tests/b.sh"),
        true,
    );
    let remote = remote_for(d.path(), false);
    use_remote(d.path(), &remote);
    let r = run(d.path(), &["spec", "verify", &the_id()]);
    assert_ne!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("1 passed") && r.out.contains("1 failed"), "{}", r.out);
    assert!(!d.path().join("ran-here-b").exists(), "the failing test did not run here");
}

/// S14: The batch markers carry each command's exit status and output back to its own test
#[test]
fn s14_batch_markers_carry_each_result_back() {
    let tag = "t4g42";
    let cmds = vec!["echo one; exit 0".to_string(), "echo two; exit 3".to_string()];
    let (_, out) = bash(&batch_script(tag, &cmds));
    let got = split_batch(tag, 2, &out).unwrap_or_else(|e| panic!("{e}:\n{out}"));
    assert_eq!(got.len(), 2);
    assert!(got[0].0 && got[0].1.contains("one") && !got[0].1.contains("two"), "{got:?}");
    assert!(!got[1].0 && got[1].1.contains("two") && !got[1].1.contains("one"), "{got:?}");
}

// ── §5 the remote proves itself, and never falls back ────────────────────────

/// S15: An unpushed commit is refused before anything runs
#[test]
fn s15_an_unpushed_commit_is_refused() {
    let d = spec_repo(&[("package.json", "{}\n"), ("tests/a.sh", PASSES), ("tests/b.sh", PASSES)], Some("tests/a.sh"), Some("tests/b.sh"), true);
    let remote = remote_for(d.path(), false);
    use_remote(d.path(), &remote);
    write(d.path(), "notes.txt", "later\n");
    git(d.path(), &["add", "-A"]);
    git(d.path(), &["commit", "-qm", "not pushed"]);
    let r = run(d.path(), &["spec", "verify", &the_id()]);
    assert_ne!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("git push"), "{}", r.out);
    assert_eq!(invocations(&remote), 0, "nothing was sent");
}

/// S16: A runner that does not name the commit, or loses a marker, is a runner failure: not a pass and not red
#[test]
fn s16_a_runner_that_does_not_prove_itself_is_a_runner_failure() {
    let d = spec_repo(&[("package.json", "{}\n"), ("tests/a.sh", PASSES), ("tests/b.sh", PASSES)], Some("tests/a.sh"), Some("tests/b.sh"), true);
    let remote = remote_for(d.path(), true);
    use_remote(d.path(), &remote);
    let r = run(d.path(), &["spec", "verify", &the_id()]);
    assert_ne!(r.code, 0, "{}", r.out);
    assert!(r.out.to_lowercase().contains("runner"), "names the runner failure:\n{}", r.out);
    assert!(!r.out.contains("2 passed"), "{}", r.out);

    // red: S1 pending, bound to a failing script; a silent runner proves nothing.
    let d = spec_repo(&[("package.json", "{}\n"), ("tests/a.sh", FAILS), ("tests/b.sh", PASSES)], None, Some("tests/b.sh"), true);
    let remote = remote_for(d.path(), true);
    use_remote(d.path(), &remote);
    let before = read(d.path(), &spec_rel());
    let head = git(d.path(), &["rev-parse", "HEAD"]);
    let r = run(d.path(), &["spec", "red", &the_id(), &head, "--test", "S1=tests/a.sh"]);
    assert_ne!(r.code, 0, "a runner failure is not red:\n{}", r.out);
    assert_eq!(read(d.path(), &spec_rel()), before, "nothing recorded");

    let tag = "t4g42";
    let (_, out) = bash(&batch_script(tag, &["echo one".to_string(), "echo two".to_string()]));
    let cut = out.rfind("t4g42").unwrap();
    let e = split_batch(tag, 2, &out[..cut.saturating_sub(1)]).unwrap_err();
    assert!(e.contains("marker"), "{e}");
}

/// S17: A tree whose tracked files differ from HEAD is refused for a remote run
#[test]
fn s17_a_tree_that_differs_from_head_is_refused() {
    let d = spec_repo(&[("package.json", "{}\n"), ("tests/a.sh", PASSES), ("tests/b.sh", PASSES)], Some("tests/a.sh"), Some("tests/b.sh"), true);
    let remote = remote_for(d.path(), false);
    use_remote(d.path(), &remote);
    write(d.path(), "package.json", "{\"changed\":true}\n");
    let r = run(d.path(), &["spec", "verify", &the_id()]);
    assert_ne!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("HEAD") && r.out.contains("push"), "{}", r.out);
    assert_eq!(invocations(&remote), 0);
}

/// S18: `--local` runs the tests here even when a remote is set
#[test]
fn s18_local_runs_here_even_with_a_remote() {
    let d = spec_repo(&[("package.json", "{}\n"), ("tests/a.sh", PASSES), ("tests/b.sh", PASSES)], Some("tests/a.sh"), Some("tests/b.sh"), true);
    let remote = remote_for(d.path(), false);
    use_remote(d.path(), &remote);
    let r = run(d.path(), &["spec", "verify", &the_id(), "--local"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert_eq!(invocations(&remote), 0, "the runner was not used");
    assert!(d.path().join("ran-here-a").exists(), "it ran here");
}

// ── §6 do gate under a remote ────────────────────────────────────────────────

fn gate_repo() -> (tempfile::TempDir, Remote) {
    let d = tempfile::tempdir().unwrap();
    let root = d.path();
    write(root, ".gitignore", ".test-home/\ntarget/\n");
    write(root, "ok.sh", "#!/bin/bash\necho ok\n");
    write(root, "f.txt", "one\n");
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["add", "-A"]);
    git(root, &["commit", "-qm", "init"]);
    let remote = remote_for(root, false);
    use_remote(root, &remote);
    git(root, &["checkout", "-q", "-b", "work"]);
    write(root, "f.txt", "two\n");
    (d, remote)
}

/// S19: `hexa do gate` under a remote refuses without `--local`
#[test]
fn s19_do_gate_under_a_remote_refuses_without_local() {
    let (d, remote) = gate_repo();
    let head = git(d.path(), &["rev-parse", "HEAD"]);
    let r = run(d.path(), &["do", "gate", "--files", "f.txt", "--evidence", "bash ok.sh", "-m", "msg"]);
    assert_ne!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("--local"), "{}", r.out);
    assert_eq!(git(d.path(), &["rev-parse", "HEAD"]), head, "nothing committed");
    assert_eq!(read(d.path(), "f.txt"), "two\n", "the change stays in the tree");
    assert_eq!(invocations(&remote), 0);
}

/// S20: `hexa do gate --local` under a remote gates here and commits
#[test]
fn s20_do_gate_local_gates_here_and_commits() {
    let (d, remote) = gate_repo();
    let head = git(d.path(), &["rev-parse", "HEAD"]);
    let r = run(d.path(), &["do", "gate", "--local", "--no-land", "--files", "f.txt", "--evidence", "bash ok.sh", "-m", "msg"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert_ne!(git(d.path(), &["rev-parse", "HEAD"]), head, "committed");
    assert_eq!(git(d.path(), &["status", "--porcelain", "--untracked-files=no"]), "", "{}", r.out);
    assert_eq!(invocations(&remote), 0);
}

// ── §7 the evidence reader counts vitest and one bash script ─────────────────

/// S21: The evidence reader counts vitest's summary and its JSON report
#[test]
fn s21_vitest_summary_and_json_are_counted() {
    let summary = " RUN  v2.1.8 /x/web\n\n ✓ src/a.test.ts (5 tests) 3ms\n\n Test Files  1 passed (1)\n      Tests  5 passed (5)\n   Duration  210ms\n";
    assert_eq!(read_gate_evidence("npx vitest run", true, summary, false), Evidence::Passed { tests: 5 });
    let json = serde_json::json!({"numTotalTests": 4, "numPassedTests": 4, "numFailedTests": 0, "numPendingTests": 0, "testResults": []}).to_string();
    assert_eq!(read_gate_evidence("npx vitest run --reporter=json", true, &json, false), Evidence::Passed { tests: 4 });
}

/// S22: A vitest run that failed a test or ran none does not pass
#[test]
fn s22_a_failing_or_empty_vitest_run_does_not_pass() {
    let failed = " Test Files  1 failed (1)\n      Tests  1 failed | 4 passed (5)\n";
    assert!(matches!(read_gate_evidence("npx vitest run", true, failed, false), Evidence::Failed { .. }));
    let skipped = " Test Files  1 skipped (1)\n      Tests  3 skipped (3)\n";
    assert_eq!(read_gate_evidence("npx vitest run", true, skipped, false), Evidence::Vacuous);
}

/// S23: A gate that is exactly one bash script counts as one passed test
#[test]
fn s23_one_bash_script_counts_as_one_test() {
    for cmd in ["bash ci/check.sh", "sh ci/check.sh --fast", "./ci/check.sh"] {
        assert!(is_single_script(cmd), "{cmd}");
        assert_eq!(read_gate_evidence(cmd, true, "ok\n", false), Evidence::Passed { tests: 1 }, "{cmd}");
    }
}

/// S24: A compound command, a failing script, or a script that ran zero tests is not counted as a script pass
#[test]
fn s24_compound_failing_or_empty_scripts_are_not_a_script_pass() {
    assert!(!is_single_script("bash a.sh && echo done"));
    for cmd in ["bash a.sh; true", "bash a.sh | tail", "bash a.sh || true", "echo a.sh", "bash -c 'x'"] {
        assert!(!is_single_script(cmd), "{cmd}");
    }
    assert_eq!(read_gate_evidence("bash a.sh && echo done", true, "ok\n", false), Evidence::Unrecognised);
    assert!(matches!(read_gate_evidence("bash a.sh", false, "ok\n", false), Evidence::Failed { .. }));
    assert_eq!(read_gate_evidence("bash a.sh", true, "running 0 tests\ntest result: ok. 0 passed; 0 failed\n", false), Evidence::Vacuous);
}

// ── §8 test files ────────────────────────────────────────────────────────────

/// S25: Test files outside tests/ are sealable by their name or their kind
#[test]
fn s25_test_files_outside_tests_are_sealable() {
    for p in ["tests/a.rs", "web/src/a.test.ts", "web/src/a.spec.tsx", "src/__tests__/x.ts", "test/x.js", "ci/check.sh"] {
        assert!(is_test_file(p), "{p}");
    }
    let d = spec_repo(
        &[("package.json", "{}\n"), ("web/src/a.test.ts", "test('adds two', () => {})\n"), ("tests/ok.sh", PASSES)],
        Some("web/src/a.test.ts::adds two"),
        Some("tests/ok.sh"),
        true,
    );
    let spec: serde_json::Value = serde_json::from_str(&read(d.path(), &spec_rel())).unwrap();
    assert!(spec["sealed"]["web/src/a.test.ts"].is_string(), "the seal records it: {}", spec["sealed"]);
}

/// S26: A source file is not a test file and is not bound as one
#[test]
fn s26_a_source_file_is_not_a_test_file() {
    for p in ["src/lib.ts", "web/src/app.tsx", "src/testing.rs", "contest/x.ts"] {
        assert!(!is_test_file(p), "{p}");
    }
    let d = spec_repo(&[("package.json", "{}\n"), ("web/src/app.tsx", "export const x = 1\n"), ("tests/ok.sh", PASSES)], None, Some("tests/ok.sh"), true);
    let before = read(d.path(), &spec_rel());
    let head = git(d.path(), &["rev-parse", "HEAD"]);
    let r = run(d.path(), &["spec", "red", &the_id(), &head, "--test", "S1=web/src/app.tsx::x"]);
    assert_ne!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("not a test file"), "{}", r.out);
    assert_eq!(read(d.path(), &spec_rel()), before, "nothing recorded");
}

// ── §9 where a spec may be accepted ──────────────────────────────────────────

/// S27: A project with a package.json and no Cargo.toml may accept a spec
#[test]
fn s27_a_package_json_project_may_accept() {
    let d = spec_repo(&[("package.json", "{}\n")], None, None, false);
    let r = run(d.path(), &["spec", "accept", &the_id(), "--by", "a reviewer"]);
    assert_eq!(r.code, 0, "{}", r.out);
    let spec: serde_json::Value = serde_json::from_str(&read(d.path(), &spec_rel())).unwrap();
    assert_eq!(spec["accepted"]["by"], "a reviewer");
}

/// S28: A project where no test could be bound is still refused, with the old wording and the way to bind another language
#[test]
fn s28_a_project_with_no_runner_is_refused() {
    let d = spec_repo(&[("README.md", "x\n")], None, None, false);
    let before = read(d.path(), &spec_rel());
    let r = run(d.path(), &["spec", "accept", &the_id(), "--by", "a reviewer"]);
    assert_ne!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("Cargo.toml") && r.out.contains("Rust") && r.out.contains("spec red") && r.out.contains("--test"), "{}", r.out);
    assert_eq!(read(d.path(), &spec_rel()), before);
}
