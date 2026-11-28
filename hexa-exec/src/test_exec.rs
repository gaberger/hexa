//! Running a scenario's test or a gate, here or through a project's remote
//! template (ADR-2611191120).
//!
//! The rules (which runner, the template, the batch markers, the judges) are
//! the domain's, reached through `hexa_core::ports::test_runner`. This module
//! reads the project's setting, builds each command, runs it, and refuses a
//! remote run that could not describe the commit concerned. A remote run that
//! does not prove itself is a runner failure: never a pass, never red, and
//! never retried here (CLAUDE.md: a gate that degrades silently is worse than
//! no gate).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use hexa_core::ports::test_runner::{
    batch_script_for, check_remote_template, fill, on_runner, ON_RUNNER_ENV, judge_counted, judge_script, judge_vitest, parse_test_ref, remote_command,
    runner_for, runner_ran, shell_quote, split_batch, vitest_pattern, Runner, Verdict,
};

/// What a skip site prints when `HEXA_NO_SKIP` forbids the skip.
pub const NOT_PROVEN_HERE: &str = "not proven here:";

/// One scenario's test, run alone.
#[derive(Debug)]
pub enum TestRun {
    Passed,
    /// The test ran and failed.
    Failed(String),
    /// It never ran: the build failed (`compile` is true) or cargo did not.
    Broken { compile: bool, tail: String },
    NotFound(String),
    Vacuous,
    Ambiguous(u64),
    NotProvenHere(String),
    /// The remote runner proved nothing: unreachable, it did not name the
    /// commit, or a marker was lost (ADR-2611191120 §5). Never a pass, never red.
    RunnerFailed(String),
}

/// The cargo arguments that run `f` in `file` and nothing else: an
/// integration test by its target, a library or binary test by its module
/// path in the target that compiles it. A module below `src/` may be compiled
/// by the library or by `main.rs`, so it is run in each one the crate has.
/// The manifest is named from `root`, so the command runs the same in any
/// checkout of the repository, here or on a remote runner (ADR-2611191120 §4).
fn cargo_args(root: &Path, krate: &Path, file: &Path, f: &str) -> Option<Vec<String>> {
    let rel = file.strip_prefix(krate).ok()?;
    let manifest = krate.strip_prefix(root).ok()?.join("Cargo.toml").display().to_string();
    let mut parts: Vec<String> = rel.with_extension("").iter().map(|s| s.to_string_lossy().to_string()).collect();
    let (target, name) = match parts.first().map(String::as_str) {
        Some("tests") if parts.len() == 2 => (vec!["--test".to_string(), parts[1].clone()], f.to_string()),
        Some("src") if parts.get(1).map(String::as_str) == Some("bin") && parts.len() > 2 => {
            // src/bin/x.rs, or src/bin/x/main.rs and the modules beside it.
            let bin = parts[2].clone();
            let mut path: Vec<String> = parts.split_off(3);
            if matches!(path.last().map(String::as_str), Some("main" | "mod")) {
                path.pop();
            }
            path.push(f.to_string());
            (vec!["--bin".to_string(), bin], path.join("::"))
        }
        Some("src") => {
            parts.remove(0);
            let root = parts.len() == 1;
            let target = match parts.last().map(String::as_str) {
                Some("main") if root => vec!["--bins".to_string()],
                Some("lib") if root => vec!["--lib".to_string()],
                _ => {
                    let src = krate.join("src");
                    let has_bin = src.join("main.rs").is_file() || src.join("bin").is_dir();
                    let mut t = Vec::new();
                    if src.join("lib.rs").is_file() || !has_bin {
                        t.push("--lib".to_string());
                    }
                    if has_bin {
                        t.push("--bins".to_string());
                    }
                    t
                }
            };
            if matches!(parts.last().map(String::as_str), Some("lib" | "mod" | "main")) {
                parts.pop();
            }
            parts.push(f.to_string());
            (target, parts.join("::"))
        }
        _ => return None,
    };
    let mut args = vec!["test".to_string(), "--manifest-path".to_string(), manifest];
    args.extend(target);
    // A module ref (`…::`) selects every test under it; a fn ref exactly one.
    let module = name.ends_with("::");
    args.extend(["--".to_string(), name]);
    if !module {
        args.push("--exact".to_string());
    }
    Some(args)
}


/// `.hexa/project.json` → `test` (§2, §3).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TestConfig {
    /// Extension (no dot) → command template with `{file}` and `{name}`.
    pub runners: BTreeMap<String, String>,
    /// The remote template with `{commit}` and `{cmd}`, when tests run elsewhere.
    pub remote: Option<String>,
}

/// The project's test setting. A `test.remote` that lacks a placeholder, or a
/// `test` section of the wrong shape, is an error, never ignored.
pub fn test_config(root: &Path) -> Result<TestConfig, String> {
    let path = root.join(".hexa/project.json");
    let Ok(text) = std::fs::read_to_string(&path) else { return Ok(TestConfig::default()) };
    let v: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: not JSON ({e}), so its test setting cannot be read", path.display()))?;
    let Some(test) = v.get("test").filter(|t| !t.is_null()) else { return Ok(TestConfig::default()) };
    let mut cfg = TestConfig::default();
    if let Some(r) = test.get("runners").filter(|r| !r.is_null()) {
        let obj = r.as_object().ok_or("test.runners in .hexa/project.json must map an extension to a command template")?;
        for (ext, t) in obj {
            let t = t.as_str().ok_or_else(|| format!("test.runners.{ext} must be a command template string"))?;
            cfg.runners.insert(ext.trim_start_matches('.').to_string(), t.to_string());
        }
    }
    if let Some(r) = test.get("remote").filter(|r| !r.is_null()) {
        let t = r.as_str().ok_or("test.remote in .hexa/project.json must be a command template string")?;
        check_remote_template(t)?;
        cfg.remote = Some(t.to_string());
    }
    // On the remote runner itself, its tests run here (ADR-2611240900 §3).
    if cfg.remote.is_some() {
        let marker = std::env::var(ON_RUNNER_ENV).ok();
        let head = full_commit(root, "HEAD");
        if on_runner(marker.as_deref(), head.as_deref())? {
            cfg.remote = None;
        }
    }
    Ok(cfg)
}

/// Where a run goes.
#[derive(Debug, Clone, Copy)]
pub enum Where<'a> {
    /// In the checkout given, on this machine.
    Here,
    /// Through the remote template, for this full commit id.
    Remote { template: &'a str, commit: &'a str },
}

/// How one test's output is judged.
#[derive(Debug, Clone)]
enum Judge {
    Cargo { module: bool },
    Vitest(String),
    Script,
    Counted,
    Pytest,
}

/// A test, ready to run from the root of a checkout.
#[derive(Debug, Clone)]
pub struct Planned {
    pub cmd: String,
    judge: Judge,
}

