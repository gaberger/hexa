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

/// The verdict for `adr`, whose gate gave `outcome`.
pub fn judge(adr: &str, outcome: &Outcome, unbuilt: &Unbuilt, ci: bool) -> Verdict {
    let _ = (adr, outcome, unbuilt, ci);
    unimplemented!("ADR-2610271110: built to its sealed spec")
}

/// What is wrong with the declaration itself: an id that names no ADR, or
/// names one that records no gate. `gated` maps every ADR id to whether it
/// records a gate.
pub fn declaration_errors(unbuilt: &Unbuilt, gated: &BTreeMap<String, bool>) -> Vec<String> {
    let _ = (unbuilt, gated);
    unimplemented!("ADR-2610271110: built to its sealed spec")
}
