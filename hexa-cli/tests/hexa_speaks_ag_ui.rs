//! hexa speaks AG-UI (ADR-2611161830): a working verb with `--events` reports
//! itself as an AG-UI 1.1 event stream, JSON lines on stdout and nothing else,
//! every line valid against the protocol's own schema.

use serde_json::Value;
use std::path::Path;
use std::process::Command;

fn validator() -> jsonschema::Validator {
    let raw = include_str!("../../hexa-core/tests/fixtures/ag-ui/schema-draft-1.1.json");
    jsonschema::draft202012::new(&serde_json::from_str::<Value>(raw).unwrap()).unwrap()
}

fn git(dir: &Path, args: &[&str]) {
    let o = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@x", "-c", "commit.gpgsign=false", "-c", "init.defaultBranch=main"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
}

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

fn hexa(dir: &Path, args: &[&str]) -> Out {
    hexa_env(dir, args, &[])
}

fn hexa_env(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Out {
    let home = dir.join(".test-home");
    std::fs::create_dir_all(&home).unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(args)
        .current_dir(dir)
        .env("HOME", &home)
        .env("HEXA_HOME", home.join(".hexa"))
        .env("CLAUDE_PROJECT_DIR", dir)
        .env("HEXA_SESSION_ID", "ag-ui-session")
        .env_remove("HEXA_EVENTS")
        .env_remove("HEXA_TASK")
        .env_remove("HEXA_INTERNAL")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@x")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@x")
        .env("NO_COLOR", "1")
        .envs(env.iter().copied())
        .output()
        .unwrap();
    Out { code: o.status.code().unwrap_or(-1), stdout: String::from_utf8_lossy(&o.stdout).into(), stderr: String::from_utf8_lossy(&o.stderr).into() }
}

fn repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    git(d.path(), &["init", "-q", "-b", "main"]);
    std::fs::create_dir_all(d.path().join(".hexa")).unwrap();
    std::fs::write(d.path().join(".gitignore"), ".hexa/\n.test-home/\n").unwrap();
    std::fs::write(d.path().join("a.txt"), "one\n").unwrap();
    std::fs::write(d.path().join("fail.sh"), "#!/bin/sh\necho 'test result: FAILED. 0 passed; 1 failed; 0 ignored'\nexit 1\n").unwrap();
    git(d.path(), &["add", "-A"]);
    git(d.path(), &["commit", "-q", "-m", "init"]);
    d
}

/// Every stdout line is a schema-valid event; returns them.
fn events(out: &Out) -> Vec<Value> {
    let v = validator();
    assert!(!out.stdout.trim().is_empty(), "no events: stderr {}", out.stderr);
    out.stdout
        .lines()
        .map(|l| {
            let doc: Value = serde_json::from_str(l).unwrap_or_else(|e| panic!("stdout carries only events, not {l:?} ({e})"));
            let errs: Vec<String> = v.iter_errors(&doc).map(|e| e.to_string()).collect();
            assert!(errs.is_empty(), "{l}: {errs:?}");
            doc
        })
        .collect()
}

fn types(evs: &[Value]) -> Vec<&str> {
    evs.iter().map(|e| e["type"].as_str().unwrap()).collect()
}

