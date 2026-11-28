//! The task ledger on disk (ADR-2611051850): `.hexa/tasks.json` in the main
//! checkout, shared by every worktree of the repository. The rules (leases,
//! overlap, what is never leasable) are the domain's, in
//! `hexa_core::ports::ledger`; this file only finds, locks, reads and writes.
//!
//! The ledger is runtime state and git-ignored. A file that exists and cannot
//! be read is an error (`ledger_unreadable`), never an empty ledger: an
//! empty answer from a ledger nobody could read would let two agents lease
//! the same path.

use hexa_core::ports::ledger::{Ledger, LedgerError};
use std::path::{Path, PathBuf};

/// The binding a session may carry in its environment.
pub const ENV_TASK: &str = "HEXA_TASK";

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub fn new_task_id() -> String {
    format!("task-{}", &uuid::Uuid::new_v4().simple().to_string()[..8])
}

fn git_out(dir: &Path, args: &[&str]) -> Option<String> {
    let o = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    o.status
        .success()
        .then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// The main checkout's root, found from any worktree through git's common
/// directory. Outside a repository, `dir` itself.
pub fn main_root(dir: &Path) -> PathBuf {
    let canon = |p: PathBuf| std::fs::canonicalize(&p).unwrap_or(p);
    if let Some(common) = git_out(dir, &["rev-parse", "--git-common-dir"]) {
        let p = PathBuf::from(&common);
        let p = if p.is_absolute() { p } else { dir.join(p) };
        let p = canon(p);
        if p.file_name().is_some_and(|n| n == ".git") {
            if let Some(parent) = p.parent() {
                return parent.to_path_buf();
            }
        }
    }
    if let Some(top) = git_out(dir, &["rev-parse", "--show-toplevel"]) {
        return canon(PathBuf::from(top));
    }
    canon(dir.to_path_buf())
}

pub fn ledger_path(dir: &Path) -> PathBuf {
    main_root(dir).join(".hexa").join("tasks.json")
}

fn unreadable(path: &Path, why: String) -> LedgerError {
    LedgerError::new(
        "ledger_unreadable",
        format!("cannot read the task ledger {}: {why}", path.display()),
        "Fix the file's permissions or content, or move it aside; hexa will not guess an empty ledger.".into(),
        None,
        true,
    )
}

/// The ledger of the repository `dir` belongs to. Absent is a true empty
/// ledger; present and unreadable is an error.
pub fn read(dir: &Path) -> Result<Ledger, LedgerError> {
    read_at(&ledger_path(dir))
}

pub fn read_at(path: &Path) -> Result<Ledger, LedgerError> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| unreadable(path, e.to_string())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Ledger::default()),
        Err(e) => Err(unreadable(path, e.to_string())),
    }
}

/// Read-modify-write under an exclusive advisory lock, written atomically
/// (temp file then rename). The closure's error leaves the file untouched.
pub fn update<T>(
    dir: &Path,
    f: impl FnOnce(&mut Ledger) -> Result<T, LedgerError>,
) -> Result<T, LedgerError> {
    let path = ledger_path(dir);
    let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
    std::fs::create_dir_all(&parent).map_err(|e| unreadable(&path, e.to_string()))?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(parent.join("tasks.lock"))
        .map_err(|e| unreadable(&path, format!("lock: {e}")))?;
    {
        use std::os::fd::AsRawFd;
        // SAFETY: flock on a descriptor this function owns for its whole body.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(unreadable(
                &path,
                format!("lock: {}", std::io::Error::last_os_error()),
            ));
        }
    }
    let mut ledger = read_at(&path)?;
    let out = f(&mut ledger)?;
    let text =
        serde_json::to_string_pretty(&ledger).map_err(|e| unreadable(&path, e.to_string()))?;
    let tmp = parent.join(format!(".tasks.json.{}.tmp", std::process::id()));
    std::fs::write(&tmp, text + "\n")
        .map_err(|e| unreadable(&path, format!("{}: {e}", tmp.display())))?;
    std::fs::rename(&tmp, &path).map_err(|e| unreadable(&path, format!("rename: {e}")))?;
    drop(lock);
    Ok(out)
}

/// Every path a spec in `dir`'s `docs/specs` seals, as repo-relative paths.
pub fn sealed(dir: &Path) -> Vec<String> {
    let mut v = hexa_exec::direct_exec::sealed_paths(dir);
    let root = main_root(dir);
    if root != dir {
        v.extend(hexa_exec::direct_exec::sealed_paths(&root));
    }
    v.sort();
    v.dedup();
    v
}

