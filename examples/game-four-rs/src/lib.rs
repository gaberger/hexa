//! Connect Four, in ports-and-adapters style.
//!
//! * `domain` holds the board and the rules. It imports nothing else.
//! * `ports` holds plain data and traits. No logic.
//! * `usecases` drives one turn using the domain and the ports.
//! * `adapters` are the real screen, the real keyboard and the seeded chooser.
//!
//! This file is the composition root. It is the **only** file that names an
//! adapter. `src/main.rs` calls [`main_with_args`] and does nothing else.

pub mod adapters;
pub mod domain;
pub mod ports;
pub mod usecases;

use std::io::{self, BufReader};
use std::process::ExitCode;

use adapters::primary::{parse, HumanRenderer, StdinInput, StrictRenderer};
use adapters::secondary::SeededChooser;
use domain::{BoardView, Disc, Game, LegalMoves};
use ports::{Choice, InputError, InputSource, Renderer, Request};
use usecases::{play_turn, TurnError, TurnResult};

/// A game cannot last longer than the board.
const MOST_MOVES: usize = 42;

/// Exit 0: the game finished, with any result, or the player quit.
const OK: u8 = 0;
/// Exit 1: a write failed.
const WRITE_FAILED: u8 = 1;
/// Exit 2: the command line was wrong.
const BAD_USAGE: u8 = 2;

/// Play one whole game.
///
/// The exit code does **not** follow the winner. A yellow win is still exit 0.
/// A code that followed the winner would fail the gate on half the seeds.
pub fn run(request: Request, out: &mut dyn Renderer, input: &mut dyn InputSource) -> ExitCode {
    let mut game = Game::new();

    // A person likes to see the empty board before their first move. A machine
    // must not: the contract says one frame per move and no frame before them.
    if matches!(request, Request::Interactive { .. }) && out.frame(&game.view()).is_err() {
        return ExitCode::from(WRITE_FAILED);
    }

    for _ in 0..MOST_MOVES {
        match play_turn(&mut game, input, out) {
            Ok(TurnResult::Continued) => {}
            Ok(TurnResult::Quit) => return ExitCode::from(OK),
            Ok(TurnResult::Finished(outcome)) => {
                if out.announce(outcome).is_err() {
                    return ExitCode::from(WRITE_FAILED);
                }
                return ExitCode::from(OK);
            }
            Err(TurnError::Render(_)) => return ExitCode::from(WRITE_FAILED),
            Err(other) => {
                eprintln!("{other}");
                return ExitCode::from(WRITE_FAILED);
            }
        }
    }

    // Forty-two discs always end the game, so this line never runs.
    eprintln!("the game did not end in 42 moves");
    ExitCode::from(WRITE_FAILED)
}

/// Read the command line, plug the real parts together, and play.
///
/// `args` still holds the program name in slot 0, exactly as
/// `std::env::args()` gives it.
pub fn main_with_args(args: Vec<String>) -> ExitCode {
    let request = match parse(&args[1..]) {
        Ok(request) => request,
        Err(error) => {
            eprintln!("{}", error.message());
            return ExitCode::from(BAD_USAGE);
        }
    };

    match request {
        Request::Demo { seed } => {
            let mut out = StrictRenderer::new(io::stdout().lock());
            let mut both = SeededChooser::new(seed);
            run(request, &mut out, &mut both)
        }
        Request::Interactive { seed } => {
            let mut out = HumanRenderer::new(io::stdout().lock());
            let mut table = Duet {
                red: StdinInput::new(BufReader::new(io::stdin().lock()), io::stdout()),
                yellow: SeededChooser::new(seed),
            };
            run(request, &mut out, &mut table)
        }
    }
}

/// Two players at one board.
///
/// This wrapper lives here because it is wiring. Putting it in an adapter
/// would make that adapter import two other adapters, which the boundary
/// forbids.
struct Duet<R: InputSource, Y: InputSource> {
    red: R,
    yellow: Y,
}

impl<R: InputSource, Y: InputSource> InputSource for Duet<R, Y> {
    fn choose(
        &mut self,
        view: &BoardView,
        legal: &LegalMoves,
        to_move: Disc,
    ) -> Result<Choice, InputError> {
        match to_move {
            Disc::Red => self.red.choose(view, legal, to_move),
            Disc::Yellow => self.yellow.choose(view, legal, to_move),
        }
    }
}
