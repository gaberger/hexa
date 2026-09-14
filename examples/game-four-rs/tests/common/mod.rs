//! Shared test helpers.
//!
//! The win checker in here is an **independent oracle**. It does not walk out
//! from the new disc the way the game does. It scans every one of the 69
//! four-in-a-row windows on the board, every time, with no cleverness at all.
//! Two roads to the same answer cannot share one misunderstanding.

#![allow(dead_code)]

use connect_four::domain::{BoardView, Disc, Game, Outcome, COLUMNS, ROWS};

/// Every four-in-a-row window on a 7x6 board: 24 across, 21 up, 12 and 12
/// diagonal. Sixty-nine in total.
pub fn windows() -> Vec<[(usize, usize); 4]> {
    let mut found = Vec::new();
    for column in 0..COLUMNS {
        for row in 0..ROWS {
            for (dc, dr) in [(1_i32, 0_i32), (0, 1), (1, 1), (1, -1)] {
                let mut cells = [(0_usize, 0_usize); 4];
                let mut fits = true;
                for (step, cell) in cells.iter_mut().enumerate() {
                    let c = column as i32 + dc * step as i32;
                    let r = row as i32 + dr * step as i32;
                    if c < 0 || c >= COLUMNS as i32 || r < 0 || r >= ROWS as i32 {
                        fits = false;
                        break;
                    }
                    *cell = (c as usize, r as usize);
                }
                if fits {
                    found.push(cells);
                }
            }
        }
    }
    found
}

/// The oracle's verdict on a board, reached by brute force.
pub fn oracle_outcome(view: &BoardView) -> Outcome {
    for window in windows() {
        let first = view.at(window[0].0, window[0].1);
        if first.is_none() {
            continue;
        }
        if window.iter().all(|(c, r)| view.at(*c, *r) == first) {
            return Outcome::Win(first.expect("checked above"));
        }
    }
    if view.disc_count() == COLUMNS * ROWS {
        Outcome::Draw
    } else {
        Outcome::InProgress
    }
}

/// Turn six printed lines, top row first, into a board we can question.
pub fn view_from_lines(lines: &[&str]) -> BoardView {
    assert_eq!(lines.len(), ROWS, "a frame is six lines");
    let mut cells = [None; COLUMNS * ROWS];
    for (offset, line) in lines.iter().enumerate() {
        let glyphs: Vec<char> = line.chars().collect();
        assert_eq!(glyphs.len(), COLUMNS, "a line is seven characters: {line:?}");
        // The first printed line is the top row, which is row 5.
        let row = ROWS - 1 - offset;
        for (column, glyph) in glyphs.iter().enumerate() {
            cells[row * COLUMNS + column] = match glyph {
                'R' => Some(Disc::Red),
                'Y' => Some(Disc::Yellow),
                '.' => None,
                other => panic!("a frame may not hold {other:?}"),
            };
        }
    }
    BoardView { cells }
}

/// Print a board the way the contract says: six lines, top row first.
pub fn lines_from_view(view: &BoardView) -> String {
    let mut text = String::new();
    for row in (0..ROWS).rev() {
        for column in 0..COLUMNS {
            text.push(match view.at(column, row) {
                Some(Disc::Red) => 'R',
                Some(Disc::Yellow) => 'Y',
                None => '.',
            });
        }
        text.push('\n');
    }
    text
}

/// The three words the program ends with.
pub fn result_words(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Win(Disc::Red) => "RED WINS",
        Outcome::Win(Disc::Yellow) => "YELLOW WINS",
        Outcome::Draw => "DRAW",
        Outcome::InProgress => "STILL PLAYING",
    }
}

/// A small generator for the tests, so a test never needs a clock or a crate.
pub struct TestRng {
    state: u64,
}

impl TestRng {
    pub fn new(seed: u64) -> TestRng {
        TestRng { state: seed }
    }

