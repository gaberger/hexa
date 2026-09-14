//! G. The binary.
//!
//! A library that passes its own tests still has to start. These tests run the
//! real program, the way a person or a gate would.

use std::io::Write;
use std::process::{Command, Stdio};

use minesweeper::adapters::secondary::SeededRng;
use minesweeper::domain::{Dims, Layout, RollError};
use minesweeper::ports::{RandomSource, RollFault};

/// The port has its own failure type. The domain has its own word for the same
/// trouble. A real use case does this translation, and so does a test.
fn as_domain(f: RollFault) -> RollError {
    match f {
        RollFault::ZeroBound => RollError::ZeroBound,
        RollFault::Stuck => RollError::Stuck,
    }
}

const BIN: &str = env!("CARGO_BIN_EXE_minesweeper");

struct Run {
    code: i32,
    out: String,
    err: String,
}

fn run(args: &[&str], stdin: &str) -> Run {
    let mut child = Command::new(BIN)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the program must start");
    {
        let mut pipe = child.stdin.take().expect("stdin");
        pipe.write_all(stdin.as_bytes()).expect("write stdin");
    }
    let out = child.wait_with_output().expect("the program must finish");
    Run {
        code: out.status.code().unwrap_or(-1),
        out: String::from_utf8_lossy(&out.stdout).into_owned(),
        err: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn last_line(s: &str) -> String {
    s.lines().last().unwrap_or("").to_string()
}

/// 35. A board with no mines is always won.
#[test]
fn a_board_with_no_mines_is_always_won() {
    for seed in ["1", "42", "43", "777", "20260914"] {
        let r = run(&["--demo", "--seed", seed, "--mines", "0"], "");
        assert_eq!(r.code, 0, "seed {seed} exited {}", r.code);
        assert_eq!(last_line(&r.out), "YOU WIN", "seed {seed}");
        assert_eq!(r.err, "", "seed {seed} wrote to the error stream");
    }
}

/// 36. The reckless player is always lost.
#[test]
fn the_reckless_player_is_always_lost() {
    for seed in ["1", "42", "43", "777", "20260914"] {
        let r = run(&["--demo", "--seed", seed, "--policy", "reckless"], "");
        assert_eq!(r.code, 0, "seed {seed} exited {}", r.code);
        assert_eq!(last_line(&r.out), "GAME OVER", "seed {seed}");
        assert_eq!(r.err, "", "seed {seed} wrote to the error stream");
    }
}

/// The thinking player always finishes, and always ends one of the two ways.
#[test]
fn the_thinking_player_always_finishes() {
    for seed in ["1", "42", "43", "777", "20260914"] {
        let r = run(&["--demo", "--seed", seed], "");
        assert_eq!(r.code, 0, "seed {seed} exited {}", r.code);
        let last = last_line(&r.out);
        assert!(
            last == "YOU WIN" || last == "GAME OVER",
            "seed {seed} ended with {last:?}"
        );
        assert_eq!(r.err, "", "seed {seed} wrote to the error stream");
    }
}

/// 37. A real interactive game, with a click on a cell known to hold a mine.
#[test]
fn stepping_on_a_mine_ends_the_game() {
    // Work out where a mine is, with the same seed and the same first click
    // the program will use. The program is a pure function of the two.
    let seed: u64 = 20260914;
    let d = Dims::new(9, 9).expect("dims");
    let first = d.coord(0, 0).expect("cell");
    let mut exclude = d.neighbours(first);
    exclude.push(first);
    let mut rng = SeededRng::new(seed);
    let layout = Layout::place(d, 10, &exclude, &mut |b| rng.next_below(b).map_err(as_domain)).expect("place");
    let mine = (0..d.total())
        .filter_map(|i| d.from_index(i))
        .find(|c| layout.is_mine(*c))
        .expect("a mine");

    let script = format!("r 0 0\nr {} {}\n", mine.x(), mine.y());
    let r = run(&["--seed", &seed.to_string()], &script);
    assert_eq!(r.code, 0);
    assert_eq!(last_line(&r.out), "GAME OVER");
    assert_eq!(r.err, "");
    // The loss screen shows the answer.
    assert!(r.out.contains('X'), "the mine you stepped on was not drawn");
    assert!(r.out.contains('*'), "the other mines were not drawn");
}

/// A click off the board is refused, and the game carries on.
#[test]
fn a_click_off_the_board_does_not_crash() {
    let r = run(&["--seed", "42"], "r 99 99\nf 99 99\nr 9 0\nq\n");
    assert_eq!(r.code, 0);
    assert_eq!(last_line(&r.out), "QUIT");
    assert_eq!(r.out.matches("That cell is not on the board.").count(), 3);
    assert_eq!(r.err, "");
}

/// 38. Stdin closed at once. The game quits and does not loop.
#[test]
fn closed_input_quits_at_once() {
    let r = run(&["--seed", "42"], "");
    assert_eq!(r.code, 0);
    assert_eq!(last_line(&r.out), "QUIT");
    assert_eq!(r.err, "");
}

/// The last three lines always read board, stats, then the ending.
#[test]
fn the_last_three_lines_are_always_the_same_shape() {
    for args in [
        vec!["--demo", "--seed", "42"],
        vec!["--demo", "--seed", "42", "--policy", "reckless"],
        vec!["--demo", "--seed", "42", "--mines", "0"],
        vec!["--seed", "42"],
    ] {
        let r = run(&args, "");
        let lines: Vec<&str> = r.out.lines().collect();
        let n = lines.len();
        assert!(n >= 3, "too few lines for {args:?}");
        let board = lines.get(n - 3).copied().unwrap_or("");
        let stats = lines.get(n - 2).copied().unwrap_or("");
        let end = lines.get(n - 1).copied().unwrap_or("");
        assert!(board.starts_with("BOARD "), "{args:?} board line: {board:?}");
        assert_eq!(board.len(), 6 + 16, "{args:?} board line length");
        assert!(stats.starts_with("STATS revealed="), "{args:?} stats: {stats:?}");
        assert!(
            end == "YOU WIN" || end == "GAME OVER" || end == "QUIT",
            "{args:?} ending: {end:?}"
        );
    }
}

/// The seed does real work: same seed, same board; different seed, different board.
#[test]
fn the_seed_does_work() {
    let a = run(&["--demo", "--seed", "42"], "");
    let b = run(&["--demo", "--seed", "42"], "");
    assert_eq!(a.out, b.out, "the same seed gave a different game");

    let board = |r: &Run| {
        r.out
            .lines()
            .find(|l| l.starts_with("BOARD "))
            .unwrap_or("")
            .to_string()
    };
    let c = run(&["--demo", "--seed", "43"], "");
    assert_ne!(board(&a), board(&c), "seed 43 gave the same board as seed 42");
}

/// 39. A bad option stops the program with code 2 and one line of reason.
#[test]
fn a_bad_option_stops_with_code_two() {
    let cases: Vec<Vec<&str>> = vec![
        vec!["--seed"],
        vec!["--seed", "abc"],
        vec!["--seed", "-1"],
        vec!["--mines", "999"],
        vec!["--width", "0"],
        vec!["--height", "0"],
        vec!["--policy", "psychic"],
        vec!["--policy"],
        vec!["--nonsense"],
    ];
    for args in cases {
        let r = run(&args, "");
        assert_eq!(r.code, 2, "{args:?} exited {}", r.code);
        assert_eq!(r.err.lines().count(), 1, "{args:?} said: {:?}", r.err);
        assert_eq!(r.out, "", "{args:?} printed to the normal output");
    }
}

/// The help text works and leaves with code 0.
#[test]
fn help_works() {
    let r = run(&["--help"], "");
    assert_eq!(r.code, 0);
    assert!(r.out.contains("--demo"));
    assert!(r.out.contains("--seed"));
    assert_eq!(r.err, "");
}

/// 40. The reader can go away at any time. The program ends quietly.
#[test]
fn a_closed_pipe_ends_the_program_quietly() {
    let out = Command::new("sh")
        .arg("-c")
        .arg(format!("'{BIN}' --demo --seed 42 | head -1"))
        .output()
        .expect("the shell must run");
    assert!(out.status.success(), "the pipeline failed");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!err.contains("panic"), "the program panicked: {err}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.starts_with("BOARD "), "first line was {text:?}");
}

/// 41. The start script is runnable, always builds, and passes options through.
#[test]
fn the_start_script_is_ready_to_run() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("run.sh");
    let text = std::fs::read_to_string(&path).expect("run.sh must exist");
    assert!(text.contains("cargo build --release"), "run.sh must always build");
    assert!(text.contains("\"$@\""), "run.sh must pass options through");
    assert!(text.contains("set -euo pipefail"), "run.sh must stop on an error");
    assert!(text.contains("cd \"$(dirname \"$0\")\""), "run.sh must move to its own folder");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Git records only the owner execute bit, and a checkout applies the
        // local umask to the rest. So the owner bit is the one that matters.
        let mode = std::fs::metadata(&path).expect("metadata").permissions().mode();
        assert_ne!(mode & 0o100, 0, "run.sh must have the execute bit set");
    }
}

