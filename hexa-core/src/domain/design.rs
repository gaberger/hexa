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
    let mut bad = Vec::new();
    for (path, want) in &pin.files {
        if !present.iter().any(|p| p == path) {
            bad.push(format!("{path}: recorded in the pin but missing"));
            continue;
        }
        match read(path) {
            Some(bytes) if sha256(&bytes) == *want => {}
            Some(_) => bad.push(format!("{path}: differs from the pinned sha256")),
            None => bad.push(format!("{path}: recorded in the pin but unreadable")),
        }
    }
    for path in present {
        if !pin.files.iter().any(|(p, _)| p == path) {
            bad.push(format!("{path}: present but not recorded in the pin"));
        }
    }
    if bad.is_empty() { Ok(()) } else { Err(bad) }
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
    use Stage::*;
    match preset {
        Preset::Full => vec![ZoomOut, Comprehension, Critic, IntermediateZoomOut, ImplementationReadiness],
        Preset::FeatureSpecOnly => vec![ZoomOut, Comprehension, Critic, IntermediateZoomOut],
        Preset::ImplementationPlanOnly => vec![ImplementationReadiness],
    }
}

/// EGM's stage number: 5 to 9.
pub fn stage_number(stage: Stage) -> u8 {
    match stage {
        Stage::ZoomOut => 5,
        Stage::Comprehension => 6,
        Stage::Critic => 7,
        Stage::IntermediateZoomOut => 8,
        Stage::ImplementationReadiness => 9,
    }
}

/// The stage EGM numbers `n`, if any.
pub fn stage_from_number(n: u8) -> Option<Stage> {
    stages_for(Preset::Full).into_iter().find(|s| stage_number(*s) == n)
}

/// EGM's name for a stage.
pub fn stage_name(stage: Stage) -> &'static str {
    match stage {
        Stage::ZoomOut => "Initial zoom out",
        Stage::Comprehension => "Comprehension test",
        Stage::Critic => "Critic review",
        Stage::IntermediateZoomOut => "Intermediate zoom out",
        Stage::ImplementationReadiness => "Implementation readiness",
    }
}

/// The reviewer personas a stage runs, by their vendored file stem
/// (`agents/<stem>.md`).
pub fn stage_reviewers(stage: Stage) -> Vec<&'static str> {
    match stage {
        Stage::ZoomOut | Stage::IntermediateZoomOut => vec!["zoom_out"],
        Stage::Comprehension => vec!["comprehension"],
        Stage::Critic => vec!["design_critic", "structural", "style"],
        Stage::ImplementationReadiness => vec!["implementation_readiness", "codebase_details", "style"],
    }
}

/// The document standard a reviewer applies in a stage, by its vendored file
/// stem (`resources/document_standards/<stem>.md`), as EGM's review skill
/// assigns them. `None`: the persona applies its own lens only.
pub fn stage_standard(stage: Stage, reviewer: &str) -> Option<&'static str> {
    match (stage, reviewer) {
        (Stage::ZoomOut | Stage::IntermediateZoomOut, "zoom_out") => Some("zoom-out-standard"),
        (Stage::Comprehension, "comprehension") => Some("re-implementation-standard"),
        (Stage::Critic, "design_critic" | "structural") => Some("feature-spec-standard"),
        (Stage::ImplementationReadiness, "implementation_readiness" | "codebase_details") => {
            Some("implementation-readiness-standard")
        }
        _ => None,
    }
}

/// The persona that deduplicates a stage's findings.
pub const SYNTHESIZER: &str = "synthesizer";

/// Every path `document` names in backquotes that `exists` says is in the
/// project, in first-mention order, each once.
pub fn referenced_paths(document: &str, exists: &dyn Fn(&str) -> bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for span in backquoted(document) {
        if inside_project(span) && exists(span) && !out.iter().any(|o| o == span) {
            out.push(span.to_string());
        }
    }
    out
}

/// Backquoted spans that look like paths and that [`referenced_paths`] does
/// not return: outside the project, or not there. In first-mention order,
/// each once. A goldfish is not given them; the report lists them.
pub fn unresolved_paths(document: &str, exists: &dyn Fn(&str) -> bool) -> Vec<String> {
    let resolved = referenced_paths(document, exists);
    let mut out: Vec<String> = Vec::new();
    for span in backquoted(document) {
        if looks_like_path(span) && !resolved.iter().any(|r| r == span) && !out.iter().any(|o| o == span) {
            out.push(span.to_string());
        }
    }
    out
}

