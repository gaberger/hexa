//! F. The screen and the keyboard.
//!
//! A `Notice` is an enum, not a string. So there is no path for text you typed
//! to reach your terminal. These tests hold that line from the outside.

use std::io::{self, Cursor, Write};

use minesweeper::adapters::primary::{
    stdin_input::parse_line, QuietRenderer, StdinInput, TerminalRenderer,
};
use minesweeper::adapters::secondary::SeededRng;
use minesweeper::ports::{BoardView, Command, Glyph, Notice, Phase, Renderer};
use minesweeper::usecases::{new_game, run_game, EndReason, GameConfig};

fn view(width: u16, height: u16) -> BoardView {
    let total = usize::from(width) * usize::from(height);
    let mut glyphs = vec![Glyph::Hidden; total];
    for (i, g) in glyphs.iter_mut().enumerate() {
        *g = match i % 6 {
            0 => Glyph::Hidden,
            1 => Glyph::Flag,
            2 => Glyph::Empty,
            3 => Glyph::Count(1),
            4 => Glyph::Count(8),
            _ => Glyph::Hidden,
        };
    }
    BoardView {
        width,
        height,
        glyphs,
        mines_total: 10,
        flags_placed: 2,
        revealed_safe: 3,
        phase: Phase::Playing,
    }
}

fn every_notice() -> Vec<Notice> {
    vec![
        Notice::Welcome,
        Notice::Prompt,
        Notice::BadCommand,
        Notice::OffBoard,
        Notice::AlreadyRevealed,
        Notice::CellIsFlagged,
        Notice::CannotFlagRevealed,
        Notice::GameOver,
        Notice::YouWin,
        Notice::Quit,
        Notice::Fingerprint([0x1f, 0x3a, 0x9c, 0x04, 0xd7, 0xb2, 0xe1, 0x85]),
        Notice::Stats {
            revealed: 63,
            flags: 10,
        },
    ]
}

/// 30. Every message prints printable ASCII only.
#[test]
fn every_message_is_printable() {
    for n in every_notice() {
        let mut r = TerminalRenderer::new(Vec::new());
        r.notice(n).expect("write");
        let bytes = r.into_inner();
        assert!(!bytes.is_empty(), "{n:?} printed nothing");
        for b in bytes {
            assert!(
                b == b'\n' || (0x20..0x7f).contains(&b),
                "{n:?} printed byte {b:#04x}"
            );
        }
    }

    // The drawn board is printable too.
    let mut r = TerminalRenderer::new(Vec::new());
    r.render(&view(9, 9)).expect("render");
    for b in r.into_inner() {
        assert!(b == b'\n' || (0x20..0x7f).contains(&b), "board byte {b:#04x}");
    }

    // The board line always reads the same way.
    let mut r = TerminalRenderer::new(Vec::new());
    r.notice(Notice::Fingerprint([
        0x1f, 0x3a, 0x9c, 0x04, 0xd7, 0xb2, 0xe1, 0x85,
    ]))
    .expect("write");
    assert_eq!(
        String::from_utf8(r.into_inner()).expect("utf8"),
        "BOARD 1f3a9c04d7b2e185\n"
    );

    let mut r = TerminalRenderer::new(Vec::new());
    r.notice(Notice::Stats {
        revealed: 63,
        flags: 10,
    })
    .expect("write");
    assert_eq!(
        String::from_utf8(r.into_inner()).expect("utf8"),
        "STATS revealed=63 flags=10\n"
    );
}

/// 31. Type an escape sequence. Not one byte of it reaches the screen.
#[test]
fn your_typing_cannot_reach_the_terminal() {
    let attack = "\x1b]0;pwned\x07";
    assert_eq!(parse_line(attack), Command::Unknown);

    let script = format!("{attack}\nr \x1b[2J 0\nq\n");
    let mut s = new_game(GameConfig::default()).expect("new game");
    let mut r = TerminalRenderer::new(Vec::new());
    let mut i = StdinInput::new(Cursor::new(script.into_bytes()));
    let mut rng = SeededRng::new(42);
    let end = run_game(&mut s, &mut r, &mut i, &mut rng).expect("finish");
    assert_eq!(end, EndReason::Quit);

    let out = r.into_inner();
    assert!(!out.contains(&0x1b), "an escape byte reached the screen");
    assert!(!out.contains(&0x07), "a bell byte reached the screen");
    let text = String::from_utf8(out).expect("utf8");
    assert!(!text.contains("pwned"), "typed text reached the screen");
    assert!(text.contains("I do not understand that"));
}

