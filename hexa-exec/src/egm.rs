//! The Elephant-Goldfish Model (EGM, <https://github.com/google/elephant-goldfish-model>,
//! Google LLC, Apache-2.0) as hexa runs it (ADR-2610271736): `hexa design`
//! and `hexa drift`, on EGM's prompts vendored and pinned.
//!
//! A goldfish is one tool-less completion whose whole input
//! `design::goldfish_prompt` builds. The elephant is a completion too, given
//! the project's context in its prompt: neither can write a file, so the only
//! files a run writes are the ones written here, and every one goes through
//! [`Writer`], which allows the artifact directory and `docs/analysis/` and
//! nothing else (§8).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use futures::future::join_all;
use hexa_core::domain::design::{self, Pin, Preset, Stage};
use sha2::{Digest, Sha256};

/// Each referenced source file is cut at this many bytes (§4).
const SOURCE_CAP: usize = 12_000;
/// At most this many referenced files go into one prompt; the rest are listed
/// as not supplied. A prompt travels as one argument to the `claude` CLI.
const MAX_SOURCES: usize = 8;
/// Each project-context file the elephant is given is cut at this many bytes.
const CONTEXT_CAP: usize = 12_000;
const MAX_TOKENS: u32 = 16_000;
const DEFAULT_ARTIFACT_DIR: &str = "docs/designs";
const ANALYSIS_DIR: &str = "docs/analysis";

type Res<T> = Result<T, String>;

/// Which stages a review runs.
#[derive(Debug, Clone, Copy)]
pub enum ReviewScope {
    /// One stage, by EGM's number (5 to 9).
    Stage(u8),
    Full,
    FeatureSpecOnly,
    ImplementationPlanOnly,
}

// ------------------------------------------------------------------ EGM

/// The vendored EGM, verified against its pin (§1).
pub struct Egm {
    pin: Pin,
    files: BTreeMap<String, Vec<u8>>,
}

impl Egm {
    /// `upstream` is `UPSTREAM.json`; `files` every other vendored file, by
    /// its path under the vendored root. Refused, naming each bad file, when
    /// the copy does not match its pin.
    pub fn verified(upstream: &str, files: Vec<(String, Vec<u8>)>) -> Res<Self> {
        let json: serde_json::Value = serde_json::from_str(upstream).map_err(|e| format!("UPSTREAM.json: {e}"))?;
        let field = |k: &str| json.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string();
        let recorded = json
            .get("files")
            .and_then(|f| f.as_object())
            .ok_or("UPSTREAM.json has no files")?
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
            .collect();
        let pin = Pin { repository: field("repository"), commit: field("commit"), files: recorded };
        let files: BTreeMap<String, Vec<u8>> = files.into_iter().collect();
        let present: Vec<String> = files.keys().cloned().collect();
        let read = |p: &str| files.get(p).cloned();
        let sha = |b: &[u8]| format!("{:x}", Sha256::digest(b));
        design::verify_pin(&pin, &present, &read, &sha).map_err(|bad| {
            format!("the vendored EGM does not match its pin; update EGM by a new pin, never an edit: {}", bad.join("; "))
        })?;
        Ok(Self { pin, files })
    }

    fn file(&self, rel: &str) -> Res<String> {
        self.files
            .get(rel)
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .ok_or_else(|| format!("EGM's {rel} is not vendored"))
    }

    fn persona(&self, stem: &str) -> Res<String> {
        self.file(&format!("agents/{stem}.md"))
    }

    fn standard(&self, stem: &str) -> Res<String> {
        self.file(&format!("resources/document_standards/{stem}.md"))
    }

    /// The attribution line every EGM-derived artifact carries (§2).
    fn credit(&self, what: &str) -> String {
        format!(
            "{what}, from the Elephant-Goldfish Model (EGM, {}, Google LLC, Apache-2.0) at commit {}",
            self.pin.repository, self.pin.commit
        )
    }
}

// ------------------------------------------------------------------ models

/// The goldfish's model: `named`, else tier t2.5, else `claude-text`. An
/// agent route is refused before any call (§3).
pub fn goldfish_model(named: Option<&str>) -> Res<String> {
    let model = named
        .map(str::to_string)
        .or_else(|| hexa_infer::tier_model("t2.5"))
        .unwrap_or_else(hexa_infer::tool_less_default);
    refuse_an_agent(&model, "a goldfish must see only its inputs (ADR-2610271736 §3)")?;
    Ok(model)
}

