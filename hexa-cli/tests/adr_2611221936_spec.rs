#![allow(unused, clippy::all)]
//! Integration tests for ADR-2611221936: the Stop hook's counter is session
//! state, kept out of git.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use hexa_cli::commands::task_ledger;
use hexa_core::domain::project_floor;

const LEDGER_LINES: [&str; 3] = ["tasks.json", "tasks.lock", ".tasks.json.*.tmp"];
const COUNTER_LINE: &str = "stop-blocks.json";
const REQUIRED: [&str; 4] = [
    "tasks.json",
    "tasks.lock",
    ".tasks.json.*.tmp",
    "stop-blocks.json",
];

fn hexa_bin() -> &'static str {
    env!("CARGO_BIN_EXE_hexa")
}

fn hexa_cmd(dir: &Path, home: &Path) -> Command {
    let mut cmd = Command::new(hexa_bin());
    cmd.current_dir(dir)
        .env("HOME", home)
        .env("CLAUDE_PROJECT_DIR", dir)
        .env_remove(task_ledger::ENV_TASK)
        .env("NO_COLOR", "1");
    cmd
}

fn run_init(dir: &Path, home: &Path) {
    let out = hexa_cmd(dir, home)
        .arg("init")
        .stdin(Stdio::null())
        .output()
        .expect("run hexa init");
    assert!(
        out.status.success(),
        "hexa init failed: status {:?}\nstdout:\n{}\nstderr:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn git(dir: &Path, home: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdin(Stdio::null())
        .output()
        .expect("run git")
}

fn hexa_gitignore(root: &Path) -> PathBuf {
    root.join(".hexa").join(".gitignore")
}

fn read_hexa_gitignore(root: &Path) -> String {
    let p = hexa_gitignore(root);
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

fn trimmed_lines(text: &str) -> Vec<String> {
    text.lines().map(|l| l.trim().to_string()).collect()
}

fn count_line(text: &str, line: &str) -> usize {
    text.lines().filter(|l| l.trim() == line).count()
}

fn write_hexa_gitignore(root: &Path, content: &str) {
    fs::create_dir_all(root.join(".hexa")).unwrap();
    fs::write(hexa_gitignore(root), content).unwrap();
}

/// S01: hexa init in a fresh project ignores the Stop hook's counter
#[test]
fn s01_init_in_fresh_project_ignores_stop_counter() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    assert!(!dir.path().join(".hexa").exists());

    run_init(dir.path(), home.path());

    let text = read_hexa_gitignore(dir.path());
    let lines = trimmed_lines(&text);
    assert!(
        lines.iter().any(|l| l == COUNTER_LINE),
        ".hexa/.gitignore lacks `{COUNTER_LINE}`:\n{text}"
    );
    for l in LEDGER_LINES {
        assert!(
            lines.iter().any(|x| x == l),
            ".hexa/.gitignore lacks `{l}`:\n{text}"
        );
    }
}

/// S02: A task ledger write in an existing project adds the missing line
#[test]
fn s02_ledger_write_in_existing_project_adds_missing_line() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write_hexa_gitignore(root, "tasks.json\ntasks.lock\n.tasks.json.*.tmp\n");

    // The task ledger keeps its files ignored through ensure_ignored when it writes.
    let wrote = task_ledger::ensure_ignored(root).expect("ensure_ignored");
    assert!(
        wrote,
        "ensure_ignored should report it wrote the missing line"
    );

    let text = read_hexa_gitignore(root);
    assert_eq!(
        count_line(&text, COUNTER_LINE),
        1,
        "counter line missing or duplicated:\n{text}"
    );
    for l in LEDGER_LINES {
        assert_eq!(
            count_line(&text, l),
            1,
            "`{l}` should still be present once:\n{text}"
        );
    }
}