/// The environment a test runs under. `path_env` and `target_dir` are for a
/// run here; a remote runner has its own PATH and target.
pub fn env_prefix(path_env: Option<&str>, target_dir: Option<&Path>) -> String {
    let path = path_env.map(|p| format!(" PATH={p:?}")).unwrap_or_default();
    let target = target_dir.map(|t| format!(" CARGO_TARGET_DIR={:?}", t.display().to_string())).unwrap_or_default();
    // CARGO_TERM_COLOR=never: this output is parsed, and an inherited
    // `always` (CI sets it) wraps the lines read here in escape codes.
    format!("export{path} HEXA_NO_SKIP=1 CARGO_TERM_COLOR=never{target}; ")
}

/// The nearest directory at or above `file`, inside `root`, holding `marker`.
/// The files that make a directory the one pytest runs from.
const PYTEST_MARKERS: [&str; 4] = ["pyproject.toml", "pytest.ini", "tox.ini", "setup.cfg"];

fn nearest(root: &Path, file: &Path, marker: &str) -> Option<PathBuf> {
    let mut dir = file.parent()?;
    loop {
        if dir.join(marker).exists() {
            return Some(dir.to_path_buf());
        }
        if dir == root {
            return None;
        }
        dir = dir.parent()?;
    }
}

/// `p` relative to `base`, through `..` where `p` is not below it: both are
/// paths in one repository, so they share its root.
fn pathdiff(p: &Path, base: &Path) -> String {
    let p: Vec<_> = p.components().collect();
    let b: Vec<_> = base.components().collect();
    let common = p.iter().zip(b.iter()).take_while(|(x, y)| x == y).count();
    let mut out: Vec<String> = std::iter::repeat_n("..".to_string(), b.len() - common).collect();
    out.extend(p[common..].iter().map(|c| c.as_os_str().to_string_lossy().to_string()));
    out.join("/")
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root).map(|r| r.to_string_lossy().to_string()).unwrap_or_else(|_| p.display().to_string())
}

/// Plan the test `r` in the checkout at `root` (§1, §2). `prefix` is
/// [`env_prefix`]. A ref that names no runnable test is its `TestRun`.
pub fn plan_test(root: &Path, r: &str, cfg: &TestConfig, prefix: &str) -> Result<Planned, TestRun> {
    let Some(t) = parse_test_ref(r) else {
        return Err(TestRun::NotFound(format!(
            "{r} is not a test reference: `<path>.rs::<fn>`, `<path>.ts::<test name>`, `<path>.sh[::<arg>]`, or `<path>.<ext>::<name>` for a runner in test.runners"
        )));
    };
    let Some(runner) = runner_for(t.ext(), &cfg.runners) else {
        return Err(TestRun::NotFound(format!(
            "no runner for `.{}` ({r}): add one to test.runners in .hexa/project.json, e.g. \"{}\": \"<command with {{file}} and {{name}}>\"",
            t.ext(),
            t.ext()
        )));
    };
    let abs = root.join(&t.file);
    if !abs.is_file() {
        return Err(TestRun::NotFound(format!("{} not found", t.file)));
    }
    let name = t.name.clone().unwrap_or_default();
    match runner {
        Runner::Cargo => {
            let module = name.ends_with("::");
            let leaf = name.trim_end_matches("::").rsplit("::").next().unwrap_or(&name).to_string();
            let text = std::fs::read_to_string(&abs).unwrap_or_default();
            let declared = if module { format!("mod {leaf} ") } else { format!("fn {leaf}(") };
            if !text.contains(&declared) {
                return Err(TestRun::NotFound(format!("{leaf} not found in {}", t.file)));
            }
            let Some(args) = nearest(root, &abs, "Cargo.toml").and_then(|k| cargo_args(root, &k, &abs, &name))
            else {
                return Err(TestRun::NotFound(format!("{} is in no crate's tests/ or src/", t.file)));
            };
            let quoted: Vec<String> = args.iter().map(|a| shell_quote(a)).collect();
            Ok(Planned { cmd: format!("{prefix}cargo {}", quoted.join(" ")), judge: Judge::Cargo { module } })
        }
        Runner::Vitest => {
            let Some(pkg) = nearest(root, &abs, "package.json") else {
                return Err(TestRun::NotFound(format!("no package.json at or above {}: vitest runs from its package", t.file)));
            };
            let in_pkg = rel(&pkg, &abs);
            let cd = if pkg == root { String::new() } else { format!("cd {} && ", shell_quote(&rel(root, &pkg))) };
            // npm_config_yes=false: npx never fetches a vitest the package does
            // not have; a missing runner fails loudly instead of a stranger
            // version running in its place.
            let cmd = format!(
                "{prefix}export npm_config_yes=false; {cd}npx vitest run {} -t {} --reporter=json",
                shell_quote(&in_pkg),
                shell_quote(&vitest_pattern(&name))
            );
            Ok(Planned { cmd, judge: Judge::Vitest(name) })
        }
        Runner::Bash => {
            let arg = t.name.as_deref().map(|a| format!(" {}", shell_quote(a))).unwrap_or_default();
            Ok(Planned { cmd: format!("{prefix}bash {}{arg}", shell_quote(&t.file)), judge: Judge::Script })
        }
        Runner::Pytest => {
            // The test must be in the file, as for cargo: pytest exits 4 for a
            // name it cannot find, and an exit that is not 0 must not be red.
            // A parametrized case is named `test_p[1]`; the function is `test_p`.
            let base = name.split_once('[').map_or(name.as_str(), |(f, _)| f);
            let leaf = base.rsplit("::").next().unwrap_or(base);
            let text = std::fs::read_to_string(&abs).unwrap_or_default();
            if leaf.is_empty() || !text.contains(&format!("def {leaf}(")) {
                return Err(TestRun::NotFound(format!("{leaf} not found in {}", t.file)));
            }
            // From the nearest directory that configures pytest, as a person
            // would run it there (ADR-2611211324 §8); the repository root if none.
            let dir = PYTEST_MARKERS.iter().filter_map(|m| nearest(root, &abs, m)).max_by_key(|d| d.components().count());
            let dir = dir.unwrap_or_else(|| root.to_path_buf());
            let cd = if dir == root { String::new() } else { format!("cd {} && ", shell_quote(&rel(root, &dir))) };
            let target = format!("{}::{name}", rel(&dir, &abs));
            let python = project_python(root, &dir);
            Ok(Planned { cmd: format!("{prefix}{cd}{python} -m pytest {} -q", shell_quote(&target)), judge: Judge::Pytest })
        }
        Runner::Configured(template) => {
            let cmd = fill(&template, &[("file", &t.file), ("name", &name)]);
            Ok(Planned { cmd: format!("{prefix}{cmd}"), judge: Judge::Counted })
        }
    }
}

fn tail(out: &str, n: usize) -> String {
    let lines: Vec<&str> = out.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n") + "\n"
}