#[test]
fn finish_with_events_is_one_run_whose_activity_is_the_open_work() {
    let d = repo();
    std::fs::write(d.path().join("a.txt"), "changed\n").unwrap();
    let out = hexa(d.path(), &["finish", "--events"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let evs = events(&out);
    let t = types(&evs);
    assert_eq!(t.first(), Some(&"RUN_STARTED"));
    assert_eq!(t.last(), Some(&"RUN_FINISHED"));
    assert_eq!(evs[0]["protocolVersion"], "1.1");
    assert_eq!(evs[0]["threadId"], "ag-ui-session", "the thread is the loop session");
    let act: Vec<&Value> = evs.iter().filter(|e| e["type"] == "ACTIVITY_SNAPSHOT").collect();
    assert_eq!(act.len(), 1);
    assert_eq!(act[0]["activityType"], "hexa.open_work");
    assert!(act[0]["content"]["open"].as_array().unwrap().iter().any(|i| i["kind"] == "uncommitted"), "{}", act[0]);
    assert!(out.stderr.contains("open work"), "the person's text goes to stderr: {}", out.stderr);
}

#[test]
fn events_to_a_file_leave_stdout_to_the_person() {
    let d = repo();
    let path = d.path().join(".hexa").join("events.jsonl");
    let out = hexa(d.path(), &["finish", &format!("--events={}", path.display())]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(out.stdout.contains("nothing open"), "stdout {} stderr {}", out.stdout, out.stderr);
    let file = Out { code: 0, stdout: std::fs::read_to_string(&path).unwrap(), stderr: String::new() };
    assert_eq!(types(&events(&file)), vec!["RUN_STARTED", "STEP_STARTED", "ACTIVITY_SNAPSHOT", "STEP_FINISHED", "RUN_FINISHED"]);
}

#[test]
fn a_failing_gate_ends_the_run_in_run_error_with_its_steps_closed() {
    let d = repo();
    std::fs::write(d.path().join("a.txt"), "changed\n").unwrap();
    let out = hexa(d.path(), &["do", "gate", "--uncounted", "--files", "a.txt", "-e", "sh fail.sh", "-m", "x", "--events"]);
    assert_ne!(out.code, 0);
    let evs = events(&out);
    let t = types(&evs);
    assert_eq!(t.first(), Some(&"RUN_STARTED"));
    assert_eq!(t.last(), Some(&"RUN_ERROR"), "{t:?}");
    assert_eq!(evs.last().unwrap()["code"], "gate_failed");
    let opened = evs.iter().position(|e| e["type"] == "STEP_STARTED" && e["stepName"] == "gate").expect("a gate step");
    let closed = evs.iter().position(|e| e["type"] == "STEP_FINISHED" && e["stepName"] == "gate").expect("closed");
    assert!(opened < closed && closed < evs.len() - 1);
}

// ── Every working verb is a run (fix for 26.11.13, where only `finish` and
// `do gate` opened one and `analyze`, `loop`, `spec verify` and `do runs`
// left `--events=<path>` an empty file and `--events` an empty stdout). ──

/// The four ways of turning events on (ADR-2611161830 §1).
#[derive(Clone, Copy, Debug)]
enum Way {
    Flag,
    FlagPath,
    Env1,
    EnvPath,
}

const WAYS: [Way; 4] = [Way::Flag, Way::FlagPath, Way::Env1, Way::EnvPath];

/// Run `args` with events on by `way`; return the exit code and the events,
/// each validated against the schema. Zero events fails.
fn run_with(dir: &Path, args: &[&str], way: Way) -> (i32, Vec<Value>, String) {
    let path = dir.join(".hexa").join(format!("events-{way:?}.jsonl"));
    let _ = std::fs::remove_file(&path);
    let path_s = path.display().to_string();
    let flag = format!("--events={path_s}");
    let mut a: Vec<&str> = Vec::new();
    match way {
        Way::Flag => a.push("--events"),
        Way::FlagPath => a.push(&flag),
        _ => {}
    }
    a.extend_from_slice(args);
    let env: Vec<(&str, &str)> = match way {
        Way::Env1 => vec![("HEXA_EVENTS", "1")],
        Way::EnvPath => vec![("HEXA_EVENTS", path_s.as_str())],
        _ => vec![],
    };
    let out = hexa_env(dir, &a, &env);
    let stream = match way {
        Way::Flag | Way::Env1 => Out { code: out.code, stdout: out.stdout.clone(), stderr: out.stderr.clone() },
        Way::FlagPath | Way::EnvPath => {
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            Out { code: out.code, stdout: text, stderr: out.stderr.clone() }
        }
    };
    assert!(!stream.stdout.trim().is_empty(), "{args:?} via {way:?}: zero events (exit {}); stderr: {}", out.code, out.stderr);
    (out.code, events(&stream), out.stderr)
}

/// One run: `RUN_STARTED` first (protocol 1.1), exactly one terminal event
/// last, `RUN_FINISHED` on exit 0 and `RUN_ERROR` otherwise, and the verb's
/// own step opened and closed inside it.
fn assert_one_run(args: &[&str], step: &str, way: Way, code: i32, evs: &[Value]) {
    let t = types(evs);
    assert_eq!(t.first(), Some(&"RUN_STARTED"), "{args:?} via {way:?}: {t:?}");
    assert_eq!(evs[0]["protocolVersion"], "1.1");
    assert_eq!(t.iter().filter(|x| **x == "RUN_STARTED").count(), 1, "{t:?}");
    let terminal = if code == 0 { "RUN_FINISHED" } else { "RUN_ERROR" };
    assert_eq!(t.last(), Some(&terminal), "{args:?} via {way:?} exited {code}: {t:?}");
    assert_eq!(t.iter().filter(|x| **x == "RUN_FINISHED" || **x == "RUN_ERROR").count(), 1, "{t:?}");
    let opened = evs.iter().position(|e| e["type"] == "STEP_STARTED" && e["stepName"] == step).unwrap_or_else(|| panic!("{args:?} via {way:?}: no step {step:?} in {t:?}"));
    let closed = evs.iter().position(|e| e["type"] == "STEP_FINISHED" && e["stepName"] == step).unwrap_or_else(|| panic!("{args:?} via {way:?}: step {step:?} not closed in {t:?}"));
    assert!(opened < closed && closed < evs.len() - 1);
    assert!(evs.iter().all(|e| e["timestamp"].is_i64()), "every event is timed, so a consumer can measure the run");
}

fn every_way(args: &[&str], step: &str) {
    let d = repo();
    for way in WAYS {
        let (code, evs, _) = run_with(d.path(), args, way);
        assert_one_run(args, step, way, code, &evs);
    }
}

#[test]
fn analyze_speaks_ag_ui_every_way() {
    every_way(&["analyze", "."], "analyze");
}

#[test]
fn analyze_that_exits_early_still_ends_its_run() {
    // `--exit-code` leaves through process::exit; the run must still end
    let d = repo();
    for way in WAYS {
        let (code, evs, _) = run_with(d.path(), &["analyze", ".", "--exit-code"], way);
        assert_one_run(&["analyze", ".", "--exit-code"], "analyze", way, code, &evs);
    }
}

#[test]
fn loop_speaks_ag_ui_every_way() {
    every_way(&["loop"], "loop");
    every_way(&["loop", "task", "add", "one step"], "loop task add");
}

#[test]
fn spec_verify_speaks_ag_ui_every_way() {
    every_way(&["spec", "verify"], "spec verify");
}

#[test]
fn do_speaks_ag_ui_every_way() {
    every_way(&["do", "runs"], "do runs");
}

#[test]
fn do_gate_keeps_its_phases_inside_the_verb_step() {
    let d = repo();
    std::fs::write(d.path().join("a.txt"), "changed\n").unwrap();
    for way in WAYS {
        let (code, evs, _) = run_with(d.path(), &["do", "gate", "--uncounted", "--files", "a.txt", "-e", "sh fail.sh", "-m", "x"], way);
        assert_ne!(code, 0);
        assert_one_run(&["do", "gate"], "do gate", way, code, &evs);
        assert_eq!(evs.last().unwrap()["code"], "gate_failed", "the verb's own failure code survives");
    }
}

#[test]
fn a_hook_run_by_the_agent_never_writes_events_to_its_stdout() {
    // Claude Code reads a hook's stdout; HEXA_EVENTS=1 exported in the shell
    // must not turn that into an event stream
    let d = repo();
    std::fs::write(d.path().join(".hexa").join("project.json"), "{\"name\":\"t\"}").unwrap();
    let plain = hexa(d.path(), &["hook", "session-start"]);
    let out = hexa_env(d.path(), &["hook", "session-start"], &[("HEXA_EVENTS", "1")]);
    assert!(!out.stdout.contains("RUN_STARTED"), "a hook's stdout is the agent's: {}", out.stdout);
    assert!(!plain.stdout.trim().is_empty(), "the hook prints to the agent: {}", plain.stderr);
    assert_eq!(out.stdout.lines().count(), plain.stdout.lines().count(), "the hook's stdout stays on stdout, not moved to stderr");
}
