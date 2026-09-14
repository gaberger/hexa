//! The corners of the rules.

mod common;

use common::{check_invariants, play_random_game, TestRng, WIN42_MOVES};
use connect_four::domain::{Column, Disc, Game, MoveError, Outcome, COLUMNS, ROWS};

fn column(raw: u8) -> Column {
    Column::new(raw).expect("the test uses columns 0..=6")
}

fn play(game: &mut Game, moves: &[u8]) -> Outcome {
    let mut last = Outcome::InProgress;
    for raw in moves {
        last = game.drop(column(*raw)).expect("the test plays legal moves");
    }
    last
}

/// One drop joins two groups and makes five. "Exactly four" would miss it.
#[test]
fn five_in_a_row_wins() {
    let mut game = Game::new();
    // Red takes 0, 1, 3, 4 on the floor. Yellow stacks out of the way.
    let outcome = play(&mut game, &[0, 6, 1, 6, 3, 6, 4, 5, 2]);
    assert_eq!(outcome, Outcome::Win(Disc::Red));

    let view = game.view();
    for spot in 0..5 {
        assert_eq!(
            view.at(spot, 0),
            Some(Disc::Red),
            "the floor should hold five red discs"
        );
    }
}

/// A disc in each of the four corners, walked in all four directions. An
/// unsigned step left from column 0 would wrap and panic here.
#[test]
fn corner_diagonals_do_not_panic() {
    let mut game = Game::new();
    let outcome = play(&mut game, &[0, 0, 6, 6, 0, 0, 6, 6, 0, 0, 6, 6]);
    assert_eq!(outcome, Outcome::InProgress);

    let view = game.view();
    for corner in [(0, 0), (0, ROWS - 1), (COLUMNS - 1, 0), (COLUMNS - 1, ROWS - 1)] {
        assert!(
            view.at(corner.0, corner.1).is_some(),
            "corner {corner:?} should hold a disc"
        );
    }
    assert_eq!(game.height(0), ROWS);
    assert_eq!(game.height(COLUMNS - 1), ROWS);
}

/// The forty-second disc is allowed to win. A draw check that runs first would
/// steal it.
#[test]
fn win_beats_draw_on_move_42() {
    let mut game = Game::new();
    for (step, raw) in WIN42_MOVES.iter().enumerate() {
        let outcome = game.drop(column(*raw)).expect("a frozen move is legal");
        if step < 41 {
            assert_eq!(outcome, Outcome::InProgress, "move {step} ended too early");
        } else {
            assert_eq!(
                outcome,
                Outcome::Win(Disc::Yellow),
                "the last disc wins, it does not draw"
            );
        }
    }
    assert_eq!(game.move_count(), 42);
    assert_eq!(game.view().disc_count(), 42);
}

/// After a win the game is closed. Nothing more moves.
#[test]
fn game_is_final() {
    let mut game = Game::new();
    play(&mut game, &[0, 6, 1, 6, 3, 6, 4, 5, 2]);
    assert_eq!(game.outcome(), Outcome::Win(Disc::Red));

    let before = game.view();
    let count_before = game.move_count();
    for raw in 0..7_u8 {
        assert_eq!(game.drop(column(raw)), Err(MoveError::GameOver));
    }
    assert_eq!(game.view(), before, "a refused move changed the board");
    assert_eq!(game.move_count(), count_before);
}

/// A full column and a column that does not exist both change nothing.
#[test]
fn illegal_moves_change_nothing() {
    assert_eq!(Column::new(7), Err(MoveError::OutOfRange));
    assert_eq!(Column::new(255), Err(MoveError::OutOfRange));

    let mut game = Game::new();
    play(&mut game, &[0, 0, 0, 0, 0, 0]);
    assert_eq!(game.height(0), ROWS);
    assert_eq!(game.outcome(), Outcome::InProgress);

    let before = game.view();
    assert_eq!(game.drop(column(0)), Err(MoveError::ColumnFull));
    assert_eq!(game.view(), before, "a full column changed the board");
    assert_eq!(game.move_count(), 6);
    assert!(
        !game.legal_moves().contains(column(0)),
        "a full column is not a legal move"
    );
    assert_eq!(game.legal_moves().len(), 6);
}

/// The four promises of the spec, over ten thousand random games.
#[test]
fn invariants_hold() {
    let mut rng = TestRng::new(0xBADC0DE);
    for _ in 0..10_000 {
        let game = play_random_game(&mut rng, check_invariants);
        assert!(game.outcome().is_final());

        // Promise four: after the end, a drop changes nothing.
        let mut closed = game;
        let before = closed.view();
        for raw in 0..7_u8 {
            assert_eq!(closed.drop(column(raw)), Err(MoveError::GameOver));
        }
        assert_eq!(closed.view(), before);
    }
}

/// Whose turn it is comes from the move count, never from a stored flag.
#[test]
fn red_moves_first_and_the_turn_alternates() {
    let mut game = Game::new();
    assert_eq!(game.to_move(), Disc::Red);
    for step in 0..6_u8 {
        let expected = if step % 2 == 0 { Disc::Red } else { Disc::Yellow };
        assert_eq!(game.to_move(), expected);
        game.drop(column(step % 7)).expect("legal");
    }
}
