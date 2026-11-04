//! ADR-2611031914: the hard cases the sealed scenarios do not reach. Written
//! from the build spec's test plan (§9), not from the code: paging that is
//! capped or repeats, a stale search index, a lost create, a quoted marker,
//! credentials in the remote URL, the shared-store guard, sync end to end, two
//! processes at once, plain http, drafts, PR guards, redirects, gh's argv, and
//! the hosts file.
#![allow(unused_unsafe)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hexa_core::ports::forge::{issue_from_memory, key_marker, ForgeError, ForgeKind, ForgeTarget, PrDraft};
use hexa_exec::forge_filing::{self, Filed};
use hexa_exec::forge_forgejo::ForgeForgejo;
use hexa_exec::forge_github::ForgeGithub;
use hexa_exec::ports::{Forge, MemoryScope, MemoryStore};

// ── an HTTP double whose answers a test writes ──────────────────────────────

#[derive(Debug, Clone)]
struct Req {
    method: String,
    target: String,
    raw: String,
    body: String,
}

impl Req {
    fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or("")
    }
    fn param(&self, k: &str) -> Option<String> {
        let q = self.target.split_once('?')?.1;
        q.split('&').find_map(|p| {
            let (a, b) = p.split_once('=').unwrap_or((p, ""));
            (a == k).then(|| urlencoding::decode(&b.replace('+', " ")).map(|c| c.into_owned()).unwrap_or_default())
        })
    }
    fn page(&self) -> usize {
        self.param("page").and_then(|p| p.parse().ok()).unwrap_or(1)
    }
}

enum Reply {
    Json(u16, String),
    Headers(u16, Vec<(String, String)>, String),
    /// Close the connection without answering.
    Drop,
}

type Handler = Arc<dyn Fn(&Req) -> Reply + Send + Sync>;

struct Double {
    base: String,
    port: u16,
    log: Arc<Mutex<Vec<Req>>>,
}

impl Double {
    fn start(h: Handler) -> Double {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let log: Arc<Mutex<Vec<Req>>> = Arc::default();
        let log2 = log.clone();
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let (h, log) = (h.clone(), log2.clone());
                std::thread::spawn(move || serve(s, &h, &log));
            }
        });
        Double { base: format!("http://127.0.0.1:{port}"), port, log }
    }
    fn reqs(&self) -> Vec<Req> {
        self.log.lock().unwrap().clone()
    }
    fn posts(&self) -> usize {
        self.reqs().iter().filter(|r| r.method == "POST").count()
    }
}

fn serve(mut s: TcpStream, h: &Handler, log: &Mutex<Vec<Req>>) {
    let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    let end = loop {
        match s.read(&mut tmp) {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break p + 4;
        }
    };
    let head = String::from_utf8_lossy(&buf[..end]).to_string();
    let cl: usize = head
        .lines()
        .find_map(|l| l.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case("content-length")).map(|(_, v)| v.trim().parse().unwrap_or(0)))
        .unwrap_or(0);
    while buf.len() < end + cl {
        match s.read(&mut tmp) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
    }
    let body = String::from_utf8_lossy(&buf[end..(end + cl).min(buf.len())]).to_string();
    let mut first = head.lines().next().unwrap_or("").split_whitespace();
    let req = Req {
        method: first.next().unwrap_or("").into(),
        target: first.next().unwrap_or("").into(),
        raw: format!("{head}{body}"),
        body,
    };
    log.lock().unwrap().push(req.clone());
    let (code, headers, payload) = match h(&req) {
        Reply::Json(c, b) => (c, vec![], b),
        Reply::Headers(c, hs, b) => (c, hs, b),
        Reply::Drop => return,
    };
    let mut resp = format!("HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n", payload.len());
    for (k, v) in headers {
        resp.push_str(&format!("{k}: {v}\r\n"));
    }
    resp.push_str("\r\n");
    resp.push_str(&payload);
    let _ = s.write_all(resp.as_bytes());
}

