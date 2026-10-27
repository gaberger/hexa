//! `hexa do run --also <PATH>` exists, says what it does, and refuses a path
//! that leaves the repository before anything runs. The help also names the
//! delegate's timeout variable, which was configurable but documented nowhere.
//! Written before the flag.

use std::process::Command;

fn help() -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(["do", "run", "--help"])
        .output()
        .expect("run hexa do run --help");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn do_run_offers_a_repeatable_also() {
    let h = help();
    assert!(h.contains("--also <ALSO>"), "{h}");
}

#[test]
fn do_run_documents_the_delegate_timeout() {
    let h = help();
    assert!(h.contains("CLAUDE_TIMEOUT_SECS"), "{h}");
}

#[test]
fn an_also_path_outside_the_repository_is_refused_before_anything_runs() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let git = |args: &[&str]| {
        let o = Command::new("git").args(args).current_dir(dir).output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    };
    git(&["init", "-q"]);
    git(&["-c", "user.name=t", "-c", "user.email=t@x", "commit", "-q", "--allow-empty", "-m", "init"]);
    std::fs::write(dir.join("a.rs"), "// a\n").unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args([
            "do", "run", "touch a file", "--file", "a.rs", "--evidence", "touch evidence-ran",
            "--model", "claude-code", "--also", "../outside.rs",
        ])
        .current_dir(dir)
        .env("HOME", dir)
        .env("HEXA_PROJECT_ROOT", dir)
        .env("HEXA_CLAUDE_BINARY", "/bin/false")
        .output()
        .expect("run hexa do run");
    let text = String::from_utf8_lossy(&out.stderr).into_owned() + &String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success(), "{text}");
    assert!(text.contains("--also") && text.contains(".."), "the refusal names the flag and the reason: {text}");
    assert!(!dir.join("evidence-ran").exists(), "nothing ran: {text}");
}
