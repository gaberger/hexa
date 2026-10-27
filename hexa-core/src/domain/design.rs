//! Elephant-Goldfish design review, as hexa runs it (ADR-2610271736). The
//! method and its prompts are EGM's (<https://github.com/google/elephant-goldfish-model>,
//! Google LLC, Apache-2.0), vendored at a pinned commit. Pure: the caller
//! reads files, runs the calls and writes the reports.

/// The vendored copy of EGM, as `UPSTREAM.json` records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pin {
    pub repository: String,
    pub commit: String,
    /// `(path relative to the vendored root, sha256 hex)`.
    pub files: Vec<(String, String)>,
}

/// Does the vendored copy match its pin? `present` lists the files actually
/// there; `read` returns a file's bytes; `sha256` hashes them. `Err` names
/// every file that differs, is missing, or is present but unrecorded.
pub fn verify_pin(
    pin: &Pin,
    present: &[String],
    read: &dyn Fn(&str) -> Option<Vec<u8>>,
    sha256: &dyn Fn(&[u8]) -> String,
) -> Result<(), Vec<String>> {
    let _ = (pin, present, read, sha256);
    unimplemented!("ADR-2610271736: built to its sealed spec")
}

/// EGM's review stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    /// 5: initial zoom out.
    ZoomOut,
    /// 6: comprehension test.
    Comprehension,
    /// 7: design critic, structural and style.
    Critic,
    /// 8: intermediate zoom out.
    IntermediateZoomOut,
    /// 9: implementation readiness.
    ImplementationReadiness,
}

/// Which stages a review runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    Full,
    FeatureSpecOnly,
    ImplementationPlanOnly,
}

/// The stages `preset` runs, in the order they run.
pub fn stages_for(preset: Preset) -> Vec<Stage> {
    let _ = preset;
    unimplemented!("ADR-2610271736: built to its sealed spec")
}

/// EGM's stage number: 5 to 9.
pub fn stage_number(stage: Stage) -> u8 {
    let _ = stage;
    unimplemented!("ADR-2610271736: built to its sealed spec")
}

/// The reviewer personas a stage runs, by their vendored file stem
/// (`agents/<stem>.md`).
pub fn stage_reviewers(stage: Stage) -> Vec<&'static str> {
    let _ = stage;
    unimplemented!("ADR-2610271736: built to its sealed spec")
}

/// Every path `document` names in backquotes that `exists` says is in the
/// project, in first-mention order, each once.
pub fn referenced_paths(document: &str, exists: &dyn Fn(&str) -> bool) -> Vec<String> {
    let _ = (document, exists);
    unimplemented!("ADR-2610271736: built to its sealed spec")
}

/// The whole input of one goldfish: the persona, the standard it applies,
/// the document, and the referenced sources (`(path, contents)`), each source
/// cut to `cap_bytes` with the cut stated. Nothing else goes in.
pub fn goldfish_prompt(
    persona: &str,
    standard: Option<&str>,
    document: &str,
    sources: &[(String, String)],
    cap_bytes: usize,
) -> String {
    let _ = (persona, standard, document, sources, cap_bytes);
    unimplemented!("ADR-2610271736: built to its sealed spec")
}
