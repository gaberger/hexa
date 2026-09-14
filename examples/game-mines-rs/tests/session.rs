//! H. The session, the flags you place first, and the replay.
//!
//! The mines are placed **after** your first click. So a game is decided by two
//! things, not one: the seed and the first cell. These tests hold that rule,
//! hold the flags you put down before any board exists, and prove that a
//! generator which cannot give a number never reaches the board.

use std::io;

use minesweeper::adapters::secondary::SeededRng;
use minesweeper::domain::{CellState, PlacementError, RollError};
use minesweeper::ports::{BoardView, Command, Glyph, Notice, RandomSource, Renderer, RollFault};
use minesweeper::usecases::{
    new_game, project_view, run_game, take_command, Fault, GameConfig, GameError, Step,
};

fn config(w: u16, h: u16, mines: usize) -> GameConfig {
    GameConfig {
        width: w,
        height: h,
        mine_count: mines,
    }
}

/// A screen that keeps every message instead of drawing it. It lets a test ask
/// what the player was told, and what the player was never told.
#[derive(Debug, Default)]
struct Recorder {
    notices: Vec<Notice>,
    frames: usize,
}

impl Recorder {
    fn saw_an_ending(&self) -> bool {
        self.notices.iter().any(|n| {
            matches!(n, Notice::GameOver) || matches!(n, Notice::YouWin) || matches!(n, Notice::Quit)
        })
    }
}

impl Renderer for Recorder {
    fn render(&mut self, _view: &BoardView) -> io::Result<()> {
        self.frames += 1;
        Ok(())
    }

    fn notice(&mut self, notice: Notice) -> io::Result<()> {
        self.notices.push(notice);
        Ok(())
    }
}

/// A generator that can never give a number. Every draw is stuck.
#[derive(Debug, Default)]
struct StuckRng {
    asked: usize,
}

impl RandomSource for StuckRng {
    fn next_below(&mut self, _bound: u32) -> Result<u32, RollFault> {
        self.asked += 1;
        Err(RollFault::Stuck)
    }
}

/// An input source that hands back a fixed list, then ends.
#[derive(Debug)]
struct Script {
    steps: Vec<Command>,
    at: usize,
}

impl Script {
    fn new(steps: Vec<Command>) -> Script {
        Script { steps, at: 0 }
    }
}

impl minesweeper::ports::InputSource for Script {
    fn next(&mut self, _view: &BoardView) -> io::Result<Option<Command>> {
        let out = self.steps.get(self.at).copied();
        self.at += 1;
        Ok(out)
    }
}

/// 53. A flag put down before the first click is still there afterwards.
///
/// The flag is held in the session, because no board exists yet. When the first
/// click builds the board, every such flag must be carried over. A design that
/// forgets them loses your work without a word.
#[test]
fn a_flag_placed_before_the_first_click_survives() {
    let mut s = new_game(config(9, 9, 10)).expect("new game");
    let mut rng = SeededRng::new(5);

    assert!(s.board().is_none(), "no board may exist before the first click");
    assert_eq!(
        take_command(&mut s, Command::Flag { x: 8, y: 8 }, &mut rng),
        Step::Applied
    );
    // The flag shows through the window even with no board behind it.
    let before = project_view(&s);
    assert_eq!(before.flags_placed, 1);
    assert_eq!(before.glyphs.get(80), Some(&Glyph::Flag));

    // Now click a different cell. This is what builds the board.
    let step = take_command(&mut s, Command::Reveal { x: 0, y: 0 }, &mut rng);
    assert!(
        matches!(step, Step::Applied | Step::Ended(_)),
        "the first click was not applied: {step:?}"
    );

    let board = s.board().expect("the first click must build the board");
    let flagged = board
        .dims()
        .coord(8, 8)
        .expect("cell 8 8");
    assert_eq!(
        board.cell_state(flagged),
        CellState::Flagged,
        "the flag did not survive the first click"
    );
    assert_eq!(board.flags_placed(), 1, "the flag count is wrong");

    let after = project_view(&s);
    assert_eq!(after.flags_placed, 1);
    assert_eq!(after.glyphs.get(80), Some(&Glyph::Flag));
}

