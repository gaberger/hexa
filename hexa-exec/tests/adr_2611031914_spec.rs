mod part_1 {
#[allow(unused_imports)]
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use hexa_core::ports::forge::{
    forge_for_remote, ForgeError, ForgeHost, ForgeKind, ForgeTarget, IssueDraft, PrDraft,
};
use hexa_exec::ports::Forge;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[allow(unused_unsafe)]
fn set_env(k: &str, v: &str) {
    unsafe { std::env::set_var(k, v) }
}

#[allow(unused_unsafe)]
fn unset_env(k: &str) {
    unsafe { std::env::remove_var(k) }
}

#[derive(Debug, Clone)]
struct Req {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl Req {
    fn header(&self, name: &str) -> Option<String> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
    }
    fn path_only(&self) -> &str {
        self.path.split('?').next().unwrap_or("")
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Normal,
    Unauthorized,
}

struct Double {
    url: String,
    port: u16,
    requests: Arc<Mutex<Vec<Req>>>,
}

impl Double {
    fn start(mode: Mode) -> Double {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let url = format!("http://127.0.0.1:{port}");
        let requests: Arc<Mutex<Vec<Req>>> = Arc::new(Mutex::new(Vec::new()));
        let reqs = requests.clone();
        let base = url.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(match stream.try_clone() {
                    Ok(s) => s,
                    Err(_) => continue,
                });
                let mut first = String::new();
                if reader.read_line(&mut first).is_err() || first.trim().is_empty() {
                    continue;
                }
                let mut parts = first.split_whitespace();
                let method = parts.next().unwrap_or("").to_string();
                let path = parts.next().unwrap_or("").to_string();
                let mut headers = Vec::new();
                let mut len = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).is_err() {
                        break;
                    }
                    let l = line.trim_end();
                    if l.is_empty() {
                        break;
                    }
                    if let Some((k, v)) = l.split_once(':') {
                        let k = k.trim().to_string();
                        let v = v.trim().to_string();
                        if k.eq_ignore_ascii_case("content-length") {
                            len = v.parse().unwrap_or(0);
                        }
                        headers.push((k, v));
                    }
                }
                let mut body = vec![0u8; len];
                let _ = reader.read_exact(&mut body);
                let req = Req { method: method.clone(), path: path.clone(), headers, body: String::from_utf8_lossy(&body).to_string() };
                reqs.lock().unwrap().push(req);

                let p = path.split('?').next().unwrap_or("").to_string();
                let (status, payload) = if mode == Mode::Unauthorized {
                    ("401 Unauthorized", r#"{"message":"token is required"}"#.to_string())
                } else if method == "POST" && p.ends_with("/issues") {
                    (
                        "201 Created",
                        format!(
                            r#"{{"id":1007,"number":7,"title":"t","state":"open","html_url":"{base}/me/strata/issues/7","url":"{base}/api/v1/repos/me/strata/issues/7"}}"#
                        ),
                    )
                } else if method == "POST" && p.ends_with("/pulls") {
                    (
                        "201 Created",
                        format!(
                            r#"{{"id":1009,"number":9,"title":"t","state":"open","html_url":"{base}/me/strata/pulls/9","url":"{base}/api/v1/repos/me/strata/pulls/9"}}"#
                        ),
                    )
                } else if p.ends_with("/issues") || p.ends_with("/pulls") || p.contains("search") {
                    ("200 OK", "[]".to_string())
                } else {
                    (
                        "200 OK",
                        r#"{"version":"16.0.6+gitea-1.22.0","full_name":"me/strata","name":"strata","default_branch":"main","login":"me","id":1}"#.to_string(),
                    )
                };
                let resp = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                    payload.len()
                );
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.flush();
            }
        });
        Double { url, port, requests }
    }

    fn requests(&self) -> Vec<Req> {
        self.requests.lock().unwrap().clone()
    }
}

fn forgejo_target(token_env: &str) -> ForgeTarget {
    ForgeTarget {
        kind: ForgeKind::Forgejo,
        host: "git.local".to_string(),
        owner: "me".to_string(),
        repo: "strata".to_string(),
        token_env: Some(token_env.to_string()),
    }
}

fn hexa_bin() -> PathBuf {
    if let Ok(p) = std::env::var("HEXA_BIN") {
        return PathBuf::from(p);
    }
    let exe = std::env::current_exe().expect("current exe");
    let mut dir = exe.parent().unwrap().to_path_buf();
    if dir.ends_with("deps") {
        dir = dir.parent().unwrap().to_path_buf();
    }
    let bin = dir.join("hexa");
    assert!(bin.exists(), "the hexa binary was not found at {}", bin.display());
    bin
}

fn run_hexa(args: &[&str], cwd: &Path, home: &Path, envs: &[(&str, &str)]) -> (bool, String, String) {
    let mut cmd = Command::new(hexa_bin());
    cmd.args(args)
        .current_dir(cwd)
        .env("HEXA_HOME", home)
        .env("HOME", home)
        .env_remove("STRATA_TOKEN");
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run hexa");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

fn git_repo(dir: &Path, origin: &str) {
    let ok = Command::new("git").arg("init").arg("-q").current_dir(dir).status().expect("git init").success();
    assert!(ok, "git init failed");
    let ok = Command::new("git")
        .args(["remote", "add", "origin", origin])
        .current_dir(dir)
        .status()
        .expect("git remote")
        .success();
    assert!(ok, "git remote add failed");
}

fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut m = BTreeMap::new();
    for e in walkdir::WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
        let p = e.path();
        if p.components().any(|c| c.as_os_str() == ".git") {
            continue;
        }
        if e.file_type().is_file() {
            m.insert(p.to_path_buf(), std::fs::read(p).unwrap_or_default());
        } else if e.file_type().is_dir() {
            m.insert(p.to_path_buf(), b"<dir>".to_vec());
        }
    }
    m
}

fn json_has_string(v: &serde_json::Value, s: &str) -> bool {
    match v {
        serde_json::Value::String(x) => x == s,
        serde_json::Value::Array(a) => a.iter().any(|x| json_has_string(x, s)),
        serde_json::Value::Object(o) => o.iter().any(|(k, x)| k == s || json_has_string(x, s)),
        _ => false,
    }
}

fn find_host_obj<'a>(v: &'a serde_json::Value, host: &str) -> Option<&'a serde_json::Value> {
    match v {
        serde_json::Value::Array(a) => a.iter().find_map(|x| find_host_obj(x, host)),
        serde_json::Value::Object(o) => {
            if o.values().any(|x| x.as_str() == Some(host)) || o.contains_key(host) {
                if let Some(inner) = o.get(host) {
                    if inner.is_object() {
                        return Some(inner);
                    }
                }
                return Some(v);
            }
            o.values().find_map(|x| find_host_obj(x, host))
        }
        _ => None,
    }
}

/// S01: The Forgejo adapter opens an issue through the REST API v1 with the token from the named variable
#[test]
fn s01_forgejo_adapter_opens_issue_with_token() {
    let _g = env_lock();
    set_env("STRATA_TOKEN", "t0k");
    let double = Double::start(Mode::Normal);
    let target = forgejo_target("STRATA_TOKEN");
    let forge = match hexa_exec::forge_forgejo::ForgeForgejo::new(&target, &double.url) {
        Ok(f) => f,
        Err(e) => panic!("new failed: {e:?}"),
    };
    let draft = IssueDraft {
        key: "bug:red-check-accepts-stubs".to_string(),
        title: "The red check accepts stub tests".to_string(),
        body: "The red check accepts stub tests.\n\n<!-- hexa-key: bug:red-check-accepts-stubs -->".to_string(),
    };
    let got = forge.open_issue(&draft);
    unset_env("STRATA_TOKEN");
    let got = got.expect("open_issue");

    let posts: Vec<Req> = double
        .requests()
        .into_iter()
        .filter(|r| r.method == "POST" && r.path_only() == "/api/v1/repos/me/strata/issues")
        .collect();
    assert_eq!(posts.len(), 1, "expected one POST to issues, got {:?}", double.requests());
    let post = &posts[0];
    assert_eq!(post.header("authorization").as_deref(), Some("token t0k"));
    let body: serde_json::Value = serde_json::from_str(&post.body).expect("json body");
    assert_eq!(body["title"].as_str(), Some(draft.title.as_str()));
    assert_eq!(body["body"].as_str(), Some(draft.body.as_str()));

    assert_eq!(got.number, 7);
    assert_eq!(got.url, format!("{}/me/strata/issues/7", double.url));
    assert!(got.open);
}