/// The elephant's model: `named`, else tier t3, else `claude-text`. It too
/// is a completion: an agent could write outside the artifact directory (§8).
pub fn elephant_model(named: Option<&str>) -> Res<String> {
    let model = named
        .map(str::to_string)
        .or_else(|| hexa_infer::tier_model("t3"))
        .unwrap_or_else(hexa_infer::tool_less_default);
    refuse_an_agent(&model, "the elephant must not write files, and only hexa writes design artifacts (ADR-2610271736 §8)")?;
    Ok(model)
}

fn refuse_an_agent(model: &str, why: &str) -> Res<()> {
    if hexa_infer::wiring::served_by_frontier_cli(model) {
        return Err(format!(
            "refusing '{model}': it is served by an agent that could read and edit the tree, and {why}. \
             Name a completion route, e.g. claude-text or claude-text:<model>, with --model or the tier"
        ));
    }
    Ok(())
}

async fn complete(model: &str, prompt: &str) -> Res<String> {
    hexa_infer::complete_text(model, "", prompt, MAX_TOKENS).await.map_err(|e| format!("{model}: {e}"))
}

// ------------------------------------------------------------------ files

/// Writes only under the allowed directories, inside the project (§8).
pub struct Writer {
    root: PathBuf,
    allowed: Vec<String>,
}

impl Writer {
    fn write(&self, rel: &str, text: &str, say: &dyn Fn(&str)) -> Res<()> {
        let ok = design::is_inside_project(rel)
            && self.allowed.iter().any(|d| rel.starts_with(&format!("{}/", d.trim_end_matches('/'))));
        if !ok {
            return Err(format!(
                "refusing to write {rel}: hexa design and hexa drift write only under {}",
                self.allowed.join(" and ")
            ));
        }
        let path = self.root.join(rel);
        let parent = path.parent().ok_or_else(|| format!("{rel} has no parent"))?;
        std::fs::create_dir_all(parent).map_err(|e| format!("{rel}: {e}"))?;
        let real_parent = parent.canonicalize().map_err(|e| format!("{rel}: {e}"))?;
        let real_root = self.root.canonicalize().map_err(|e| format!("{}: {e}", self.root.display()))?;
        if !real_parent.starts_with(&real_root) {
            return Err(format!("refusing to write {rel}: it resolves outside the project"));
        }
        std::fs::write(&path, text).map_err(|e| format!("writing {rel}: {e}"))?;
        say(&format!("wrote {rel}"));
        Ok(())
    }
}

/// Where a design run works: the project root and its artifact directory.
pub struct Workspace {
    pub root: PathBuf,
    pub dir: String,
    writer: Writer,
}

impl Workspace {
    /// The directory the `EGM artifact directory: <path>` line in CLAUDE.md
    /// names, else `docs/designs` (§6).
    pub fn at(root: PathBuf) -> Res<Self> {
        let claude_md = std::fs::read_to_string(root.join("CLAUDE.md")).unwrap_or_default();
        let dir = match design::artifact_dir_setting(&claude_md) {
            Some(d) if design::is_inside_project(&d) => d,
            Some(d) => return Err(format!("CLAUDE.md names the EGM artifact directory {d}, which is outside the project")),
            None => DEFAULT_ARTIFACT_DIR.to_string(),
        };
        let writer = Writer { root: root.clone(), allowed: vec![dir.clone(), ANALYSIS_DIR.to_string()] };
        Ok(Self { root, dir, writer })
    }

    /// `rel` (from the current directory) relative to the project root, if inside it.
    fn relative(&self, rel: &str) -> Res<String> {
        let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
        let abs = cwd.join(rel).canonicalize().map_err(|_| format!("{rel} not found"))?;
        let root = self.root.canonicalize().map_err(|e| e.to_string())?;
        let r = abs.strip_prefix(&root).map_err(|_| format!("{rel} is outside the project"))?;
        Ok(r.to_string_lossy().replace('\\', "/"))
    }

    fn read(&self, rel: &str) -> Res<String> {
        std::fs::read_to_string(self.root.join(rel)).map_err(|e| format!("reading {rel}: {e}"))
    }
}

