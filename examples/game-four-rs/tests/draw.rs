//! The draw case, which random play almost never reaches.

mod common;

use common::{lines_from_view, DRAW_BOARD, DRAW_MOVES};
use common::{ScriptedInput, Transcript};
use connect_four::domain::{Column, Game, Outcome};
use connect_four::ports::Request;

/// The frozen transcript fills the board and nobody makes a line.
#[test]
fn full_board_draws() {
    let mut game = Game::new();
    for (step, raw) in DRAW_MOVES.iter().enumerate() {
        let column = Column::new(*raw).expect("a frozen column is in range");
        let outcome = game.drop(column).expect("a frozen move is legal");
        if step < 41 {
            assert_eq!(
                outcome,
                Outcome::InProgress,
                "move {step} ended the game early"
            );
        } else {
            assert_eq!(outcome, Outcome::Draw, "the last move is a draw");
        }
    }
    assert_eq!(game.move_count(), 42);
    assert_eq!(game.outcome(), Outcome::Draw);
    assert_eq!(lines_from_view(&game.view()), DRAW_BOARD.join("\n") + "\n");
}

/// The same transcript, driven through the whole program.
#[test]
fn draw_end_to_end() {
    let mut input = ScriptedInput::new(&DRAW_MOVES);
    let mut out = Transcript::new();
    let code = connect_four::run(Request::Demo { seed: 0 }, &mut out, &mut input);
    assert_eq!(format!("{code:?}"), format!("{:?}", std::process::ExitCode::SUCCESS));

    // Build what the frames must be, one move at a time, without asking the
    // program.
    let mut expected = String::new();
    let mut shadow = Game::new();
    for raw in DRAW_MOVES {
        shadow
            .drop(Column::new(raw).expect("in range"))
            .expect("legal");
        expected.push_str(&lines_from_view(&shadow.view()));
    }
    expected.push_str("DRAW\n");

    assert_eq!(out.text, expected);
    let lines = out.text.lines().count();
    assert_eq!(lines, 6 * 42 + 1, "six lines a move, plus the result");
    assert!(out.text.ends_with("DRAW\n"));
}