/// A stateful Forgejo double for owner `acme`, repo `strata`: issues listed
/// with `q` filtering when `index` is true, paged by `cap` per page.
#[derive(Default)]
struct Issues {
    list: Vec<(u64, String, bool)>,
    next: u64,
}

fn issue_json(base: &str, n: u64, body: &str, open: bool) -> serde_json::Value {
    serde_json::json!({"number": n, "title": "t", "body": body, "state": if open {"open"} else {"closed"},
        "html_url": format!("{base}/acme/strata/issues/{n}")})
}

fn forgejo_double(state: Arc<Mutex<Issues>>, cap: usize, index: bool, post_delay: Duration) -> Double {
    let base: Arc<Mutex<String>> = Arc::default();
    let b2 = base.clone();
    let d = Double::start(Arc::new(move |r: &Req| {
        let base = b2.lock().unwrap().clone();
        let p = r.path().trim_end_matches('/');
        if r.method == "POST" && p.ends_with("/issues") {
            std::thread::sleep(post_delay);
            let v: serde_json::Value = serde_json::from_str(&r.body).unwrap_or_default();
            let mut st = state.lock().unwrap();
            st.next += 1;
            let n = st.next;
            let body = v["body"].as_str().unwrap_or("").to_string();
            st.list.push((n, body.clone(), true));
            return Reply::Json(201, issue_json(&base, n, &body, true).to_string());
        }
        if r.method == "GET" && p.ends_with("/issues") {
            let st = state.lock().unwrap();
            let q = r.param("q").unwrap_or_default();
            if !q.is_empty() && !index {
                return Reply::Json(200, "[]".into());
            }
            let all: Vec<serde_json::Value> = st
                .list
                .iter()
                .filter(|(_, b, _)| q.is_empty() || b.contains(&q))
                .map(|(n, b, o)| issue_json(&base, *n, b, *o))
                .collect();
            let page = r.page();
            let items: Vec<_> = all.into_iter().skip((page - 1) * cap).take(cap).collect();
            return Reply::Json(200, serde_json::Value::Array(items).to_string());
        }
        Reply::Json(200, "[]".into())
    }));
    *base.lock().unwrap() = d.base.clone();
    d
}

fn target(var: &str) -> ForgeTarget {
    ForgeTarget { kind: ForgeKind::Forgejo, host: "git.local".into(), owner: "acme".into(), repo: "strata".into(), token_env: Some(var.into()) }
}

fn forgejo_at(d: &Double, test: &str) -> ForgeForgejo {
    let var = format!("HEXA_HARD_TOKEN_{test}");
    unsafe { std::env::set_var(&var, format!("tok-{test}")) };
    ForgeForgejo::new(&target(&var), &d.base).expect("new")
}

#[derive(Default)]
struct MemStore(Mutex<Vec<(String, String)>>);

impl MemoryStore for MemStore {
    fn path(&self, _: MemoryScope) -> PathBuf {
        PathBuf::new()
    }
    fn put(&self, _: MemoryScope, k: &str, v: &str) -> Result<(), String> {
        let mut e = self.0.lock().unwrap();
        e.retain(|(x, _)| x != k);
        e.push((k.into(), v.into()));
        Ok(())
    }
    fn get(&self, _: MemoryScope, k: &str) -> Option<String> {
        self.0.lock().unwrap().iter().find(|(x, _)| x == k).map(|(_, v)| v.clone())
    }
    fn entries(&self, _: MemoryScope, limit: usize) -> Vec<(String, String)> {
        self.0.lock().unwrap().iter().rev().take(limit).cloned().collect()
    }
    fn search(&self, _: MemoryScope, _: &str) -> Vec<(String, String)> {
        vec![]
    }
    fn delete(&self, _: MemoryScope, k: &str) -> Result<bool, String> {
        let mut e = self.0.lock().unwrap();
        let n = e.len();
        e.retain(|(x, _)| x != k);
        Ok(n != e.len())
    }
}

// ── adapter cases ────────────────────────────────────────────────────────────

