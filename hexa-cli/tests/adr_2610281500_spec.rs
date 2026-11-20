//! ADR-2610281500: a managed `hexa do run --json`, end to end, in a fresh
//! git repository, against a loopback stub standing in for the harness's
//! model proxy. Every model request reaches the stub with the token and
//! nothing reaches the decoy (`ANTHROPIC_BASE_URL`); the working tree holds
//! only the task's change and its commit (§6); the evidence command never
//! sees the token or the managed settings (§7); the result is one JSON
//! object with the ADR's codes and exit statuses (§2, §5, §8, §9).
//!
//! Written before the code, from the ADR; not derived from it.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const TOKEN: &str = "tok-e2e-0123456789abcdef";
const MARK: &str = "managed-was-here";

// ----------------------------------------------------------------- stub ----

#[derive(Clone, Debug)]
struct Seen {
    path: String,
    headers: Vec<(String, String)>,
}

struct Stub {
    port: u16,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Stub {
    /// Answers every request with `status` and `body`.
    fn start(status: u16, body: String) -> Stub {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        std::thread::spawn(move || {
            for c in l.incoming().flatten() {
                let (s, body) = (s.clone(), body.clone());
                std::thread::spawn(move || serve(c, s, status, &body));
            }
        });
        Stub { port, seen }
    }
    fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

fn serve(c: TcpStream, seen: Arc<Mutex<Vec<Seen>>>, status: u16, body: &str) {
    let mut r = BufReader::new(c.try_clone().unwrap());
    let mut first = String::new();
    if r.read_line(&mut first).is_err() {
        return;
    }
    let mut headers = Vec::new();
    let mut len = 0usize;
    loop {
        let mut l = String::new();
        if r.read_line(&mut l).unwrap_or(0) == 0 || l == "\r\n" {
            break;
        }
        if let Some((k, v)) = l.trim_end().split_once(':') {
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim().to_string());
            if k == "content-length" {
                len = v.parse().unwrap_or(0);
            }
            headers.push((k, v));
        }
    }
    let mut b = vec![0u8; len];
    let _ = r.read_exact(&mut b);
    let path = first.split(' ').nth(1).unwrap_or("").to_string();
    seen.lock().unwrap().push(Seen { path, headers });
    let mut c = c;
    let _ = write!(
        c,
        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
}

fn edit_reply() -> String {
    let text = format!("MODE: append\n```\n{MARK}\n```\n");
    serde_json::json!({
        "id": "msg_1", "type": "message", "role": "assistant", "model": "m-two",
        "content": [{"type": "text", "text": text}],
        "stop_reason": "end_turn", "stop_sequence": null,
        "usage": {"input_tokens": 10, "output_tokens": 3}
    })
    .to_string()
}

// ------------------------------------------------------------------ rig ----

struct Rig {
    _dir: tempfile::TempDir,
    parent: PathBuf,
    repo: PathBuf,
    state: PathBuf,
    home: PathBuf,
    probe: PathBuf,
}

fn git(repo: &Path, args: &[&str]) -> String {
    let o = Command::new("git").args(args).current_dir(repo).output().unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8(o.stdout).unwrap()
}

fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let parent = dir.path().join("p");
    let repo = parent.join("repo");
    let state = dir.path().join("state");
    let home = dir.path().join("home");
    let probe = dir.path().join("probe");
    for d in [&repo, &state, &home, &probe] {
        std::fs::create_dir_all(d).unwrap();
    }
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("notes.txt"), "start\n").unwrap();
    git(&repo, &["add", "notes.txt"]);
    git(&repo, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "start"]);
    Rig { _dir: dir, parent, repo, state, home, probe }
}

fn tiers() -> String {
    r#"{"t1":{"model":"m-two","api":"anthropic"},"t2":{"model":"m-two","api":"anthropic"},"t2.5":{"model":"m-two","api":"anthropic"},"t3":{"model":"m-two","api":"anthropic"},"react":{"model":"m-two","api":"anthropic"}}"#.into()
}

fn token_pipe() -> OwnedFd {
    let (r, mut w) = std::io::pipe().unwrap();
    w.write_all(TOKEN.as_bytes()).unwrap();
    drop(w);
    OwnedFd::from(r)
}

