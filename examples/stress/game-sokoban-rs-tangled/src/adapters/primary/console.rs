//! Renders a board as text and turns keys into moves.

use crate::ports::level_source::{Level, Pos, Tile};
use crate::ports::move_recorder::Dir;
use crate::usecases::play::Progress;
// STRESS: violation — adapters must not import from other adapters.
// The console builds its own recorder instead of being handed one by the
// composition root, so the driving side now depends on the driven side.
use crate::adapters::secondary::memory_recorder::MemoryRecorder;

/// WASD and the vi keys, because a puzzle played in a terminal gets both.
pub fn key(ch: char) -> Option<Dir> {
    match ch.to_ascii_lowercase() {
        'w' | 'k' => Some(Dir::Up),
        's' | 'j' => Some(Dir::Down),
        'a' | 'h' => Some(Dir::Left),
        'd' | 'l' => Some(Dir::Right),
        _ => None,
    }
}

/// The conventional Sokoban charset, so a rendered board can be pasted back
/// into `Level::parse` and read the same way.
pub fn render(level: &Level, progress: &Progress) -> String {
    let mut out = String::new();
    for row in 0..level.rows() {
        for col in 0..level.cols() {
            let at = Pos { row, col };
            let tile = level.tile(at);
            let ch = if progress.board.player == at {
                if tile == Tile::Goal { '+' } else { '@' }
            } else if progress.board.has_box(at) {
                if tile == Tile::Goal { '*' } else { '$' }
            } else {
                match tile {
                    Tile::Wall => '#',
                    Tile::Goal => '.',
                    Tile::Floor => ' ',
                }
            };
            out.push(ch);
        }
        out.push('\n');
    }
    out
}

/// The tangle made visible: the driving adapter constructs a driven one.
pub fn own_recorder() -> MemoryRecorder {
    MemoryRecorder::default()
}
