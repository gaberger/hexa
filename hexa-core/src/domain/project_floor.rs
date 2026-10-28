//! What a new project holds on day one (ADR-2610281552). Pure.

/// Which ledger a cited ADR id belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CitedLedger {
    /// The project's own `docs/adrs/`.
    Project,
    /// One of hexa's own decisions, which hexa's shipped files cite.
    Hexa,
    /// Neither: a dangling citation.
    Dangling,
}

/// The project's ledger first, then hexa's.
pub fn cited_ledger(id: &str, project_ids: &[String], hexa_ids: &[String]) -> CitedLedger {
    let _ = (id, project_ids, hexa_ids);
    unimplemented!("built to its sealed spec")
}

/// The `.gitignore` lines a new project of `lang` needs.
pub fn gitignore_lines(lang: &str) -> Vec<&'static str> {
    let _ = lang;
    unimplemented!("built to its sealed spec")
}

/// The `.gitignore` to write: `existing` with each missing line appended
/// once, or `None` when nothing is missing.
pub fn merged_gitignore(existing: Option<&str>, lang: &str) -> Option<String> {
    let _ = (existing, lang);
    unimplemented!("built to its sealed spec")
}
