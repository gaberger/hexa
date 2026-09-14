//! Minesweeper, in ports and adapters style.
//!
//! * `domain` holds the board and the rules. It imports nothing else.
//! * `ports` holds plain data and traits. No logic.
//! * `usecases` drives the game using the domain and the ports.
//! * `adapters` are the real screen, the real keyboard and the seeded dice.
//!
//! This file is the composition root. It is the **only** file that plugs an
//! adapter into a use case, and the only file that names one. `src/main.rs`
//! calls [`run`] and does nothing else.

pub mod adapters;
pub mod domain;
pub mod ports;
pub mod usecases;

pub use cli::run;

/// The wiring. It lives inside `lib.rs` so that no second file has to import
/// an adapter.
///
/// The lints below are denied here for the same reason they are denied in the
/// domain: the game must stop because the player asked, never because the
/// program fell over.
mod cli {
    #![deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects
    )]

    use std::io::{self, Write};

    use crate::adapters::primary::{
        Policy, QuietRenderer, SolverInput, StdinInput, TerminalRenderer,
    };
    use crate::adapters::secondary::SeededRng;
    use crate::domain::{ConfigError, MAX_CELLS};
    use crate::usecases::{new_game, run_game, GameConfig, GameError};

    const USAGE: &str = "\
minesweeper - Minesweeper for the terminal

USAGE:
    minesweeper [OPTIONS]

OPTIONS:
    --demo                     play a scripted game and print only the result
    --seed <n>                 set the seed (works with and without --demo)
    --policy <deduce|reckless> the demo player to use        [default: deduce]
    --width <n>                board width                   [default: 9]
    --height <n>               board height                  [default: 9]
    --mines <n>                number of mines               [default: 10]
    -h, --help                 show this text

LIMITS:
    width x height must be {MAX} cells or fewer.
    Leave room for the first click: mines must be at most (cells - 9).

With no options the game is interactive. Type 'r <col> <row>' to reveal,
'f <col> <row>' to flag, and 'q' to quit. Columns and rows start at 0, and
0 0 is the top left cell.";

    /// The help text, with the real size cap written into it.
    fn usage_text() -> String {
        USAGE.replace("{MAX}", &MAX_CELLS.to_string())
    }

    #[derive(Debug)]
    struct Args {
        demo: bool,
        help: bool,
        seed: Option<u64>,
        policy: Policy,
        width: u16,
        height: u16,
        mines: usize,
    }

    impl Default for Args {
        fn default() -> Args {
            Args {
                demo: false,
                help: false,
                seed: None,
                policy: Policy::Deduce,
                width: 9,
                height: 9,
                mines: 10,
            }
        }
    }

    fn parse_args<I: Iterator<Item = String>>(mut it: I) -> Result<Args, &'static str> {
        let mut args = Args::default();
        while let Some(a) = it.next() {
            match a.as_str() {
                "--demo" => args.demo = true,
                "-h" | "--help" => args.help = true,
                "--seed" => {
                    let v = it.next().ok_or("--seed needs a whole number")?;
                    args.seed = Some(v.parse::<u64>().map_err(|_| "--seed needs a whole number")?);
                }
                "--policy" => {
                    let v = it.next().ok_or("--policy needs deduce or reckless")?;
                    args.policy = match v.as_str() {
                        "deduce" => Policy::Deduce,
                        "reckless" => Policy::Reckless,
                        _ => return Err("--policy needs deduce or reckless"),
                    };
                }
                "--width" => {
                    let v = it.next().ok_or("--width needs a whole number")?;
                    args.width = v.parse::<u16>().map_err(|_| "--width needs a whole number")?;
                }
                "--height" => {
                    let v = it.next().ok_or("--height needs a whole number")?;
                    args.height = v
                        .parse::<u16>()
                        .map_err(|_| "--height needs a whole number")?;
                }
                "--mines" => {
                    let v = it.next().ok_or("--mines needs a whole number")?;
                    args.mines = v
                        .parse::<usize>()
                        .map_err(|_| "--mines needs a whole number")?;
                }
                _ => return Err("unknown option; run with --help"),
            }
        }
        Ok(args)
    }

    /// Say why the board cannot exist. The size message names the real cap, so
    /// the player learns the rule instead of guessing at it.
    fn config_text(e: ConfigError) -> String {
        match e {
            ConfigError::ZeroWidth => "--width must be 1 or more".to_string(),
            ConfigError::ZeroHeight => "--height must be 1 or more".to_string(),
            ConfigError::TooLarge => {
                format!("that board is too large; the cap is {MAX_CELLS} cells")
            }
            ConfigError::TooManyMines => {
                "too many mines for that board; leave room for 9 cells".to_string()
            }
        }
    }

    /// Say one line on the error stream. It never panics, even if the stream is gone.
    fn say_err(msg: &str) {
        let mut e = io::stderr();
        let _ = e.write_all(msg.as_bytes());
        let _ = e.write_all(b"\n");
        let _ = e.flush();
    }

    fn say_out(msg: &str) {
        let mut o = io::stdout();
        let _ = o.write_all(msg.as_bytes());
        let _ = o.write_all(b"\n");
        let _ = o.flush();
    }

    fn clock_seed() -> u64 {
        match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => d.as_nanos() as u64,
            Err(_) => 0x2545_F491_4F6C_DD1D,
        }
    }

    /// Build the game from the options, play it, and give back the exit code.
    ///
    /// `0` a finished game · `2` a bad option · `3` an input or output failure.
    pub fn run<I: Iterator<Item = String>>(argv: I) -> i32 {
        let args = match parse_args(argv) {
            Ok(a) => a,
            Err(m) => {
                say_err(m);
                return 2;
            }
        };
        if args.help {
            say_out(&usage_text());
            return 0;
        }

        let cfg = GameConfig {
            width: args.width,
            height: args.height,
            mine_count: args.mines,
        };
        let mut session = match new_game(cfg) {
            Ok(s) => s,
            Err(e) => {
                say_err(&config_text(e));
                return 2;
            }
        };

        let mut rng = SeededRng::new(args.seed.unwrap_or_else(clock_seed));
        let stdout = io::stdout();

        let result = if args.demo {
            let mut renderer = QuietRenderer::new(stdout.lock());
            let mut input = SolverInput::new(args.policy);
            run_game(&mut session, &mut renderer, &mut input, &mut rng)
        } else {
            let stdin = io::stdin();
            let mut renderer = TerminalRenderer::new(stdout.lock());
            let mut input = StdinInput::new(stdin.lock());
            run_game(&mut session, &mut renderer, &mut input, &mut rng)
        };

        match result {
            Ok(_) => 0,
            // The reader went away. End quietly; this is not a fault.
            Err(GameError::Io(e)) if e.kind() == io::ErrorKind::BrokenPipe => 0,
            Err(GameError::Io(e)) => {
                say_err(&format!("input or output failed: {e}"));
                3
            }
            Err(GameError::Fault(f)) => {
                say_err(&format!("internal fault: {f:?}"));
                3
            }
        }
    }
}
