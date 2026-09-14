//! The gate for this fixture's *behaviour*. It is green, and stays green:
//! the architecture is what is wrong here, not the game.

use game_sokoban_rs_tangled::adapters::primary::console;
use game_sokoban_rs_tangled::adapters::secondary::builtin_levels::BuiltinLevels;
use game_sokoban_rs_tangled::adapters::secondary::memory_recorder::MemoryRecorder;
use game_sokoban_rs_tangled::domain::level::{Level, LevelError, Tile};
use game_sokoban_rs_tangled::domain::position::{Dir, Pos};
use game_sokoban_rs_tangled::domain::push::{apply, commit, Board, Outcome};
use game_sokoban_rs_tangled::domain::score::{rate, Rating};
use game_sokoban_rs_tangled::ports::level_source::{LevelSource, Par};
use game_sokoban_rs_tangled::ports::move_recorder::MoveRecorder;
use game_sokoban_rs_tangled::usecases::play::{progress, start, step};
use game_sokoban_rs_tangled::usecases::undo::{replay, undo};
use game_sokoban_rs_tangled::Game;

const ONE_PUSH: &str = "\
#####
#@$.#
#####";

const ROOM: &str = "\
#######
## . ##
#  $  #
#  @  #
#######";

fn level(text: &str) -> Level {
    Level::parse(text).expect("fixture level parses")
}

#[test]
fn parse_reads_the_conventional_charset() {
    let l = level(ONE_PUSH);
    assert_eq!((l.rows(), l.cols()), (3, 5));
    assert_eq!(l.start, Pos { row: 1, col: 1 });
    assert_eq!(l.boxes, vec![Pos { row: 1, col: 2 }]);
    assert_eq!(l.goals(), vec![Pos { row: 1, col: 3 }]);
    assert_eq!(l.tile(Pos { row: 0, col: 0 }), Tile::Wall);
}

#[test]
fn a_level_with_no_player_is_rejected() {
    assert_eq!(Level::parse("#####\n# $.#\n#####"), Err(LevelError::NoPlayer));
}

/// A puzzle that cannot be solved is an authoring mistake, caught at parse
/// rather than discovered by a player who can never win.
#[test]
fn boxes_and_goals_must_balance() {
    let two_goals_one_box = Level::parse("######\n#@$..#\n######");
    assert_eq!(two_goals_one_box, Err(LevelError::BoxesAndGoalsDiffer { boxes: 1, goals: 2 }));
    let two_boxes_one_goal = Level::parse("######\n#@$$.#\n######");
    assert_eq!(two_boxes_one_goal, Err(LevelError::BoxesAndGoalsDiffer { boxes: 2, goals: 1 }));
}

/// `*` is a box already on a goal and `+` is the player on one. Both count
/// toward the balance, and a level that starts solved is legal.
#[test]
fn a_box_on_a_goal_and_a_player_on_a_goal_parse() {
    // `+` is the player standing on a goal and `*` a box already on one.
    // Both cells count as goals, so this level is two boxes against two.
    let l = level("#####\n#+*$#\n#####");
    assert_eq!(l.start, Pos { row: 1, col: 1 });
    assert_eq!(l.boxes, vec![Pos { row: 1, col: 2 }, Pos { row: 1, col: 3 }]);
    assert_eq!(l.goals(), vec![Pos { row: 1, col: 1 }, Pos { row: 1, col: 2 }]);
    assert!(!Board::new(&l).solved(&l), "one box is still off a goal");
}

#[test]
fn walking_into_a_wall_is_blocked_and_changes_nothing() {
    let l = level(ONE_PUSH);
    let b = Board::new(&l);
    let out = apply(&l, &b, Dir::Left);
    assert_eq!(out, Outcome::Blocked);
    assert_eq!(commit(&b, &out), b);
}

#[test]
fn walking_onto_floor_moves_the_player_only() {
    let l = level(ROOM);
    let b = Board::new(&l);
    let out = apply(&l, &b, Dir::Left);
    assert_eq!(out, Outcome::Walked { to: Pos { row: 3, col: 2 } });
    let after = commit(&b, &out);
    assert_eq!(after.player, Pos { row: 3, col: 2 });
    assert_eq!(after.boxes, b.boxes);
}

#[test]
fn pushing_the_last_box_home_solves_the_puzzle() {
    let l = level(ONE_PUSH);
    let b = Board::new(&l);
    assert!(!b.solved(&l));
    let out = apply(&l, &b, Dir::Right);
    assert_eq!(
        out,
        Outcome::Pushed {
            to: Pos { row: 1, col: 2 },
            box_from: Pos { row: 1, col: 2 },
            box_to: Pos { row: 1, col: 3 },
        }
    );
    let after = commit(&b, &out);
    assert!(after.solved(&l));
    assert!(l.landed_home(&out));
}

/// Push the box right until it is against the far wall, then once more.
/// The last press has nowhere to put the box and must be blocked.
#[test]
fn a_box_against_a_wall_does_not_move() {
    let l = level("######\n#@$ .#\n######");
    let mut board = Board::new(&l);
    for _ in 0..2 {
        let out = apply(&l, &board, Dir::Right);
        assert!(matches!(out, Outcome::Pushed { .. }), "expected a push, got {out:?}");
        board = commit(&board, &out);
    }
    assert!(board.solved(&l), "two pushes put the box on the goal");
    assert_eq!(apply(&l, &board, Dir::Right), Outcome::Blocked, "the wall is beyond the box");
}