/// Does `rel` name a file inside the project?
fn exists_in(root: &Path) -> impl Fn(&str) -> bool + '_ {
    move |rel: &str| {
        if !design::is_inside_project(rel) {
            return false;
        }
        let p = root.join(rel);
        match (p.canonicalize(), root.canonicalize()) {
            (Ok(real), Ok(r)) => p.is_file() && real.starts_with(r),
            _ => false,
        }
    }
}

/// The source a document names (§4): what goes into the prompt, what was
/// left out for the file budget, and what did not resolve.
struct Sources {
    supplied: Vec<(String, String)>,
    omitted: Vec<String>,
    unresolved: Vec<String>,
}

fn sources_of(root: &Path, document: &str, exclude: &str) -> Sources {
    let exists = exists_in(root);
    let mut paths = design::referenced_paths(document, &exists);
    paths.retain(|p| p != exclude);
    let omitted = paths.split_off(paths.len().min(MAX_SOURCES));
    let supplied = paths
        .into_iter()
        .map(|p| {
            let text = std::fs::read(root.join(&p)).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
            (p, text)
        })
        .collect();
    Sources { supplied, omitted, unresolved: design::unresolved_paths(document, &exists) }
}

impl Sources {
    fn describe(&self) -> String {
        let list = |v: &[String]| v.iter().map(|p| format!("`{p}`")).collect::<Vec<_>>().join(", ");
        let mut s = String::new();
        if !self.supplied.is_empty() {
            let names: Vec<String> = self.supplied.iter().map(|(p, _)| p.clone()).collect();
            s.push_str(&format!("Source supplied (each cut at {SOURCE_CAP} bytes): {}\n", list(&names)));
        }
        if !self.omitted.is_empty() {
            s.push_str(&format!("Source not supplied (over the {MAX_SOURCES}-file budget): {}\n", list(&self.omitted)));
        }
        if !self.unresolved.is_empty() {
            s.push_str(&format!(
                "Unresolved paths (outside the project or not found; not read): {}\n",
                list(&self.unresolved)
            ));
        }
        s
    }
}

/// The project context the elephant holds: CLAUDE.md, README.md and
/// ARCHITECTURE.md, each cut, then the source `document` names.
fn project_context(root: &Path, document: &str) -> String {
    let mut out = String::new();
    for f in ["CLAUDE.md", "README.md", "ARCHITECTURE.md"] {
        if let Ok(text) = std::fs::read_to_string(root.join(f)) {
            out.push_str(&format!("## {f}\n\n{}\n\n", cut(&text, CONTEXT_CAP)));
        }
    }
    for (p, text) in sources_of(root, document, "").supplied {
        out.push_str(&format!("## {p}\n\n{}\n\n", cut(&text, CONTEXT_CAP)));
    }
    out
}