/// `path` as a repo-relative, `/`-separated string under `root`, resolving
/// symlinks in the part that exists (macOS `/var` is `/private/var`).
/// `None` when `path` is not under `root`.
pub fn relative_to(root: &Path, path: &Path) -> Option<String> {
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let abs = canonical_lenient(path);
    let rel = abs.strip_prefix(&root).ok()?;
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Canonicalise the deepest ancestor that exists and re-attach the rest, so a
/// file that is about to be created still resolves.
pub fn canonical_lenient(path: &Path) -> PathBuf {
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    let mut cur = path.to_path_buf();
    loop {
        if let Ok(c) = std::fs::canonicalize(&cur) {
            let mut out = c;
            for t in tail.iter().rev() {
                out.push(t);
            }
            return out;
        }
        match (
            cur.file_name().map(|n| n.to_os_string()),
            cur.parent().map(Path::to_path_buf),
        ) {
            (Some(n), Some(p)) => {
                tail.push(n);
                cur = p;
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// Keep the ledger out of version control: `.hexa/.gitignore` names it and its
/// lock. Idempotent; returns whether it wrote anything. `hexa init` and the
/// first `register` call it, so a project that never re-ran init is covered.
pub fn ensure_ignored(root: &Path) -> std::io::Result<bool> {
    let dir = root.join(".hexa");
    let file = dir.join(".gitignore");
    let have = std::fs::read_to_string(&file).unwrap_or_default();
    // The Stop hook's per-session counter is session state too
    // (ADR-2611221936 §1).
    let want = ["tasks.json", "tasks.lock", ".tasks.json.*.tmp", "stop-blocks.json"];
    let missing: Vec<&str> = want
        .iter()
        .copied()
        .filter(|w| !have.lines().any(|l| l.trim() == *w))
        .collect();
    if missing.is_empty() {
        return Ok(false);
    }
    std::fs::create_dir_all(&dir)?;
    let mut text = have;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    for m in missing {
        text.push_str(m);
        text.push('\n');
    }
    std::fs::write(&file, text)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hexa_core::ports::ledger::{Task, TaskStatus};

    fn task(id: &str) -> Task {
        Task {
            id: id.into(),
            name: id.into(),
            owner: None,
            worktree: None,
            gate: None,
            leases: vec!["src/a".into()],
            frozen: Vec::new(),
            status: TaskStatus::Claimed,
            run: None,
            outcome: None,
            commit: None,
            note: None,
            created: now(),
            updated: now(),
        }
    }

    fn repo() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let ok = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(d.path())
            .status()
            .unwrap();
        assert!(ok.success());
        d
    }

    #[test]
    fn an_absent_ledger_is_empty_and_an_unparseable_one_is_an_error() {
        let d = repo();
        assert!(read(d.path()).unwrap().tasks.is_empty());
        std::fs::create_dir_all(d.path().join(".hexa")).unwrap();
        std::fs::write(d.path().join(".hexa/tasks.json"), "{not json").unwrap();
        let e = read(d.path()).unwrap_err();
        assert_eq!(e.code, "ledger_unreadable");
        assert!(e.retryable && !e.hint.is_empty());
    }

    #[test]
    fn update_persists_and_a_failing_closure_leaves_the_file_alone() {
        let d = repo();
        update(d.path(), |l| {
            l.tasks.push(task("t1"));
            Ok(())
        })
        .unwrap();
        let err: Result<(), _> = update(d.path(), |l| {
            l.tasks.push(task("t2"));
            Err(LedgerError::new("x", "no".into(), "no".into(), None, false))
        });
        assert!(err.is_err());
        let l = read(d.path()).unwrap();
        assert_eq!(
            l.tasks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            vec!["t1"]
        );
    }

    #[test]
    fn a_linked_worktree_sees_the_main_checkouts_ledger() {
        let d = repo();
        let g = |args: &[&str]| {
            let o = std::process::Command::new("git")
                .args(["-c", "user.email=t@t", "-c", "user.name=t"])
                .args(args)
                .current_dir(d.path())
                .output()
                .unwrap();
            assert!(
                o.status.success(),
                "{args:?}: {}",
                String::from_utf8_lossy(&o.stderr)
            );
        };
        g(&["commit", "-q", "--allow-empty", "-m", "i"]);
        let wt = d.path().join("wt");
        g(&["worktree", "add", "-q", wt.to_str().unwrap(), "-b", "other"]);
        update(&wt, |l| {
            l.tasks.push(task("from-worktree"));
            Ok(())
        })
        .unwrap();
        let from_main = read(d.path()).unwrap();
        assert_eq!(from_main.tasks.len(), 1, "one ledger for both checkouts");
        assert_eq!(ledger_path(&wt), ledger_path(d.path()));
        assert!(
            !wt.join(".hexa/tasks.json").exists(),
            "none in the worktree"
        );
    }

    #[test]
    fn relative_to_resolves_symlinks_and_a_file_that_does_not_exist_yet() {
        let d = repo();
        std::fs::create_dir_all(d.path().join("src")).unwrap();
        let rel = relative_to(d.path(), &d.path().join("src/new/file.rs")).unwrap();
        assert_eq!(rel, "src/new/file.rs");
        assert!(relative_to(d.path(), Path::new("/etc/hosts")).is_none());
    }

    #[test]
    fn ensure_ignored_is_idempotent() {
        let d = tempfile::tempdir().unwrap();
        assert!(ensure_ignored(d.path()).unwrap());
        assert!(!ensure_ignored(d.path()).unwrap());
        let t = std::fs::read_to_string(d.path().join(".hexa/.gitignore")).unwrap();
        assert!(t.contains("tasks.json") && t.contains("tasks.lock"));
    }
}