/// S02: The GitHub adapter drives the gh CLI rather than HTTP
#[test]
fn s02_github_adapter_drives_gh_cli() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let rec = dir.path().join("gh-calls.txt");
    let stub = dir.path().join("gh");
    let script = format!(
        r#"#!/bin/sh
rec="{rec}"
{{
  echo "---"
  for a in "$@"; do
    printf '%s\n' "$a"
    if [ -f "$a" ]; then cat "$a"; echo; fi
  done
}} >> "$rec"
case "$1" in
  api) echo '{{"number":42,"url":"https://github.com/me/hexa/pull/42","html_url":"https://github.com/me/hexa/pull/42"}}' ;;
  *)
    case "$*" in
      *create*) echo "https://github.com/me/hexa/pull/42" ;;
      *list*|*view*) echo '[]' ;;
      *) echo "https://github.com/me/hexa/pull/42" ;;
    esac ;;
esac
"#,
        rec = rec.display()
    );
    std::fs::write(&stub, script).unwrap();
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();

    let target = ForgeTarget {
        kind: ForgeKind::Github,
        host: "github.com".to_string(),
        owner: "me".to_string(),
        repo: "hexa".to_string(),
        token_env: None,
    };
    let forge = hexa_exec::forge_github::ForgeGithub::new(&target, &stub);
    let draft = PrDraft {
        head: "feature-forge-x".to_string(),
        base: "main-base".to_string(),
        title: "Add the forge port".to_string(),
        body: "Body of the forge pull request".to_string(),
        draft: false,
    };
    let pr = forge.open_pr(&draft).expect("open_pr");
    assert_eq!(pr.number, 42);
    assert!(pr.url.ends_with("42"), "url {}", pr.url);

    let recorded = std::fs::read_to_string(&rec).expect("the stub gh was invoked");
    let found = recorded.split("---\n").any(|inv| {
        (inv.contains("create") || inv.contains("POST"))
            && inv.contains("me/hexa")
            && inv.contains(&draft.head)
            && inv.contains(&draft.base)
            && inv.contains(&draft.title)
            && inv.contains(&draft.body)
    });
    assert!(found, "no invocation created the pull request: {recorded}");
}

/// S03: The Forgejo adapter refuses to be built without its token
#[test]
fn s03_forgejo_adapter_refuses_without_token() {
    let _g = env_lock();
    let double = Double::start(Mode::Normal);
    let target = forgejo_target("STRATA_TOKEN");

    unset_env("STRATA_TOKEN");
    match hexa_exec::forge_forgejo::ForgeForgejo::new(&target, &double.url) {
        Err(ForgeError::Unauthorized(m)) => assert!(m.contains("STRATA_TOKEN"), "msg: {m}"),
        Err(e) => panic!("expected Unauthorized, got {e:?}"),
        Ok(_) => panic!("expected Unauthorized with STRATA_TOKEN unset"),
    }

    set_env("STRATA_TOKEN", "");
    let r = hexa_exec::forge_forgejo::ForgeForgejo::new(&target, &double.url);
    unset_env("STRATA_TOKEN");
    match r {
        Err(ForgeError::Unauthorized(m)) => assert!(m.contains("STRATA_TOKEN"), "msg: {m}"),
        Err(e) => panic!("expected Unauthorized, got {e:?}"),
        Ok(_) => panic!("expected Unauthorized with STRATA_TOKEN empty"),
    }

    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(double.requests().is_empty(), "the double received {:?}", double.requests());
}

/// S04: A rejected token is Unauthorized naming the variable, never its value
#[test]
fn s04_rejected_token_is_unauthorized_without_value() {
    let secret = "secret-value-123";
    let double = Double::start(Mode::Unauthorized);
    {
        let _g = env_lock();
        set_env("STRATA_TOKEN", secret);
        let target = forgejo_target("STRATA_TOKEN");
        let r = match hexa_exec::forge_forgejo::ForgeForgejo::new(&target, &double.url) {
            Ok(f) => f.find_issue("bug:x"),
            Err(e) => Err(e),
        };
        unset_env("STRATA_TOKEN");
        match r {
            Err(ForgeError::Unauthorized(m)) => {
                assert!(m.contains("STRATA_TOKEN"), "msg: {m}");
                assert!(!m.contains(secret), "msg leaks the token: {m}");
            }
            other => panic!("expected Unauthorized, got {other:?}"),
        }
    }

    let home = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    let host = format!("127.0.0.1:{}", double.port);
    git_repo(repo.path(), &format!("http://{host}/me/strata.git"));
    let envs = [("STRATA_TOKEN", secret)];
    run_hexa(&["forge", "add", &host, "--kind", "forgejo", "--token-env", "STRATA_TOKEN"], repo.path(), home.path(), &envs);
    run_hexa(&["memory", "store", "bug:x", "The red check accepts stub tests."], repo.path(), home.path(), &envs);
    let (_ok, out, err) = run_hexa(&["issue", "file", "bug:x", "--send"], repo.path(), home.path(), &envs);
    let all = format!("{out}\n{err}");
    assert!(all.contains("STRATA_TOKEN"), "output does not name STRATA_TOKEN: {all}");
    assert!(!all.contains(secret), "output leaks the token: {all}");
}

/// S05: A github.com https remote maps to a GitHub target
#[test]
fn s05_github_https_remote_maps_to_github() {
    let got = forge_for_remote("https://github.com/acme/hexa.git", &[]);
    assert_eq!(
        got,
        Some(ForgeTarget {
            kind: ForgeKind::Github,
            host: "github.com".to_string(),
            owner: "acme".to_string(),
            repo: "hexa".to_string(),
            token_env: None,
        })
    );
}

/// S06: An ssh remote on a host listed in forges.json maps to the recorded kind and variable
#[test]
fn s06_ssh_remote_on_listed_host_maps_to_recorded_kind() {
    let hosts = vec![ForgeHost {
        host: "git.local".to_string(),
        kind: ForgeKind::Forgejo,
        token_env: Some("STRATA_TOKEN".to_string()),
    }];
    let got = forge_for_remote("git@git.local:me/strata.git", &hosts);
    assert_eq!(
        got,
        Some(ForgeTarget {
            kind: ForgeKind::Forgejo,
            host: "git.local".to_string(),
            owner: "me".to_string(),
            repo: "strata".to_string(),
            token_env: Some("STRATA_TOKEN".to_string()),
        })
    );
}

/// S07: forge add records the host, kind and variable name
#[test]
fn s07_forge_add_records_host_kind_and_variable() {
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let (ok, out, err) = run_hexa(
        &["forge", "add", "git.local", "--kind", "forgejo", "--token-env", "STRATA_TOKEN"],
        cwd.path(),
        home.path(),
        &[("STRATA_TOKEN", "t0k")],
    );
    assert!(ok, "forge add failed: {out}\n{err}");
    let text = std::fs::read_to_string(home.path().join("forges.json")).expect("forges.json written");
    let v: serde_json::Value = serde_json::from_str(&text).expect("forges.json is JSON");
    let obj = find_host_obj(&v, "git.local").unwrap_or_else(|| panic!("git.local not listed: {text}"));
    assert!(json_has_string(obj, "forgejo"), "kind not forgejo: {text}");
    assert!(json_has_string(obj, "STRATA_TOKEN"), "variable not recorded: {text}");

    for e in walkdir::WalkDir::new(home.path()).into_iter().filter_map(|e| e.ok()) {
        if e.file_type().is_file() {
            let bytes = std::fs::read(e.path()).unwrap_or_default();
            assert!(
                !String::from_utf8_lossy(&bytes).contains("t0k"),
                "{} contains the token",
                e.path().display()
            );
        }
    }
}

/// S08: An unknown host has no target, and the verbs say which host and how to add it
#[test]
fn s08_unknown_host_has_no_target_and_cli_explains() {
    let url = "https://code.example.org/me/x.git";
    assert_eq!(forge_for_remote(url, &[]), None);

    let home = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    git_repo(repo.path(), url);
    run_hexa(&["memory", "store", "bug:x", "The red check accepts stub tests."], repo.path(), home.path(), &[]);
    let (_ok, out, err) = run_hexa(&["issue", "file", "bug:x"], repo.path(), home.path(), &[]);
    let all = format!("{out}\n{err}");
    assert!(all.contains("code.example.org"), "unknown host not named: {all}");
    assert!(all.contains("hexa forge add"), "`hexa forge add` not shown: {all}");
}

