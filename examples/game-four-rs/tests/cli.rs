//! The command line table, and the end-of-input promise.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_connect-four")
}

/// Run the program with no keyboard behind it.
fn run(args: &[&str]) -> (i32, String, String) {
    let done = Command::new(binary())
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("the binary runs");
    (
        done.status.code().expect("the program exits, it is not killed"),
        String::from_utf8_lossy(&done.stdout).to_string(),
        String::from_utf8_lossy(&done.stderr).to_string(),
    )
}

/// Every row of the table in section 7 of the spec.
#[test]
fn cli_parse_table() {
    // The good rows. Exit 0.
    let (code, out, err) = run(&["--demo", "--seed", "7"]);
    assert_eq!(code, 0, "--demo --seed 7");
    assert!(!out.is_empty(), "--demo --seed 7 printed nothing");
    assert!(err.is_empty(), "--demo --seed 7 complained: {err}");

    for good in [vec!["--seed", "7"], vec![]] {
        let (code, _, _) = run(&good);
        assert_eq!(code, 0, "{good:?} should be interactive and exit 0");
    }

    // A seed of zero is a real seed, not a missing one.
    let (code, out, _) = run(&["--demo", "--seed", "0"]);
    assert_eq!(code, 0, "--demo --seed 0");
    assert!(!out.is_empty());

    // The bad rows. Exit 2, one line on stderr, nothing on stdout.
    let bad: [(&[&str], &str); 8] = [
        (&["--demo"], "--demo needs --seed <n>"),
        (&["--seed"], "--seed needs a value"),
        (&["--seed", "abc"], "--seed needs a decimal number"),
        (&["--seed", "-1"], "--seed needs a decimal number"),
        (&["--seed", "0x10"], "--seed needs a decimal number"),
        (
            &["--seed", "18446744073709551616"],
            "--seed needs a decimal number",
        ),
        (&["--wat"], "usage: connect-four [--demo] [--seed <n>]"),
        (&["extra"], "usage: connect-four [--demo] [--seed <n>]"),
    ];
    for (args, message) in bad {
        let (code, out, err) = run(args);
        assert_eq!(code, 2, "{args:?} should exit 2");
        assert!(out.is_empty(), "{args:?} printed to stdout: {out}");
        assert_eq!(err.trim_end(), message, "{args:?} said the wrong thing");
        assert_eq!(err.lines().count(), 1, "{args:?} wrote more than one line");
        assert!(!err.contains("panicked"), "{args:?} panicked");
    }

    // An extra word after a good pair is still an error, not a silent ignore.
    let (code, _, err) = run(&["--demo", "--seed", "7", "extra"]);
    assert_eq!(code, 2);
    assert!(err.starts_with("usage:"));
}

/// End of input must never block. That is the classic hang for this program.
#[test]
fn eof_quits_cleanly() {
    let started = Instant::now();
    let mut child = Command::new(binary())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("the binary starts");

    loop {
        if let Some(status) = child.try_wait().expect("the child can be asked") {
            assert_eq!(status.code(), Some(0), "end of input should exit 0");
            return;
        }
        if started.elapsed() > Duration::from_secs(2) {
            let _ = child.kill();
            panic!("the program was still waiting after two seconds");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A full column costs the player nothing: the program asks again instead of
/// giving the turn away.
#[test]
fn a_full_column_re_prompts() {
    use std::io::Write;

    let mut child = Command::new(binary())
        .args(["--seed", "3"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the binary starts");

    {
        let stdin = child.stdin.as_mut().expect("the pipe is open");
        // "9" and "banana" are not columns. "q" then stops the game.
        stdin.write_all(b"9\nbanana\n\nq\n").expect("the pipe takes bytes");
    }
    let done = child.wait_with_output().expect("the child finishes");
    assert_eq!(done.status.code(), Some(0), "quitting exits 0");

    let err = String::from_utf8_lossy(&done.stderr);
    assert!(
        err.contains("type a number from 1 to 7"),
        "a typo should be answered on stderr, got: {err}"
    );
    let out = String::from_utf8_lossy(&done.stdout);
    assert!(out.contains("1234567"), "the human board shows a ruler");
}