/// S03: After init the counter file does not show in git status
#[test]
fn s03_counter_file_not_in_git_status_after_init() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let root = dir.path();

    let out = git(root, home.path(), &["init", "-q"]);
    assert!(
        out.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    run_init(root, home.path());

    fs::create_dir_all(root.join(".hexa")).unwrap();
    fs::write(
        root.join(".hexa").join("stop-blocks.json"),
        "{\"some-session\": 1}\n",
    )
    .unwrap();

    let status = git(
        root,
        home.path(),
        &["status", "--porcelain", "--untracked-files=all"],
    );
    assert!(
        status.status.success(),
        "git status failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    let status_text = String::from_utf8_lossy(&status.stdout);
    assert!(
        !status_text.lines().any(|l| l.contains("stop-blocks.json")),
        "git status shows the counter file:\n{status_text}"
    );

    let check = git(
        root,
        home.path(),
        &["check-ignore", ".hexa/stop-blocks.json"],
    );
    assert!(
        check.status.success(),
        "git check-ignore does not report .hexa/stop-blocks.json as ignored (status {:?}): {}",
        check.status,
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(String::from_utf8_lossy(&check.stdout).contains("stop-blocks.json"));
}

/// S06: For any starting content, ensure_ignored leaves every required line present exactly once and is idempotent
#[test]
fn s06_ensure_ignored_property_over_starting_contents() {
    // Build a set of starting contents: absent, empty, and many subsets and
    // orderings of the required lines mixed with arbitrary other lines.
    let others = ["notes.md", "# my comment", "scratch/", "*.log"];
    let mut cases: Vec<Option<String>> = vec![None, Some(String::new()), Some("\n".into())];

    for mask in 0u32..16 {
        let mut subset: Vec<&str> = REQUIRED
            .iter()
            .enumerate()
            .filter(|(i, _)| mask & (1 << i) != 0)
            .map(|(_, l)| *l)
            .collect();
        for rot in 0..3usize {
            let mut lines: Vec<&str> = subset.clone();
            if !lines.is_empty() {
                let r = rot % lines.len();
                lines.rotate_left(r);
            }
            if rot == 1 {
                lines.reverse();
            }
            // Interleave some other lines.
            let mut mixed: Vec<&str> = Vec::new();
            for (i, l) in lines.iter().enumerate() {
                if (i + rot + mask as usize) % 2 == 0 {
                    mixed.push(others[(i + rot) % others.len()]);
                }
                mixed.push(l);
            }
            if rot == 2 {
                mixed.push(others[mask as usize % others.len()]);
            }
            let joined = mixed.join("\n");
            // With and without a trailing newline.
            cases.push(Some(joined.clone()));
            cases.push(Some(format!("{joined}\n")));
        }
        subset.clear();
    }
    cases.push(Some("notes.md".into()));
    cases.push(Some("notes.md\n*.log\n".into()));

    for (idx, case) in cases.iter().enumerate() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        match case {
            None => {}
            Some(c) => write_hexa_gitignore(root, c),
        }

        task_ledger::ensure_ignored(root)
            .unwrap_or_else(|e| panic!("case {idx} ({case:?}): ensure_ignored failed: {e}"));
        let first = fs::read_to_string(hexa_gitignore(root))
            .unwrap_or_else(|e| panic!("case {idx} ({case:?}): no .hexa/.gitignore: {e}"));

        for req in REQUIRED {
            assert_eq!(
                count_line(&first, req),
                1,
                "case {idx} ({case:?}): `{req}` should occur exactly once:\n{first}"
            );
        }

        if let Some(c) = case {
            for l in c.lines().map(str::trim).filter(|l| !l.is_empty()) {
                if REQUIRED.contains(&l) {
                    continue;
                }
                assert!(
                    count_line(&first, l) >= count_line(c, l),
                    "case {idx} ({case:?}): other line `{l}` was not kept:\n{first}"
                );
            }
        }

        let wrote_again = task_ledger::ensure_ignored(root)
            .unwrap_or_else(|e| panic!("case {idx}: second ensure_ignored failed: {e}"));
        let second = fs::read_to_string(hexa_gitignore(root)).unwrap();
        assert!(
            !wrote_again,
            "case {idx} ({case:?}): second run reported a write"
        );
        assert_eq!(
            first, second,
            "case {idx} ({case:?}): second run changed the file"
        );
    }
}

/// S07: The task ledger's existing ignore lines are still written
#[test]
fn s07_init_still_writes_ledger_lines() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();

    run_init(dir.path(), home.path());

    let text = read_hexa_gitignore(dir.path());
    for l in LEDGER_LINES {
        assert_eq!(
            count_line(&text, l),
            1,
            "`{l}` should be in .hexa/.gitignore once:\n{text}"
        );
    }
    // This decision's line is written with them.
    assert_eq!(
        count_line(&text, COUNTER_LINE),
        1,
        "counter line missing:\n{text}"
    );
}

/// S08: The root .gitignore is not changed by this decision
#[test]
fn s08_root_gitignore_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();

    run_init(dir.path(), home.path());

    let root_ignore =
        fs::read_to_string(dir.path().join(".gitignore")).expect("root .gitignore written by init");
    assert!(
        !root_ignore
            .lines()
            .any(|l| l.trim() == "stop-blocks.json" || l.trim() == ".hexa/stop-blocks.json"),
        "root .gitignore names the counter:\n{root_ignore}"
    );

    let candidates = ["", "rust", "ts", "go", "python", "unknown"];
    let matches = candidates.iter().any(|lang| {
        project_floor::merged_gitignore(None, lang).as_deref() == Some(root_ignore.as_str())
    });
    assert!(
        matches,
        "root .gitignore does not equal what merged_gitignore produces for the floor's lines:\n{root_ignore}"
    );

    // The hexa ledger's .gitignore does carry the counter (this decision's change).
    let hexa_text = read_hexa_gitignore(dir.path());
    assert_eq!(
        count_line(&hexa_text, COUNTER_LINE),
        1,
        ".hexa/.gitignore lacks counter:\n{hexa_text}"
    );
}