/// S09: forge status reports and writes nothing
#[test]
fn s09_forge_status_reports_and_writes_nothing() {
    let double = Double::start(Mode::Normal);
    let home = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    let host = format!("127.0.0.1:{}", double.port);
    git_repo(repo.path(), &format!("http://{host}/me/strata.git"));
    std::fs::create_dir_all(repo.path().join(".hexa")).unwrap();
    let envs = [("STRATA_TOKEN", "t0k")];
    let (ok, out, err) =
        run_hexa(&["forge", "add", &host, "--kind", "forgejo", "--token-env", "STRATA_TOKEN"], repo.path(), home.path(), &envs);
    assert!(ok, "forge add failed: {out}\n{err}");

    let before_home = snapshot(home.path());
    let before_repo = snapshot(repo.path());
    let (_ok, out, err) = run_hexa(&["forge", "status"], repo.path(), home.path(), &envs);
    let all = format!("{out}\n{err}");
    assert!(all.contains("strata"), "target not printed: {all}");
    assert!(all.contains("127.0.0.1"), "host not printed: {all}");
    assert!(!all.contains("t0k"), "token printed: {all}");

    assert_eq!(snapshot(home.path()), before_home, "HEXA_HOME changed");
    assert_eq!(snapshot(repo.path()), before_repo, "the repository changed");
    let writes: Vec<Req> = double
        .requests()
        .into_iter()
        .filter(|r| matches!(r.method.as_str(), "POST" | "PUT" | "PATCH" | "DELETE"))
        .collect();
    assert!(writes.is_empty(), "status sent requests that create: {writes:?}");
}

/// S10: The https and ssh forms of a remote map to the same target
#[test]
fn s10_https_and_ssh_forms_map_to_same_target() {
    let hosts = ["github.com", "git.local", "code.example.org", "forge.acme.io"];
    let owners = ["me", "acme", "a-b"];
    let repos = ["strata", "hexa", "x_y"];
    for host in hosts {
        let lists: Vec<Vec<ForgeHost>> = vec![
            vec![],
            vec![ForgeHost { host: host.to_string(), kind: ForgeKind::Forgejo, token_env: Some("T_ENV".to_string()) }],
            vec![ForgeHost { host: host.to_string(), kind: ForgeKind::Github, token_env: None }],
        ];
        for list in &lists {
            for owner in owners {
                for repo in repos {
                    let https = forge_for_remote(&format!("https://{host}/{owner}/{repo}.git"), list);
                    let ssh = forge_for_remote(&format!("git@{host}:{owner}/{repo}.git"), list);
                    assert_eq!(https, ssh, "{host} {owner}/{repo} with {list:?}");
                    let expect_some = host == "github.com" || list.iter().any(|h| h.host == host);
                    assert_eq!(https.is_some(), expect_some, "{host} {owner}/{repo} with {list:?}");
                }
            }
        }
    }
}
}

mod part_2 {
#![allow(dead_code, unused_imports, unused_unsafe, clippy::all)]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hexa_core::ports::forge::{
    issue_from_memory, key_marker, pr_body, ForgeKind, ForgeTarget, IssueDraft, PrDraft,
};
use hexa_exec::forge_forgejo::ForgeForgejo;
use hexa_exec::forge_github::ForgeGithub;
use hexa_exec::ports::{Forge, MemoryScope, MemoryStore};

// ---------------------------------------------------------------------------
// A local HTTP double of the Forgejo (Gitea-compatible) REST API v1.
// ---------------------------------------------------------------------------

const OWNER: &str = "acme";
const REPO: &str = "strata";

struct DIssue {
    number: u64,
    title: String,
    body: String,
    open: bool,
}

struct DPull {
    number: u64,
    head: String,
    base: String,
    title: String,
    body: String,
    open: bool,
}

#[derive(Default)]
struct DState {
    base: String,
    token: String,
    next: u64,
    issues: Vec<DIssue>,
    pulls: Vec<DPull>,
    /// (method, path, body)
    requests: Vec<(String, String, String)>,
}

struct Double {
    base: String,
    state: Arc<Mutex<DState>>,
}

impl Double {
    fn start(token: &str) -> Double {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind double");
        let port = listener.local_addr().unwrap().port();
        let base = format!("http://127.0.0.1:{port}");
        let state = Arc::new(Mutex::new(DState {
            base: base.clone(),
            token: token.to_string(),
            next: 1,
            ..Default::default()
        }));
        let st = state.clone();
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                if let Ok(s) = conn {
                    let st = st.clone();
                    std::thread::spawn(move || handle(s, &st));
                }
            }
        });
        Double { base, state }
    }

    fn html_url(&self, kind: &str, n: u64) -> String {
        format!("{}/{OWNER}/{REPO}/{kind}/{n}", self.base)
    }

    fn add_issue(&self, title: &str, body: &str, open: bool) -> String {
        let mut st = self.state.lock().unwrap();
        let n = st.next;
        st.next += 1;
        st.issues.push(DIssue { number: n, title: title.into(), body: body.into(), open });
        drop(st);
        self.html_url("issues", n)
    }

    fn posts(&self) -> usize {
        self.state.lock().unwrap().requests.iter().filter(|r| r.0 == "POST").count()
    }

    fn issue_posts(&self) -> Vec<String> {
        self.state
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|r| r.0 == "POST" && r.1.trim_end_matches('/').ends_with("/issues"))
            .map(|r| r.2.clone())
            .collect()
    }

    fn pull_posts(&self) -> Vec<String> {
        self.state
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|r| r.0 == "POST" && r.1.trim_end_matches('/').ends_with("/pulls"))
            .map(|r| r.2.clone())
            .collect()
    }

    fn issues_with_marker(&self, key: &str) -> Vec<(u64, String)> {
        let m = key_marker(key);
        let st = self.state.lock().unwrap();
        st.issues
            .iter()
            .filter(|i| i.body.contains(&m))
            .map(|i| (i.number, format!("{}/{OWNER}/{REPO}/issues/{}", st.base, i.number)))
            .collect()
    }
}

fn find_sub(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}

fn respond(s: &mut TcpStream, code: u16, body: &str) {
    let reason = match code {
        200 => "OK",
        201 => "Created",
        401 => "Unauthorized",
        404 => "Not Found",
        _ => "Error",
    };
    let resp = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = s.write_all(resp.as_bytes());
    let _ = s.flush();
}

fn issue_json(base: &str, i: &DIssue) -> serde_json::Value {
    serde_json::json!({
        "id": i.number,
        "number": i.number,
        "title": i.title,
        "body": i.body,
        "state": if i.open { "open" } else { "closed" },
        "html_url": format!("{base}/{OWNER}/{REPO}/issues/{}", i.number),
        "url": format!("{base}/api/v1/repos/{OWNER}/{REPO}/issues/{}", i.number),
        "pull_request": null,
        "repository": { "owner": OWNER, "name": REPO, "full_name": format!("{OWNER}/{REPO}") }
    })
}

fn pull_json(base: &str, p: &DPull) -> serde_json::Value {
    serde_json::json!({
        "id": p.number,
        "number": p.number,
        "title": p.title,
        "body": p.body,
        "state": if p.open { "open" } else { "closed" },
        "html_url": format!("{base}/{OWNER}/{REPO}/pulls/{}", p.number),
        "url": format!("{base}/api/v1/repos/{OWNER}/{REPO}/pulls/{}", p.number),
        "head": { "ref": p.head, "label": p.head },
        "base": { "ref": p.base, "label": p.base }
    })
}

