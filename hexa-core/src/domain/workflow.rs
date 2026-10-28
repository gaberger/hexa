//! The hexa loop's stages and the design step's next action
//! (ADR-2610281011). Pure: the caller reads the loop, the files and git.

use super::design::review_covers;

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
    use LoopStage::*;
    vec![Design, Decide, Gate, Build, Harden, Done]
}

/// The stage `word` names (`design`, `decide`, …), if any.
pub fn parse_stage(word: &str) -> Option<LoopStage> {
    loop_stages().into_iter().find(|s| stage_word(*s) == word)
}

/// The word for `stage`, as `hexa loop stage` takes it.
pub fn stage_word(stage: LoopStage) -> &'static str {
    match stage {
        LoopStage::Design => "design",
        LoopStage::Decide => "decide",
        LoopStage::Gate => "gate",
        LoopStage::Build => "build",
        LoopStage::Harden => "harden",
        LoopStage::Done => "done",
    }
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
    let Some(draft) = &state.draft else { return DesignNext::Nothing };
    let covered = state
        .draft_sha256
        .as_deref()
        .is_some_and(|h| state.reviews.iter().any(|r| review_covers(r, h).is_ok()));
    if !covered {
        DesignNext::Review(draft.clone())
    } else if !state.committed {
        DesignNext::Commit(draft.clone())
    } else {
        DesignNext::ToAdr(draft.clone())
    }
}
