//! Three puzzles compiled into the binary.

use crate::ports::level_source::{Level, LevelSource, Par};

/// Correct shape: the adapter takes its types from the port, not the domain.
pub struct BuiltinLevels;

const LEVELS: [(&str, usize); 3] = [
    (
        "\
#####
#@$.#
#####",
        1,
    ),
    (
        "\
######
#@ $.#
######",
        2,
    ),
    (
        "\
#######
## . ##
#  $  #
#  @  #
#######",
        1,
    ),
];

impl LevelSource for BuiltinLevels {
    fn count(&self) -> usize {
        LEVELS.len()
    }

    fn load(&self, index: usize) -> Option<(Level, Par)> {
        let (text, moves) = LEVELS.get(index)?;
        // A level that fails to parse is a bug in this file, not a runtime
        // condition: returning None would hide it as "no such level".
        let level = Level::parse(text).expect("a builtin level must parse");
        Some((level, Par { moves: *moves }))
    }
}