/// Case 1: A server that caps a page at 30 items: the marker on page 3 is found.
#[test]
fn a_capped_server_is_paged_to_the_end() {
    let st = Arc::new(Mutex::new(Issues::default()));
    for n in 1..=70 {
        let body = if n == 65 { format!("deep\n\n{}", key_marker("bug:deep")) } else { format!("issue {n}") };
        st.lock().unwrap().list.push((n, body, true));
    }
    st.lock().unwrap().next = 70;
    let d = forgejo_double(st, 30, false, Duration::ZERO);
    let f = forgejo_at(&d, "CAP");
    let got = f.find_issue("bug:deep").expect("find").expect("found on page 3");
    assert_eq!(got.number, 65);
    assert_eq!(d.posts(), 0);
}

/// Case 2: A server that repeats its page forever: the listing refuses, in bounded time.
#[test]
fn a_repeating_page_refuses_and_ends() {
    let d = Double::start(Arc::new(|r: &Req| {
        if r.path().ends_with("/issues") {
            Reply::Json(200, serde_json::json!([{"number": 1, "body": "no marker", "state": "open", "html_url": "http://x/1"}]).to_string())
        } else {
            Reply::Json(200, "[]".into())
        }
    }));
    let f = forgejo_at(&d, "REPEAT");
    match f.find_issue("bug:x") {
        Err(ForgeError::Refused(_)) => {}
        other => panic!("expected Refused, got {other:?}"),
    }
    assert!(d.reqs().len() < 10, "{} requests", d.reqs().len());
    assert_eq!(d.posts(), 0);
}

/// Case 3: A stale search index: `q=` finds nothing, the full listing does.
#[test]
fn a_stale_index_is_not_trusted() {
    let st = Arc::new(Mutex::new(Issues::default()));
    st.lock().unwrap().list.push((4, format!("x\n\n{}", key_marker("bug:stale")), false));
    let d = forgejo_double(st, 50, false, Duration::ZERO);
    let f = forgejo_at(&d, "STALE");
    let got = f.find_issue("bug:stale").unwrap().expect("found by the listing");
    assert_eq!(got.number, 4);
    assert!(!got.open);
    assert_eq!(d.posts(), 0);
}

/// Case 4: The create is stored but its answer is lost: one POST, the issue adopted, the link recorded.
#[test]
fn a_lost_create_is_adopted_not_reposted() {
    let st = Arc::new(Mutex::new(Issues::default()));
    let st2 = st.clone();
    let d = Double::start(Arc::new(move |r: &Req| {
        let p = r.path().trim_end_matches('/');
        if r.method == "POST" && p.ends_with("/issues") {
            let v: serde_json::Value = serde_json::from_str(&r.body).unwrap_or_default();
            st2.lock().unwrap().list.push((11, v["body"].as_str().unwrap_or("").into(), true));
            return Reply::Drop;
        }
        let items: Vec<_> = if r.page() > 1 {
            vec![]
        } else {
            st2.lock().unwrap().list.iter().map(|(n, b, o)| issue_json("http://f", *n, b, *o)).collect()
        };
        Reply::Json(200, serde_json::Value::Array(items).to_string())
    }));
    let f = forgejo_at(&d, "LOST");
    let store = MemStore::default();
    let draft = issue_from_memory("bug:lost", "Lost answer. Detail.", "strata");
    let got = forge_filing::file_issue(&f, &store, &draft).expect("adopted");
    assert_eq!(got, Filed::Opened("http://f/acme/strata/issues/11".into()));
    assert_eq!(d.posts(), 1, "never re-posted");
    assert_eq!(store.get(MemoryScope::Project, "issue:bug:lost").as_deref(), Some("http://f/acme/strata/issues/11"));
    assert_eq!(st.lock().unwrap().list.len(), 1);
}