/// Two boxes in a line do not move together. This is the classic Sokoban
/// bug: allowing it makes dead puzzles solvable.
#[test]
fn a_box_never_pushes_another_box() {
    let l = level("########\n#@$$ ..#\n########");
    let b = Board::new(&l);
    assert_eq!(apply(&l, &b, Dir::Right), Outcome::Blocked);
}

/// The corner case `Dir::step` exists for: a level with no border wall.
/// A saturating step would make the edge its own neighbour and report a
/// move that did not happen.
#[test]
fn a_step_off_the_grid_is_none_not_a_clamp() {
    assert_eq!(Dir::Up.step(Pos { row: 0, col: 0 }, 3, 3), None);
    assert_eq!(Dir::Left.step(Pos { row: 0, col: 0 }, 3, 3), None);
    assert_eq!(Dir::Down.step(Pos { row: 2, col: 2 }, 3, 3), None);
    assert_eq!(Dir::Right.step(Pos { row: 2, col: 2 }, 3, 3), None);
    assert_eq!(Dir::Right.step(Pos { row: 1, col: 1 }, 3, 3), Some(Pos { row: 1, col: 2 }));
}

#[test]
fn a_session_plays_through_a_builtin_level() {
    let source = BuiltinLevels;
    assert_eq!(source.count(), 3);
    let mut session = start(&source, 0).expect("level 0 loads");
    let mut recorder = MemoryRecorder::default();
    let out = step(&mut session, &mut recorder, Dir::Right);
    assert!(matches!(out, Outcome::Pushed { .. }));
    let p = progress(&session);
    assert!(p.solved);
    assert_eq!(p.moves_taken, 1);
    assert_eq!(recorder.history(), vec![Dir::Right]);
}

#[test]
fn a_missing_level_is_an_error_not_a_panic() {
    let source = BuiltinLevels;
    assert!(start(&source, 99).is_err());
}

/// A blocked keypress is returned to the caller but never recorded, or an
/// undo would consume a move the player never made.
#[test]
fn a_blocked_move_is_not_recorded() {
    let source = BuiltinLevels;
    let mut session = start(&source, 0).expect("level 0 loads");
    let mut recorder = MemoryRecorder::default();
    assert_eq!(step(&mut session, &mut recorder, Dir::Left), Outcome::Blocked);
    assert!(recorder.history().is_empty());
    assert_eq!(progress(&session).moves_taken, 0);
}

#[test]
fn undo_steps_the_board_back() {
    let l = level(ROOM);
    let mut recorder = MemoryRecorder::default();
    recorder.record(Dir::Left);
    recorder.record(Dir::Up);
    let after_two = replay(&l, &recorder.history());
    let after_one = undo(&l, &mut recorder);
    assert_ne!(after_one, after_two);
    assert_eq!(after_one, replay(&l, &[Dir::Left]));
    assert_eq!(recorder.history(), vec![Dir::Left]);
}

/// Replay is the oracle undo is checked against, and it must agree with
/// stepping the same moves forward one at a time.
#[test]
fn replay_agrees_with_stepping_forward() {
    let l = level(ROOM);
    let moves = [Dir::Left, Dir::Up, Dir::Right, Dir::Up];
    let mut board = Board::new(&l);
    for d in moves {
        let out = apply(&l, &board, d);
        if out != Outcome::Blocked {
            board = commit(&board, &out);
        }
    }
    assert_eq!(board, replay(&l, &moves));
}

/// A rendered board parses back to the level it came from, which is the
/// cheapest check that the charset is written the way it is read.
#[test]
fn a_rendered_board_round_trips_through_parse() {
    let source = BuiltinLevels;
    let session = start(&source, 2).expect("level 2 loads");
    let text = console::render(&session.level, &progress(&session));
    let reparsed = Level::parse(&text).expect("a rendered board parses");
    assert_eq!(reparsed.start, session.level.start);
    assert_eq!(reparsed.boxes, session.level.boxes);
    assert_eq!(reparsed.goals(), session.level.goals());
}

#[test]
fn keys_map_to_directions_and_nothing_else_does() {
    assert_eq!(console::key('w'), Some(Dir::Up));
    assert_eq!(console::key('K'), Some(Dir::Up));
    assert_eq!(console::key('j'), Some(Dir::Down));
    assert_eq!(console::key('a'), Some(Dir::Left));
    assert_eq!(console::key('l'), Some(Dir::Right));
    assert_eq!(console::key('q'), None);
}

/// Blocked keypresses must not count against par, or a player fails a
/// level by bumping into a wall.
#[test]
fn par_counts_moves_taken_not_keys_pressed() {
    let taken = Outcome::Walked { to: Pos { row: 1, col: 1 } };
    let blocked = Outcome::Blocked;
    assert_eq!(rate(&[taken.clone(), blocked.clone(), blocked], Par { moves: 2 }), Rating::UnderPar);
    assert_eq!(rate(&[taken.clone(), taken.clone()], Par { moves: 2 }), Rating::AtPar);
    assert_eq!(rate(&[taken.clone(), taken.clone(), taken], Par { moves: 2 }), Rating::OverPar);
}

#[test]
fn the_composition_root_wires_a_playable_game() {
    let game = Game::new();
    assert_eq!(game.source.count(), 3);
    assert!(game.recorder.history().is_empty());
}
