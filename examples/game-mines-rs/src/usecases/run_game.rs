//! The one loop. The demo and the interactive game both use it.

use crate::ports::input::InputSource;
use crate::ports::random::RandomSource;
use crate::ports::renderer::Renderer;
use crate::ports::view::Notice;
use crate::usecases::new_game::{EndReason, GameError, Session};
use crate::usecases::project_view::project_view;
use crate::usecases::take_command::{take_command, Step};

/// Play until the game ends, the player quits, or the input runs out.
///
/// The last three messages are always the board line, the stats line and the
/// ending line, in that order.
pub fn run_game(
    s: &mut Session,
    r: &mut dyn Renderer,
    i: &mut dyn InputSource,
    rng: &mut dyn RandomSource,
) -> Result<EndReason, GameError> {
    r.notice(Notice::Welcome)?;

    let end: EndReason;
    loop {
        let view = project_view(s);
        r.render(&view)?;
        r.notice(Notice::Prompt)?;
        match i.next(&view)? {
            None => {
                end = EndReason::Quit;
                break;
            }
            Some(cmd) => match take_command(s, cmd, rng) {
                Step::Applied => {}
                Step::Refused(n) => r.notice(n)?,
                Step::Ended(e) => {
                    end = e;
                    break;
                }
                Step::Fault(f) => return Err(GameError::Fault(f)),
            },
        }
    }

    let view = project_view(s);
    r.render(&view)?;
    r.notice(Notice::Fingerprint(s.fingerprint()))?;
    r.notice(Notice::Stats {
        revealed: view.revealed_safe,
        flags: view.flags_placed,
    })?;
    r.notice(match end {
        EndReason::Won => Notice::YouWin,
        EndReason::Lost => Notice::GameOver,
        EndReason::Quit => Notice::Quit,
    })?;
    Ok(end)
}