    pub fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// Play a whole random game, calling `after_move` once per disc.
pub fn play_random_game<F: FnMut(&Game)>(rng: &mut TestRng, mut after_move: F) -> Game {
    let mut game = Game::new();
    while !game.outcome().is_final() {
        let legal = game.legal_moves();
        assert!(!legal.is_empty(), "a game in progress always has a move");
        let pick = (rng.next() % legal.len() as u64) as usize;
        let column = legal.iterate().nth(pick).expect("the pick is in range");
        game.drop(column).expect("a legal column is legal");
        after_move(&game);
    }
    game
}

/// Check the four promises of the spec against a board, from outside.
pub fn check_invariants(game: &Game) {
    let view = game.view();
    let mut red = 0;
    let mut yellow = 0;
    for column in 0..COLUMNS {
        let mut hole_below = false;
        for row in 0..ROWS {
            match view.at(column, row) {
                Some(disc) => {
                    assert!(!hole_below, "column {column} has a floating disc");
                    match disc {
                        Disc::Red => red += 1,
                        Disc::Yellow => yellow += 1,
                    }
                }
                None => hole_below = true,
            }
        }
    }
    let total = red + yellow;
    assert!(total <= COLUMNS * ROWS, "more than 42 discs");
    assert_eq!(total, game.move_count(), "discs and moves disagree");
    assert!(
        red == yellow || red == yellow + 1,
        "out of turn: {red} red, {yellow} yellow"
    );
}

/// A full board with no line of four, found once by search and frozen here.
/// Two random players almost never draw, so without this list the draw rule
/// ships with no coverage at all.
pub const DRAW_MOVES: [u8; 42] = [
    0, 6, 1, 2, 1, 6, 5, 1, 3, 5, 6, 6, 1, 2, 2, 3, 5, 3, 1, 6, 5, 1, 0, 3, 5, 5, 0, 0, 2, 0, 3, 0,
    4, 2, 4, 2, 4, 4, 6, 4, 3, 4,
];

/// The board the draw transcript ends on, top row first.
pub const DRAW_BOARD: [&str; 6] = [
    "YYYRYYR", "YRYRYRY", "YRRYYRY", "RYRYRRR", "RRYYRYY", "RRYRRRY",
];

/// A full board whose forty-second disc completes a line. Yellow fills column
/// 3 from row 2 to row 5, and the last disc is the top one.
pub const WIN42_MOVES: [u8; 42] = [
    3, 6, 2, 1, 5, 6, 0, 4, 6, 6, 3, 3, 4, 3, 0, 2, 2, 1, 2, 3, 6, 6, 4, 1, 0, 0, 4, 0, 5, 0, 1, 4,
    2, 2, 5, 5, 4, 5, 1, 5, 1, 3,
];

// ---------------------------------------------------------------------------
// Test doubles for the two ports.
// ---------------------------------------------------------------------------

use connect_four::adapters::secondary::SeededChooser;
use connect_four::domain::{Column, LegalMoves};
use connect_four::ports::{Choice, InputError, InputSource, RenderError, Renderer, Request};

/// A screen made of a string.
pub struct Transcript {
    pub text: String,
}

impl Transcript {
    pub fn new() -> Transcript {
        Transcript {
            text: String::new(),
        }
    }
}

impl Renderer for Transcript {
    fn frame(&mut self, view: &BoardView) -> Result<(), RenderError> {
        self.text.push_str(&lines_from_view(view));
        Ok(())
    }

    fn announce(&mut self, outcome: Outcome) -> Result<(), RenderError> {
        self.text.push_str(result_words(outcome));
        self.text.push('\n');
        Ok(())
    }
}

/// A player who reads moves from a list and quits when the list runs out.
pub struct ScriptedInput {
    pub moves: Vec<u8>,
    pub next: usize,
}

impl ScriptedInput {
    pub fn new(moves: &[u8]) -> ScriptedInput {
        ScriptedInput {
            moves: moves.to_vec(),
            next: 0,
        }
    }
}

impl InputSource for ScriptedInput {
    fn choose(
        &mut self,
        _view: &BoardView,
        _legal: &LegalMoves,
        _to_move: Disc,
    ) -> Result<Choice, InputError> {
        match self.moves.get(self.next) {
            Some(raw) => {
                self.next += 1;
                Column::new(*raw)
                    .map(Choice::Play)
                    .map_err(|_| InputError::Unreadable)
            }
            None => Ok(Choice::Quit),
        }
    }
}

/// Play one demo game in this process, exactly as the binary does, and give
/// back everything that would have reached the screen.
pub fn play_seed(seed: u64) -> String {
    let mut out = Transcript::new();
    let mut chooser = SeededChooser::new(seed);
    connect_four::run(Request::Demo { seed }, &mut out, &mut chooser);
    out.text
}
