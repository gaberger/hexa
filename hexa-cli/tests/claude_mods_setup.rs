//! Setup's Claude Code mods step through the binary (ADR-2610301129 §4):
//! dry run, opt-out, an already-initialized project, and no home directory.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// A fake `claude` in `root/bin` that logs every call and answers lists with `[]`.
fn fake_claude(root: &Path) -> (PathBuf, PathBuf) {
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let log = root.join("calls.log");
    let script = format!(
        "#!/bin/sh\necho \"$*\" >> \"{}\"\ncase \"$*\" in *list*) echo '[]';; esac\nexit 0\n",
        log.display()
    );
    let path = bin.join("claude");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    (bin, log)
}

fn path_with(bin: &Path) -> std::ffi::OsString {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let rest = std::env::split_paths(&path).filter(|d| !d.join("claude").exists());
    std::env::join_paths(std::iter::once(bin.to_path_buf()).chain(rest)).unwrap()
}

fn hexa(args: &[&str], home: Option<&Path>, bin: &Path, cwd: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_hexa"));
    cmd.args(args)
        .env("PATH", path_with(bin))
        .env_remove("HEXA_NO_CLAUDE_MODS")
        .current_dir(cwd);
    cmd.stdin(Stdio::null());
    match home {
        Some(h) => cmd.env("HOME", h),
        None => cmd.env("HOME", ""),
    };
    cmd
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn log_of(log: &Path) -> String {
    std::fs::read_to_string(log).unwrap_or_default()
}

#[test]
fn a_dry_run_writes_nothing_and_runs_no_claude() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let (bin, log) = fake_claude(root.path());
    let out = hexa(
        &["bootstrap", "--dry-run", "--skip-prereq", "--skip-models"],
        Some(&home),
        &bin,
        &home,
    )
    .output()
    .unwrap();
    assert!(
        text(&out).contains("would extract Claude Code mods"),
        "{}",
        text(&out)
    );
    assert!(!home.join(".hexa/claude-mods").exists());
    assert_eq!(log_of(&log), "");
}

#[test]
fn the_opt_out_skips_extraction_and_claude() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let project = root.path().join("project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    let (bin, log) = fake_claude(root.path());
    let out = hexa(&["dev", "init"], Some(&home), &bin, &project)
        .env("HEXA_NO_CLAUDE_MODS", "1")
        .output()
        .unwrap();
    assert!(text(&out).contains("HEXA_NO_CLAUDE_MODS"), "{}", text(&out));
    assert!(!home.join(".hexa/claude-mods").exists());
    assert_eq!(log_of(&log), "");
}

#[test]
fn an_initialized_project_still_gets_the_mods_and_the_same_error() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let project = root.path().join("project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(project.join(".hexa")).unwrap();
    let (bin, log) = fake_claude(root.path());
    let out = hexa(&["dev", "init"], Some(&home), &bin, &project)
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "already initialized must still fail:\n{}",
        text(&out)
    );
    assert!(text(&out).contains("already initialized"), "{}", text(&out));
    assert!(home
        .join(".hexa/claude-mods/.claude-plugin/marketplace.json")
        .is_file());
    assert!(
        log_of(&log).contains("plugin install question-alert@hexa --scope user"),
        "{}",
        log_of(&log)
    );
}

#[test]
fn a_rerun_is_quiet_once_installed() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let (bin, _) = fake_claude(root.path());
    // The fake lists nothing installed, so to see "up to date" the listing
    // has to say so.
    let script = bin.join("claude");
    let body = std::fs::read_to_string(&script).unwrap().replace(
        "*list*) echo '[]';;",
        "\"plugin list --json\") echo '[{\"id\":\"question-alert@hexa\",\"scope\":\"user\"}]';; *list*) echo '[]';;",
    );
    std::fs::write(&script, body).unwrap();
    let out = hexa(&["dev", "init"], Some(&home), &bin, &home)
        .output()
        .unwrap();
    assert!(
        text(&out).contains("Claude Code mods up to date"),
        "{}",
        text(&out)
    );
}

#[test]
fn no_home_directory_warns_and_keeps_the_exit_status() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let a = root.path().join("a");
    let b = root.path().join("b");
    for d in [&home, &a, &b] {
        std::fs::create_dir_all(d).unwrap();
    }
    let (bin, _) = fake_claude(root.path());
    let with_home = hexa(&["dev", "init"], Some(&home), &bin, &a)
        .output()
        .unwrap();
    let without = hexa(&["dev", "init"], None, &bin, &b).output().unwrap();
    assert!(
        text(&without).contains("⚠ Claude Code mods"),
        "{}",
        text(&without)
    );
    assert_eq!(
        without.status.code(),
        with_home.status.code(),
        "{}",
        text(&without)
    );
}