fn cut(text: &str, cap: usize) -> &str {
    if text.len() <= cap {
        return text;
    }
    let mut end = cap;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

// ------------------------------------------------------------------ the elephant

/// One elephant completion on EGM skills, run without a person to ask.
fn elephant_prompt(egm: &Egm, skills: &[&str], artifact: &str, context: &str, inputs: &str) -> Res<String> {
    let mut p = format!(
        "You are the elephant in an Elephant-Goldfish design workflow, run by hexa as one completion. \
         There is no one to ask and no tool to call: where the instructions below say to ask the user or to \
         spawn reviewers, decide the most defensible option yourself and state the assumption in the document; \
         hexa runs the reviewers separately. You write no file and no production code. Reply with the complete \
         Markdown of {artifact} and nothing else.\n\n"
    );
    for s in skills {
        p.push_str(&format!("# EGM instructions: {s}\n\n{}\n\n", egm.file(s)?));
    }
    p.push_str(&format!("# EGM execution instructions\n\n{}\n\n", egm.file("resources/execution_instructions.md")?));
    p.push_str(&format!("# Project context\n\n{context}\n"));
    p.push_str(inputs);
    Ok(p)
}

/// A reply that is one fenced Markdown block, unwrapped.
fn unfence(reply: &str) -> String {
    let t = reply.trim();
    let lines: Vec<&str> = t.lines().collect();
    if lines.len() >= 2 {
        let first = lines[0].trim();
        let last = lines[lines.len() - 1].trim();
        if (first == "```" || first == "```markdown" || first == "```md") && last == "```" {
            return lines[1..lines.len() - 1].join("\n");
        }
    }
    t.to_string()
}

/// The HTML comment hexa puts at the top of a document it wrote.
fn header(line: &str) -> String {
    format!("<!-- {} -->\n\n", line.replace("--", "\u{2013}"))
}

/// `text` without a header hexa put there.
fn strip_header(text: &str) -> &str {
    match text.strip_prefix("<!-- hexa design") {
        Some(rest) => rest.split_once("-->").map(|(_, r)| r.trim_start()).unwrap_or(text),
        None => text,
    }
}

/// `hexa design brainstorm` (§6): the elephant writes `concepts_brief.md`,
/// then the critic goldfish reviews it into `concepts_brief_critique.md`.
pub async fn brainstorm(
    egm: &Egm,
    ws: &Workspace,
    idea: &str,
    elephant: &str,
    critic: &str,
    force: bool,
    say: &dyn Fn(&str),
) -> Res<()> {
    let brief_rel = format!("{}/concepts_brief.md", ws.dir);
    if ws.root.join(&brief_rel).exists() && !force {
        return Err(format!("{brief_rel} exists; --force replaces it"));
    }
    say(&format!("the elephant on {elephant}, on egm-brainstorm"));
    let prompt = elephant_prompt(
        egm,
        &["skills/egm-brainstorm/SKILL.md"],
        "concepts_brief.md",
        &project_context(&ws.root, idea),
        &format!("# The idea\n\n{idea}\n"),
    )?;
    let brief = unfence(&complete(elephant, &prompt).await?);
    let credit = egm.credit(&format!("hexa design brainstorm: the elephant ({elephant}) on egm-brainstorm"));
    ws.writer.write(&brief_rel, &format!("{}{brief}\n", header(&credit)), say)?;

    say(&format!("the critic goldfish (design_critic) on {critic}"));
    let sources = sources_of(&ws.root, &brief, &brief_rel);
    let prompt = design::goldfish_prompt(
        &egm.persona("design_critic")?,
        Some(&egm.standard("heilmeier-standard")?),
        &brief,
        &sources.supplied,
        SOURCE_CAP,
    );
    let items = design::list_items(&complete(critic, &prompt).await?);
    let mut text = format!(
        "# Critique of the concepts brief\n\nBrief: `{brief_rel}`\nReviewer: persona `design_critic`, standard \
         `heilmeier-standard`, on `{critic}`\n{}\n{}\n",
        egm.credit("Written by hexa design brainstorm"),
        sources.describe()
    );
    text.push_str(&items_markdown(&items, "C"));
    ws.writer.write(&format!("{}/concepts_brief_critique.md", ws.dir), &text, say)
}

/// `hexa design draft` (§6): the elephant writes `design_draft.md` from a brief.
pub async fn draft(
    egm: &Egm,
    ws: &Workspace,
    brief: Option<&str>,
    elephant: &str,
    force: bool,
    say: &dyn Fn(&str),
) -> Res<String> {
    let brief_rel = match brief {
        Some(b) => ws.relative(b)?,
        None => format!("{}/concepts_brief.md", ws.dir),
    };
    let brief_text = ws
        .read(&brief_rel)
        .map_err(|_| format!("no brief at {brief_rel}: run `hexa design brainstorm \"<idea>\"` or name one"))?;
    let draft_rel = format!("{}/design_draft.md", ws.dir);
    if ws.root.join(&draft_rel).exists() && !force {
        return Err(format!("{draft_rel} exists; --force replaces it"));
    }
    say(&format!("{brief_rel}: the elephant on {elephant}, on egm-draft"));
    let brief_text = strip_header(&brief_text);
    let prompt = elephant_prompt(
        egm,
        &["skills/egm-draft/SKILL.md", "skills/egm-draft/references/design_doc.md"],
        "design_draft.md",
        &project_context(&ws.root, brief_text),
        &format!("# The concepts brief\n\n{brief_text}\n"),
    )?;
    let text = unfence(&complete(elephant, &prompt).await?);
    let credit = egm.credit(&format!("hexa design draft: the elephant ({elephant}) on egm-draft, from {brief_rel}"));
    ws.writer.write(&draft_rel, &format!("{}{text}\n", header(&credit)), say)?;
    Ok(draft_rel)
}

/// `hexa design resolve` (§6): the elephant revises the draft the report
/// names with only the accepted findings. Returns the draft's path.
pub async fn resolve(
    egm: &Egm,
    ws: &Workspace,
    report: &str,
    accept: &[String],
    elephant: &str,
    say: &dyn Fn(&str),
) -> Res<String> {
    let report_text = std::fs::read_to_string(report).map_err(|e| format!("reading {report}: {e}"))?;
    let accept: Vec<String> = accept.iter().map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).collect();
    let (chosen, missing) = design::accepted_findings(&report_text, &accept);
    for m in &missing {
        say(&format!("{report} has no finding {m}"));
    }
    if chosen.is_empty() {
        return Err(format!("none of {} is a finding in {report}", accept.join(", ")));
    }
    let draft_rel = report_text
        .lines()
        .find_map(|l| l.trim().strip_prefix("Draft:").map(|d| d.trim().trim_matches('`').trim().to_string()))
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| format!("{}/design_draft.md", ws.dir));
    if !design::is_inside_project(&draft_rel) || !draft_rel.starts_with(&format!("{}/", ws.dir)) {
        return Err(format!(
            "{report} names the draft {draft_rel}, outside the artifact directory {}/, where resolve writes",
            ws.dir
        ));
    }
    let draft_text = ws.read(&draft_rel)?;
    let draft_text = strip_header(&draft_text);
    let ids: Vec<&str> = chosen.iter().map(|f| f.id.as_str()).collect();
    say(&format!("{draft_rel} with {}: the elephant on {elephant}, on egm-resolve", ids.join(", ")));
    let findings: String = chosen.iter().map(|f| format!("- {}: {}\n", f.id, f.text)).collect();
    let prompt = elephant_prompt(
        egm,
        &["skills/egm-resolve/SKILL.md"],
        "the revised design draft",
        &project_context(&ws.root, draft_text),
        &format!(
            "# The draft\n\n{draft_text}\n\n# The findings the person accepted\n\nApply these and only these. \
             Every other finding was declined.\n\n{findings}"
        ),
    )?;
    let text = unfence(&complete(elephant, &prompt).await?);
    let credit = egm.credit(&format!(
        "hexa design resolve: the elephant ({elephant}) on egm-resolve applied {} from {report}",
        ids.join(", ")
    ));
    ws.writer.write(&draft_rel, &format!("{}{text}\n", header(&credit)), say)?;
    Ok(draft_rel)
}

