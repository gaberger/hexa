//! Where the next move comes from.

use std::io;

use crate::ports::view::{BoardView, Command};

/// Gives the next move. `Ok(None)` means the input ended.
///
/// `next` receives the view. The demo player is an input adapter, so it sees
/// the same data you see, and no more. It cannot read the mines, because the
/// type it receives does not hold them.
pub trait InputSource {
    fn next(&mut self, view: &BoardView) -> io::Result<Option<Command>>;
}