/// Case 5: A marker quoted mid-body, or a longer key's marker, is not a match.
#[test]
fn only_the_exact_last_line_marker_counts() {
    let st = Arc::new(Mutex::new(Issues::default()));
    st.lock().unwrap().list.push((1, format!("Quoting {} here.\n\nmore text", key_marker("bug:y")), true));
    st.lock().unwrap().list.push((2, format!("x\n\n{}", key_marker("bug:x-y")), true));
    let d = forgejo_double(st, 50, true, Duration::ZERO);
    let f = forgejo_at(&d, "QUOTE");
    assert!(f.find_issue("bug:y").unwrap().is_none());
    assert!(f.find_issue("bug:x").unwrap().is_none());
    assert_eq!(f.find_issue("bug:x-y").unwrap().map(|i| i.number), Some(2));
}

/// Case 12a: A Forgejo draft is a `WIP: ` title, since API v1 has no draft field.
#[test]
fn a_forgejo_draft_is_a_wip_title() {
    let d = Double::start(Arc::new(|r: &Req| {
        if r.method == "POST" {
            Reply::Json(201, r#"{"number":3,"html_url":"http://f/acme/strata/pulls/3"}"#.into())
        } else {
            Reply::Json(200, "[]".into())
        }
    }));
    let f = forgejo_at(&d, "DRAFT");
    let pr = PrDraft { head: "feat/x".into(), base: "main".into(), title: "Add it".into(), body: "b".into(), draft: true };
    f.open_pr(&pr).unwrap();
    let post = d.reqs().into_iter().find(|r| r.method == "POST").unwrap();
    let v: serde_json::Value = serde_json::from_str(&post.body).unwrap();
    assert_eq!(v["title"].as_str(), Some("WIP: Add it"));
}

/// Case 13a: A fork's pull request with the same branch name is not this repository's.
#[test]
fn a_fork_pr_with_the_same_head_is_not_a_match() {
    let d = Double::start(Arc::new(|r: &Req| {
        if r.page() > 1 {
            return Reply::Json(200, "[]".into());
        }
        Reply::Json(
            200,
            serde_json::json!([{"number": 5, "html_url": "http://f/5", "head": {"ref": "feat/x", "repo": {"full_name": "someone/strata"}}}]).to_string(),
        )
    }));
    let f = forgejo_at(&d, "FORK");
    assert!(f.find_pr("feat/x").unwrap().is_none());
}

/// Case 14: A redirect is not followed: the token never reaches the other origin.
#[test]
fn a_redirect_is_refused_and_not_followed() {
    let other = Double::start(Arc::new(|_: &Req| Reply::Json(200, "[]".into())));
    let loc = format!("{}/api/v1/repos/acme/strata/issues", other.base);
    let d = Double::start(Arc::new(move |_: &Req| Reply::Headers(302, vec![("Location".into(), loc.clone())], "{}".into())));
    let f = forgejo_at(&d, "REDIRECT");
    match f.find_issue("bug:x") {
        Err(ForgeError::Refused(_)) => {}
        other => panic!("expected Refused, got {other:?}"),
    }
    std::thread::sleep(Duration::from_millis(100));
    assert!(other.reqs().is_empty(), "the redirect was followed");
}

// ── gh ───────────────────────────────────────────────────────────────────────

#[cfg(unix)]
fn gh_stub(dir: &Path) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let log = dir.join("argv.log");
    let gh = dir.join("gh");
    let script = format!(
        r#"#!/bin/sh
echo "---" >> '{log}'
for a in "$@"; do
  printf 'ARG %s\n' "$a" >> '{log}'
  if [ -f "$a" ]; then printf 'FILE %s\n' "$(ls -l "$a" | cut -c1-10)" >> '{log}'; fi
done
case "$2" in
  create) echo 'https://github.com/acme/hexa/pull/12' ;;
  *) echo '[]' ;;
esac
"#,
        log = log.display()
    );
    std::fs::write(&gh, script).unwrap();
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    (gh, log)
}