/// `hexa do run --json --fast` in managed mode. `over` changes or (with
/// `None`) removes variables; the token fd is inherited as fd 3.
fn run(rig: &Rig, url: &str, evidence: &str, over: &[(&str, Option<String>)]) -> Output {
    let decoy = Stub::start(200, edit_reply());
    let fd = token_pipe();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_hexa"));
    cmd.args(["do", "run", "Append the marker line", "--file", "notes.txt", "--fast", "--json", "--uncounted", "--evidence", evidence])
        .current_dir(&rig.repo)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap())
        .env("HOME", &rig.home)
        .env("GIT_AUTHOR_NAME", "hexa").env("GIT_AUTHOR_EMAIL", "hexa@test")
        .env("GIT_COMMITTER_NAME", "hexa").env("GIT_COMMITTER_EMAIL", "hexa@test")
        .env("HEXA_MANAGED", "1")
        .env("HEXA_MANAGED_URL", url)
        .env("HEXA_MANAGED_TOKEN_FD", "3")
        .env("HEXA_MANAGED_TIERS", tiers())
        .env("HEXA_STATE_DIR", &rig.state)
        // Settings managed mode must ignore.
        .env("ANTHROPIC_BASE_URL", decoy.url())
        .env("ANTHROPIC_API_KEY", "sk-ant-decoy")
        .env("HEXA_OLLAMA_HOST", decoy.url())
        .env("HEXA_DIRECT_MODEL", "decoy-model")
        .env("PROBE", &rig.probe)
        .stdin(Stdio::null());
    for (k, v) in over {
        match v {
            Some(v) => cmd.env(k, v),
            None => cmd.env_remove(k),
        };
    }
    let raw = fd.as_raw_fd();
    // SAFETY: dup2 is async-signal-safe; it puts the token pipe at fd 3 in the child.
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(move || {
            if libc_dup2(raw, 3) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let out = cmd.output().unwrap();
    assert!(decoy.seen().is_empty(), "a request went to the decoy: {:?}", decoy.seen());
    out
}

unsafe extern "C" {
    fn dup2(old: i32, new: i32) -> i32;
}

fn libc_dup2(old: i32, new: i32) -> i32 {
    // SAFETY: plain syscall wrapper.
    unsafe { dup2(old, new) }
}

/// The one JSON object on stdout.
fn result(o: &Output) -> serde_json::Value {
    let out = String::from_utf8_lossy(&o.stdout);
    let lines: Vec<&str> = out.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 1, "stdout must be one JSON object:\n{out}\nstderr:\n{}", String::from_utf8_lossy(&o.stderr));
    serde_json::from_str(lines[0]).unwrap_or_else(|e| panic!("{e}: {}", lines[0]))
}

fn files_under(d: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(d) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(files_under(&p));
            }
            out.push(p);
        }
    }
    out
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

// ---------------------------------------------------------------- tests ----

