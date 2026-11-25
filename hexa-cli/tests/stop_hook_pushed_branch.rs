//! The Stop hook does not send a session back for a branch that is pushed and
//! waiting on its pull request (the addendum of 2026-11-25 to "a plan ends on
//! main").
//!
//! On 2026-11-24 and 25, in a project whose main is protected and lands only
//! through pull requests, the hook blocked every stop of every agent in a
//! worktree whose branch was pushed and in review, counted the branch against
//! a local main that was 18 commits stale, and named a forced local merge as
//! the fix. Each agent had to refuse it, dozens of times.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SESSION: &str = "stop-hook-pushed-branch-session";

fn hexa_in(dir: &Path, home: &Path, args: &[&str], stdin: Option<&str>) -> String {
    let mut c = Command::new(env!("CARGO_BIN_EXE_hexa"));
    c.args(args)
        .current_dir(dir)
        .env("HOME", home)
        .env("HEXA_HOME", home.join(".hexa"))
        .env("CLAUDE_PROJECT_DIR", dir)
        .env("HEXA_SESSION_ID", SESSION)
        .env_remove("HEXA_SESSION_PID")
        .env_remove("CLAUDE_CODE_SESSION_ID")
        .env_remove("CLAUDE_SESSION_ID")
        .env_remove("HEXA_TASK")
        .env_remove("HEXA_INTERNAL")
        .env("NO_COLOR", "1")
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = c.spawn().expect("run hexa");
    if let Some(s) = stdin {
        use std::io::Write;
        child.stdin.take().unwrap().write_all(s.as_bytes()).unwrap();
    }
    let o = child.wait_with_output().unwrap();
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

fn git(dir: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@x", "-c", "commit.gpgsign=false", "-c", "init.defaultBranch=main"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

/// A clone on main with a bare origin beside it, and a worktree on `feature`
/// holding one commit that is not on main.
struct Repo {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    main: PathBuf,
    feature: PathBuf,
    home: PathBuf,
}

impl Repo {
    fn new() -> Repo {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let home = root.join("home");
        std::fs::create_dir_all(&home).unwrap();
        git(&root, &["init", "-q", "--bare", "origin.git"]);
        git(&root, &["clone", "-q", "origin.git", "work"]);
        let main = root.join("work");
        git(&main, &["checkout", "-q", "-b", "main"]);
        std::fs::write(main.join("a.txt"), "one\n").unwrap();
        std::fs::write(main.join(".gitignore"), ".hexa/\n").unwrap();
        std::fs::create_dir_all(main.join(".hexa")).unwrap();
        git(&main, &["add", "-A"]);
        git(&main, &["commit", "-q", "-m", "init"]);
        git(&main, &["push", "-q", "-u", "origin", "main"]);
        let feature = root.join("feature");
        git(&main, &["worktree", "add", "-q", "-b", "feature", feature.to_str().unwrap(), "main"]);
        std::fs::create_dir_all(feature.join(".hexa")).unwrap();
        std::fs::write(feature.join("b.txt"), "work\n").unwrap();
        git(&feature, &["add", "-A"]);
        git(&feature, &["commit", "-q", "-m", "the work"]);
        Repo { _tmp: tmp, root, main, feature, home }
    }

    fn push_feature(&self) {
        git(&self.feature, &["push", "-q", "-u", "origin", "feature"]);
    }

    /// `hexa hook stop` in the feature worktree.
    fn stop(&self) -> String {
        let stop = serde_json::json!({ "session_id": SESSION, "stop_hook_active": false, "hook_event_name": "Stop" }).to_string();
        hexa_in(&self.feature, &self.home, &["hook", "stop"], Some(&stop))
    }

    /// The kinds `hexa finish --json` lists, from the main checkout.
    fn finish(&self) -> serde_json::Value {
        let out = hexa_in(&self.main, &self.home, &["finish", "--json"], None);
        serde_json::from_str(&out).unwrap_or_else(|e| panic!("finish --json is JSON ({e}): {out}"))
    }
}

fn blocked(out: &str) -> bool {
    out.contains("\"decision\":\"block\"")
}

fn unlanded(v: &serde_json::Value) -> Vec<String> {
    v["open"]
        .as_array()
        .expect("an open list")
        .iter()
        .filter(|i| i["kind"] == "unlanded")
        .map(|i| i["what"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_pushed_branch_waiting_on_its_pull_request_does_not_block_the_stop() {
    let r = Repo::new();
    r.push_feature();
    let out = r.stop();
    assert!(!blocked(&out), "a branch that is pushed as it stands is in review, not unfinished: {out}");
    assert!(out.contains("in-review") && out.contains("feature"), "the stop still says the branch is not on main: {out}");
    assert!(out.contains("pull request"), "and names the pull request as the way it lands: {out}");
    assert!(!out.contains("worktree merge"), "never a forced local merge: {out}");
}

#[test]
fn finish_still_lists_a_pushed_branch_as_unlanded() {
    let r = Repo::new();
    r.push_feature();
    let open = unlanded(&r.finish());
    assert_eq!(open.len(), 1, "hexa finish (and stage done, which reads it) still counts the branch: {open:?}");
    assert!(open[0].contains("feature") && open[0].contains("1 commit"), "{open:?}");
}

#[test]
fn a_branch_never_pushed_still_blocks_the_stop() {
    let r = Repo::new();
    let out = r.stop();
    assert!(blocked(&out), "work that exists only here is unfinished: {out}");
    assert!(out.contains("unlanded") && out.contains("feature"), "{out}");
}

#[test]
fn a_pushed_branch_with_a_commit_not_pushed_still_blocks_and_is_told_to_push() {
    let r = Repo::new();
    r.push_feature();
    std::fs::write(r.feature.join("c.txt"), "more\n").unwrap();
    git(&r.feature, &["add", "-A"]);
    git(&r.feature, &["commit", "-q", "-m", "more work"]);
    let out = r.stop();
    assert!(blocked(&out), "the remote does not hold this head: {out}");
    assert!(out.contains("git push"), "the way out is a push: {out}");
    assert!(!out.contains("worktree merge"), "a branch with an upstream lands through it, never by a forced local merge: {out}");
}

#[test]
fn a_branch_merged_on_the_remote_is_landed_though_local_main_is_stale() {
    let r = Repo::new();
    r.push_feature();
    // the pull request merged on the remote; this clone fetched, and its local main was never moved
    git(&r.root.join("origin.git"), &["update-ref", "refs/heads/main", "refs/heads/feature"]);
    git(&r.main, &["fetch", "-q", "origin"]);
    assert_ne!(git(&r.main, &["rev-parse", "main"]), git(&r.main, &["rev-parse", "origin/main"]), "local main is stale");
    let out = r.stop();
    assert!(!blocked(&out), "a commit on origin's main is on main: {out}");
    assert!(!out.contains("in-review") && !out.contains("unlanded"), "nothing is left to say about it: {out}");
    assert!(unlanded(&r.finish()).is_empty(), "and hexa finish does not count it: {:?}", r.finish());
}