/// The text between each pair of backquotes.
fn backquoted(document: &str) -> impl Iterator<Item = &str> {
    document.split('`').skip(1).step_by(2).filter(|s| !s.is_empty())
}

/// A relative path that does not climb out of the project.
fn inside_project(path: &str) -> bool {
    !path.starts_with('/')
        && !path.starts_with('\\')
        && !path.starts_with('~')
        && !path.chars().nth(1).is_some_and(|c| c == ':')
        && !path.split(['/', '\\']).any(|c| c == "..")
}

/// One word holding a directory separator or a file extension.
fn looks_like_path(span: &str) -> bool {
    if span.is_empty() || span.chars().any(char::is_whitespace) || span.contains("://") {
        return false;
    }
    let ext = span
        .rsplit_once('.')
        .is_some_and(|(stem, ext)| !stem.is_empty() && !ext.is_empty() && ext.len() <= 5 && ext.chars().all(|c| c.is_ascii_alphanumeric()));
    span.contains('/') || ext
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
    let mut out = String::new();
    out.push_str(
        "You are one reviewer in an Elephant-Goldfish review, run by hexa as a single completion with no \
         tools. Everything you may use is below: your persona, the standard you apply, the document under \
         review, and the source files the document names. You cannot open any other file. Report your \
         findings as a Markdown list, one `- ` item per finding, citing the passage or file it concerns. \
         If you have none, reply `No findings.`\n\n",
    );
    out.push_str("# Persona\n\n");
    out.push_str(persona);
    out.push_str("\n\n");
    if let Some(standard) = standard {
        out.push_str("# Standard\n\n");
        out.push_str(standard);
        out.push_str("\n\n");
    }
    out.push_str("# Document under review\n\n");
    out.push_str(document);
    out.push_str("\n\n");
    if !sources.is_empty() {
        out.push_str(&format!(
            "# Referenced source\n\nEach file is cut at {cap_bytes} bytes; a cut file says so.\n\n"
        ));
        for (path, contents) in sources {
            let cut = cut_at(contents, cap_bytes);
            if cut.len() < contents.len() {
                out.push_str(&format!(
                    "## `{path}` (cut: the first {} of {} bytes, at the {cap_bytes}-byte cap)\n\n",
                    cut.len(),
                    contents.len()
                ));
            } else {
                out.push_str(&format!("## `{path}`\n\n"));
            }
            let fence = fence_for(cut);
            out.push_str(&format!("{fence}\n{cut}\n{fence}\n\n"));
        }
    }
    out
}

/// `text` cut to at most `cap` bytes, at a character boundary.
fn cut_at(text: &str, cap: usize) -> &str {
    if text.len() <= cap {
        return text;
    }
    let mut end = cap;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// A backquote fence longer than any run of backquotes in `text`.
fn fence_for(text: &str) -> String {
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    "`".repeat(longest.max(2) + 1)
}

/// One finding in a review report: its id (`S<stage>-<n>`) and its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub id: String,
    pub text: String,
}

/// The list items of a reply, each with its continuation lines. A reply with
/// no list item has none: "No findings." is not a finding. Lines inside a
/// code fence never start an item.
pub fn list_items(reply: &str) -> Vec<String> {
    let mut items: Vec<String> = Vec::new();
    let mut in_fence = false;
    for line in reply.lines() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            if let Some(last) = items.last_mut() {
                last.push('\n');
                last.push_str(line);
            }
            continue;
        }
        if !in_fence && line.len() - t.len() < 2 {
            if let Some(rest) = item_text(t) {
                items.push(rest.to_string());
                continue;
            }
        }
        if t.is_empty() && !in_fence {
            continue;
        }
        if let Some(last) = items.last_mut() {
            last.push('\n');
            last.push_str(line);
        }
    }
    items.into_iter().map(|i| i.trim_end().to_string()).filter(|i| !i.is_empty()).collect()
}