fn handle(mut s: TcpStream, st: &Arc<Mutex<DState>>) {
    let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    let header_end = loop {
        let n = match s.read(&mut tmp) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        buf.extend_from_slice(&tmp[..n]);
        if let Some(p) = find_sub(&buf, b"\r\n\r\n") {
            break p + 4;
        }
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let mut lines = head.split("\r\n");
    let first = lines.next().unwrap_or("");
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();
    let mut headers = HashMap::new();
    for l in lines {
        if let Some((k, v)) = l.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    let cl: usize = headers.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
    while buf.len() < header_end + cl {
        match s.read(&mut tmp) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
    }
    let end = (header_end + cl).min(buf.len());
    let body = String::from_utf8_lossy(&buf[header_end..end]).to_string();

    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (target.clone(), String::new()),
    };
    let params: HashMap<String, String> = query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            let dec = |x: &str| {
                urlencoding::decode(&x.replace('+', " ")).map(|c| c.into_owned()).unwrap_or_else(|_| x.to_string())
            };
            (dec(k), dec(v))
        })
        .collect();

    let mut g = st.lock().unwrap();
    g.requests.push((method.clone(), path.clone(), body.clone()));

    let auth = headers.get("authorization").cloned().unwrap_or_default();
    if auth != format!("token {}", g.token) && auth != format!("Bearer {}", g.token) {
        drop(g);
        respond(&mut s, 401, r#"{"message":"token is required"}"#);
        return;
    }

    let base = g.base.clone();
    let prefix = format!("/api/v1/repos/{OWNER}/{REPO}");
    let p = path.trim_end_matches('/').to_string();
    let page: u64 = params.get("page").and_then(|v| v.parse().ok()).unwrap_or(1);
    let state_ok = |open: bool| match params.get("state").map(String::as_str) {
        Some("all") => true,
        Some("closed") => !open,
        _ => open,
    };

    let (code, out) = if method == "GET" && p == prefix {
        (200, serde_json::json!({"name": REPO, "full_name": format!("{OWNER}/{REPO}"), "default_branch": "main"}))
    } else if method == "GET" && p == "/api/v1/user" {
        (200, serde_json::json!({"login": "hexa"}))
    } else if method == "GET" && p == "/api/v1/version" {
        (200, serde_json::json!({"version": "16.0.6+gitea-1.22.0"}))
    } else if method == "GET" && (p.ends_with("/issues") || p.ends_with("/issues/search")) {
        let q = params.get("q").cloned().unwrap_or_default();
        let list: Vec<serde_json::Value> = if page > 1 {
            vec![]
        } else {
            g.issues
                .iter()
                .filter(|i| state_ok(i.open))
                .filter(|i| q.trim().is_empty() || i.body.contains(&q) || i.title.contains(&q))
                .map(|i| issue_json(&base, i))
                .collect()
        };
        (200, serde_json::Value::Array(list))
    } else if method == "POST" && p == format!("{prefix}/issues") {
        let v: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
        let n = g.next;
        g.next += 1;
        let i = DIssue {
            number: n,
            title: v["title"].as_str().unwrap_or("").to_string(),
            body: v["body"].as_str().unwrap_or("").to_string(),
            open: true,
        };
        let j = issue_json(&base, &i);
        g.issues.push(i);
        (201, j)
    } else if method == "GET" && p == format!("{prefix}/pulls") {
        let list: Vec<serde_json::Value> = if page > 1 {
            vec![]
        } else {
            g.pulls.iter().filter(|x| state_ok(x.open)).map(|x| pull_json(&base, x)).collect()
        };
        (200, serde_json::Value::Array(list))
    } else if method == "POST" && p == format!("{prefix}/pulls") {
        let v: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
        let n = g.next;
        g.next += 1;
        let pr = DPull {
            number: n,
            head: v["head"].as_str().unwrap_or("").to_string(),
            base: v["base"].as_str().unwrap_or("").to_string(),
            title: v["title"].as_str().unwrap_or("").to_string(),
            body: v["body"].as_str().unwrap_or("").to_string(),
            open: true,
        };
        let j = pull_json(&base, &pr);
        g.pulls.push(pr);
        (201, j)
    } else {
        (404, serde_json::json!({"message": "not found"}))
    };
    drop(g);
    respond(&mut s, code, &out.to_string());
}

// ---------------------------------------------------------------------------
// Helpers: targets, token variables, an in-memory store, a recording `gh`.
// ---------------------------------------------------------------------------

#[allow(unused_unsafe)]
fn set_token(var: &str, value: &str) {
    unsafe { std::env::set_var(var, value) };
}

fn forgejo_target(var: &str) -> ForgeTarget {
    ForgeTarget {
        kind: ForgeKind::Forgejo,
        host: "git.local".into(),
        owner: OWNER.into(),
        repo: REPO.into(),
        token_env: Some(var.into()),
    }
}

fn forgejo(test: &str) -> (Double, ForgeForgejo) {
    let var = format!("HEXA_PART2_TOKEN_{test}");
    let token = format!("tok{test}value");
    set_token(&var, &token);
    let double = Double::start(&token);
    let forge = ForgeForgejo::new(&forgejo_target(&var), &double.base).expect("ForgeForgejo::new");
    (double, forge)
}

#[derive(Default)]
struct MemStore {
    entries: Mutex<Vec<(String, String)>>,
}

impl MemoryStore for MemStore {
    fn path(&self, _scope: MemoryScope) -> PathBuf {
        PathBuf::new()
    }
    fn put(&self, _scope: MemoryScope, key: &str, value: &str) -> Result<(), String> {
        let mut e = self.entries.lock().unwrap();
        e.retain(|(k, _)| k != key);
        e.push((key.to_string(), value.to_string()));
        Ok(())
    }
    fn get(&self, _scope: MemoryScope, key: &str) -> Option<String> {
        self.entries.lock().unwrap().iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    }
    fn entries(&self, _scope: MemoryScope, limit: usize) -> Vec<(String, String)> {
        let e = self.entries.lock().unwrap();
        e.iter().rev().take(limit).cloned().collect()
    }
    fn search(&self, _scope: MemoryScope, query: &str) -> Vec<(String, String)> {
        self.entries.lock().unwrap().iter().filter(|(k, v)| k.contains(query) || v.contains(query)).cloned().collect()
    }
    fn delete(&self, _scope: MemoryScope, key: &str) -> Result<bool, String> {
        let mut e = self.entries.lock().unwrap();
        let before = e.len();
        e.retain(|(k, _)| k != key);
        Ok(e.len() != before)
    }
}

const P: MemoryScope = MemoryScope::Project;

/// `hexa issue file <key> --send`, as the decision states it: search for the
/// marker, open only when none exists, record the URL under `issue:<key>`.
fn file_issue(forge: &dyn Forge, store: &MemStore, key: &str) -> String {
    let value = store.get(P, key).expect("key recorded");
    let url = match forge.find_issue(key).expect("find_issue") {
        Some(found) => found.url,
        None => {
            let d = issue_from_memory(key, &value, "strata");
            forge.open_issue(&d).expect("open_issue").url
        }
    };
    store.put(P, &format!("issue:{key}"), &url).unwrap();
    url
}

/// `hexa issue sync [--prefix p] --send`.
fn sync(forge: &dyn Forge, store: &MemStore, prefix: Option<&str>) {
    let keys: Vec<String> = store
        .entries(P, usize::MAX)
        .into_iter()
        .map(|(k, _)| k)
        .filter(|k| k.starts_with("bug:") || k.starts_with("gap:"))
        .filter(|k| prefix.map_or(true, |p| k.starts_with(p)))
        .filter(|k| store.get(P, &format!("issue:{k}")).is_none())
        .collect();
    for k in keys {
        file_issue(forge, store, &k);
    }
}

fn ends_with_marker(body: &str, key: &str) -> bool {
    body.trim_end().ends_with(&key_marker(key))
}

#[cfg(unix)]
fn stub_gh(dir: &Path) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let log = dir.join("gh.log");
    let gh = dir.join("gh");
    let script = format!(
        r#"#!/bin/sh
printf '%s\n' "$*" >> '{log}'
if [ "$2" = "create" ]; then
  echo 'https://github.com/acme/hexa/pull/7'
  exit 0
fi
case "$*" in
  *POST*) echo '{{"number":7,"html_url":"https://github.com/acme/hexa/pull/7","url":"https://github.com/acme/hexa/pull/7"}}' ;;
  *) echo '[]' ;;
esac
"#,
        log = log.display()
    );
    std::fs::write(&gh, script).unwrap();
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    (gh, log)
}

fn gh_creates(log: &Path) -> usize {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter(|l| l.starts_with("pr create") || l.starts_with("issue create") || l.contains("POST"))
        .count()
}

fn github_target() -> ForgeTarget {
    ForgeTarget {
        kind: ForgeKind::Github,
        host: "github.com".into(),
        owner: "acme".into(),
        repo: "hexa".into(),
        token_env: None,
    }
}

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

/// S11: issue file without --send prints the issue and sends nothing
#[test]
fn s11_issue_file_dry_run_prints_and_sends_nothing() {
    let (double, forge) = forgejo("S11");
    let store = MemStore::default();
    let key = "bug:red-check-accepts-stubs";
    let value = "The red check accepts stub tests. A test that only calls todo!() is counted as red.";
    store.put(P, key, value).unwrap();

    let d = issue_from_memory(key, &store.get(P, key).unwrap(), "strata");
    assert_eq!(d.key, key);
    assert!(!d.title.trim().is_empty());
    assert!(d.title.chars().count() <= 80);
    assert!(ends_with_marker(&d.body, key), "body must end with the marker: {}", d.body);
    assert!(d.body.contains("strata"));

    // A dry run may look, but never posts.
    let _ = forge.find_issue(key).expect("find_issue");
    assert_eq!(double.posts(), 0, "a dry run must send no POST");
    assert!(store.get(P, &format!("issue:{key}")).is_none());
}