/// The README tells a player how to play.
#[test]
fn the_readme_says_how_to_play() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md");
    let text = std::fs::read_to_string(&path).expect("README.md must exist");
    for needle in [
        "r <col> <row>",
        "f <col> <row>",
        "--demo",
        "--seed",
        "GAME OVER",
        "YOU WIN",
        "QUIT",
        "./run.sh",
    ] {
        assert!(text.contains(needle), "README.md never mentions {needle:?}");
    }
}

/// 59. Nothing prints after the ending line.
///
/// The ending is the last word. A program that keeps talking after "GAME OVER"
/// makes a gate that reads the last line unreliable.
#[test]
fn nothing_prints_after_the_ending_line() {
    for args in [
        vec!["--demo", "--seed", "42"],
        vec!["--demo", "--seed", "1"],
        vec!["--demo", "--seed", "1", "--policy", "reckless"],
        vec!["--demo", "--seed", "42", "--mines", "0"],
    ] {
        let r = run(&args, "");
        assert_eq!(r.code, 0, "{args:?} exited {}", r.code);

        let end = last_line(&r.out);
        assert!(
            end == "YOU WIN" || end == "GAME OVER" || end == "QUIT",
            "{args:?} ended with {end:?}"
        );

        // The ending word is said once, and once only.
        assert_eq!(
            r.out.matches(&end).count(),
            1,
            "{args:?} said {end:?} more than once"
        );

        // The output stops right after that line, with one newline and no more.
        assert!(
            r.out.ends_with(&format!("{end}\n")),
            "{args:?} printed something after the ending"
        );
        assert_eq!(r.err, "", "{args:?} wrote to the error stream");
    }
}

/// An input failure leaves with code 3, and prints no ending.
///
/// This is the branch the demo player's round cap uses. A cap, a broken reader
/// and unreadable bytes all arrive as one `io` failure, and `run` must turn
/// every one of them into code 3 rather than into a word the gate believes.
#[test]
fn an_input_failure_exits_three_and_prints_no_ending() {
    let mut child = Command::new(BIN)
        .args(["--seed", "42"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the program must start");
    {
        let mut pipe = child.stdin.take().expect("stdin");
        // Bytes that are not text. The reader cannot make a line of them.
        pipe.write_all(b"r 0 0\n\xff\xfe\n").expect("write stdin");
    }
    let out = child.wait_with_output().expect("the program must finish");

    assert_eq!(out.status.code(), Some(3), "an input failure must exit 3");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(!text.contains("YOU WIN"), "a failure printed a win");
    assert!(!text.contains("GAME OVER"), "a failure printed a loss");
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(err.lines().count(), 1, "the reason must be one line: {err:?}");
    assert!(!err.contains("panic"), "the program panicked: {err}");
}