/// What one run of a planned test established.
pub fn judge(p: &Planned, ok: bool, out: &str) -> TestRun {
    if let Some(line) = out.lines().find(|l| l.contains(NOT_PROVEN_HERE)) {
        let reason = line.split(NOT_PROVEN_HERE).nth(1).unwrap_or("").trim();
        return TestRun::NotProvenHere(reason.trim_matches(|c| c == '"' || c == '\'').to_string());
    }
    let verdict = match &p.judge {
        Judge::Cargo { module } => return judge_cargo(*module, ok, out),
        Judge::Vitest(name) => judge_vitest(out, name),
        Judge::Script => judge_script(ok, out),
        Judge::Counted => judge_counted(ok, out),
        Judge::Pytest => match pytest_ran_nothing(out) {
            Some(run) => return run,
            None => judge_counted(ok, out),
        },
    };
    match verdict {
        Verdict::Passed => TestRun::Passed,
        Verdict::Failed(why) => TestRun::Failed(why),
        Verdict::Vacuous => TestRun::Vacuous,
        Verdict::Ambiguous(n) => TestRun::Ambiguous(n),
        Verdict::Unreadable(why) => TestRun::Broken { compile: false, tail: format!("{why}\n{}", tail(out, 10)) },
    }
}

/// The project's own Python for work run from `dir`, as vitest is the
/// package's own (ADR-2611230832): uv when a uv.lock is at or above `dir`, else
/// a virtualenv's interpreter there, else python3. Searched from `dir` up to
/// `root`. A relative path is relative to `dir`, so a command that names it
/// names no path of this machine and runs the same under test.remote.
pub fn project_python(root: &Path, dir: &Path) -> String {
    let from = dir.join("_");
    if nearest(root, &from, "uv.lock").is_some() {
        return "uv run python".to_string();
    }
    // The nearest interpreter wins; at the same depth `.venv` wins over
    // `venv`, as ADR-2611230832 §2 lists them.
    let mut found: Option<(usize, PathBuf)> = None;
    for m in [".venv/bin/python", "venv/bin/python"] {
        if let Some(d) = nearest(root, &from, m) {
            let depth = d.components().count();
            if found.as_ref().is_none_or(|(best, _)| depth > *best) {
                found = Some((depth, d.join(m)));
            }
        }
    }
    match found {
        Some((_, v)) => {
            let v = pathdiff(&v, dir);
            if v.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+".contains(c)) { v } else { shell_quote(&v) }
        }
        None => "python3".to_string(),
    }
}

/// pytest's own account of a run that ran no test, read before its exit
/// code: exit 4 (no such test, a conftest that does not import, a usage
/// error such as an `addopts` plugin that is not installed), exit 5 (none
/// collected) and a Python without pytest all exit non-zero, and none of
/// them is a test that failed.
fn pytest_ran_nothing(out: &str) -> Option<TestRun> {
    if out.contains("No module named pytest") {
        return Some(TestRun::Broken { compile: false, tail: format!("pytest is not installed for python3\n{}", tail(out, 10)) });
    }
    if out.lines().any(|l| l.starts_with("ImportError while loading conftest") || l.starts_with("ERROR: usage:")) {
        return Some(TestRun::Broken { compile: false, tail: tail(out, 10) });
    }
    if out.lines().any(|l| l.starts_with("ERROR: not found:")) {
        return Some(TestRun::NotFound(tail(out, 10)));
    }
    let summary = out.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("");
    if summary.trim_matches(|c| c == '=' || c == ' ').starts_with("no tests ran") {
        return Some(TestRun::Vacuous);
    }
    None
}

fn judge_cargo(module: bool, ok: bool, out: &str) -> TestRun {
    if !ok {
        if out.contains("test result: FAILED") {
            return TestRun::Failed(tail(out, 10));
        }
        let compile = out.contains("could not compile") || out.contains("error[E");
        return TestRun::Broken { compile, tail: tail(out, 10) };
    }
    let passed: u64 = out
        .lines()
        .filter_map(|l| l.strip_prefix("test result: ok. "))
        .filter_map(|l| l.split(" passed").next()?.trim().parse::<u64>().ok())
        .sum();
    match passed {
        0 => TestRun::Vacuous,
        1 => TestRun::Passed,
        // A module scenario passes when every test in it ran and passed.
        _ if module => TestRun::Passed,
        n => TestRun::Ambiguous(n),
    }
}

fn run_tag() -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default();
    format!("{:x}{:x}", t, std::process::id())
}

/// Run every plan in `dir` (§4). Here, one at a time; remote, all in one run
/// whose output must name the commit and carry every end marker (§5).
pub async fn run_plans(dir: &Path, at: Where<'_>, plans: &[Planned]) -> Vec<TestRun> {
    if plans.is_empty() {
        return Vec::new();
    }
    match at {
        Where::Here => {
            let mut out = Vec::with_capacity(plans.len());
            for p in plans {
                let (ok, text) = crate::direct_exec::run_evidence(&p.cmd, dir).await;
                out.push(judge(p, ok, &text));
            }
            out
        }
        Where::Remote { template, commit } => match run_remote(dir, template, commit, plans.iter().map(|p| p.cmd.clone()).collect()).await {
            Ok(segments) => plans.iter().zip(segments).map(|(p, (ok, text))| judge(p, ok, &text)).collect(),
            Err(why) => plans.iter().map(|_| TestRun::RunnerFailed(why.clone())).collect(),
        },
    }
}

/// Send `cmds` for `commit` through `template` in one run, from `dir`; each
/// command's (exit 0, output), or why the runner proved nothing (§4, §5).
async fn run_remote(dir: &Path, template: &str, commit: &str, cmds: Vec<String>) -> Result<Vec<(bool, String)>, String> {
    run_remote_full(dir, template, commit, cmds).await.map(|(v, _)| v)
}

