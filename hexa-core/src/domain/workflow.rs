//! The hexa loop's stages and the design step's next action
//! (ADR-2610281011). Pure: the caller reads the loop, the files and git.

/// The loop's stages, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LoopStage {
    Design,
    Decide,
    Gate,
    Build,
    Harden,
    Done,
}

/// Every stage, in the order the loop runs them.
pub fn loop_stages() -> Vec<LoopStage> {
    unimplemented!("ADR-2610281011: built to its sealed spec")
}

/// The stage `word` names (`design`, `decide`, …), if any.
pub fn parse_stage(word: &str) -> Option<LoopStage> {
    let _ = word;
    unimplemented!("ADR-2610281011: built to its sealed spec")
}

/// The word for `stage`, as `hexa loop stage` takes it.
pub fn stage_word(stage: LoopStage) -> &'static str {
    let _ = stage;
    unimplemented!("ADR-2610281011: built to its sealed spec")
}

/// What the design step has on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesignState {
    /// The draft the loop records, relative to the project root.
    pub draft: Option<String>,
    /// The sha256 hex of the draft's current bytes.
    pub draft_sha256: Option<String>,
    /// The text of every review report in the artifact directory.
    pub reviews: Vec<String>,
    /// The draft is tracked by git and has no uncommitted change.
    pub committed: bool,
}

/// The next design action `hexa go` suggests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesignNext {
    /// No draft is recorded.
    Nothing,
    /// No review covers the draft's current bytes: `hexa design review <draft>`.
    Review(String),
    /// Reviewed, with uncommitted changes: commit it.
    Commit(String),
    /// Reviewed and committed: `hexa design adr <draft>`.
    ToAdr(String),
}

pub fn next_design_action(state: &DesignState) -> DesignNext {
    let _ = state;
    unimplemented!("ADR-2610281011: built to its sealed spec")
}