/// S12: gap pr keeps printing, and --send opens it under pr open's rules
#[cfg(unix)]
#[test]
fn s12_gap_pr_prints_then_send_opens_one() {
    let dir = tempfile::tempdir().unwrap();
    let (gh, log) = stub_gh(dir.path());
    let forge = ForgeGithub::new(&github_target(), &gh);

    let g = hexa_core::domain::guide::find("no-completion-model").expect("guide");
    let text = hexa_core::domain::guide::pr_text(&g, None, false);
    assert!(text.contains("no-completion-model"));
    assert_eq!(gh_creates(&log), 0, "printing creates nothing");

    let head = "gap/no-completion-model";
    assert!(forge.find_pr(head).expect("find_pr").is_none());
    assert_eq!(gh_creates(&log), 0);

    let (title, body) = text.split_once('\n').unwrap();
    let d = PrDraft {
        head: head.into(),
        base: "main".into(),
        title: title.trim().into(),
        body: body.trim().into(),
        draft: false,
    };
    let pr = forge.open_pr(&d).expect("open_pr");
    assert!(pr.url.contains("github.com"));
    assert_eq!(gh_creates(&log), 1, "exactly one pull-request create");
}

/// S13: A dry run of sync or pr open opens nothing and records nothing
#[test]
fn s13_dry_run_sync_and_pr_open_send_nothing() {
    let (double, forge) = forgejo("S13");
    let store = MemStore::default();
    store.put(P, "bug:one", "First bug. Details.").unwrap();
    store.put(P, "bug:two", "Second bug. Details.").unwrap();

    for k in ["bug:one", "bug:two"] {
        assert!(forge.find_issue(k).expect("find_issue").is_none());
        let d = issue_from_memory(k, &store.get(P, k).unwrap(), "strata");
        assert!(ends_with_marker(&d.body, k));
    }
    assert!(forge.find_pr("feat/x").expect("find_pr").is_none());
    let commits = vec!["Add forge port".to_string(), "Wire forgejo adapter".to_string()];
    let body = pr_body(&commits, None, None, None);
    for c in &commits {
        assert!(body.contains(c.as_str()));
    }

    assert_eq!(double.posts(), 0);
    assert!(store.entries(P, usize::MAX).iter().all(|(k, _)| !k.starts_with("issue:")));
}

/// S14: No verb creates anything without --send
#[cfg(unix)]
#[test]
fn s14_nothing_created_without_send() {
    let (double, forgejo) = forgejo("S14");
    let dir = tempfile::tempdir().unwrap();
    let (gh, log) = stub_gh(dir.path());
    let github = ForgeGithub::new(&github_target(), &gh);
    let store = MemStore::default();

    let keys = ["bug:a", "bug:red-check-accepts-stubs", "gap:no-completion-model", "gap:x-y"];
    for k in keys {
        store.put(P, k, "Something is wrong. More text here.").unwrap();
        let d = issue_from_memory(k, "Something is wrong. More text here.", "strata");
        assert!(ends_with_marker(&d.body, k));
        let forges: [&dyn Forge; 2] = [&forgejo, &github];
        for f in forges {
            let _ = f.find_issue(k).expect("find_issue");
            let _ = f.find_pr("feat/x").expect("find_pr");
        }
    }
    assert_eq!(double.posts(), 0);
    assert_eq!(gh_creates(&log), 0);
    assert!(store.entries(P, usize::MAX).iter().all(|(k, _)| !k.starts_with("issue:")));
}

/// S15: issue file --send opens one issue carrying the marker and records its URL
#[test]
fn s15_issue_file_send_opens_one_with_marker() {
    let (double, forge) = forgejo("S15");
    let store = MemStore::default();
    let key = "bug:foreign-adr-ids";
    store.put(P, key, "Installed assets carry foreign ADR ids. They come from hexa's own docs.").unwrap();

    let url = file_issue(&forge, &store, key);

    let posts = double.issue_posts();
    assert_eq!(posts.len(), 1, "exactly one issue POST");
    let v: serde_json::Value = serde_json::from_str(&posts[0]).expect("json body");
    assert!(ends_with_marker(v["body"].as_str().unwrap_or(""), key));
    let on_double = double.issues_with_marker(key);
    assert_eq!(on_double.len(), 1);
    assert_eq!(url, on_double[0].1, "the URL returned is the double's");
    assert_eq!(store.get(P, &format!("issue:{key}")), Some(on_double[0].1.clone()));
}

/// S16: sync files only keys without an issue: entry, limited by --prefix
#[test]
fn s16_sync_files_only_unfiled_keys_and_honours_prefix() {
    let seed = |store: &MemStore| {
        store.put(P, "bug:a", "Bug a. Detail.").unwrap();
        store.put(P, "bug:b", "Bug b. Detail.").unwrap();
        store.put(P, "issue:bug:b", "https://git.local/acme/strata/issues/99").unwrap();
        store.put(P, "gap:c", "Gap c. Detail.").unwrap();
        store.put(P, "note:d", "Note d. Detail.").unwrap();
    };

    let (double, forge) = forgejo("S16a");
    let store = MemStore::default();
    seed(&store);
    sync(&forge, &store, None);
    assert_eq!(double.issue_posts().len(), 2);
    assert_eq!(double.issues_with_marker("bug:a").len(), 1);
    assert_eq!(double.issues_with_marker("gap:c").len(), 1);
    assert!(double.issues_with_marker("bug:b").is_empty());
    assert!(double.issues_with_marker("note:d").is_empty());

    let (double2, forge2) = forgejo("S16b");
    let store2 = MemStore::default();
    seed(&store2);
    sync(&forge2, &store2, Some("gap:"));
    assert_eq!(double2.issue_posts().len(), 1);
    assert_eq!(double2.issues_with_marker("gap:c").len(), 1);
    assert!(double2.issues_with_marker("bug:a").is_empty());
    assert!(double2.issues_with_marker("bug:b").is_empty());
    assert!(double2.issues_with_marker("note:d").is_empty());
}

/// S17: A second send finds the issue and opens none
#[test]
fn s17_second_send_finds_existing_issue() {
    let (double, forge) = forgejo("S17");
    let store = MemStore::default();
    let key = "bug:foreign-adr-ids";
    store.put(P, key, "Installed assets carry foreign ADR ids. Detail.").unwrap();
    let first = file_issue(&forge, &store, key);
    assert_eq!(double.issue_posts().len(), 1);

    let found = forge.find_issue(key).expect("find_issue").expect("the issue is found");
    assert_eq!(found.url, first);
    let again = file_issue(&forge, &store, key);
    assert_eq!(again, first);
    assert_eq!(double.issue_posts().len(), 1, "no second issue POST");
}

/// S18: A closed issue with the marker still counts as filed
#[test]
fn s18_closed_issue_with_marker_counts_as_filed() {
    let (double, forge) = forgejo("S18");
    let key = "bug:x";
    let url = double.add_issue("Bug x", &format!("Old report.\n\n{}", key_marker(key)), false);
    let store = MemStore::default();
    store.put(P, key, "Bug x happens. Detail.").unwrap();

    let found = forge.find_issue(key).expect("find_issue").expect("closed issue found");
    assert!(!found.open);
    assert_eq!(found.url, url);

    let stored = file_issue(&forge, &store, key);
    assert_eq!(stored, url);
    assert_eq!(double.issue_posts().len(), 0, "no issue opened");
    assert_eq!(store.get(P, "issue:bug:x"), Some(url));
}

/// S19: Filing n times files once
#[test]
fn s19_filing_n_times_files_once() {
    let (double, forge) = forgejo("S19");
    let store = MemStore::default();
    store.put(P, "bug:repeat", "Repeated bug. Detail.").unwrap();
    store.put(P, "gap:repeat-sync", "Repeated gap. Detail.").unwrap();

    for _ in 0..3 {
        file_issue(&forge, &store, "bug:repeat");
    }
    let on = double.issues_with_marker("bug:repeat");
    assert_eq!(on.len(), 1);
    assert_eq!(store.get(P, "issue:bug:repeat"), Some(on[0].1.clone()));

    for _ in 0..3 {
        sync(&forge, &store, Some("gap:"));
        // a retry that lost its memory entry must still not duplicate
        store.delete(P, "issue:gap:repeat-sync").unwrap();
    }
    sync(&forge, &store, Some("gap:"));
    let on = double.issues_with_marker("gap:repeat-sync");
    assert_eq!(on.len(), 1);
    assert_eq!(store.get(P, "issue:gap:repeat-sync"), Some(on[0].1.clone()));
}