/// [`run_remote`], with the runner's whole output too: what it printed around
/// the commands (which commit it checked out) is evidence as well.
async fn run_remote_full(
    dir: &Path,
    template: &str,
    commit: &str,
    cmds: Vec<String>,
) -> Result<(Vec<(bool, String)>, String), String> {
    let n = run_tag();
    let full = remote_command(template, commit, &batch_script_for(&n, commit, &cmds))?;
    let (_, out) = crate::direct_exec::run_evidence(&full, dir).await;
    if !runner_ran(&out, commit) {
        return Err(format!(
            "runner failure: the remote runner's output does not name commit {commit}, so it is not known to have run it \
             (unreachable, or it checked out something else). Nothing was run here instead. Its last lines:\n{}",
            tail(&out, 8)
        ));
    }
    let split = split_batch(&n, cmds.len(), &out).map_err(|e| format!("runner failure: {e}. Its last lines:\n{}", tail(&out, 8)))?;
    Ok((split, out))
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let o = std::process::Command::new("git").args(args).current_dir(dir).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// The full id of `rev` in `dir`.
pub fn full_commit(dir: &Path, rev: &str) -> Option<String> {
    git(dir, &["rev-parse", "--verify", "--quiet", &format!("{rev}^{{commit}}")])
}

/// Refuse a remote run that could not describe `commit` (§5): a commit on no
/// remote-tracking ref, or, when `head_tree`, a tree whose tracked files
/// differ from HEAD.
pub fn remote_preflight(dir: &Path, commit: &str, head_tree: bool) -> Result<(), String> {
    let short: String = commit.chars().take(12).collect();
    let pushed = git(dir, &["for-each-ref", "--contains", commit, "--count=1", "--format=%(refname)", "refs/remotes"])
        .is_some_and(|s| !s.is_empty());
    if !pushed {
        return Err(format!(
            "{short} is on no remote branch, so the remote runner (test.remote) cannot check it out: push it first \
             (`git push`), or run here with --local"
        ));
    }
    if head_tree {
        let dirty = git(dir, &["status", "--porcelain", "--untracked-files=no"]).unwrap_or_default();
        if !dirty.is_empty() {
            let files: Vec<&str> = dirty.lines().filter_map(|l| l.get(3..)).take(5).collect();
            return Err(format!(
                "tracked files differ from HEAD ({}): a remote run tests HEAD {short}, not this tree. Commit and push, \
                 or run here with --local",
                files.join(", ")
            ));
        }
    }
    Ok(())
}

/// Link each `node_modules` the checkout at `root` has into the checkout at
/// `wt` of an older commit, beside the same `package.json`, so vitest runs
/// there with the project's installed dependencies (a fresh checkout has none).
pub fn link_node_modules(root: &Path, wt: &Path) {
    fn walk(root: &Path, wt: &Path, rel: &Path, depth: usize) {
        let here = root.join(rel);
        if here.join("package.json").is_file() && here.join("node_modules").is_dir() && !wt.join(rel).join("node_modules").exists()
            && wt.join(rel).join("package.json").is_file()
        {
            #[cfg(unix)]
            let _ = std::os::unix::fs::symlink(here.join("node_modules"), wt.join(rel).join("node_modules"));
        }
        if depth == 0 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(&here) else { return };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if e.path().is_dir() && !name.starts_with('.') && !matches!(name.as_str(), "node_modules" | "target") {
                walk(root, wt, &rel.join(&name), depth - 1);
            }
        }
    }
    walk(root, wt, Path::new(""), 3);
}

/// Run a gate command for HEAD of `dir` (§4): here, or through the remote.
/// `Err` is a refusal or a runner failure, never a gate result.
pub async fn run_gate(dir: &Path, gate: &str, cfg: &TestConfig, local: bool) -> Result<(bool, String), String> {
    run_gate_and(dir, gate, &[], cfg, local).await.map(|mut v| v.remove(0))
}

/// The code a refusal of an unpushed HEAD carries (ADR-2611240900 §4).
const HEAD_NOT_PUSHED: &str = "head-not-pushed";

/// Run `gate`, then each of `also`, for HEAD of `dir`: here one after another,
/// or through the remote in **one** run (ADR-2611191120 §4, ADR-2611240900 §4).
/// Under the remote, HEAD must be on the remote, as the remote itself reports
/// it. One `(exit 0, output)` per command, the gate's first.
pub async fn run_gate_and(dir: &Path, gate: &str, also: &[String], cfg: &TestConfig, local: bool) -> Result<Vec<(bool, String)>, String> {
    let mut cmds = vec![gate.to_string()];
    cmds.extend(also.iter().cloned());
    match cfg.remote.as_deref().filter(|_| !local) {
        None => {
            let mut out = Vec::with_capacity(cmds.len());
            for c in &cmds {
                out.push(crate::direct_exec::run_evidence(c, dir).await);
            }
            Ok(out)
        }
        Some(template) => {
            let head = full_commit(dir, "HEAD").ok_or("HEAD is not a commit: there is nothing for the remote runner to check out")?;
            remote_holds(dir, &head)?;
            remote_preflight(dir, &head, true)?;
            println!("  sending the gate to the remote runner for {}", &head[..12]);
            run_remote(dir, template, &head, cmds).await
        }
    }
}

/// Refuse, with [`HEAD_NOT_PUSHED`], a `commit` the remote does not hold
/// (ADR-2611240900 §4). The branch's upstream head is read from the remote
/// itself (`git ls-remote`): it must be `commit`, or a commit `commit` is an
/// ancestor of. A branch with no upstream falls back to the local
/// remote-tracking refs (ADR-2611191120 §5).
fn remote_holds(dir: &Path, commit: &str) -> Result<(), String> {
    let short: String = commit.chars().take(12).collect();
    let branch = git(dir, &["symbolic-ref", "--quiet", "--short", "HEAD"]);
    let upstream = branch.as_deref().and_then(|b| {
        let remote = git(dir, &["config", &format!("branch.{b}.remote")])?;
        let merge = git(dir, &["config", &format!("branch.{b}.merge")])?;
        Some((b.to_string(), remote, merge))
    });
    let Some((branch, remote, merge)) = upstream else {
        let tracked = git(dir, &["for-each-ref", "--contains", commit, "--count=1", "--format=%(refname)", "refs/remotes"])
            .is_some_and(|s| !s.is_empty());
        if tracked {
            return Ok(());
        }
        return Err(format!(
            "{HEAD_NOT_PUSHED}: {short} is on no remote branch, so the remote runner (test.remote) cannot check it out. \
             Push it (`git push -u origin <branch>`), or run here with --local"
        ));
    };
    let theirs = git(dir, &["ls-remote", &remote, &merge])
        .ok_or_else(|| format!("{HEAD_NOT_PUSHED}: {remote} could not be read (`git ls-remote {remote} {merge}` failed), so it is not known to hold {short}. Check that {remote} is reachable, then run again"))?;
    let theirs = theirs.split_whitespace().next().unwrap_or("").to_string();
    let push = format!("`git push {remote} {branch}`");
    if theirs.is_empty() {
        return Err(format!(
            "{HEAD_NOT_PUSHED}: {remote} has no {merge}, so the remote runner (test.remote) cannot check out {short}, whatever the local tracking ref says. {push}, or run here with --local"
        ));
    }
    if theirs == commit {
        return Ok(());
    }
    let ancestor = std::process::Command::new("git")
        .args(["merge-base", "--is-ancestor", commit, &theirs])
        .current_dir(dir)
        .status()
        .is_ok_and(|s| s.success());
    if ancestor {
        return Ok(());
    }
    let theirs_short: String = theirs.chars().take(12).collect();
    Err(format!(
        "{HEAD_NOT_PUSHED}: {remote} reports {merge} at {theirs_short}, which does not hold {short}. {push} \
         (if {remote} moved on, `git pull --rebase` first), or run here with --local"
    ))
}

/// The remote a scratch candidate is pushed to: the branch's upstream remote,
/// else `origin` (ADR-2611240900 §1).
pub fn scratch_remote(root: &Path) -> Option<String> {
    let from_branch = git(root, &["symbolic-ref", "--quiet", "--short", "HEAD"]).and_then(|b| git(root, &["config", &format!("branch.{b}.remote")]));
    from_branch.or_else(|| git(root, &["remote"]).and_then(|r| r.lines().any(|l| l.trim() == "origin").then(|| "origin".to_string())))
}

