//! Copy the board into plain data, so an adapter never touches the domain.

use crate::domain::status::{CellState, Status};
use crate::ports::view::{BoardView, Glyph, Phase};
use crate::usecases::new_game::Session;

/// Build one frame.
///
/// `Mine` and `Blast` are written only when the phase is `Lost`. So a mine
/// cannot leak through the window while you play, and the loss screen shows
/// the whole answer.
pub fn project_view(s: &Session) -> BoardView {
    let dims = s.dims();
    let total = dims.total();
    let mut glyphs = Vec::with_capacity(total);

    let board = s.board();
    let phase = match board.map(|b| b.status()) {
        Some(Status::Won) => Phase::Won,
        Some(Status::Lost) => Phase::Lost,
        _ => Phase::Playing,
    };

    for i in 0..total {
        let c = match dims.from_index(i) {
            Some(c) => c,
            None => {
                glyphs.push(Glyph::Hidden);
                continue;
            }
        };
        let g = match board {
            None => {
                if s.pre_flags().get(i).copied() == Some(true) {
                    Glyph::Flag
                } else {
                    Glyph::Hidden
                }
            }
            Some(b) => {
                if phase == Phase::Lost && b.blast() == Some(c) {
                    Glyph::Blast
                } else if phase == Phase::Lost && b.is_mine(c) {
                    Glyph::Mine
                } else {
                    match b.cell_state(c) {
                        CellState::Hidden => Glyph::Hidden,
                        CellState::Flagged => Glyph::Flag,
                        CellState::Revealed => {
                            let n = b.adjacent(c);
                            if n == 0 {
                                Glyph::Empty
                            } else {
                                Glyph::Count(n)
                            }
                        }
                    }
                }
            }
        };
        glyphs.push(g);
    }

    let flags_placed = match board {
        Some(b) => b.flags_placed(),
        None => s.pre_flags().iter().filter(|f| **f).count(),
    };
    let revealed_safe = board.map(|b| b.revealed_safe()).unwrap_or(0);

    BoardView {
        width: dims.width(),
        height: dims.height(),
        glyphs,
        mines_total: s.mine_count(),
        flags_placed,
        revealed_safe,
        phase,
    }
}
