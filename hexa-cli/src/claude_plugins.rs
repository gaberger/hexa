//! `ClaudePlugins` over the Claude CLI (ADR-2610301129 §4).
//!
//! Every call is a subprocess with stdin closed, both pipes drained on their
//! own threads, and a deadline. On expiry the child's whole process group is
//! killed, so a node or git grandchild cannot write on after hexa gives up.
//! Nothing here panics: setup must carry on whatever the CLI does.

use hexa_core::ports::claude_mods::ClaudePlugins;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// The marketplace hexa registers its mods under.
const MARKETPLACE: &str = "hexa";

/// How long a CLI call may take.
#[derive(Debug, Clone, Copy)]
pub struct Deadlines {
    /// `--version` and the list commands.
    pub read: Duration,
    /// `marketplace add` and `install`, which may fetch.
    pub write: Duration,
}

impl Default for Deadlines {
    fn default() -> Self {
        Deadlines {
            read: Duration::from_secs(15),
            write: Duration::from_secs(300),
        }
    }
}

/// The Claude CLI at `bin`.
pub struct ClaudeCli {
    bin: PathBuf,
    deadlines: Deadlines,
    /// Why `add_marketplace` refused: a `hexa` marketplace the user owns
    /// points elsewhere, so installing from it would install someone else's
    /// plugin under a ✓.
    refused: Mutex<Option<String>>,
}

impl ClaudeCli {
    pub fn new(bin: PathBuf, deadlines: Deadlines) -> Self {
        ClaudeCli {
            bin,
            deadlines,
            refused: Mutex::new(None),
        }
    }

    fn run(&self, args: &[&std::ffi::OsStr], deadline: Duration) -> Result<String, String> {
        // "Text file busy": another process still holds the executable open
        // for writing, which passes in moments (a fork that inherited the
        // write handle, an installer finishing). Retry briefly before failing.
        let spawn = || {
            Command::new(&self.bin)
                .args(args)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .process_group(0)
                .spawn()
        };
        let mut attempt = spawn();
        for wait_ms in [10, 50, 200, 500] {
            match &attempt {
                Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                    std::thread::sleep(Duration::from_millis(wait_ms));
                    attempt = spawn();
                }
                _ => break,
            }
        }
        let mut child = attempt.map_err(|e| format!("cannot run {}: {e}", self.bin.display()))?;
        let stdout = drain(child.stdout.take());
        let stderr = drain(child.stderr.take());
        let status = wait_or_kill(&mut child, deadline);
        let stdout = joined(stdout);
        let stderr = joined(stderr);
        match status {
            None => Err(format!(
                "{} timed out after {}s",
                self.bin.display(),
                deadline.as_secs()
            )),
            Some(s) if s.success() => Ok(stdout),
            Some(s) => {
                let code = s
                    .code()
                    .map_or_else(|| "a signal".to_string(), |c| c.to_string());
                Err(first_nonempty(&[&stderr, &stdout]).unwrap_or_else(|| format!("exit {code}")))
            }
        }
    }

    /// Where the `hexa` marketplace is registered, if anywhere.
    fn registered_at(&self) -> Option<PathBuf> {
        let out = self
            .run(
                &os(&["plugin", "marketplace", "list", "--json"]),
                self.deadlines.read,
            )
            .ok()?;
        let list: serde_json::Value = serde_json::from_str(&out).ok()?;
        list.as_array()?.iter().find_map(|m| {
            if m.get("name")?.as_str()? != MARKETPLACE {
                return None;
            }
            ["path", "installLocation", "source"]
                .iter()
                .find_map(|k| match m.get(*k)? {
                    serde_json::Value::String(s) => Some(PathBuf::from(s)),
                    serde_json::Value::Object(o) => o.get("path")?.as_str().map(PathBuf::from),
                    _ => None,
                })
        })
    }
}

impl ClaudePlugins for ClaudeCli {
    fn available(&self) -> bool {
        self.run(&os(&["--version"]), self.deadlines.read).is_ok()
    }

    fn installed(&self) -> Result<Vec<String>, String> {
        let out = self.run(&os(&["plugin", "list", "--json"]), self.deadlines.read)?;
        user_scope_ids(&out)
    }

