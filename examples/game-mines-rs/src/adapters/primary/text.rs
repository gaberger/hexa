//! The words and the signs the screen uses.
//!
//! This is not an adapter. It is a leaf of plain text helpers, so that two
//! screens can share one wording without either one importing the other.

use crate::ports::view::{BoardView, Glyph, Notice};

/// One character for one cell. ASCII only.
pub fn glyph_char(g: Glyph) -> char {
    match g {
        Glyph::Hidden => '.',
        Glyph::Flag => 'F',
        Glyph::Empty => ' ',
        Glyph::Count(n) => match n {
            1..=8 => char::from_digit(u32::from(n), 10).unwrap_or('?'),
            _ => '?',
        },
        Glyph::Mine => '*',
        Glyph::Blast => 'X',
    }
}

/// How wide the row labels must be, so every row line is the same length.
fn label_width(height: u16) -> usize {
    let last = height.saturating_sub(1);
    let mut w = 1usize;
    let mut n = last / 10;
    while n > 0 {
        w = w.saturating_add(1);
        n /= 10;
    }
    w
}

/// Build the whole frame as one string. The caller writes it in one go, so a
/// half drawn frame can never appear.
pub fn frame(view: &BoardView) -> String {
    let lw = label_width(view.height);
    let mut out = String::new();
    out.push('\n');

    // The column numbers, above the grid.
    for _ in 0..lw {
        out.push(' ');
    }
    for x in 0..view.width {
        out.push(' ');
        out.push(char::from_digit(u32::from(x % 10), 10).unwrap_or('?'));
    }
    out.push('\n');

    for y in 0..view.height {
        let label = format!("{:>width$}", y, width = lw);
        out.push_str(&label);
        for x in 0..view.width {
            let i = usize::from(y)
                .checked_mul(usize::from(view.width))
                .and_then(|v| v.checked_add(usize::from(x)));
            let g = i
                .and_then(|i| view.glyphs.get(i).copied())
                .unwrap_or(Glyph::Hidden);
            out.push(' ');
            out.push(glyph_char(g));
        }
        out.push('\n');
    }
    out
}

fn hex8(bytes: [u8; 8]) -> String {
    let mut s = String::with_capacity(16);
    for b in bytes {
        let hi = b >> 4;
        let lo = b & 0x0f;
        s.push(char::from_digit(u32::from(hi), 16).unwrap_or('0'));
        s.push(char::from_digit(u32::from(lo), 16).unwrap_or('0'));
    }
    s
}

/// The words for each message. No message can carry text from the keyboard.
pub fn notice_text(n: Notice) -> String {
    match n {
        Notice::Welcome => {
            "MINESWEEPER - r <col> <row> reveals, f <col> <row> flags, q quits\n".to_string()
        }
        Notice::Prompt => "> ".to_string(),
        Notice::BadCommand => "I do not understand that. Try: r 3 4\n".to_string(),
        Notice::OffBoard => "That cell is not on the board.\n".to_string(),
        Notice::AlreadyRevealed => "That cell is already open.\n".to_string(),
        Notice::CellIsFlagged => "That cell has a flag. Take the flag off first.\n".to_string(),
        Notice::CannotFlagRevealed => "You cannot flag an open cell.\n".to_string(),
        Notice::GameOver => "GAME OVER\n".to_string(),
        Notice::YouWin => "YOU WIN\n".to_string(),
        Notice::Quit => "QUIT\n".to_string(),
        Notice::Fingerprint(b) => format!("BOARD {}\n", hex8(b)),
        Notice::Stats { revealed, flags } => {
            format!("STATS revealed={} flags={}\n", revealed, flags)
        }
    }
}