/// Case 12b, 15: gh's argv: `--title=` keeps a leading `-` a value, `--head=` and
/// `--draft` are passed, and the body is only in a 0600 file gone afterwards.
#[cfg(unix)]
#[test]
fn gh_gets_values_as_flag_equals_and_the_body_in_a_private_file() {
    let dir = tempfile::tempdir().unwrap();
    let (gh, log) = gh_stub(dir.path());
    let t = ForgeTarget { kind: ForgeKind::Github, host: "github.com".into(), owner: "acme".into(), repo: "hexa".into(), token_env: None };
    let f = ForgeGithub::new(&t, &gh);
    let d = PrDraft { head: "feat/x".into(), base: "main".into(), title: "-x starts with a dash".into(), body: "SECRET-FREE BODY TEXT".into(), draft: true };
    let pr = f.open_pr(&d).unwrap();
    assert_eq!(pr.number, 12);
    let rec = std::fs::read_to_string(&log).unwrap();
    assert!(rec.contains("ARG --title=-x starts with a dash"), "{rec}");
    assert!(rec.contains("ARG --head=feat/x"), "{rec}");
    assert!(rec.contains("ARG --draft"), "{rec}");
    assert!(!rec.contains("SECRET-FREE BODY TEXT"), "the body is in argv: {rec}");
    assert!(rec.contains("FILE -rw-------"), "the body file is not 0600: {rec}");
    let path = rec.lines().skip_while(|l| *l != "ARG --body-file").nth(1).and_then(|l| l.strip_prefix("ARG ")).expect("body file path");
    assert!(!Path::new(path).exists(), "the body file was left behind");
}

// ── the CLI ──────────────────────────────────────────────────────────────────

fn hexa_bin() -> PathBuf {
    if let Ok(p) = std::env::var("HEXA_BIN") {
        return PathBuf::from(p);
    }
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/debug/hexa");
    assert!(p.exists(), "build hexa-cli first: {}", p.display());
    p
}