/// Run `cmd` through `template` for the tree of `root` as it stands
/// (ADR-2611240900 §1): a scratch candidate (HEAD's tree, every tracked change,
/// and the untracked files not in `leave_out`, HEAD its parent) is built in a
/// throwaway index, pushed to `refs/heads/hexa-gate/<first 12>` on
/// [`scratch_remote`], run, and the ref deleted on every path. Nothing moves
/// here: not the branch, not the index. `Ok` is the command's (exit 0, output)
/// and the candidate's id; `Err` is a refusal or a runner failure, never a
/// gate result.
pub async fn run_tree_remote(
    root: &Path,
    template: &str,
    cmd: &str,
    leave_out: &std::collections::BTreeSet<String>,
) -> Result<(bool, String, String), String> {
    let remote = scratch_remote(root).ok_or_else(|| {
        "runner failure: this repository has no remote to push the candidate to, so the remote runner (test.remote) cannot see it: `git remote add origin <url>`, or run here with --local".to_string()
    })?;
    let head = full_commit(root, "HEAD").ok_or("HEAD is not a commit: there is nothing to build a candidate on")?;
    let index = git_ok(root, &["rev-parse", "--git-path", "hexa-tree-index"], &[])?;
    let index = if Path::new(&index).is_absolute() { PathBuf::from(index) } else { root.join(index) };
    let _ = std::fs::remove_file(&index);
    let idx = index.to_string_lossy().to_string();
    let env = [("GIT_INDEX_FILE", idx.as_str())];
    let built = (|| -> Result<String, String> {
        git_ok(root, &["read-tree", "HEAD"], &env)?;
        git_ok(root, &["add", "-A", "--", "."], &env)?;
        for p in leave_out {
            git_ok(root, &["rm", "-q", "--cached", "--ignore-unmatch", "--", p], &env)?;
        }
        let tree = git_ok(root, &["write-tree"], &env)?;
        // A scratch commit that is only pushed to the runner and never kept, so it names hexa, not the operator: a host
        // with no git identity (a CI runner) must still be able to gate a fix (spec_author's scratch commits do the same).
        git_ok(root, &["-c", "user.name=hexa-harden", "-c", "user.email=harden@hexa.local", "commit-tree", &tree, "-p", &head, "-m", "hexa harden: a scratch candidate of the tree, gated on the remote runner (ADR-2611240900 §1)"], &[])
    })();
    let _ = std::fs::remove_file(&index);
    let candidate = built.map_err(|e| format!("the candidate commit could not be built: {e}"))?;
    let short: String = candidate.chars().take(12).collect();
    let temp = format!("refs/heads/hexa-gate/{short}");
    git_ok(root, &["push", "-q", &remote, &format!("{candidate}:{temp}")], &[])
        .map_err(|e| format!("runner failure: the candidate {short} could not be pushed to {remote} ({temp}): {e}"))?;
    let ran = run_remote(root, template, &candidate, vec![cmd.to_string()]).await;
    let _ = git_ok(root, &["push", "-q", &remote, "--delete", &temp], &[]);
    let (ok, out) = ran?.remove(0);
    Ok((ok, out, candidate))
}

/// How many lines of the remote run's output are kept as a gate's evidence.
const EVIDENCE_LINES: usize = 30;

/// A change `hexa do gate` committed after its gate passed on the remote
/// runner, and that the remote then reported holding (ADR-2611231400 §3).
#[derive(Debug, Clone)]
pub struct RemoteGated {
    /// The commit the runner tested, which is the commit the branch now points at.
    pub commit: String,
    /// The branch it went on.
    pub branch: String,
    /// The remote it was pushed to.
    pub remote: String,
    /// The branch's head as `git ls-remote` read it back after the push.
    pub remote_head: String,
    /// The last [`EVIDENCE_LINES`] lines of the runner's output.
    pub evidence: String,
    /// The verdict: counts, and for a red the first failing line.
    pub judged: crate::direct_exec::Judged,
}

/// What `hexa do gate --dry-run` would do (ADR-2611231400 §6).
#[derive(Debug, Clone)]
pub struct GatePlan {
    /// HEAD's full id: the candidate's parent.
    pub parent: String,
    pub branch: String,
    pub files: Vec<String>,
    /// The upstream's remote and branch ref, under `test.remote`.
    pub upstream: Option<(String, String)>,
    /// The `test.remote` command, with `{commit}` shown as `<candidate>`.
    pub run: Option<String>,
}

fn git_ok(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Result<String, String> {
    let mut c = std::process::Command::new("git");
    c.args(args).current_dir(dir);
    for (k, v) in env {
        c.env(k, v);
    }
    let o = c.output().map_err(|e| format!("git {}: {e}", args.first().unwrap_or(&"")))?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
    } else {
        Err(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&o.stderr).trim()))
    }
}

use crate::direct_exec::{Expect, GateFailure};

/// HEAD's full id and its branch, or the refusal of a gate with nowhere to commit.
fn head_and_branch(root: &Path) -> Result<(String, String), Box<GateFailure>> {
    let head = full_commit(root, "HEAD").ok_or_else(|| {
        GateFailure::new("refused", "HEAD is not a commit: there is nothing to build the candidate on", "make a first commit, then gate", None, false)
    })?;
    let branch = git_ok(root, &["symbolic-ref", "--quiet", "--short", "HEAD"], &[]).map_err(|_| {
        GateFailure::new("refused", "HEAD is detached: a gate commits onto a branch", "`git switch -c <branch>`, then run the same command", None, false)
    })?;
    Ok((head, branch))
}

/// The remote and branch ref `branch` tracks. A branch with none is refused
/// rather than guessed at (ADR-2611221610 §7), and so is a branch that tracks
/// a branch of another name: pushing to that upstream would put the commit on
/// another branch, typically the base (gaberger/hexa#75).
fn upstream_of(root: &Path, branch: &str) -> Result<(String, String), Box<GateFailure>> {
    let (remote, merge) = git_ok(root, &["config", &format!("branch.{branch}.remote")], &[])
        .and_then(|r| git_ok(root, &["config", &format!("branch.{branch}.merge")], &[]).map(|m| (r, m)))
        .map_err(|_| {
            GateFailure::new(
                "no-upstream",
                format!(
                    "refused: {branch} tracks no remote branch, so a remote gate (test.remote) has nowhere to push the candidate; nothing committed"
                ),
                format!("`git push -u origin {branch}` first, then run the same command; or gate here with --local"),
                None,
                false,
            )
        })?;
    if merge != format!("refs/heads/{branch}") {
        let theirs = merge.strip_prefix("refs/heads/").unwrap_or(&merge);
        return Err(GateFailure::new(
            "no-upstream",
            format!(
                "refused: {branch} tracks {remote}/{theirs}, not a branch of its own, so a remote gate would push the commit to {theirs}; nothing committed"
            ),
            format!("`git push -u {remote} {branch}` first, so {branch} tracks its own name, then run the same command; or gate here with --local"),
            None,
            false,
        ));
    }
    Ok((remote, merge))
}

