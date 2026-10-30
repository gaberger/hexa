//! The Claude CLI adapter behind `ClaudePlugins` (ADR-2610301129 §4), driven
//! against scripted fake `claude` binaries: deadlines, group kills, large
//! output, scope, conflicts, and one-line errors.

use hexa_cli::claude_plugins::Deadlines;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A fake `claude` that logs its arguments and runs `body` (a POSIX sh case
/// over "$*").
fn fake(dir: &Path, body: &str) -> (PathBuf, PathBuf) {
    let log = dir.join("calls.log");
    let bin = dir.join("claude");
    let script = format!(
        "#!/bin/sh\necho \"$*\" >> \"{}\"\ncase \"$*\" in\n{body}\nesac\nexit 0\n",
        log.display()
    );
    std::fs::write(&bin, script).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    (bin, log)
}

/// For the timeout tests: short, so a hang is over fast.
fn quick() -> Deadlines {
    Deadlines {
        read: Duration::from_secs(1),
        write: Duration::from_secs(1),
    }
}

/// For everything else: long enough that a loaded machine is not a timeout.
fn roomy() -> Deadlines {
    Deadlines {
        read: Duration::from_secs(10),
        write: Duration::from_secs(10),
    }
}

fn log_of(log: &Path) -> String {
    std::fs::read_to_string(log).unwrap_or_default()
}

#[test]
fn a_hung_cli_is_not_available_within_the_deadline() {
    let dir = tempfile::tempdir().unwrap();
    let (bin, _) = fake(dir.path(), "  --version) sleep 600;;");
    let started = Instant::now();
    assert!(!hexa_cli::claude_plugins_at(&bin, quick()).available());
    assert!(
        started.elapsed() < Duration::from_secs(1 + 3),
        "took {:?}",
        started.elapsed()
    );
}

#[test]
fn a_timeout_kills_the_whole_process_group() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("grandchild.pid");
    let body = format!(
        "  --version) sleep 600 & echo $! > \"{}\"; sleep 600;;",
        pid_file.display()
    );
    let (bin, _) = fake(dir.path(), &body);
    assert!(!hexa_cli::claude_plugins_at(&bin, quick()).available());
    let pid = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .to_string();
    let alive = std::process::Command::new("kill")
        .args(["-0", &pid])
        .status()
        .unwrap()
        .success();
    assert!(!alive, "grandchild {pid} outlived the timeout");
}

#[test]
fn a_large_listing_does_not_deadlock() {
    let dir = tempfile::tempdir().unwrap();
    let entries: Vec<String> = (0..6000)
        .map(|i| format!("{{\"id\":\"p{i:05}@somewhere-long-enough\",\"scope\":\"user\"}}"))
        .collect();
    let listing = dir.path().join("list.json");
    std::fs::write(&listing, format!("[{}]", entries.join(","))).unwrap();
    assert!(std::fs::metadata(&listing).unwrap().len() > 300_000);
    let (bin, _) = fake(
        dir.path(),
        &format!("  \"plugin list --json\") cat \"{}\";;", listing.display()),
    );
    let ids = hexa_cli::claude_plugins_at(&bin, roomy())
        .installed()
        .unwrap();
    assert_eq!(ids.len(), 6000);
}

#[test]
fn only_a_user_scope_install_counts() {
    let dir = tempfile::tempdir().unwrap();
    let (bin, _) = fake(
        dir.path(),
        r#"  "plugin list --json") echo '[{"id":"question-alert@hexa","scope":"project"}]';;"#,
    );
    assert_eq!(
        hexa_cli::claude_plugins_at(&bin, roomy()).installed(),
        Ok(vec![])
    );
    let (bin, _) = fake(
        dir.path(),
        r#"  "plugin list --json") echo '[{"id":"question-alert@hexa","scope":"user","enabled":false}]';;"#,
    );
    assert_eq!(
        hexa_cli::claude_plugins_at(&bin, roomy()).installed(),
        Ok(vec!["question-alert@hexa".to_string()])
    );
}

#[test]
fn a_malformed_listing_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let (bin, _) = fake(
        dir.path(),
        r#"  "plugin list --json") echo '{"plugins": 3}';;"#,
    );
    assert!(hexa_cli::claude_plugins_at(&bin, roomy())
        .installed()
        .is_err());
}

#[test]
fn the_same_folder_through_a_symlink_is_not_added_again() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("claude-mods");
    std::fs::create_dir_all(&real).unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let body = format!(
        "  \"plugin marketplace list --json\") echo '[{{\"name\":\"hexa\",\"path\":\"{}/\"}}]';;",
        link.display()
    );
    let (bin, log) = fake(dir.path(), &body);
    assert_eq!(
        hexa_cli::claude_plugins_at(&bin, roomy()).add_marketplace(&real),
        Ok(())
    );
    assert!(
        !log_of(&log).contains("marketplace add"),
        "{}",
        log_of(&log)
    );
}

#[test]
fn a_hexa_marketplace_elsewhere_is_refused_and_nothing_installs() {
    let dir = tempfile::tempdir().unwrap();
    let ours = dir.path().join("claude-mods");
    std::fs::create_dir_all(&ours).unwrap();
    let (bin, log) = fake(
        dir.path(),
        r#"  "plugin marketplace list --json") echo '[{"name":"hexa","path":"/elsewhere"}]';;"#,
    );
    let cli = hexa_cli::claude_plugins_at(&bin, roomy());
    let err = cli.add_marketplace(&ours).unwrap_err();
    assert!(err.contains("marketplace remove hexa"), "{err}");
    assert!(cli.install("question-alert@hexa").is_err());
    let calls = log_of(&log);
    assert!(!calls.contains("plugin install"), "{calls}");
    assert!(!calls.contains("marketplace add"), "{calls}");
}

#[test]
fn an_unregistered_folder_is_added_as_given() {
    let dir = tempfile::tempdir().unwrap();
    let ours = dir.path().join("claude-mods");
    let (bin, log) = fake(dir.path(), "  *list*) echo '[]';;");
    let cli = hexa_cli::claude_plugins_at(&bin, roomy());
    assert_eq!(cli.add_marketplace(&ours), Ok(()));
    assert_eq!(cli.install("question-alert@hexa"), Ok(()));
    let calls = log_of(&log);
    assert!(
        calls.contains(&format!("plugin marketplace add {}", ours.display())),
        "{calls}"
    );
    assert!(
        calls.contains("plugin install question-alert@hexa --scope user"),
        "{calls}"
    );
}

#[test]
fn a_multi_line_error_prints_on_one_line() {
    let dir = tempfile::tempdir().unwrap();
    let (bin, _) = fake(
        dir.path(),
        "  \"plugin install\"*) printf 'first\\n\\n  second\\n' >&2; exit 3;;",
    );
    let err = hexa_cli::claude_plugins_at(&bin, roomy())
        .install("x@hexa")
        .unwrap_err();
    assert_eq!(err, "first; second");
}

#[test]
fn a_missing_binary_is_not_available() {
    let dir = tempfile::tempdir().unwrap();
    assert!(!hexa_cli::claude_plugins_at(&dir.path().join("nope"), roomy()).available());
}