// ------------------------------------------------------------------ the goldfish

fn items_markdown(items: &[String], prefix: &str) -> String {
    if items.is_empty() {
        return "No findings.\n".to_string();
    }
    items
        .iter()
        .enumerate()
        .map(|(i, t)| format!("- {prefix}-{}: {}\n", i + 1, t.replace('\n', "\n  ")))
        .collect()
}

/// `hexa design review` (§5): EGM's stages in order, the reviewers of a stage
/// concurrently, each stage deduplicated by the synthesizer goldfish. Writes
/// `goldfish_review_report.md` in the artifact directory and returns its
/// path; never edits the draft.
pub async fn review(
    egm: &Egm,
    ws: &Workspace,
    draft: &str,
    scope: ReviewScope,
    model: &str,
    say: &dyn Fn(&str),
) -> Res<String> {
    let draft_rel = ws.relative(draft)?;
    let document = ws.read(&draft_rel)?;
    let stages = match scope {
        ReviewScope::Stage(n) => {
            vec![design::stage_from_number(n).ok_or_else(|| format!("--stage {n}: EGM's review stages are 5 to 9"))?]
        }
        ReviewScope::Full => design::stages_for(Preset::Full),
        ReviewScope::FeatureSpecOnly => design::stages_for(Preset::FeatureSpecOnly),
        ReviewScope::ImplementationPlanOnly => design::stages_for(Preset::ImplementationPlanOnly),
    };
    let sources = sources_of(&ws.root, &document, &draft_rel);
    let mut report = format!(
        "# Design review of {draft_rel}\n\nDraft: `{draft_rel}`\nModel: `{model}`\n{}. Each reviewer is one \
         tool-less completion given its persona, its standard, the draft and the source the draft names; \
         a synthesizer deduplicates each section.\n{}\n",
        egm.credit("Written by hexa design review"),
        sources.describe()
    );
    for stage in stages {
        let n = design::stage_number(stage);
        let reviewers = design::stage_reviewers(stage);
        say(&format!("stage {n}, {}: {}", design::stage_name(stage), reviewers.join(", ")));
        // Concurrent within the stage; the next stage starts only when every
        // reviewer of this one has returned (§5).
        let calls = reviewers.iter().map(|stem| {
            let (document, sources) = (&document, &sources);
            async move {
                let persona = egm.persona(stem)?;
                let standard = design::stage_standard(stage, stem).map(|s| egm.standard(s)).transpose()?;
                let prompt =
                    design::goldfish_prompt(&persona, standard.as_deref(), document, &sources.supplied, SOURCE_CAP);
                Ok::<_, String>((*stem, complete(model, &prompt).await?))
            }
        });
        let replies = join_all(calls).await.into_iter().collect::<Res<Vec<_>>>()?;
        let items = synthesize(egm, model, stage, &replies).await?;
        let findings = design::number_findings(stage, &items);
        let personas: Vec<String> = reviewers
            .iter()
            .map(|s| match design::stage_standard(stage, s) {
                Some(std) => format!("`{s}` (standard `{std}`)"),
                None => format!("`{s}`"),
            })
            .collect();
        report.push_str(&format!(
            "## Stage {n}: {}\n\nPersonas: {}; synthesized by `{}`.\n\n",
            design::stage_name(stage),
            personas.join(", "),
            design::SYNTHESIZER
        ));
        if findings.is_empty() {
            report.push_str("No findings.\n");
        }
        for f in &findings {
            report.push_str(&format!("- {}: {}\n", f.id, f.text.replace('\n', "\n  ")));
        }
        report.push('\n');
    }
    let report_rel = format!("{}/goldfish_review_report.md", ws.dir);
    ws.writer.write(&report_rel, &report, say)?;
    Ok(report_rel)
}