#[test]
fn a_managed_run_edits_commits_and_reports_one_json_result() {
    let r = rig();
    let s = Stub::start(200, edit_reply());
    let evidence = format!("grep -q {MARK} notes.txt && env > \"$PROBE/env\" && ls /proc/self/fd > \"$PROBE/fds\"");
    let o = run(&r, &s.url(), &evidence, &[]);
    let v = result(&o);
    assert_eq!(o.status.code(), Some(0), "{v}\n{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(v["ok"], true);
    assert_eq!(v["code"], serde_json::Value::Null);
    assert!(v["attempts"].as_u64().unwrap() >= 1);
    assert!(v["message"].is_string());
    let head = git(&r.repo, &["rev-parse", "HEAD"]);
    assert_eq!(v["commit"].as_str().unwrap(), head.trim());

    // Every model request went to the managed URL with the token.
    let seen = s.seen();
    assert!(!seen.is_empty());
    for req in &seen {
        assert_eq!(req.path, "/v1/messages");
        assert!(req.headers.contains(&("x-api-key".into(), TOKEN.into())), "{req:?}");
    }

    // §6: the working tree holds only the task's change, committed.
    assert_eq!(git(&r.repo, &["status", "--porcelain", "--ignored"]), "", "the tree is not clean");
    assert!(std::fs::read_to_string(r.repo.join("notes.txt")).unwrap().contains(MARK));
    assert!(!r.repo.join(".hexa").exists(), "a .hexa in the repository");
    let siblings: Vec<_> = std::fs::read_dir(&r.parent).unwrap().flatten().map(|e| e.file_name()).collect();
    assert_eq!(siblings, vec![std::ffi::OsString::from("repo")], "something next to the repository");
    assert!(files_under(&r.home).is_empty(), "hexa wrote under HOME: {:?}", files_under(&r.home));

    // §7: the evidence never saw the token or the managed settings.
    let env = std::fs::read_to_string(r.probe.join("env")).unwrap();
    assert!(!env.contains(TOKEN));
    for var in ["HEXA_MANAGED_URL=", "HEXA_MANAGED_TOKEN_FD=", "HEXA_MANAGED_TIERS=", "ANTHROPIC_BASE_URL=", "ANTHROPIC_API_KEY="] {
        assert!(!env.lines().any(|l| l.starts_with(var)), "the evidence saw {var}");
    }
    let fds = std::fs::read_to_string(r.probe.join("fds")).unwrap();
    assert!(!fds.lines().any(|l| l.trim() == "3"), "the token fd reached the evidence: {fds}");
}

#[test]
fn a_missing_setting_exits_64_with_code_config_and_calls_nothing() {
    let r = rig();
    let s = Stub::start(200, edit_reply());
    for (var, value) in [
        ("HEXA_MANAGED_TIERS", None),
        ("HEXA_STATE_DIR", None),
        ("HEXA_MANAGED_TIERS", Some("{}".to_string())),
        ("HEXA_MANAGED_DELEGATE", Some("claude".to_string())),
    ] {
        let o = run(&r, &s.url(), "true", &[(var, value.clone())]);
        assert_eq!(o.status.code(), Some(64), "{var}={value:?}");
        let v = result(&o);
        assert_eq!(v["ok"], false);
        assert_eq!(v["code"], "config");
        assert!(v["message"].as_str().unwrap().contains(var), "{v}");
        assert!(!String::from_utf8_lossy(&o.stderr).contains(TOKEN));
    }
    assert!(s.seen().is_empty(), "a model call before the settings were checked");
    assert_eq!(git(&r.repo, &["status", "--porcelain", "--ignored"]), "");
}

#[test]
fn a_failing_gate_is_exit_1_with_code_gate_failed_and_no_commit() {
    let r = rig();
    let s = Stub::start(200, edit_reply());
    let before = git(&r.repo, &["rev-parse", "HEAD"]);
    let o = run(&r, &s.url(), "false", &[]);
    assert_eq!(o.status.code(), Some(1));
    let v = result(&o);
    assert_eq!(v["ok"], false);
    assert_eq!(v["code"], "gate_failed");
    assert_eq!(v["commit"], serde_json::Value::Null);
    assert_eq!(git(&r.repo, &["rev-parse", "HEAD"]), before);
    assert_eq!(git(&r.repo, &["status", "--porcelain", "--ignored"]), "");
}

#[test]
fn a_402_stops_at_once_with_code_budget() {
    let r = rig();
    let s = Stub::start(402, r#"{"type":"error","error":{"type":"budget","message":"cap"}}"#.into());
    let o = run(&r, &s.url(), "true", &[]);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(result(&o)["code"], "budget");
    assert_eq!(s.seen().len(), 1, "a 402 was retried");
}

#[test]
fn a_401_is_code_auth() {
    let r = rig();
    let s = Stub::start(401, r#"{"type":"error"}"#.into());
    let o = run(&r, &s.url(), "true", &[]);
    assert_eq!(result(&o)["code"], "auth");
    assert_eq!(s.seen().len(), 1);
}

#[test]
fn a_past_deadline_is_code_timeout_before_any_call() {
    let r = rig();
    let s = Stub::start(200, edit_reply());
    let o = run(&r, &s.url(), "true", &[("HEXA_MANAGED_DEADLINE", Some((now() - 1).to_string()))]);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(result(&o)["code"], "timeout");
    assert!(s.seen().is_empty());
    assert_eq!(git(&r.repo, &["status", "--porcelain", "--ignored"]), "");
}