fn json_has_count_for(v: &serde_json::Value, sid: &str, count: u64) -> bool {
    use serde_json::Value;
    fn contains_number(v: &Value, n: u64) -> bool {
        match v {
            Value::Number(x) => x.as_u64() == Some(n),
            Value::Object(m) => m.values().any(|x| contains_number(x, n)),
            Value::Array(a) => a.iter().any(|x| contains_number(x, n)),
            _ => false,
        }
    }
    match v {
        Value::Object(m) => {
            if let Some(inner) = m.get(sid) {
                if contains_number(inner, count) {
                    return true;
                }
            }
            let names_sid = m.values().any(|x| x.as_str() == Some(sid));
            if names_sid && m.values().any(|x| x.as_u64() == Some(count)) {
                return true;
            }
            m.values().any(|x| json_has_count_for(x, sid, count))
        }
        Value::Array(a) => a.iter().any(|x| json_has_count_for(x, sid, count)),
        _ => false,
    }
}

fn run_hook(dir: &Path, home: &Path, event: &str, input: &serde_json::Value) -> Output {
    use std::io::Write;
    let mut cmd = hexa_cmd(dir, home);
    // Claude Code runs a hook with its session's id in the environment, the
    // one the payload names; without it hexa reads the calling process's.
    if let Some(sid) = input["session_id"].as_str() {
        cmd.env("HEXA_SESSION_ID", sid);
    }
    let mut child = cmd
        .args(["hook", event])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn hexa hook");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    child.wait_with_output().expect("wait hexa hook")
}

/// S09: What the Stop hook records is unchanged
#[test]
fn s09_stop_hook_still_records_counter() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let root = dir.path();

    let out = git(root, home.path(), &["init", "-q"]);
    assert!(out.status.success(), "git init failed");
    run_init(root, home.path());

    let sid = "adr-2611221936-session";
    // The session edits a tracked file and does not commit it: open work.
    // (The hook reads tracked files only, `--untracked-files=no`, so the file
    // is committed first, then changed.)
    let file = root.join("work.txt");
    fs::write(&file, "start\n").unwrap();
    for args in [&["add", "work.txt"][..], &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "start"][..]] {
        let out = git(root, home.path(), args);
        assert!(out.status.success(), "git {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    fs::write(&file, "unfinished\n").unwrap();
    let edit = serde_json::json!({
        "session_id": sid,
        "hook_event_name": "PostToolUse",
        "tool_name": "Write",
        "tool_input": { "file_path": file.to_string_lossy(), "content": "unfinished\n" },
        "tool_response": { "filePath": file.to_string_lossy(), "success": true },
        "cwd": root.to_string_lossy(),
    });
    let _ = run_hook(root, home.path(), "post-edit", &edit);

    for active in [false, true] {
        let stop = serde_json::json!({
            "session_id": sid,
            "hook_event_name": "Stop",
            "stop_hook_active": active,
            "cwd": root.to_string_lossy(),
        });
        let _ = run_hook(root, home.path(), "stop", &stop);
    }

    let counter = root.join(".hexa").join("stop-blocks.json");
    let text = fs::read_to_string(&counter)
        .unwrap_or_else(|e| panic!("Stop hook did not write {}: {e}", counter.display()));
    let v: serde_json::Value = serde_json::from_str(&text).expect("stop-blocks.json is JSON");
    assert!(
        json_has_count_for(&v, sid, 2),
        "stop-blocks.json does not record 2 consecutive blocks for {sid}:\n{text}"
    );
}

/// The `.gitignore` lines of the project floor before this ADR.
fn pre_adr_floor_lines(lang: &str) -> Vec<&'static str> {
    let mut lines: Vec<&'static str> = match lang {
        "rust" => vec!["target/"],
        "ts" => vec!["node_modules/", "dist/"],
        _ => Vec::new(),
    };
    lines.extend([
        ".hexa/loop.json",
        ".hexa/loop.lock",
        ".env*",
        "!.env.example",
    ]);
    lines
}

fn reference_merge(existing: Option<&str>, lang: &str) -> Option<String> {
    let text = existing.unwrap_or("");
    let missing: Vec<&str> = pre_adr_floor_lines(lang)
        .into_iter()
        .filter(|l| !text.lines().any(|x| x.trim() == *l))
        .collect();
    if missing.is_empty() {
        return None;
    }
    let mut out = text.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    for l in missing {
        out.push_str(l);
        out.push('\n');
    }
    Some(out)
}