/// S20: pr open --send opens a pull request for a pushed branch against the default branch
#[test]
fn s20_pr_open_send_opens_one_against_default_branch() {
    let (double, forge) = forgejo("S20");
    assert!(forge.find_pr("feat/x").expect("find_pr").is_none());

    let commits = vec!["Add the forge port".to_string(), "Open pull requests on Forgejo".to_string()];
    let body = pr_body(&commits, None, None, None);
    let d = PrDraft {
        head: "feat/x".into(),
        base: "main".into(),
        title: commits[0].clone(),
        body: body.clone(),
        draft: false,
    };
    let pr = forge.open_pr(&d).expect("open_pr");

    let posts = double.pull_posts();
    assert_eq!(posts.len(), 1, "exactly one pull-request POST");
    let v: serde_json::Value = serde_json::from_str(&posts[0]).expect("json body");
    assert_eq!(v["head"].as_str(), Some("feat/x"));
    assert_eq!(v["base"].as_str(), Some("main"));
    let sent = v["body"].as_str().unwrap_or("");
    for c in &commits {
        assert!(sent.contains(c.as_str()), "body lists `{c}`");
    }
    let st = double.state.lock().unwrap();
    let expected = format!("{}/{OWNER}/{REPO}/pulls/{}", st.base, st.pulls[0].number);
    drop(st);
    assert_eq!(pr.url, expected);

    let found = forge.find_pr("feat/x").expect("find_pr").expect("open pr found");
    assert_eq!(found.url, pr.url);
}
}

mod part_3 {
#![allow(unused_unsafe)]
#![allow(dead_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use hexa_core::ports::forge::{
    issue_from_memory, key_marker, looks_secret, pr_body, ForgeKind, ForgeTarget, PrDraft,
};
use hexa_exec::ports::Forge;

// ---------------------------------------------------------------------------
// A local HTTP double of the Forgejo REST API v1, recording every request.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Recorded {
    method: String,
    path: String,
    body: String,
}

type Handler = fn(&str, &str, &str) -> (u16, String);

struct Double {
    base_url: String,
    log: Arc<Mutex<Vec<Recorded>>>,
}

impl Double {
    fn requests(&self) -> Vec<Recorded> {
        self.log.lock().unwrap().clone()
    }
}

fn start_double(handler: Handler) -> Double {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind double");
    let addr = listener.local_addr().unwrap();
    let log: Arc<Mutex<Vec<Recorded>>> = Arc::new(Mutex::new(Vec::new()));
    let log2 = log.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            if reader.read_line(&mut request_line).is_err() {
                continue;
            }
            let mut parts = request_line.split_whitespace();
            let method = parts.next().unwrap_or("").to_string();
            let path = parts.next().unwrap_or("").to_string();
            let mut content_length = 0usize;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                let trimmed = line.trim_end();
                if trimmed.is_empty() {
                    break;
                }
                if let Some((k, v)) = trimmed.split_once(':') {
                    if k.trim().eq_ignore_ascii_case("content-length") {
                        content_length = v.trim().parse().unwrap_or(0);
                    }
                }
            }
            let mut body = vec![0u8; content_length];
            let _ = reader.read_exact(&mut body);
            let body = String::from_utf8_lossy(&body).to_string();
            log2.lock().unwrap().push(Recorded { method: method.clone(), path: path.clone(), body: body.clone() });
            let (status, resp) = handler(&method, &path, &body);
            let out = format!(
                "HTTP/1.1 {} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                status,
                resp.len(),
                resp
            );
            let _ = stream.write_all(out.as_bytes());
            let _ = stream.flush();
        }
    });
    Double { base_url: format!("http://{addr}"), log }
}

fn existing_pr_json() -> String {
    serde_json::json!({
        "number": 7,
        "html_url": "http://git.local/o/r/pulls/7",
        "url": "http://git.local/api/v1/repos/o/r/pulls/7",
        "state": "open",
        "title": "Existing",
        "body": "",
        "head": {"ref": "feat/x", "label": "feat/x", "sha": "abc"},
        "base": {"ref": "main", "label": "main", "sha": "def"}
    })
    .to_string()
}

fn repo_json() -> String {
    serde_json::json!({"full_name": "o/r", "name": "r", "default_branch": "main", "owner": {"login": "o"}}).to_string()
}

fn routes_with_existing_pr(method: &str, path: &str, _body: &str) -> (u16, String) {
    let p = path.split('?').next().unwrap_or("");
    if p.contains("/pulls") {
        if method == "POST" {
            return (
                201,
                serde_json::json!({"number": 8, "html_url": "http://git.local/o/r/pulls/8", "state": "open"}).to_string(),
            );
        }
        if p.ends_with("/pulls") {
            return (200, format!("[{}]", existing_pr_json()));
        }
        return (200, existing_pr_json());
    }
    if p.ends_with("/repos/o/r") {
        return (200, repo_json());
    }
    if p.ends_with("/user") {
        return (200, serde_json::json!({"login": "t"}).to_string());
    }
    (200, "[]".to_string())
}

fn routes_without_pr(method: &str, path: &str, body: &str) -> (u16, String) {
    let p = path.split('?').next().unwrap_or("");
    if p.contains("/pulls") {
        if method == "POST" {
            let sent: serde_json::Value = serde_json::from_str(body).unwrap_or(serde_json::Value::Null);
            return (
                201,
                serde_json::json!({
                    "number": 9,
                    "html_url": "http://git.local/o/r/pulls/9",
                    "state": "open",
                    "title": sent.get("title").cloned().unwrap_or(serde_json::Value::Null),
                    "body": sent.get("body").cloned().unwrap_or(serde_json::Value::Null),
                    "head": {"ref": "feat/x", "label": "feat/x"},
                    "base": {"ref": "main", "label": "main"}
                })
                .to_string(),
            );
        }
        if p.ends_with("/pulls") {
            return (200, "[]".to_string());
        }
        return (404, serde_json::json!({"message": "not found"}).to_string());
    }
    if p.ends_with("/repos/o/r") {
        return (200, repo_json());
    }
    (200, "[]".to_string())
}

fn forgejo_target(token_env: &str) -> ForgeTarget {
    ForgeTarget {
        kind: ForgeKind::Forgejo,
        host: "git.local".to_string(),
        owner: "o".to_string(),
        repo: "r".to_string(),
        token_env: Some(token_env.to_string()),
    }
}

// ---------------------------------------------------------------------------
// The hexa CLI and git, for the end-to-end scenarios.
// ---------------------------------------------------------------------------

fn hexa_bin() -> PathBuf {
    if let Ok(p) = std::env::var("HEXA_BIN") {
        return PathBuf::from(p);
    }
    let target = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("target"));
    for profile in ["debug", "release"] {
        let p = target.join(profile).join("hexa");
        if p.exists() {
            return p;
        }
    }
    panic!("the hexa binary was not found under {}; build hexa-cli first", target.display());
}