/// The branch's head on `remote`, read back from the remote itself.
fn remote_head(root: &Path, remote: &str, merge: &str) -> Option<String> {
    git_ok(root, &["ls-remote", remote, merge], &[]).ok()?.split_whitespace().next().map(str::to_string)
}

/// What `hexa do gate` would do, after every check it makes before it builds
/// a candidate, doing none of it (ADR-2611231400 §6). `template` is the
/// `test.remote` template, or `None` for a gate run here.
pub async fn gate_plan(root: &Path, files: &[String], evidence: &str, template: Option<&str>) -> Result<GatePlan, Box<GateFailure>> {
    let paths = crate::direct_exec::gate_checks(root, files).await.map_err(GateFailure::from_checks)?;
    let (parent, branch) = head_and_branch(root)?;
    let (upstream, run) = match template {
        None => (None, None),
        Some(t) => {
            let up = upstream_of(root, &branch)?;
            check_remote_template(t).map_err(|e| GateFailure::new("refused", e, "fix test.remote in .hexa/project.json", None, false))?;
            (Some(up), Some(fill(t, &[("commit", "<candidate>"), ("cmd", evidence)])))
        }
    };
    Ok(GatePlan { parent, branch, files: paths, upstream, run })
}

/// `hexa do gate` under `test.remote` (ADR-2611221610 §7, ADR-2611231400).
/// The same checks as a local gate; then a candidate commit (HEAD's tree with
/// `files` as they are in the working tree, HEAD its parent) is built without
/// moving the branch, pushed to `refs/heads/hexa-gate/<id>` on the branch's
/// remote, and the gate runs through `template` for it. Only a verdict that
/// meets `expect` moves the branch, by compare-and-swap, to that very commit;
/// then the branch is pushed, its head is read back from the remote and must
/// be the commit, and the output's tail is noted on the commit under
/// `refs/notes/hexa-gate`. The temporary ref is deleted in every case. A
/// failure, a vacuous run or a runner failure commits nothing and leaves the
/// change in the tree.
pub async fn gate_patch_remote(
    root: &Path,
    files: &[String],
    evidence: &str,
    message: &str,
    uncounted: bool,
    template: &str,
    expect: Expect,
) -> Result<RemoteGated, Box<GateFailure>> {
    let paths = crate::direct_exec::gate_checks(root, files).await.map_err(GateFailure::from_checks)?;
    let (head, branch) = head_and_branch(root)?;
    let (remote, merge) = upstream_of(root, &branch)?;

    // The candidate: HEAD's tree plus the named files, in a throwaway index.
    let refuse = |why: String| GateFailure::new("refused", why, "read the git error, fix it, then run the same command again", None, true);
    let index = git_ok(root, &["rev-parse", "--git-path", "hexa-gate-index"], &[]).map_err(refuse)?;
    let index = if Path::new(&index).is_absolute() { PathBuf::from(index) } else { root.join(index) };
    let _ = std::fs::remove_file(&index);
    let idx = index.to_string_lossy().to_string();
    let env = [("GIT_INDEX_FILE", idx.as_str())];
    let built = (|| -> Result<String, String> {
        git_ok(root, &["read-tree", "HEAD"], &env)?;
        let mut add = vec!["add", "-A", "--"];
        add.extend(paths.iter().map(String::as_str));
        git_ok(root, &add, &env)?;
        let tree = git_ok(root, &["write-tree"], &env)?;
        let adr = if expect == Expect::Red { "ADR-2611231400 §4" } else { "ADR-2611221610 §7" };
        let msg = format!(
            "{}\n\nGate: {evidence} ({}; remote: test.remote; hexa do gate, {adr})",
            message.trim(),
            crate::direct_exec::gate_how(uncounted, expect)
        );
        git_ok(root, &["commit-tree", &tree, "-p", &head, "-m", &msg], &[])
    })();
    let _ = std::fs::remove_file(&index);
    let candidate = built.map_err(|e| refuse(format!("the candidate commit could not be built: {e}")))?;
    let with_commit = |mut f: Box<GateFailure>| {
        f.commit = Some(candidate.clone());
        f
    };
    let short: String = candidate.chars().take(12).collect();
    let temp = format!("refs/heads/hexa-gate/{short}");
    git_ok(root, &["push", "-q", &remote, &format!("{candidate}:{temp}")], &[]).map_err(|e| {
        with_commit(GateFailure::new(
            "push-refused",
            format!("the candidate {short} could not be pushed to {remote} ({temp}): {e}; nothing committed, the change is left in the tree"),
            format!("check that {remote} is reachable and accepts `refs/heads/hexa-gate/*`, then run the same command again"),
            None,
            true,
        ))
    })?;
    println!("  candidate {short} pushed to {remote} ({temp}); sending the gate to the remote runner (test.remote)");
    let ran = run_remote_full(root, template, &candidate, vec![evidence.to_string()]).await;
    let _ = git_ok(root, &["push", "-q", &remote, "--delete", &temp], &[]);
    let (mut split, full) = ran.map_err(|e| {
        let wrong = e.starts_with("runner failure");
        with_commit(GateFailure::new(
            if wrong { "remote-runner-wrong-commit" } else { "refused" },
            format!("{e}\nnothing committed; the change is left in the tree"),
            if wrong {
                format!("the runner did not show it ran {short}: check that it is reachable and checks out the commit it is given (test.remote), then run again")
            } else {
                "fix test.remote in .hexa/project.json".to_string()
            },
            None,
            wrong,
        ))
    })?;
    let (passed, output) = split.remove(0);
    let kept = tail(&full, EVIDENCE_LINES);
    let judged = crate::direct_exec::judge_gate(evidence, passed, &output, uncounted, expect, true).map_err(|mut f| {
        f.output = Some(kept.clone());
        with_commit(f)
    })?;
    // The branch moves to the commit that was tested, only if it is still where the candidate was built.
    git_ok(root, &["update-ref", &format!("refs/heads/{branch}"), &candidate, &head], &[]).map_err(|e| {
        let mut f = GateFailure::new(
            "branch-moved",
            format!("the gate passed on {short}, but {branch} moved while it ran ({e}); nothing committed, the change is left in the tree"),
            "run the same command again: it builds a new candidate on the branch as it is now",
            None,
            true,
        );
        f.output = Some(kept.clone());
        with_commit(f)
    })?;
    let mut reset = vec!["reset", "-q", "--"];
    reset.extend(paths.iter().map(String::as_str));
    let _ = git_ok(root, &reset, &[]);
    let _ = git_ok(root, &["notes", "--ref=hexa-gate", "add", "-f", "-m", &kept, &candidate], &[]);
    // Done only when the remote says so (ADR-2611231400 §3).
    let not_there = |code: &'static str, why: String| {
        let mut f = GateFailure::new(
            code,
            why,
            format!("the commit stands on {branch} here: `git push {remote} {branch}`; if {remote} moved on, rebase and gate again"),
            None,
            true,
        );
        f.output = Some(kept.clone());
        f.tests = judged.tests;
        with_commit(f)
    };
    git_ok(root, &["push", "-q", &remote, &format!("refs/heads/{branch}:{merge}")], &[])
        .map_err(|e| not_there("push-refused", format!("the gate passed on {short} and {branch} moved to it here, but {remote} refused the push: {e}")))?;
    let _ = git_ok(root, &["push", "-q", &remote, "refs/notes/hexa-gate"], &[]);
    let read = remote_head(root, &remote, &merge).unwrap_or_default();
    if read != candidate {
        let shown = if read.is_empty() { "nothing".to_string() } else { read.clone() };
        return Err(not_there("remote-head-mismatch", format!("{branch} was pushed, but {remote} reports {merge} at {shown}, not {candidate}")));
    }
    Ok(RemoteGated { commit: candidate, branch, remote, remote_head: read, evidence: kept, judged })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_library_test_runs_by_its_module_path() {
        let k = Path::new("/w/hexa-cli");
        let a = cargo_args(k, k, &k.join("src/commands/adr/mod.rs"), "adr_gates::f").unwrap();
        assert!(a.ends_with(&["--lib".into(), "--".into(), "commands::adr::adr_gates::f".into(), "--exact".into()]), "{a:?}");
        let a = cargo_args(k, k, &k.join("tests/x.rs"), "f").unwrap();
        assert!(a.ends_with(&["--test".into(), "x".into(), "--".into(), "f".into(), "--exact".into()]), "{a:?}");
        assert!(cargo_args(k, k, &k.join("benches/b.rs"), "f").is_none());
    }

    #[test]
    fn a_binary_test_runs_in_its_binary_target() {
        let dir = tempfile::tempdir().unwrap();
        let k = dir.path();
        std::fs::create_dir_all(k.join("src/bin")).unwrap();
        std::fs::write(k.join("src/lib.rs"), "").unwrap();
        std::fs::write(k.join("src/main.rs"), "").unwrap();
        let tail = |a: Vec<String>, n: usize| a[a.len() - n..].to_vec();
        let a = cargo_args(k, k, &k.join("src/main.rs"), "f").unwrap();
        assert_eq!(tail(a, 4), ["--bins", "--", "f", "--exact"], "src/main.rs is the binary, not the library");
        let a = cargo_args(k, k, &k.join("src/bin/x.rs"), "f").unwrap();
        assert_eq!(tail(a, 5), ["--bin", "x", "--", "f", "--exact"], "not the bogus path bin::x::f");
        let a = cargo_args(k, k, &k.join("src/bin/x/util.rs"), "f").unwrap();
        assert_eq!(tail(a, 5), ["--bin", "x", "--", "util::f", "--exact"]);
        let a = cargo_args(k, k, &k.join("src/lib.rs"), "f").unwrap();
        assert_eq!(tail(a, 4), ["--lib", "--", "f", "--exact"]);
        let a = cargo_args(k, k, &k.join("src/cli/mod.rs"), "f").unwrap();
        assert_eq!(tail(a, 5), ["--lib", "--bins", "--", "cli::f", "--exact"], "a module main.rs may own");
        std::fs::remove_file(k.join("src/lib.rs")).unwrap();
        let a = cargo_args(k, k, &k.join("src/cli/mod.rs"), "f").unwrap();
        assert_eq!(tail(a, 4), ["--bins", "--", "cli::f", "--exact"], "no library target to ask for");
    }

    // ── no substituted value can execute (ADR-2611191120 §2, §3) ─────────────

    /// Names a hostile scenario ref or file could carry: each tries to run
    /// `touch pwned` by breaking out of its quoting.
    const HOSTILE: [&str; 8] = [
        "a'; touch pwned; '",
        "$(touch pwned)",
        "`touch pwned`",
        "\"; touch pwned; \"",
        "x\ntouch pwned",
        "x' && touch pwned && echo '",
        "{file} {name} {cmd} {commit}",
        "\\'; touch pwned #",
    ];

    fn sh(dir: &Path, cmd: &str) -> (bool, String) {
        let o = std::process::Command::new("bash").arg("-c").arg(cmd).current_dir(dir).output().unwrap();
        (o.status.success(), String::from_utf8_lossy(&o.stdout).to_string())
    }

    fn assert_inert(dir: &Path, what: &str) {
        assert!(!dir.join("pwned").exists(), "{what:?} executed `touch pwned`");
    }

    #[test]
    fn a_hostile_name_reaches_a_bash_test_as_one_literal_argument() {
        for name in HOSTILE {
            let d = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(d.path().join("tests")).unwrap();
            std::fs::write(d.path().join("tests/echo.sh"), "printf '%s' \"$1\"; [ $# -eq 1 ]\n").unwrap();
            let p = plan_test(d.path(), &format!("tests/echo.sh::{name}"), &TestConfig::default(), "").unwrap();
            let (ok, out) = sh(d.path(), &p.cmd);
            assert!(ok, "one argument for {name:?}: {}", p.cmd);
            assert_eq!(out, name, "the script got the name unchanged: {}", p.cmd);
            assert_inert(d.path(), name);
        }
    }

    #[test]
    fn a_hostile_file_and_name_reach_a_configured_runner_unchanged() {
        let mut cfg = TestConfig::default();
        cfg.runners.insert("py".into(), "printf '%s\\n' {file} {name}".into());
        for name in HOSTILE {
            let d = tempfile::tempdir().unwrap();
            // A file name can carry the attack too.
            let file = "tests/a;touch pwned;$(touch pwned).py";
            std::fs::create_dir_all(d.path().join("tests")).unwrap();
            std::fs::write(d.path().join(file), "").unwrap();
            let p = plan_test(d.path(), &format!("{file}::{name}"), &cfg, "").unwrap();
            let (ok, out) = sh(d.path(), &p.cmd);
            assert!(ok, "{}", p.cmd);
            assert_eq!(out, format!("{file}\n{name}\n"), "file and name are two literal arguments: {}", p.cmd);
            assert_inert(d.path(), name);
        }
    }

    #[test]
    fn a_hostile_name_reaches_vitest_only_as_its_escaped_pattern() {
        for name in HOSTILE {
            let d = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(d.path().join("web/src")).unwrap();
            std::fs::create_dir_all(d.path().join("bin")).unwrap();
            std::fs::write(d.path().join("web/package.json"), "{}").unwrap();
            std::fs::write(d.path().join("web/src/a.test.ts"), "").unwrap();
            // An npx that prints each argument it received on its own line, NUL-separated.
            let npx = d.path().join("bin/npx");
            std::fs::write(&npx, "#!/bin/bash\nprintf '%s\\0' \"$@\"\n").unwrap();
            #[cfg(unix)]
            std::fs::set_permissions(&npx, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
            let path = format!("{}:{}", d.path().join("bin").display(), std::env::var("PATH").unwrap_or_default());
            let p = plan_test(d.path(), &format!("web/src/a.test.ts::{name}"), &TestConfig::default(), &env_prefix(Some(&path), None)).unwrap();
            let (ok, out) = sh(d.path(), &p.cmd);
            assert!(ok, "{}", p.cmd);
            let args: Vec<&str> = out.trim_end_matches('\0').split('\0').collect();
            let pattern = hexa_core::ports::test_runner::vitest_pattern(name);
            assert_eq!(args, ["vitest", "run", "src/a.test.ts", "-t", pattern.as_str(), "--reporter=json"], "{}", p.cmd);
            assert_inert(d.path(), name);
            assert_inert(&d.path().join("web"), name);
        }
    }

    #[test]
    fn a_hostile_command_reaches_the_remote_template_as_one_word() {
        let commit = "0123456789abcdef0123456789abcdef01234567";
        for cmd in HOSTILE {
            let d = tempfile::tempdir().unwrap();
            let full = hexa_core::ports::test_runner::remote_command("printf '%s|%s' {commit} {cmd}", commit, cmd).unwrap();
            let (ok, out) = sh(d.path(), &full);
            assert!(ok, "{full}");
            assert_eq!(out, format!("{commit}|{cmd}"), "{full}");
            assert_inert(d.path(), cmd);
            // And the batch script that goes through it runs the command
            // only inside `bash -c`, with its text intact.
            let script = hexa_core::ports::test_runner::batch_script("t", &[format!("printf '%s' {}", shell_quote(cmd))]);
            let (_, out) = sh(d.path(), &script);
            let got = hexa_core::ports::test_runner::split_batch("t", 1, &out).unwrap();
            assert_eq!(got[0].1.trim_end_matches('\n'), cmd.trim_end_matches('\n'), "{script}");
            assert_inert(d.path(), cmd);
        }
    }

    // ── a pytest run that never ran the test is not red (ADR-2611211324 §8) ──

    fn pytest_plan(dir: &Path, body: &str, name: &str) -> Result<Planned, TestRun> {
        std::fs::write(dir.join("test_x.py"), body).unwrap();
        plan_test(dir, &format!("test_x.py::{name}"), &TestConfig::default(), "")
    }

    #[test]
    fn a_pytest_test_the_file_does_not_define_is_not_found() {
        let d = tempfile::tempdir().unwrap();
        let got = pytest_plan(d.path(), "def test_a():\n    assert True\n", "test_missing");
        assert!(matches!(got, Err(TestRun::NotFound(_))), "{got:?}");
        let got = pytest_plan(d.path(), "class TestK:\n    async def test_m(self):\n        pass\n", "TestK::test_m");
        assert!(got.is_ok(), "a method in a class, by its leaf: {got:?}");
        let got = pytest_plan(d.path(), "import pytest\n@pytest.mark.parametrize('a', [1, 2])\ndef test_p(a):\n    assert a\n", "test_p[1]");
        assert!(got.is_ok(), "one case of a parametrized test, by its id: {got:?}");
        let got = pytest_plan(d.path(), "def test_p(a):\n    assert a\n", "test_p[a::b]");
        assert!(got.is_ok(), "an id holding `::` is still test_p's: {got:?}");
    }

    /// pytest's own words, captured from pytest 9 (exit 4, 5, and a missing
    /// pytest's exit 1): none ran the test, so none is the red a sealed test
    /// must show. A real failure still is.
    #[test]
    fn a_pytest_run_that_ran_nothing_is_never_red() {
        let d = tempfile::tempdir().unwrap();
        let p = pytest_plan(d.path(), "def test_a():\n    assert True\n", "test_a").unwrap();
        let not_found = "ERROR: not found: /tmp/p/test_x.py::test_a\n(no match in any of [<Module test_x.py>])\n\n\nno tests ran in 0.00s\n";
        let none_ran = "\nno tests ran in 0.00s\n";
        let no_pytest = "/usr/bin/python3: No module named pytest\n";
        let bad_conftest = "ImportError while loading conftest '/tmp/p/conftest.py'.\nconftest.py:1: in <module>\n    import not_a_module_xyz\nE   ModuleNotFoundError: No module named 'not_a_module_xyz'\n";
        let usage = "ERROR: usage: python -m pytest [options] [file_or_dir] [file_or_dir] [...]\npython -m pytest: error: unrecognized arguments: --cov\n  inifile: /tmp/p/pyproject.toml\n  rootdir: /tmp/p\n\n";
        for out in [not_found, none_ran, no_pytest, bad_conftest, usage] {
            let got = judge(&p, false, out);
            assert!(!matches!(got, TestRun::Failed(_) | TestRun::Passed), "{out:?} judged {got:?}");
        }
        let failed = "test_x.py:2: AssertionError\n=========================== short test summary info ============================\nFAILED test_x.py::test_a - assert 1 == 2\n1 failed in 0.01s\n";
        assert!(matches!(judge(&p, false, failed), TestRun::Failed(_)), "a real failure is red");
        assert!(matches!(judge(&p, true, ".   [100%]\n1 passed in 0.00s\n"), TestRun::Passed));
    }

    /// A repository with branch `branch` whose upstream is `remote`'s `merge`.
    fn tracking(branch: &str, merge: &str) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let git = |a: &[&str]| assert!(std::process::Command::new("git").args(a).current_dir(d.path()).status().unwrap().success(), "{a:?}");
        git(&["init", "-q", "-b", branch]);
        git(&["config", &format!("branch.{branch}.remote"), "origin"]);
        git(&["config", &format!("branch.{branch}.merge"), merge]);
        d
    }

    #[test]
    fn a_branch_tracking_its_own_name_has_an_upstream() {
        let d = tracking("feat", "refs/heads/feat");
        assert_eq!(upstream_of(d.path(), "feat").unwrap(), ("origin".to_string(), "refs/heads/feat".to_string()));
    }

    /// gaberger/hexa#75: a worktree made with `git worktree add -b t3/x <dir>
    /// origin/main` tracks origin/main, and the remote gate pushed its commit
    /// to main. Pushing to the upstream is pushing to another branch: refused
    /// by name, as a branch with no upstream of its own is (ADR-2611221610 §7).
    #[test]
    fn a_branch_tracking_another_branch_is_refused_by_name() {
        let d = tracking("t3/dead-exports", "refs/heads/main");
        let f = upstream_of(d.path(), "t3/dead-exports").unwrap_err();
        assert_eq!(f.code, "no-upstream");
        assert!(f.why.contains("t3/dead-exports") && f.why.contains("main"), "{}", f.why);
        assert!(f.hint.contains("git push -u origin t3/dead-exports"), "{}", f.hint);
        assert!(!f.retryable);
    }
}
