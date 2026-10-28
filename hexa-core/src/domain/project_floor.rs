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

/// The ADR ids a ledger's file names hold: each `ADR-*.md` name's id,
/// sorted, each once. What the build embeds as hexa's own decisions.
pub fn adr_ids(file_names: &[String]) -> Vec<String> {
    let mut ids: Vec<String> = file_names.iter().filter_map(|n| adr_id_of(n)).collect();
    ids.sort();
    ids.dedup();
    ids
}

/// `ADR-2610281552-slug.md` → `ADR-2610281552`; `ADR-2026-04-05-0900-x.md`
/// → `ADR-2026-04-05-0900`: every all-digit, dash-separated group after
/// `ADR-`. `None` for a name that is not an `ADR-*.md` decision.
fn adr_id_of(name: &str) -> Option<String> {
    let stem = name.strip_suffix(".md")?.strip_prefix("ADR-")?;
    let groups: Vec<&str> = stem.split('-').take_while(|g| !g.is_empty() && g.bytes().all(|b| b.is_ascii_digit())).collect();
    (!groups.is_empty()).then(|| format!("ADR-{}", groups.join("-")))
}

/// The project's ledger first, then hexa's.
pub fn cited_ledger(id: &str, project_ids: &[String], hexa_ids: &[String]) -> CitedLedger {
    if project_ids.iter().any(|p| p == id) {
        CitedLedger::Project
    } else if hexa_ids.iter().any(|h| h == id) {
        CitedLedger::Hexa
    } else {
        CitedLedger::Dangling
    }
}

/// The `.gitignore` lines a new project of `lang` needs.
pub fn gitignore_lines(lang: &str) -> Vec<&'static str> {
    let mut lines: Vec<&'static str> = match lang {
        "rust" => vec!["target/"],
        "ts" => vec!["node_modules/", "dist/"],
        _ => Vec::new(),
    };
    // One checkout's session state, never shared.
    lines.extend([".hexa/loop.json", ".hexa/loop.lock"]);
    lines
}

/// The `.gitignore` to write: `existing` with each missing line appended
/// once, or `None` when nothing is missing.
pub fn merged_gitignore(existing: Option<&str>, lang: &str) -> Option<String> {
    let text = existing.unwrap_or("");
    let present = |l: &str| text.lines().any(|x| x.trim() == l);
    let missing: Vec<&str> = gitignore_lines(lang).into_iter().filter(|l| !present(l)).collect();
    if missing.is_empty() {
        return None;
    }
    let mut out = text.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    for l in missing {
        out.push_str(l);
        out.push('\n');
    }
    Some(out)
}
