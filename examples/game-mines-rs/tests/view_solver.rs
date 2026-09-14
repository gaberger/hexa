//! E. The window and the demo player.
//!
//! The demo player is handed a `BoardView`. That type does not hold the mines,
//! so the player cannot read them. These tests check the window really is
//! closed while you play, and that the player really can lose.

use minesweeper::adapters::primary::{Policy, QuietRenderer, SolverInput};
use minesweeper::adapters::secondary::SeededRng;
use minesweeper::ports::{BoardView, Command, Glyph, InputSource, Phase};
use minesweeper::usecases::{
    new_game, project_view, run_game, take_command, EndReason, GameConfig, Step,
};

fn config(mines: usize) -> GameConfig {
    GameConfig {
        width: 9,
        height: 9,
        mine_count: mines,
    }
}

/// Play one whole game with the demo player and report how it ended.
fn play(seed: u64, policy: Policy, mines: usize) -> EndReason {
    let mut s = new_game(config(mines)).expect("new game");
    let mut r = QuietRenderer::new(Vec::new());
    let mut i = SolverInput::new(policy);
    let mut rng = SeededRng::new(seed);
    run_game(&mut s, &mut r, &mut i, &mut rng).expect("the game must finish")
}

/// 25. While you play, no mine shows through the window. This checks the data,
///     not the printed text, so it cannot pass by accident on an empty board.
#[test]
fn a_game_in_play_never_shows_a_mine() {
    for seed in 0..50u64 {
        let mut s = new_game(config(10)).expect("new game");
        let mut rng = SeededRng::new(seed);
        assert_eq!(
            take_command(&mut s, Command::Reveal { x: 0, y: 0 }, &mut rng),
            Step::Applied
        );
        // Open a few more cells, so the board really is half open.
        for (x, y) in [(8u32, 8u32), (4, 4), (8, 0), (0, 8)] {
            let _ = take_command(&mut s, Command::Reveal { x, y }, &mut rng);
            if s.board().map(|b| b.blast().is_some()) == Some(true) {
                break;
            }
        }
        let view = project_view(&s);
        if view.phase != Phase::Playing {
            continue;
        }
        let mines = view
            .glyphs
            .iter()
            .filter(|g| matches!(g, Glyph::Mine | Glyph::Blast))
            .count();
        assert_eq!(mines, 0, "a mine leaked through the window, seed {seed}");
        assert!(
            view.glyphs.contains(&Glyph::Hidden),
            "the board is not half open, seed {seed}"
        );
    }
}

/// 26. A lost game shows every mine, and exactly one of them is the one you
///     stepped on.
#[test]
fn a_lost_game_shows_the_whole_answer() {
    let mut s = new_game(config(10)).expect("new game");
    let mut rng = SeededRng::new(11);
    take_command(&mut s, Command::Reveal { x: 0, y: 0 }, &mut rng);

    // Find a mine through the board, then step on it.
    let dims = s.dims();
    let mine = (0..dims.total())
        .filter_map(|i| dims.from_index(i))
        .find(|c| s.board().map(|b| b.is_mine(*c)) == Some(true))
        .expect("the board has mines");
    let step = take_command(
        &mut s,
        Command::Reveal {
            x: u32::from(mine.x()),
            y: u32::from(mine.y()),
        },
        &mut rng,
    );
    assert_eq!(step, Step::Ended(EndReason::Lost));

    let view = project_view(&s);
    assert_eq!(view.phase, Phase::Lost);
    let blasts = view.glyphs.iter().filter(|g| **g == Glyph::Blast).count();
    assert_eq!(blasts, 1, "exactly one mine was stepped on");

    for i in 0..dims.total() {
        let c = dims.from_index(i).expect("cell");
        let is_mine = s.board().map(|b| b.is_mine(c)) == Some(true);
        let g = view.glyphs.get(i).copied().expect("glyph");
        if is_mine {
            assert!(
                g == Glyph::Mine || g == Glyph::Blast,
                "a mine was still hidden after the loss, slot {i}"
            );
        } else {
            assert!(
                g != Glyph::Mine && g != Glyph::Blast,
                "a safe cell was shown as a mine, slot {i}"
            );
        }
        if g == Glyph::Blast {
            assert_eq!(Some(c), s.board().and_then(|b| b.blast()));
        }
    }
}

/// 27. The thinking player loses sometimes. A player that never loses is
///     reading the mines.
#[test]
fn the_thinking_player_can_lose() {
    let mut wins = 0usize;
    let mut losses = 0usize;
    for seed in 0..200u64 {
        match play(seed, Policy::Deduce, 10) {
            EndReason::Won => wins += 1,
            EndReason::Lost => losses += 1,
            EndReason::Quit => panic!("the player quit, seed {seed}"),
        }
    }
    assert!(losses >= 1, "the player never lost in 200 games; it is cheating");
    assert!(wins >= 1, "the player never won in 200 games; it cannot play");
}