/// The text after a list marker (`- `, `* `, `+ `, `1. `), if `line` has one.
fn item_text(line: &str) -> Option<&str> {
    for m in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(m) {
            return Some(rest);
        }
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 {
        let rest = &line[digits..];
        return rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") "));
    }
    None
}

/// A stage's synthesized findings, numbered `S<stage>-1`, `S<stage>-2`, ….
pub fn number_findings(stage: Stage, items: &[String]) -> Vec<Finding> {
    let n = stage_number(stage);
    items
        .iter()
        .enumerate()
        .map(|(i, text)| Finding { id: format!("S{n}-{}", i + 1), text: text.clone() })
        .collect()
}

/// The findings a review report holds: every list item that opens with an id
/// `S<digits>-<digits>`, optionally bold, followed by `:` or a space.
fn report_findings(report: &str) -> Vec<Finding> {
    list_items(report)
        .into_iter()
        .filter_map(|item| {
            let t = item.trim_start_matches("**");
            let id_len = finding_id_len(t)?;
            let id = t[..id_len].to_string();
            let rest = t[id_len..].trim_start_matches("**").trim_start_matches(':').trim_start();
            Some(Finding { id, text: rest.to_string() })
        })
        .collect()
}

fn finding_id_len(t: &str) -> Option<usize> {
    let b = t.as_bytes();
    if b.first() != Some(&b'S') {
        return None;
    }
    let d1 = t[1..].bytes().take_while(u8::is_ascii_digit).count();
    if d1 == 0 || b.get(1 + d1) != Some(&b'-') {
        return None;
    }
    let d2 = t[2 + d1..].bytes().take_while(u8::is_ascii_digit).count();
    if d2 == 0 {
        return None;
    }
    let end = 2 + d1 + d2;
    match b.get(end) {
        None | Some(b':') | Some(b' ') | Some(b'*') => Some(end),
        _ => None,
    }
}

/// The findings `report` holds whose ids are in `accept`, in report order,
/// and the accepted ids it does not hold.
pub fn accepted_findings(report: &str, accept: &[String]) -> (Vec<Finding>, Vec<String>) {
    let all = report_findings(report);
    let chosen: Vec<Finding> = all.iter().filter(|f| accept.iter().any(|a| a == &f.id)).cloned().collect();
    let missing = accept.iter().filter(|a| !all.iter().any(|f| &f.id == *a)).cloned().collect();
    (chosen, missing)
}

/// The value of the `EGM artifact directory: <path>` line in `claude_md`.
pub fn artifact_dir_setting(claude_md: &str) -> Option<String> {
    claude_md.lines().find_map(|l| {
        let v = l.trim().trim_start_matches(['-', '*', ' ']).strip_prefix("EGM artifact directory:")?;
        let v = v.trim().trim_matches('`').trim();
        (!v.is_empty()).then(|| v.trim_end_matches('/').to_string())
    })
}

/// Whether a relative path stays inside the project.
pub fn is_inside_project(path: &str) -> bool {
    !path.is_empty() && inside_project(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_items_keep_continuations_and_skip_prose() {
        let reply = "Intro.\n- one\n  more\n- two\n```\n- not an item\n```\nNo findings.";
        let items = list_items(reply);
        assert_eq!(items.len(), 2);
        assert!(items[0].contains("more"));
        assert!(items[1].contains("not an item"));
        assert!(list_items("No findings.").is_empty());
    }

    #[test]
    fn report_findings_read_ids() {
        let r = "## Stage 6\n\n- S6-1: AAA\n- **S6-2**: BBB\n- S7: no\n";
        let f = report_findings(r);
        assert_eq!(f.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), ["S6-1", "S6-2"]);
        assert_eq!(f[1].text, "BBB");
    }

    #[test]
    fn unresolved_lists_missing_and_outside() {
        let doc = "`src/a.rs` `src/ghost.rs` `../x.txt` `--stage` `Foo`";
        assert_eq!(unresolved_paths(doc, &|p| p == "src/a.rs"), ["src/ghost.rs", "../x.txt"]);
    }

    #[test]
    fn artifact_dir_line() {
        assert_eq!(artifact_dir_setting("x\nEGM artifact directory: design/egm/\n").as_deref(), Some("design/egm"));
        assert_eq!(artifact_dir_setting("nothing"), None);
    }
}