fn hexa(cwd: &Path, home: &Path, envs: &[(&str, &str)], args: &[&str]) -> (bool, String) {
    let mut cmd = Command::new(hexa_bin());
    cmd.args(args).current_dir(cwd).env("HEXA_HOME", home).env("HOME", home);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run hexa");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text)
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").args(args).current_dir(dir).output().expect("run git");
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn commit(repo: &Path, file: &str, subject: &str) {
    std::fs::write(repo.join(file), subject).unwrap();
    git(repo, &["add", file]);
    git(repo, &["commit", "-q", "-m", subject]);
}

/// A work repository whose origin is `https://git.local/o/r.git`, redirected
/// by git to a local bare repository, and `git.local` added as a Forgejo host.
struct Setup {
    _tmp: tempfile::TempDir,
    home: PathBuf,
    repo: PathBuf,
    remote: PathBuf,
}

fn setup(token_env: &str, token: &str) -> Setup {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let repo = tmp.path().join("work");
    let remote = tmp.path().join("remote.git");
    std::fs::create_dir_all(&home).unwrap();
    git(tmp.path(), &["init", "-q", "--bare", "-b", "main", remote.to_str().unwrap()]);
    git(tmp.path(), &["init", "-q", "-b", "main", repo.to_str().unwrap()]);
    git(&repo, &["config", "user.name", "t"]);
    git(&repo, &["config", "user.email", "t@example.invalid"]);
    git(&repo, &["config", "commit.gpgsign", "false"]);
    git(
        &repo,
        &["config", &format!("url.{}.insteadOf", remote.display()), "https://git.local/o/r.git"],
    );
    git(&repo, &["remote", "add", "origin", "https://git.local/o/r.git"]);
    commit(&repo, "README.md", "Initial commit");
    git(&repo, &["push", "-q", "origin", "main"]);
    let (ok, out) = hexa(
        &repo,
        &home,
        &[(token_env, token)],
        &["forge", "add", "git.local", "--kind", "forgejo", "--token-env", token_env],
    );
    assert!(ok, "hexa forge add failed: {out}");
    Setup { _tmp: tmp, home, repo, remote }
}

fn remote_refs(remote: &Path) -> String {
    git(remote, &["for-each-ref"])
}

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

/// S21: pr_body carries the commits and, when given, the ADR, gate and last result
#[test]
fn s21_pr_body_carries_commits_and_evidence() {
    let commits = vec!["Add forge port".to_string(), "Add Forgejo adapter".to_string()];
    let full = pr_body(
        &commits,
        Some("ADR-2611031914"),
        Some("cargo test -p hexa-core forge"),
        Some("passed"),
    );
    assert!(full.contains("Add forge port"), "{full}");
    assert!(full.contains("Add Forgejo adapter"), "{full}");
    assert!(full.contains("ADR-2611031914"), "{full}");
    assert!(full.contains("cargo test -p hexa-core forge"), "{full}");
    assert!(full.contains("passed"), "{full}");

    let bare = pr_body(&commits, None, None, None);
    assert!(bare.contains("Add forge port"), "{bare}");
    assert!(bare.contains("Add Forgejo adapter"), "{bare}");
    assert!(!bare.contains("ADR-2611031914"), "{bare}");
    assert!(!bare.contains("cargo test -p hexa-core forge"), "{bare}");
    assert!(!bare.contains("passed"), "{bare}");
}

/// S22: An existing open pull request from the branch is found, not duplicated
#[test]
fn s22_existing_open_pull_request_is_found_not_duplicated() {
    let var = "HEXA_TEST_FORGEJO_TOKEN_S22";
    unsafe { std::env::set_var(var, "t0k-s22-value") };
    let double = start_double(routes_with_existing_pr);
    let forge = hexa_exec::forge_forgejo::ForgeForgejo::new(&forgejo_target(var), &double.base_url)
        .unwrap_or_else(|e| panic!("ForgeForgejo::new failed: {e:?}"));

    let found = forge.find_pr("feat/x").unwrap_or_else(|e| panic!("find_pr failed: {e:?}"));
    let pr = found.expect("the open pull request from feat/x is found");
    assert_eq!(pr.number, 7);
    assert_eq!(pr.url, "http://git.local/o/r/pulls/7");

    let requests = double.requests();
    assert!(!requests.is_empty(), "the forge was asked");
    assert!(
        !requests.iter().any(|r| r.method == "POST" && r.path.contains("/pulls")),
        "no pull-request POST is made: {requests:?}"
    );
}

/// S23: An unpushed branch is refused and not pushed
#[test]
fn s23_unpushed_branch_is_refused_and_not_pushed() {
    let var = "HEXA_TEST_FORGEJO_TOKEN_S23";
    let s = setup(var, "t0k-s23-value");
    let envs = [(var, "t0k-s23-value")];

    // feat/y: not on the remote at all.
    git(&s.repo, &["checkout", "-q", "-b", "feat/y"]);
    commit(&s.repo, "y.txt", "Work on y");
    let before = remote_refs(&s.remote);
    let (ok, out) = hexa(&s.repo, &s.home, &envs, &["pr", "open", "--send"]);
    assert!(!ok, "pr open on an unpushed branch must fail: {out}");
    assert!(out.to_lowercase().contains("push"), "the refusal says to push the branch: {out}");
    assert_eq!(remote_refs(&s.remote), before, "hexa never pushes");
    assert!(!before.contains("refs/heads/feat/y"));

    // feat/z: on the remote, one commit behind HEAD.
    git(&s.repo, &["checkout", "-q", "-b", "feat/z"]);
    commit(&s.repo, "z.txt", "Work on z");
    git(&s.repo, &["push", "-q", "origin", "feat/z"]);
    commit(&s.repo, "z2.txt", "More work on z");
    let before = remote_refs(&s.remote);
    let (ok, out) = hexa(&s.repo, &s.home, &envs, &["pr", "open", "--send"]);
    assert!(!ok, "pr open on a branch behind its remote copy must fail: {out}");
    assert!(out.to_lowercase().contains("push"), "the refusal says to push the branch: {out}");
    assert_eq!(remote_refs(&s.remote), before, "hexa never pushes");
}

/// S24: --body replaces the generated body
#[test]
fn s24_body_file_replaces_generated_body() {
    let var = "HEXA_TEST_FORGEJO_TOKEN_S24";
    unsafe { std::env::set_var(var, "t0k-s24-value") };
    let double = start_double(routes_without_pr);
    let forge = hexa_exec::forge_forgejo::ForgeForgejo::new(&forgejo_target(var), &double.base_url)
        .unwrap_or_else(|e| panic!("ForgeForgejo::new failed: {e:?}"));

    let tmp = tempfile::tempdir().unwrap();
    let body_file = tmp.path().join("body.md");
    std::fs::write(&body_file, "Hand-written body").unwrap();
    let body = std::fs::read_to_string(&body_file).unwrap();

    // The generated body would carry the commit subject; --body replaces it.
    let generated = pr_body(&["Add forge port".to_string()], None, None, None);
    assert!(generated.contains("Add forge port"));

    let draft = PrDraft {
        head: "feat/x".to_string(),
        base: "main".to_string(),
        title: "Add forge port".to_string(),
        body: body.clone(),
        draft: false,
    };
    let pr = forge.open_pr(&draft).unwrap_or_else(|e| panic!("open_pr failed: {e:?}"));
    assert_eq!(pr.number, 9);
    assert_eq!(pr.url, "http://git.local/o/r/pulls/9");

    let posts: Vec<Recorded> = double
        .requests()
        .into_iter()
        .filter(|r| r.method == "POST" && r.path.contains("/pulls"))
        .collect();
    assert_eq!(posts.len(), 1, "one pull-request POST: {posts:?}");
    let sent: serde_json::Value = serde_json::from_str(&posts[0].body).expect("JSON body");
    let sent_body = sent.get("body").and_then(|v| v.as_str()).unwrap_or_default();
    assert_eq!(sent_body.trim(), "Hand-written body");
    assert!(!sent_body.contains("Add forge port"));
    assert_eq!(sent.get("head").and_then(|v| v.as_str()), Some("feat/x"));
    assert_eq!(sent.get("base").and_then(|v| v.as_str()), Some("main"));
}

/// S25: looks_secret names the first line that holds a credential shape
#[test]
fn s25_looks_secret_names_first_matching_line() {
    let text = "first\nsecond\nkey ghp_abcdefghijklmnopqrstuvwxyz0123456789\nAKIAABCDEFGHIJKLMNOP";
    assert_eq!(looks_secret(text, None), Some(3));
}

/// S26: The forge's current token value is a secret shape
#[test]
fn s26_current_token_value_is_a_secret_shape() {
    let text = "line one\nmy token is t0k-local-xyz";
    assert_eq!(looks_secret(text, Some("t0k-local-xyz")), Some(2));
    assert_eq!(looks_secret(text, None), None);
}

/// S27: A send whose body holds a token-shaped string is refused before any request
#[test]
fn s27_send_with_token_shaped_body_is_refused() {
    let secret = "sk-ant-abc123def456";
    let value = format!("Leaked a key in a note.\nThe key {secret} was pasted here.");
    assert_eq!(looks_secret(&value, None), Some(2));

    let var = "HEXA_TEST_FORGEJO_TOKEN_S27";
    let s = setup(var, "t0k-s27-value");
    let envs = [(var, "t0k-s27-value")];

    let (ok, out) = hexa(&s.repo, &s.home, &envs, &["memory", "store", "bug:leak", &value]);
    assert!(ok, "hexa memory store failed: {out}");

    let (ok, out) = hexa(&s.repo, &s.home, &envs, &["issue", "file", "bug:leak", "--send"]);
    assert!(!ok, "the send must be refused: {out}");
    assert!(out.to_lowercase().contains("line 2"), "the refusal names line 2: {out}");
    assert!(!out.contains(secret), "the match is never printed: {out}");

    let memory = s.repo.join(".hexa").join("memory.jsonl");
    if memory.exists() {
        let text = std::fs::read_to_string(&memory).unwrap();
        assert!(!text.contains("issue:bug:leak"), "memory has no issue:bug:leak: {text}");
    }
    let shared = s.home.join("memory.jsonl");
    if shared.exists() {
        let text = std::fs::read_to_string(&shared).unwrap();
        assert!(!text.contains("issue:bug:leak"), "memory has no issue:bug:leak: {text}");
    }
}

/// S28: Ordinary text is not a secret
#[test]
fn s28_ordinary_text_is_not_a_secret() {
    let text = "The red check accepts stub tests.\nSee ADR-2610221540 §2.";
    assert_eq!(looks_secret(text, None), None);
}

/// S29: The first credential shape anywhere is found at its line
#[test]
fn s29_first_credential_shape_is_found_at_its_line() {
    let texts: Vec<Vec<&str>> = vec![
        vec!["The red check accepts stub tests.", "See ADR-2610221540 §2.", "Nothing else here."],
        vec!["alpha", "beta gamma", "delta", "epsilon zeta eta", "theta"],
        vec!["one line of prose", "another one"],
        vec!["", "an empty line came first", "", "and one in between"],
    ];
    let token = "t0k-local-xyz";
    let shapes: Vec<(&str, bool, bool)> = vec![
        // (shape, is a whole line, is the token value)
        ("sk-ant-abc123def456", false, false),
        ("ghp_abcdefghijklmnopqrstuvwxyz0123456789", false, false),
        ("gho_abcdefghijklmnopqrstuvwxyz0123456789", false, false),
        ("github_pat_11ABCDEFG0123456789_abcdefghijklmnop", false, false),
        ("glpat-abcdefghij0123456789", false, false),
        ("AKIAABCDEFGHIJKLMNOP", false, false),
        ("-----BEGIN RSA PRIVATE KEY-----", true, false),
        ("-----BEGIN OPENSSH PRIVATE KEY-----", true, false),
        (token, false, true),
    ];
    for lines in &texts {
        let unchanged = lines.join("\n");
        for (shape, whole_line, is_token) in &shapes {
            let tok = if *is_token { Some(token) } else { None };
            assert_eq!(looks_secret(&unchanged, tok), None, "unchanged text: {unchanged:?}");
            for n in 1..=lines.len() {
                let mut changed: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
                changed[n - 1] = if *whole_line {
                    shape.to_string()
                } else if changed[n - 1].is_empty() {
                    shape.to_string()
                } else {
                    format!("{} {}", changed[n - 1], shape)
                };
                let changed = changed.join("\n");
                assert_eq!(
                    looks_secret(&changed, tok),
                    Some(n),
                    "shape {shape:?} inserted into line {n} of {changed:?}"
                );
            }
        }
    }
}

/// S30: key_marker and issue_from_memory build the documented draft
#[test]
fn s30_key_marker_and_issue_from_memory_build_documented_draft() {
    let key = "bug:red-check-accepts-stubs";
    let value = "The red check accepts stub tests. It should reject them.";
    let marker = key_marker(key);
    assert_eq!(marker, "<!-- hexa-key: bug:red-check-accepts-stubs -->");

    let draft = issue_from_memory(key, value, "strata");
    assert_eq!(draft.key, key);
    assert_eq!(draft.title, "The red check accepts stub tests");
    assert!(draft.title.chars().count() <= 80);
    assert!(draft.body.contains(value), "{}", draft.body);
    assert!(draft.body.contains("strata"), "{}", draft.body);
    assert!(draft.body.trim_end().ends_with(&marker), "{}", draft.body);
}
}