/// 28. Rule A fires when the count minus the flags equals the hidden cells.
///
/// The subtraction is the whole point. Without it the player misses this
/// certain move and guesses instead. The second half of the test checks the
/// other direction: without the subtraction the player would flag a cell it
/// cannot be sure about.
#[test]
fn rule_a_uses_the_subtraction() {
    fn view_with_centre(count: u8) -> BoardView {
        // A 5x5 window. Everything is open and shows nothing, except the
        // middle cell, one flag beside it, and two cells still hidden.
        let mut glyphs = vec![Glyph::Empty; 25];
        let put = |g: &mut Vec<Glyph>, x: usize, y: usize, v: Glyph| {
            if let Some(slot) = g.get_mut(y * 5 + x) {
                *slot = v;
            }
        };
        put(&mut glyphs, 2, 2, Glyph::Count(count));
        put(&mut glyphs, 1, 1, Glyph::Flag);
        put(&mut glyphs, 1, 2, Glyph::Hidden);
        put(&mut glyphs, 1, 3, Glyph::Hidden);
        BoardView {
            width: 5,
            height: 5,
            glyphs,
            mines_total: 3,
            flags_placed: 1,
            revealed_safe: 21,
            phase: Phase::Playing,
        }
    }

    // The middle shows 3. One flag is already down, two cells are hidden.
    // 3 - 1 = 2, and there are 2 hidden cells. Both are mines. Flag one.
    let mut player = SolverInput::new(Policy::Deduce);
    let got = player
        .next(&view_with_centre(3))
        .expect("no input error")
        .expect("a move");
    assert_eq!(got, Command::Flag { x: 1, y: 2 }, "rule A must flag, not guess");

    // The middle shows 2. One flag is down, two cells are hidden. Only one of
    // them is a mine, and the player cannot tell which. It must not flag.
    let mut player = SolverInput::new(Policy::Deduce);
    let got = player
        .next(&view_with_centre(2))
        .expect("no input error")
        .expect("a move");
    assert_eq!(
        got,
        Command::Reveal { x: 1, y: 2 },
        "without the subtraction the player would flag a cell it cannot be sure about"
    );
}

/// 29. The reckless player loses every time, on every seed.
#[test]
fn the_reckless_player_always_loses() {
    for seed in 0..20u64 {
        assert_eq!(
            play(seed, Policy::Reckless, 10),
            EndReason::Lost,
            "the reckless player must lose, seed {seed}"
        );
    }
}

/// The demo player never runs for ever. It stops well inside its own cap.
#[test]
fn the_demo_player_always_stops() {
    for seed in 0..20u64 {
        let mut s = new_game(config(10)).expect("new game");
        let mut r = QuietRenderer::new(Vec::new());
        let mut i = SolverInput::new(Policy::Deduce);
        let mut rng = SeededRng::new(seed);
        let out = run_game(&mut s, &mut r, &mut i, &mut rng);
        assert!(out.is_ok(), "the game did not finish, seed {seed}");
        assert!(i.rounds() <= 81 * 4, "too many rounds on seed {seed}");
    }
}

/// 58. The round cap is a failure, not an ending.
///
/// A gate that hangs is bad. A gate that turns a hang into "GAME OVER" is
/// worse, because it lies. So the cap must give an error, and the loop must
/// carry that error out without printing any ending.
#[test]
fn the_round_cap_is_a_failure_not_an_ending() {
    use std::io;

    use minesweeper::ports::{InputSource as _, Notice, Renderer};

    // A screen that keeps every message instead of drawing it.
    #[derive(Default)]
    struct Recorder {
        notices: Vec<Notice>,
    }
    impl Renderer for Recorder {
        fn render(&mut self, _v: &BoardView) -> io::Result<()> {
            Ok(())
        }
        fn notice(&mut self, n: Notice) -> io::Result<()> {
            self.notices.push(n);
            Ok(())
        }
    }

    // Part one. Hand the player the same frame for ever, so it never makes
    // progress. The round count climbs, and the cap must stop it.
    let mut s = new_game(config(10)).expect("new game");
    let mut rng = SeededRng::new(5);
    assert!(matches!(
        take_command(&mut s, Command::Reveal { x: 4, y: 4 }, &mut rng),
        Step::Applied | Step::Ended(_)
    ));
    let frozen = project_view(&s);
    let cap = frozen.glyphs.len() * 4 + 8;

    let mut player = SolverInput::new(Policy::Deduce);
    for round in 1..=cap {
        let out = player.next(&frozen).unwrap_or_else(|e| {
            panic!("the player gave up at round {round}, before the cap: {e}")
        });
        assert!(out.is_some(), "the player quit at round {round}");
    }
    let over = player.next(&frozen);
    assert!(
        over.is_err(),
        "past the cap the player must fail, not quit: {over:?}"
    );
    assert_eq!(player.rounds(), cap + 1);

    // Part two. The same failure, seen through the whole loop. The loop must
    // carry it out, and must print no ending on the way.
    struct Capped {
        rounds: usize,
        cap: usize,
    }
    impl InputSource for Capped {
        fn next(&mut self, _v: &BoardView) -> io::Result<Option<Command>> {
            self.rounds += 1;
            if self.rounds > self.cap {
                // The same shape of error the real round cap gives.
                return Err(io::Error::other("solver round cap reached"));
            }
            // A move that is always refused, so the game never moves on.
            Ok(Some(Command::Reveal { x: 99, y: 99 }))
        }
    }

    let mut s = new_game(config(10)).expect("new game");
    let mut rng = SeededRng::new(5);
    let mut screen = Recorder::default();
    let mut input = Capped { rounds: 0, cap: 3 };
    let out = run_game(&mut s, &mut screen, &mut input, &mut rng);

    match out {
        Err(minesweeper::usecases::GameError::Io(e)) => {
            assert_eq!(e.to_string(), "solver round cap reached");
        }
        other => panic!("the loop turned a cap into something else: {other:?}"),
    }
    let ending_shown = screen.notices.iter().any(|n| {
        matches!(n, Notice::GameOver) || matches!(n, Notice::YouWin) || matches!(n, Notice::Quit)
    });
    assert!(
        !ending_shown,
        "the cap printed an ending: {:?}",
        screen.notices
    );
}