/// A stage's findings, deduplicated by the synthesizer goldfish, which sees
/// the reviewers' findings and nothing else.
async fn synthesize(egm: &Egm, model: &str, stage: Stage, replies: &[(&str, String)]) -> Res<Vec<String>> {
    let found: Vec<(&str, Vec<String>)> = replies.iter().map(|(s, r)| (*s, design::list_items(r))).collect();
    if found.iter().all(|(_, items)| items.is_empty()) {
        return Ok(Vec::new());
    }
    let mut findings = String::new();
    for (stem, items) in &found {
        findings.push_str(&format!("## Findings of `{stem}`\n\n"));
        for i in items {
            findings.push_str(&format!("- {}\n", i.replace('\n', "\n  ")));
        }
        findings.push('\n');
    }
    let instruction = format!(
        "hexa's instruction for this call: you are given the findings of stage {} ({}) of a design review, \
         one list per reviewer. Merge findings that say the same thing, keep every distinct one, and drop none \
         for being minor. Reply with one Markdown list, one `- ` item per finding.",
        design::stage_number(stage),
        design::stage_name(stage)
    );
    let prompt =
        design::goldfish_prompt(&egm.persona(design::SYNTHESIZER)?, Some(&instruction), &findings, &[], SOURCE_CAP);
    Ok(design::list_items(&complete(model, &prompt).await?))
}

// ------------------------------------------------------------------ drift

/// One document's drift: its divergences, empty when none.
pub struct DocDrift {
    pub document: String,
    pub divergences: Vec<String>,
}

