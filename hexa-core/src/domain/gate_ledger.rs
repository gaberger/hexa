//! Whether a run of every ADR's gate passes, given the decisions a project
//! declares not yet built (ADR-2610271110). Pure: the caller runs the gates
//! and reads `.hexa/project.json`.

use std::collections::BTreeMap;

/// What running one ADR's recorded gate established.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Passed,
    Failed,
    /// Not a command that can be executed: prose, a placeholder.
    Unrunnable,
    /// A program the gate needs is missing here.
    CannotRunHere,
    /// A test skipped for want of a toolchain.
    NotProvenHere,
    /// The ADR records no gate.
    NoGate,
}

/// One ADR's verdict in the run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Passed,
    /// Fails the run, with why.
    Fails(String),
    /// Declared unbuilt and still unbuilt: reported, never counted as a pass.
    Unbuilt { reason: String },
    /// Not proven here, outside `--ci`: reported, not failing.
    Unproven,
    NoGate,
}

/// `adr.unbuilt` in `.hexa/project.json`: ADR id → why it is not built yet.
pub type Unbuilt = BTreeMap<String, String>;

/// The verdict for `adr`, whose gate gave `outcome` (§2, §3, §5).
pub fn judge(adr: &str, outcome: &Outcome, unbuilt: &Unbuilt, ci: bool) -> Verdict {
    if let Some(reason) = unbuilt.get(adr) {
        return match outcome {
            // Built, so the declaration is now false (§3).
            Outcome::Passed => Verdict::Fails(format!(
                "{adr} is declared unbuilt but its gate passes: remove it from adr.unbuilt"
            )),
            // A declared id with no gate is the declaration's error (§4), not the gate's.
            Outcome::NoGate => Verdict::NoGate,
            _ => Verdict::Unbuilt { reason: reason.clone() },
        };
    }
    match outcome {
        Outcome::Passed => Verdict::Passed,
        Outcome::Failed => Verdict::Fails("gate failed".to_string()),
        Outcome::Unrunnable => Verdict::Fails("not a runnable command".to_string()),
        Outcome::CannotRunHere => Verdict::Fails("cannot run here".to_string()),
        Outcome::NotProvenHere if ci => Verdict::Fails("not proven here (--ci)".to_string()),
        Outcome::NotProvenHere => Verdict::Unproven,
        Outcome::NoGate => Verdict::NoGate,
    }
}

/// What is wrong with the declaration itself (§4): an id that names no ADR,
/// or names one that records no gate. `gated` maps every ADR id to whether
/// it records a gate. One error per bad id, in id order.
pub fn declaration_errors(unbuilt: &Unbuilt, gated: &BTreeMap<String, bool>) -> Vec<String> {
    unbuilt
        .keys()
        .filter_map(|id| match gated.get(id) {
            Some(true) => None,
            Some(false) => Some(format!("adr.unbuilt names {id}, which records no gate")),
            None => Some(format!("adr.unbuilt names {id}, which names no ADR in docs/adrs/")),
        })
        .collect()
}