    fn add_marketplace(&self, folder: &Path) -> Result<(), String> {
        if let Some(at) = self.registered_at() {
            if same_folder(&at, folder) {
                return Ok(());
            }
            let reason = format!(
                "marketplace {MARKETPLACE} is registered at {}; run `claude plugin marketplace remove {MARKETPLACE}` and re-run setup",
                at.display()
            );
            *self.refused.lock().unwrap_or_else(|e| e.into_inner()) = Some(reason.clone());
            return Err(reason);
        }
        // Passed as given, not canonicalized: the user sees the path they asked for.
        let mut args = os(&["plugin", "marketplace", "add"]);
        args.push(folder.as_os_str());
        self.run(&args, self.deadlines.write).map(drop)
    }

    fn install(&self, id: &str) -> Result<(), String> {
        if let Some(reason) = self
            .refused
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            return Err(format!("not installed: {reason}"));
        }
        self.run(
            &os(&["plugin", "install", id, "--scope", "user"]),
            self.deadlines.write,
        )
        .map(drop)
    }
}

/// The ids in `claude plugin list --json` installed at user scope, the scope
/// hexa installs to. A disabled one still counts: disabling is the user's call.
fn user_scope_ids(json: &str) -> Result<Vec<String>, String> {
    let list: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("plugin list is not JSON: {e}"))?;
    let entries = list.as_array().ok_or("plugin list is not a JSON array")?;
    let mut ids = Vec::new();
    for entry in entries {
        let id = entry
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or("plugin list entry has no id")?;
        if entry.get("scope").and_then(|v| v.as_str()) == Some("user") {
            ids.push(id.to_string());
        }
    }
    Ok(ids)
}

fn same_folder(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a.components().eq(b.components()),
    }
}

fn os<'a>(args: &[&'a str]) -> Vec<&'a std::ffi::OsStr> {
    args.iter().map(|a| std::ffi::OsStr::new(*a)).collect()
}

fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> Option<JoinHandle<Vec<u8>>> {
    pipe.map(|mut p| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = p.read_to_end(&mut buf);
            buf
        })
    })
}

fn joined(handle: Option<JoinHandle<Vec<u8>>>) -> String {
    let bytes = handle.and_then(|h| h.join().ok()).unwrap_or_default();
    String::from_utf8_lossy(&bytes).to_string()
}

/// Wait for `child` until `deadline`; past it, TERM then KILL its group.
/// `None` means it was killed.
fn wait_or_kill(child: &mut Child, deadline: Duration) -> Option<ExitStatus> {
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if start.elapsed() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => break,
        }
    }
    let group = -(child.id() as libc::pid_t);
    // SAFETY: kill(2) on a process group we created; no memory is touched.
    unsafe { libc::kill(group, libc::SIGTERM) };
    let grace = Instant::now();
    while grace.elapsed() < Duration::from_secs(2) {
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // The leader may be gone while grandchildren hold the pipes open.
    // SAFETY: as above.
    unsafe { libc::kill(group, libc::SIGKILL) };
    let _ = child.wait();
    None
}

/// The first non-empty text, trimmed, on one line.
fn first_nonempty(texts: &[&str]) -> Option<String> {
    texts.iter().map(|t| one_line(t)).find(|t| !t.is_empty())
}

fn one_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_user_scope_counts_and_disabled_still_counts() {
        let json = r#"[{"id":"question-alert@hexa","scope":"project"},
                       {"id":"a@hexa","scope":"user","enabled":false}]"#;
        assert_eq!(user_scope_ids(json), Ok(vec!["a@hexa".to_string()]));
    }

    #[test]
    fn a_list_of_another_shape_is_an_error() {
        assert!(user_scope_ids("{}").is_err());
        assert!(user_scope_ids("[\"a@hexa\"]").is_err());
        assert!(user_scope_ids("not json").is_err());
    }

    #[test]
    fn multi_line_errors_print_on_one_line() {
        assert_eq!(one_line("  first\n\n second \n"), "first; second");
        assert_eq!(first_nonempty(&["\n", "out"]), Some("out".to_string()));
    }
}
