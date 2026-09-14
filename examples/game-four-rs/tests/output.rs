//! The output contract. Standard output is a machine contract, so it is
//! checked byte by byte.

mod common;

use std::process::Command;

use common::view_from_lines;
use connect_four::adapters::primary::StrictRenderer;
use connect_four::domain::{Column, Disc, Game, Outcome};
use connect_four::ports::Renderer;

fn render(moves: &[u8]) -> String {
    let mut game = Game::new();
    for raw in moves {
        game.drop(Column::new(*raw).expect("in range"))
            .expect("legal");
    }
    let mut out = StrictRenderer::new(Vec::new());
    out.frame(&game.view()).expect("a vector always takes bytes");
    String::from_utf8(out.into_inner()).expect("the frame is text")
}

/// A hand-built, lopsided position with its exact six lines.
///
/// One red disc on the floor of the left column must print on the **last**
/// line, not the first. A board flipped twice would put it on the first line
/// and still look tidy, so this test is what catches the double flip.
#[test]
fn golden_frame() {
    assert_eq!(
        render(&[0]),
        ".......\n.......\n.......\n.......\n.......\nR......\n"
    );

    // Lopsided left to right as well, so a mirrored board cannot pass.
    assert_eq!(
        render(&[0, 6, 1]),
        ".......\n.......\n.......\n.......\n.......\nRR....Y\n"
    );

    // And lopsided top to bottom, with a stack that does not reach the top.
    assert_eq!(
        render(&[0, 0, 0]),
        ".......\n.......\n.......\nR......\nY......\nR......\n"
    );
}

/// The three result words, exactly as the gate reads them.
#[test]
fn announcement_words_are_exact() {
    for (outcome, words) in [
        (Outcome::Win(Disc::Red), "RED WINS\n"),
        (Outcome::Win(Disc::Yellow), "YELLOW WINS\n"),
        (Outcome::Draw, "DRAW\n"),
    ] {
        let mut out = StrictRenderer::new(Vec::new());
        out.announce(outcome).expect("a vector always takes bytes");
        assert_eq!(String::from_utf8(out.into_inner()).expect("text"), words);
    }
}

/// Index 0 of the view is the floor of the left column. Everything else in the
/// program leans on that one sentence.
#[test]
fn view_index_zero_is_the_floor() {
    let mut game = Game::new();
    game.drop(Column::new(0).expect("in range")).expect("legal");
    let view = game.view();
    assert_eq!(view.cells[0], Some(Disc::Red), "index 0 is column 0, row 0");
    assert_eq!(view.at(0, 0), Some(Disc::Red));
    assert_eq!(view.at(0, 5), None, "the top of the column is still empty");
}

/// The shape of the whole transcript, over 100 seeds.
#[test]
fn line_shape() {
    let binary = env!("CARGO_BIN_EXE_connect-four");
    for seed in 1..=100_u64 {
        let done = Command::new(binary)
            .args(["--demo", "--seed", &seed.to_string()])
            .output()
            .expect("the binary runs");
        assert!(done.status.success(), "seed {seed} did not exit 0");
        assert!(done.stderr.is_empty(), "seed {seed} wrote to stderr");

        let text = String::from_utf8(done.stdout).expect("the output is text");
        assert!(!text.contains('\r'), "seed {seed} printed a carriage return");
        assert!(text.ends_with('\n'), "seed {seed} lost its last newline");

        let lines: Vec<&str> = text.lines().collect();
        let (last, board_lines) = lines.split_last().expect("there is at least one line");
        assert!(
            ["RED WINS", "YELLOW WINS", "DRAW"].contains(last),
            "seed {seed} ended with {last:?}"
        );
        for line in board_lines {
            assert_eq!(line.len(), 7, "seed {seed} printed {line:?}");
            assert!(
                line.chars().all(|glyph| matches!(glyph, '.' | 'R' | 'Y')),
                "seed {seed} printed {line:?}"
            );
        }

        assert_eq!(board_lines.len() % 6, 0, "seed {seed} has a half frame");
        let moves = board_lines.len() / 6;
        assert!(moves >= 7, "seed {seed} claims to end in {moves} moves");
        assert!(moves <= 42, "seed {seed} played {moves} moves");
        assert_eq!(lines.len(), 6 * moves + 1);
    }
}

/// Each frame must hold one disc more than the frame before it, and the discs
/// already down must never move.
#[test]
fn frames_grow_by_one_disc() {
    let binary = env!("CARGO_BIN_EXE_connect-four");
    for seed in [1_u64, 2, 3, 99] {
        let done = Command::new(binary)
            .args(["--demo", "--seed", &seed.to_string()])
            .output()
            .expect("the binary runs");
        let text = String::from_utf8(done.stdout).expect("text");
        let lines: Vec<&str> = text.lines().collect();
        let frames = (lines.len() - 1) / 6;
        for index in 0..frames {
            let view = view_from_lines(&lines[index * 6..index * 6 + 6]);
            assert_eq!(
                view.disc_count(),
                index + 1,
                "frame {index} of seed {seed} holds the wrong number of discs"
            );
        }
    }
}