/// 32. One drawn frame is one write. A half drawn board can never appear.
#[test]
fn one_frame_is_one_write() {
    /// A sink that counts how many times it is written to.
    struct Counter {
        writes: usize,
        bytes: usize,
    }
    impl Write for Counter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.writes += 1;
            self.bytes += buf.len();
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let mut r = TerminalRenderer::new(Counter {
        writes: 0,
        bytes: 0,
    });
    r.render(&view(30, 20)).expect("render");
    let c = r.into_inner();
    assert_eq!(c.writes, 1, "the frame took {} writes", c.writes);
    assert!(c.bytes > 0);
}

/// 33. Every row of the board is the same length, so the grid lines up.
#[test]
fn the_grid_lines_up() {
    for (w, h) in [(12u16, 12u16), (9, 9), (30, 5), (5, 30), (1, 1)] {
        let mut r = TerminalRenderer::new(Vec::new());
        r.render(&view(w, h)).expect("render");
        let text = String::from_utf8(r.into_inner()).expect("utf8");
        let lines: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), usize::from(h) + 1, "a row is missing on {w}x{h}");
        let first = lines.first().map(|l| l.chars().count()).unwrap_or(0);
        for (n, line) in lines.iter().enumerate() {
            assert_eq!(
                line.chars().count(),
                first,
                "line {n} is a different length on {w}x{h}"
            );
        }
    }
}

/// 34. A line the parser does not understand becomes `Unknown`. The loop says
///     so and asks again. The keyboard adapter itself prints nothing: it holds
///     a reader and no writer, so it has nowhere to print to.
#[test]
fn a_line_we_do_not_understand_is_refused_by_the_loop() {
    for bad in ["hello", "", "   ", "r", "r 1", "r a b", "r -1 0", "z 1 1", "r 1 1 1"] {
        assert_eq!(parse_line(bad), Command::Unknown, "line {bad:?}");
    }
    assert_eq!(parse_line("r 3 4"), Command::Reveal { x: 3, y: 4 });
    assert_eq!(parse_line("f 3 4"), Command::Flag { x: 3, y: 4 });
    assert_eq!(parse_line("q"), Command::Quit);
    assert_eq!(parse_line("QUIT"), Command::Quit);

    // The adapter reads and returns. It writes nothing anywhere.
    let mut i = StdinInput::new(Cursor::new(b"hello\n".to_vec()));
    use minesweeper::ports::InputSource;
    assert_eq!(i.next(&view(9, 9)).expect("read"), Some(Command::Unknown));
    assert_eq!(i.next(&view(9, 9)).expect("read"), None, "end of input");

    // The loop is the one that speaks.
    let mut s = new_game(GameConfig::default()).expect("new game");
    let mut r = TerminalRenderer::new(Vec::new());
    let mut input = StdinInput::new(Cursor::new(b"hello\nq\n".to_vec()));
    let mut rng = SeededRng::new(1);
    run_game(&mut s, &mut r, &mut input, &mut rng).expect("finish");
    let text = String::from_utf8(r.into_inner()).expect("utf8");
    assert!(text.contains("I do not understand that. Try: r 3 4"));
    assert!(text.trim_end().ends_with("QUIT"));
}

/// The quiet screen used by the demo shows only the three closing lines.
#[test]
fn the_quiet_screen_shows_only_the_ending() {
    let mut r = QuietRenderer::new(Vec::new());
    r.render(&view(9, 9)).expect("render");
    for n in every_notice() {
        r.notice(n).expect("write");
    }
    let text = String::from_utf8(r.into_inner()).expect("utf8");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines,
        vec![
            "GAME OVER",
            "YOU WIN",
            "QUIT",
            "BOARD 1f3a9c04d7b2e185",
            "STATS revealed=63 flags=10",
        ]
    );
}
