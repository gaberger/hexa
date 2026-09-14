//! The two independent oracles. These are the tests that catch a program which
//! looks right and is not.

mod common;

use std::process::Command;

use common::{oracle_outcome, play_random_game, result_words, view_from_lines, TestRng};
use connect_four::domain::Outcome;

/// Ten thousand random games, checked by two different win checkers. The game
/// walks out from the new disc; the oracle scans all 69 windows. They must
/// never disagree, on any move of any game.
#[test]
fn oracle_agrees() {
    let mut rng = TestRng::new(0xC0FFEE);
    for _ in 0..10_000 {
        play_random_game(&mut rng, |game| {
            let view = game.view();
            assert_eq!(
                game.outcome(),
                oracle_outcome(&view),
                "the two checkers disagree on this board:\n{}",
                common::lines_from_view(&view)
            );
        });
    }
}

/// The real binary, for 100 seeds. Read the last frame off the screen and work
/// the winner out again from those forty-two characters. It must match the
/// word the program printed.
///
/// This is the test that catches a program which always prints `DRAW`.
#[test]
fn announcement_is_true() {
    let binary = env!("CARGO_BIN_EXE_connect-four");
    for seed in 1..=100_u64 {
        let done = Command::new(binary)
            .args(["--demo", "--seed", &seed.to_string()])
            .output()
            .expect("the binary runs");
        assert!(done.status.success(), "seed {seed} did not exit 0");

        let text = String::from_utf8(done.stdout).expect("the output is text");
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines.len() >= 7, "seed {seed} printed too little");

        let claimed = lines[lines.len() - 1];
        let last_frame = &lines[lines.len() - 7..lines.len() - 1];
        let view = view_from_lines(last_frame);
        let truth = oracle_outcome(&view);

        assert_ne!(
            truth,
            Outcome::InProgress,
            "seed {seed} stopped with the game unfinished"
        );
        assert_eq!(
            claimed,
            result_words(truth),
            "seed {seed} printed {claimed:?} for this board:\n{}",
            common::lines_from_view(&view)
        );
    }
}