/// 54. A first click on a cell you already flagged is refused.
///
/// It must place no mines. A refusal leaves the game exactly as it was.
#[test]
fn the_first_click_on_a_pre_flagged_cell_is_refused() {
    let mut s = new_game(config(9, 9, 10)).expect("new game");
    let mut rng = SeededRng::new(5);

    assert_eq!(
        take_command(&mut s, Command::Flag { x: 3, y: 3 }, &mut rng),
        Step::Applied
    );
    let step = take_command(&mut s, Command::Reveal { x: 3, y: 3 }, &mut rng);
    assert_eq!(step, Step::Refused(Notice::CellIsFlagged));
    assert!(
        s.board().is_none(),
        "a refused first click must not place any mines"
    );
    assert_eq!(s.fingerprint(), [0u8; 8], "a refusal built a board");

    // Take the flag off, and the same click now works.
    assert_eq!(
        take_command(&mut s, Command::Flag { x: 3, y: 3 }, &mut rng),
        Step::Applied
    );
    let step = take_command(&mut s, Command::Reveal { x: 3, y: 3 }, &mut rng);
    assert!(
        matches!(step, Step::Applied | Step::Ended(_)),
        "the click was still refused: {step:?}"
    );
    assert!(s.board().is_some());
}

/// 55. The same commands on the same seed give the same game, twice.
///
/// This is the replay check. It survives without a save file, because the game
/// is a pure function of the seed and the commands.
#[test]
fn the_same_commands_give_the_same_board_twice() {
    let script: Vec<Command> = vec![
        Command::Flag { x: 8, y: 0 },
        Command::Reveal { x: 4, y: 4 },
        Command::Flag { x: 0, y: 8 },
        Command::Reveal { x: 0, y: 0 },
        Command::Reveal { x: 8, y: 8 },
        Command::Flag { x: 0, y: 8 },
        Command::Reveal { x: 2, y: 6 },
    ];

    let play = || {
        let mut s = new_game(config(9, 9, 10)).expect("new game");
        let mut rng = SeededRng::new(5);
        let mut faults = 0usize;
        for cmd in &script {
            if let Step::Fault(_) = take_command(&mut s, *cmd, &mut rng) {
                faults += 1;
            }
        }
        assert_eq!(faults, 0, "the game faulted during a replay");
        (s.fingerprint(), project_view(&s))
    };

    let (fp_a, view_a) = play();
    let (fp_b, view_b) = play();

    assert_eq!(fp_a, fp_b, "the same commands gave two different boards");
    assert_ne!(fp_a, [0u8; 8], "no board was ever built");
    assert_eq!(view_a, view_b, "the same commands gave two different games");

    // A different seed with the same commands must give a different board.
    let other = {
        let mut s = new_game(config(9, 9, 10)).expect("new game");
        let mut rng = SeededRng::new(6);
        for cmd in &script {
            let _ = take_command(&mut s, *cmd, &mut rng);
        }
        s.fingerprint()
    };
    assert_ne!(fp_a, other, "the seed stopped deciding the board");
}

/// 56. A stuck generator never reaches the board, and never ends the game.
///
/// A fault is not a refusal and it is not an ending. The player must not be
/// told "GAME OVER" because the program broke.
#[test]
fn a_stuck_generator_never_reaches_the_board() {
    let mut s = new_game(config(9, 9, 10)).expect("new game");
    let mut rng = StuckRng::default();

    let step = take_command(&mut s, Command::Reveal { x: 4, y: 4 }, &mut rng);
    assert_eq!(
        step,
        Step::Fault(Fault::Placement(PlacementError::Roll(RollError::Stuck))),
        "a stuck generator must give a fault"
    );
    assert!(rng.asked > 0, "the generator was never asked");
    assert!(s.board().is_none(), "a fault still built a board");
    assert_eq!(s.fingerprint(), [0u8; 8]);

    // The same trouble, seen through the whole loop. No ending is printed.
    let mut s = new_game(config(9, 9, 10)).expect("new game");
    let mut rng = StuckRng::default();
    let mut screen = Recorder::default();
    let mut input = Script::new(vec![Command::Reveal { x: 4, y: 4 }]);
    let out = run_game(&mut s, &mut screen, &mut input, &mut rng);

    match out {
        Err(GameError::Fault(Fault::Placement(PlacementError::Roll(RollError::Stuck)))) => {}
        other => panic!("the loop did not report the fault: {other:?}"),
    }
    assert!(
        !screen.saw_an_ending(),
        "the player was told the game ended: {:?}",
        screen.notices
    );
    assert!(s.board().is_none(), "a fault still built a board");
}