/// `hexa drift` (§7): one goldfish per document on EGM's drift prompt.
/// Writes `docs/analysis/<today>_DRIFT_REPORT.md` and nothing else; returns
/// its path and each document's result.
pub async fn drift(
    egm: &Egm,
    root: &Path,
    dir: Option<&str>,
    model: &str,
    today: &str,
    say: &dyn Fn(&str),
) -> Res<(String, Vec<DocDrift>)> {
    const PERSONA: &str = "codebase_details";
    const SKILL: &str = "skills/egm-drift-report/SKILL.md";
    let writer = Writer { root: root.to_path_buf(), allowed: vec![ANALYSIS_DIR.to_string()] };
    let docs = drift_documents(root, dir)?;
    if docs.is_empty() {
        return Err("no Markdown document to check".into());
    }
    say(&format!("{} document(s) on {model}", docs.len()));
    let persona = egm.persona(PERSONA)?;
    let instruction = format!(
        "{}\n\nhexa's instruction for this call: apply the drift instructions above to the one document under \
         review, using only the source given here. Report each divergence as a `- ` item. If the document and the \
         code agree, reply `No drift detected.`",
        egm.file(SKILL)?
    );
    let calls = docs.iter().map(|doc| {
        let (persona, instruction) = (&persona, &instruction);
        async move {
            let text = std::fs::read_to_string(root.join(doc)).map_err(|e| format!("reading {doc}: {e}"))?;
            let sources = sources_of(root, &text, doc);
            let prompt = design::goldfish_prompt(persona, Some(instruction), &text, &sources.supplied, SOURCE_CAP);
            let reply = complete(model, &prompt).await?;
            Ok::<_, String>((doc.clone(), design::list_items(&reply), sources))
        }
    });
    let results = join_all(calls).await.into_iter().collect::<Res<Vec<_>>>()?;

    let mut report = format!(
        "# Drift report {today}\n\nModel: `{model}`\nReviewer: persona `{PERSONA}`, prompt `{SKILL}`, one tool-less \
         completion per document, given the document and the source it names.\n{}. ADRs are history and are not \
         checked.\n\n",
        egm.credit("Written by hexa drift")
    );
    let mut out = Vec::new();
    for (doc, items, sources) in results {
        report.push_str(&format!("## {doc}\n\n"));
        if items.is_empty() {
            report.push_str("No drift detected\n\n");
        } else {
            for i in &items {
                report.push_str(&format!("- {}\n", i.replace('\n', "\n  ")));
            }
            report.push('\n');
        }
        let described = sources.describe();
        if !described.is_empty() {
            report.push_str(&described);
            report.push('\n');
        }
        out.push(DocDrift { document: doc, divergences: items });
    }
    let rel = format!("{ANALYSIS_DIR}/{today}_DRIFT_REPORT.md");
    writer.write(&rel, &report, say)?;
    Ok((rel, out))
}

/// The documents `hexa drift` checks: every Markdown file under `dir`, or
/// README.md, ARCHITECTURE.md and docs/*.md. ADRs and drift reports are not.
fn drift_documents(root: &Path, dir: Option<&str>) -> Res<Vec<String>> {
    let is_adr_dir = |name: &str| matches!(name.to_ascii_lowercase().as_str(), "adr" | "adrs");
    let keep = |name: &str| {
        name.ends_with(".md") && !name.to_ascii_uppercase().starts_with("ADR-") && !name.ends_with("_DRIFT_REPORT.md")
    };
    let mut docs = Vec::new();
    match dir {
        Some(d) => {
            let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
            let base = cwd.join(d).canonicalize().map_err(|_| format!("{d} not found"))?;
            let real_root = root.canonicalize().map_err(|e| e.to_string())?;
            if !base.starts_with(&real_root) {
                return Err(format!("{d} is outside the project"));
            }
            let mut stack = vec![base];
            while let Some(p) = stack.pop() {
                for e in std::fs::read_dir(&p).map_err(|e| format!("reading {}: {e}", p.display()))?.flatten() {
                    let name = e.file_name().to_string_lossy().into_owned();
                    let path = e.path();
                    if path.is_dir() {
                        if !name.starts_with('.') && !is_adr_dir(&name) {
                            stack.push(path);
                        }
                    } else if keep(&name) {
                        if let Ok(rel) = path.strip_prefix(&real_root) {
                            docs.push(rel.to_string_lossy().replace('\\', "/"));
                        }
                    }
                }
            }
            docs.sort();
        }
        None => {
            for f in ["README.md", "ARCHITECTURE.md"] {
                if root.join(f).is_file() {
                    docs.push(f.to_string());
                }
            }
            let mut in_docs: Vec<String> = std::fs::read_dir(root.join("docs"))
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| e.path().is_file())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| keep(n))
                .map(|n| format!("docs/{n}"))
                .collect();
            in_docs.sort();
            docs.extend(in_docs);
        }
    }
    Ok(docs)
}