/// Run hexa in `cwd` with HOME at `home`; `hexa_home` sets HEXA_HOME, `None` removes it.
fn hexa(cwd: &Path, home: &Path, hexa_home: Option<&Path>, envs: &[(&str, &str)], args: &[&str]) -> (bool, String) {
    let mut c = Command::new(hexa_bin());
    c.args(args).current_dir(cwd).env("HOME", home).env_remove("HEXA_HOME").env_remove("STRATA_TOKEN");
    if let Some(h) = hexa_home {
        c.env("HEXA_HOME", h);
    }
    for (k, v) in envs {
        c.env(k, v);
    }
    let o = c.output().unwrap();
    (o.status.success(), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
}

fn git(dir: &Path, args: &[&str]) {
    let o = Command::new("git").args(args).current_dir(dir).output().unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
}

fn repo_with(origin: &str) -> tempfile::TempDir {
    let r = tempfile::tempdir().unwrap();
    git(r.path(), &["init", "-q", "-b", "main"]);
    git(r.path(), &["remote", "add", "origin", origin]);
    r
}

const TOK: (&str, &str) = ("STRATA_TOKEN", "the-strata-token-value");

/// Case 6, 16: A password in the remote is never printed or sent, and a body holding it is refused by line, without printing the body.
#[test]
fn a_remote_password_never_leaves_and_refuses_a_body() {
    let st = Arc::new(Mutex::new(Issues::default()));
    let d = forgejo_double(st, 50, true, Duration::ZERO);
    let home = tempfile::tempdir().unwrap();
    let host = format!("127.0.0.1:{}", d.port);
    let repo = repo_with(&format!("http://u:SECRETPW@{host}/acme/strata.git"));
    let h = Some(home.path());
    assert!(hexa(repo.path(), home.path(), h, &[TOK], &["forge", "add", &host, "--kind", "forgejo", "--token-env", "STRATA_TOKEN"]).0);
    hexa(repo.path(), home.path(), h, &[TOK], &["memory", "store", "bug:clean", "A clean bug. Detail."]);
    let (ok, out) = hexa(repo.path(), home.path(), h, &[TOK], &["issue", "file", "bug:clean", "--send"]);
    assert!(ok, "{out}");
    assert!(!out.contains("SECRETPW"), "{out}");
    assert!(!out.contains(TOK.1), "{out}");
    assert!(d.reqs().iter().all(|r| !r.raw.contains("SECRETPW")), "a request carried the password");
    assert_eq!(d.posts(), 1);

    hexa(repo.path(), home.path(), h, &[TOK], &["memory", "store", "bug:leaky", "A leak.\nThe pw SECRETPW is here."]);
    let (ok, out) = hexa(repo.path(), home.path(), h, &[TOK], &["issue", "file", "bug:leaky"]);
    assert!(!ok, "{out}");
    assert!(out.contains("line 2"), "{out}");
    assert!(!out.contains("SECRETPW") && !out.contains("A leak."), "the body was printed: {out}");
    assert_eq!(d.posts(), 1);
}

/// Case 7, 13b: The target comes from the raw remote, not the insteadOf rewrite; a
/// detached HEAD and a branch that is its own base are refused.
#[test]
fn pr_open_reads_the_raw_remote_and_guards_its_branch() {
    let tmp = tempfile::tempdir().unwrap();
    let (home, repo, remote) = (tmp.path().join("home"), tmp.path().join("work"), tmp.path().join("remote.git"));
    std::fs::create_dir_all(&home).unwrap();
    git(tmp.path(), &["init", "-q", "--bare", "-b", "main", remote.to_str().unwrap()]);
    git(tmp.path(), &["init", "-q", "-b", "main", repo.to_str().unwrap()]);
    for (k, v) in [("user.name", "t"), ("user.email", "t@example.invalid"), ("commit.gpgsign", "false")] {
        git(&repo, &["config", k, v]);
    }
    git(&repo, &["config", &format!("url.{}.insteadOf", remote.display()), "https://git.local/o/r.git"]);
    git(&repo, &["remote", "add", "origin", "https://git.local/o/r.git"]);
    let commit = |f: &str, m: &str| {
        std::fs::write(repo.join(f), m).unwrap();
        git(&repo, &["add", f]);
        git(&repo, &["commit", "-q", "-m", m]);
    };
    commit("a", "Initial commit");
    git(&repo, &["push", "-q", "origin", "main"]);
    let h = Some(home.as_path());
    assert!(hexa(&repo, &home, h, &[TOK], &["forge", "add", "git.local", "--kind", "forgejo", "--token-env", "STRATA_TOKEN"]).0);

    let (ok, out) = hexa(&repo, &home, h, &[TOK], &["pr", "open", "--send"]);
    assert!(!ok && out.contains("base itself"), "main against main: {out}");

    git(&repo, &["checkout", "-q", "-b", "feat/x"]);
    commit("b", "Add the thing");
    git(&repo, &["push", "-q", "origin", "feat/x"]);
    let (ok, out) = hexa(&repo, &home, h, &[TOK], &["pr", "open"]);
    assert!(ok, "{out}");
    assert!(out.contains("git.local") && out.contains("o/r"), "target not from the raw URL: {out}");
    assert!(out.contains("feat/x") && out.contains("main") && out.contains("Add the thing"), "{out}");
    assert!(out.contains("dry run"), "{out}");

    git(&repo, &["checkout", "-q", "--detach"]);
    let (ok, out) = hexa(&repo, &home, h, &[TOK], &["pr", "open"]);
    assert!(!ok && out.contains("detached"), "{out}");
}

/// Case 8: `sync --send` refuses a shared store; the dry run lists the key.
#[test]
fn sync_refuses_to_publish_a_shared_store() {
    let st = Arc::new(Mutex::new(Issues::default()));
    let d = forgejo_double(st, 50, true, Duration::ZERO);
    let home = tempfile::tempdir().unwrap();
    let host = format!("127.0.0.1:{}", d.port);
    let repo = repo_with(&format!("http://{host}/acme/strata.git"));
    let h = Some(home.path());
    hexa(repo.path(), home.path(), h, &[TOK], &["forge", "add", &host, "--kind", "forgejo", "--token-env", "STRATA_TOKEN"]);
    hexa(repo.path(), home.path(), h, &[TOK], &["memory", "store", "bug:shared", "Shared bug. Detail."]);
    let (ok, out) = hexa(repo.path(), home.path(), h, &[TOK], &["issue", "sync"]);
    assert!(ok && out.contains("bug:shared"), "{out}");
    let (ok, out) = hexa(repo.path(), home.path(), h, &[TOK], &["issue", "sync", "--send"]);
    assert!(!ok, "{out}");
    assert!(out.contains("shared store") && out.contains("memory.jsonl"), "{out}");
    assert!(d.reqs().is_empty(), "the double was asked: {:?}", d.reqs());
}

/// Case 9: Sync end to end from the repository's own store: one POST, links for the
/// found and the filed, and a rerun sends nothing.
#[test]
fn sync_files_once_and_records_every_link() {
    let st = Arc::new(Mutex::new(Issues::default()));
    st.lock().unwrap().list.push((1, format!("old\n\n{}", key_marker("bug:a")), true));
    st.lock().unwrap().next = 1;
    let d = forgejo_double(st.clone(), 50, true, Duration::ZERO);
    let home = tempfile::tempdir().unwrap();
    let host = format!("127.0.0.1:{}", d.port);
    let repo = repo_with(&format!("http://{host}/acme/strata.git"));
    std::fs::create_dir_all(repo.path().join(".hexa")).unwrap();
    let (r, hm) = (repo.path(), home.path());
    assert!(hexa(r, hm, None, &[TOK], &["forge", "add", &host, "--kind", "forgejo", "--token-env", "STRATA_TOKEN"]).0);
    for (k, v) in [("bug:a", "Bug a. Detail."), ("bug:b", "Bug b. Detail."), ("gap:c", "Gap c. Detail."), ("issue:gap:c", "http://f/c")] {
        assert!(hexa(r, hm, None, &[], &["memory", "store", k, v]).0);
    }
    let (ok, out) = hexa(r, hm, None, &[TOK], &["issue", "sync", "--send"]);
    assert!(ok, "{out}");
    assert_eq!(d.posts(), 1, "{out}");
    let posted: Vec<String> = d.reqs().into_iter().filter(|q| q.method == "POST").map(|q| q.body).collect();
    assert!(posted[0].contains("hexa-key: bug:b"), "{posted:?}");
    let mem = std::fs::read_to_string(r.join(".hexa/memory.jsonl")).unwrap();
    assert!(mem.contains("issue:bug:a") && mem.contains("issues/1"), "{mem}");
    assert!(mem.contains("issue:bug:b") && mem.contains("issues/2"), "{mem}");

    let (ok, out) = hexa(r, hm, None, &[TOK], &["issue", "sync", "--send"]);
    assert!(ok, "{out}");
    assert_eq!(d.posts(), 1, "a rerun posted: {out}");
}

/// Case 10: Two processes filing the same key at once file it once.
#[test]
fn two_processes_file_once() {
    let st = Arc::new(Mutex::new(Issues::default()));
    let d = forgejo_double(st, 50, true, Duration::from_millis(300));
    let home = tempfile::tempdir().unwrap();
    let host = format!("127.0.0.1:{}", d.port);
    let repo = repo_with(&format!("http://{host}/acme/strata.git"));
    let h = Some(home.path());
    hexa(repo.path(), home.path(), h, &[TOK], &["forge", "add", &host, "--kind", "forgejo", "--token-env", "STRATA_TOKEN"]);
    hexa(repo.path(), home.path(), h, &[], &["memory", "store", "bug:z", "Bug z. Detail."]);
    let spawn = || {
        Command::new(hexa_bin())
            .args(["issue", "file", "bug:z", "--send"])
            .current_dir(repo.path())
            .env("HOME", home.path())
            .env("HEXA_HOME", home.path())
            .env(TOK.0, TOK.1)
            .output()
    };
    let (a, b) = std::thread::scope(|s| {
        let a = s.spawn(spawn);
        let b = s.spawn(spawn);
        (a.join().unwrap().unwrap(), b.join().unwrap().unwrap())
    });
    assert!(a.status.success() && b.status.success(), "{a:?}\n{b:?}");
    assert_eq!(d.posts(), 1);
}

/// Case 11: Plain http to a host that is not loopback is refused before any request.
#[test]
fn plain_http_to_a_lan_host_is_refused() {
    let home = tempfile::tempdir().unwrap();
    let repo = repo_with("http://forge.lan/o/r");
    let h = Some(home.path());
    hexa(repo.path(), home.path(), h, &[TOK], &["forge", "add", "forge.lan", "--kind", "forgejo", "--token-env", "STRATA_TOKEN"]);
    hexa(repo.path(), home.path(), h, &[], &["memory", "store", "bug:x", "Bug x. Detail."]);
    let (ok, out) = hexa(repo.path(), home.path(), h, &[TOK], &["issue", "file", "bug:x", "--send"]);
    assert!(!ok && out.contains("plain http"), "{out}");
}

/// Case 17: An unparsable hosts file is never overwritten; concurrent adds all survive.
#[test]
fn the_hosts_file_is_never_clobbered() {
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let file = home.path().join("forges.json");
    std::fs::write(&file, b"{ not json").unwrap();
    let (ok, out) = hexa(cwd.path(), home.path(), Some(home.path()), &[], &["forge", "add", "a.example", "--kind", "github"]);
    assert!(!ok, "{out}");
    assert_eq!(std::fs::read(&file).unwrap(), b"{ not json");
    std::fs::remove_file(&file).unwrap();

    let hosts: Vec<String> = (0..6).map(|i| format!("h{i}.example")).collect();
    std::thread::scope(|s| {
        for h in &hosts {
            let (home, cwd) = (home.path(), cwd.path());
            s.spawn(move || assert!(hexa(cwd, home, Some(home), &[], &["forge", "add", h, "--kind", "forgejo", "--token-env", "T"]).0));
        }
    });
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    for h in &hosts {
        assert!(v.as_array().unwrap().iter().any(|e| e["host"] == h.as_str()), "{h} lost: {v}");
    }
    let (ok, _) = hexa(cwd.path(), home.path(), Some(home.path()), &[], &["forge", "add", "x.example", "--kind", "forgejo", "--token-env", "ghp_abcdefghijklmnopqrstuvwxyz0123"]);
    assert!(!ok, "a pasted token was taken as a variable name");
}

/// Case 18: `forge status` on a rejected token: names the variable, never the value, exits 1, writes nothing.
#[test]
fn forge_status_on_a_rejected_token() {
    let d = Double::start(Arc::new(|_: &Req| Reply::Json(401, r#"{"message":"bad token"}"#.into())));
    let home = tempfile::tempdir().unwrap();
    let host = format!("127.0.0.1:{}", d.port);
    let repo = repo_with(&format!("http://{host}/acme/strata.git"));
    let h = Some(home.path());
    hexa(repo.path(), home.path(), h, &[TOK], &["forge", "add", &host, "--kind", "forgejo", "--token-env", "STRATA_TOKEN"]);
    let snap = |p: &Path| -> Vec<(PathBuf, Vec<u8>)> {
        walkdir::WalkDir::new(p).into_iter().flatten().map(|e| (e.path().to_path_buf(), std::fs::read(e.path()).unwrap_or_default())).collect()
    };
    let before = snap(home.path());
    let (ok, out) = hexa(repo.path(), home.path(), h, &[TOK], &["forge", "status"]);
    assert!(!ok, "{out}");
    assert!(out.contains("STRATA_TOKEN") && !out.contains(TOK.1), "{out}");
    assert_eq!(snap(home.path()), before, "status wrote to HEXA_HOME");
}