mod part_4 {
#[allow(unused_imports)]
use std::path::PathBuf;

/// Locate the `hexa` binary built from hexa-cli.
fn hexa_bin() -> PathBuf {
    if let Some(p) = option_env!("CARGO_BIN_EXE_hexa") {
        return PathBuf::from(p);
    }
    if let Ok(p) = std::env::var("HEXA_BIN") {
        return PathBuf::from(p);
    }
    let exe = std::env::current_exe().expect("current exe");
    // target/<profile>/deps/<test> -> target/<profile>/hexa
    let mut dir = exe.parent().expect("deps dir").to_path_buf();
    if dir.ends_with("deps") {
        dir = dir.parent().expect("profile dir").to_path_buf();
    }
    let candidate = dir.join(if cfg!(windows) { "hexa.exe" } else { "hexa" });
    assert!(
        candidate.exists(),
        "the hexa binary was not found at {}; build hexa-cli first",
        candidate.display()
    );
    candidate
}

/// S31: A long first sentence is cut to 80 characters
#[test]
fn s31_long_first_sentence_is_cut_to_80_characters() {
    let first: String = "the red check accepts stub tests when a scenario is only asserted by name "
        .repeat(4)
        .chars()
        .take(200)
        .collect();
    let first = first.replace('.', ",").replace(':', ",");
    assert_eq!(first.chars().count(), 200);
    let value = format!("{first}. A second sentence follows with more detail.");

    let d = hexa_core::ports::forge::issue_from_memory("bug:long", &value, "strata");

    assert!(
        d.title.chars().count() <= 80,
        "title is {} characters: {:?}",
        d.title.chars().count(),
        d.title
    );
    assert!(!d.title.trim().is_empty(), "title must not be empty");
    assert!(
        d.body.contains(&value),
        "the body must still hold the whole value; body was: {}",
        d.body
    );
    assert!(d.body.contains("strata"), "the body names the project");
    assert!(
        d.body.contains("<!-- hexa-key: bug:long -->"),
        "the body carries the marker"
    );
    assert_eq!(d.key, "bug:long");
}

/// S32: The hosts file follows HEXA_HOME and leaves ~/.hexa alone
#[test]
fn s32_hosts_file_follows_hexa_home_and_leaves_home_alone() {
    let hexa_home = tempfile::tempdir().expect("tempdir");
    let fake_home = tempfile::tempdir().expect("tempdir");
    let cwd = tempfile::tempdir().expect("tempdir");

    // Seed ~/.hexa/forges.json in the (fake) home and snapshot it.
    let home_hexa = fake_home.path().join(".hexa");
    std::fs::create_dir_all(&home_hexa).expect("mkdir");
    let home_file = home_hexa.join("forges.json");
    let seeded = b"[{\"host\":\"seed.example\",\"kind\":\"github\",\"token_env\":null}]".to_vec();
    std::fs::write(&home_file, &seeded).expect("seed");

    // Snapshot the real ~/.hexa/forges.json as well.
    let real_home_file = std::env::var("HOME")
        .ok()
        .map(|h| PathBuf::from(h).join(".hexa").join("forges.json"));
    let real_before = real_home_file.as_ref().and_then(|p| std::fs::read(p).ok());

    let out = std::process::Command::new(hexa_bin())
        .args(["forge", "add", "git.local", "--kind", "forgejo", "--token-env", "STRATA_TOKEN"])
        .env("HEXA_HOME", hexa_home.path())
        .env("HOME", fake_home.path())
        .env_remove("STRATA_TOKEN")
        .current_dir(cwd.path())
        .output()
        .expect("run hexa");

    assert!(
        out.status.success(),
        "hexa forge add failed: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    let written = hexa_home.path().join("forges.json");
    let text = std::fs::read_to_string(&written)
        .unwrap_or_else(|e| panic!("{} was not written: {e}", written.display()));
    let _: serde_json::Value = serde_json::from_str(&text).expect("forges.json is JSON");
    assert!(text.contains("git.local"), "host recorded: {text}");
    assert!(text.contains("forgejo"), "kind recorded: {text}");
    assert!(text.contains("STRATA_TOKEN"), "token variable name recorded: {text}");

    let home_after = std::fs::read(&home_file).expect("home file still there");
    assert_eq!(home_after, seeded, "~/.hexa/forges.json must be unchanged");

    if let Some(p) = real_home_file {
        let real_after = std::fs::read(&p).ok();
        assert_eq!(real_after, real_before, "the real ~/.hexa/forges.json must be unchanged");
    }
}
}

