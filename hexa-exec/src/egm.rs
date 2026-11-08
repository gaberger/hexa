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
use hexa_core::domain::workflow::{self, DesignNext, DesignState};
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

/// The project context the elephant holds: CLAUDE.md, README.md,
/// ARCHITECTURE.md and the build manifest, each cut, then the source
/// `document` names. The manifest says what language the project is in: a
/// template CLAUDE.md alone once led the elephant to design a Rust project
/// in TypeScript.
fn project_context(root: &Path, document: &str) -> String {
    let mut out = String::new();
    for f in ["CLAUDE.md", "README.md", "ARCHITECTURE.md", "Cargo.toml", "go.mod", "package.json"] {
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
    say: &dyn Fn(&str),
) -> Res<()> {
    let brief_rel = format!("{}/concepts_brief.md", ws.dir);
    keep_previous(ws, &brief_rel, say)?;
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

/// Before an artifact is rewritten, its current text is kept beside it as
/// `<name>.previous.md`, so a person's edits are never lost to a re-run.
fn keep_previous(ws: &Workspace, rel: &str, say: &dyn Fn(&str)) -> Res<()> {
    let Ok(text) = std::fs::read_to_string(ws.root.join(rel)) else { return Ok(()) };
    let previous = format!("{}.previous.md", rel.trim_end_matches(".md"));
    ws.writer.write(&previous, &text, say)
}

/// `hexa design draft` (§6): the elephant writes `design_draft.md` from a brief.
pub async fn draft(
    egm: &Egm,
    ws: &Workspace,
    brief: Option<&str>,
    elephant: &str,
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
    keep_previous(ws, &draft_rel, say)?;
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
    let bytes = std::fs::read(ws.root.join(&draft_rel)).map_err(|e| format!("reading {draft_rel}: {e}"))?;
    let document = String::from_utf8_lossy(&bytes).into_owned();
    // The bytes this review read (ADR-2610281011 §2), said before any call.
    let draft_sha = sha256_hex(&bytes);
    say(&format!("Draft sha256: {draft_sha}"));
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
        "# Design review of {draft_rel}\n\nDraft: `{draft_rel}`\nDraft sha256: {draft_sha}\nModel: `{model}`\n{}. Each reviewer is one \
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
    // One reviewer: its list is the stage's findings; there is nothing to
    // merge (ADR-2610281516 §4).
    if !design::needs_synthesis(stage) {
        return Ok(found.into_iter().flat_map(|(_, items)| items).collect());
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
    since: Option<&str>,
    model: &str,
    today: &str,
    say: &dyn Fn(&str),
) -> Res<(String, Vec<DocDrift>)> {
    const PERSONA: &str = "codebase_details";
    const SKILL: &str = "skills/egm-drift-report/SKILL.md";
    let writer = Writer { root: root.to_path_buf(), allowed: vec![ANALYSIS_DIR.to_string()] };
    let mut docs = drift_documents(root, dir)?;
    if docs.is_empty() {
        return Err("no Markdown document to check".into());
    }
    // Only what a change touched (ADR-2610281011 §6).
    if let Some(base) = since {
        let changed = changed_since(root, base)?;
        let named: Vec<(String, Vec<String>)> = docs
            .iter()
            .map(|d| {
                let text = std::fs::read_to_string(root.join(d)).unwrap_or_default();
                (d.clone(), design::referenced_paths(&text, &|_| true))
            })
            .collect();
        docs = design::docs_touched(&named, &changed);
        if docs.is_empty() {
            let line = format!("No document touched since {base}");
            say(&line);
            let report = format!("# Drift report {today}\n\n{line}.\n");
            let rel = format!("{ANALYSIS_DIR}/{today}_DRIFT_REPORT.md");
            writer.write(&rel, &report, say)?;
            return Ok((rel, Vec::new()));
        }
    }
    say(&format!("{} document(s) on {model}: {}", docs.len(), docs.join(", ")));
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

// ------------------------------------------------------------------ git

fn git(root: &Path, args: &[&str]) -> Res<std::process::Output> {
    std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| format!("git {}: {e}", args.join(" ")))
}

/// Paths `git diff --name-only <base>` lists: committed and uncommitted changes since `base`.
fn changed_since(root: &Path, base: &str) -> Res<Vec<String>> {
    let out = git(root, &["diff", "--name-only", base, "--"])?;
    if !out.status.success() {
        return Err(format!("git diff {base}: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).lines().map(str::to_string).filter(|l| !l.is_empty()).collect())
}

fn tracked(root: &Path, rel: &str) -> bool {
    git(root, &["ls-files", "--error-unmatch", "--", rel]).is_ok_and(|o| o.status.success())
}

/// Tracked by git, with no uncommitted change.
fn committed(root: &Path, rel: &str) -> bool {
    tracked(root, rel)
        && git(root, &["status", "--porcelain", "--", rel]).is_ok_and(|o| o.status.success() && o.stdout.is_empty())
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

// ------------------------------------------------------------------ the loop's design step

/// The loop's stage words, in order (ADR-2610281011 §1).
pub fn loop_stage_words() -> Vec<&'static str> {
    workflow::loop_stages().into_iter().map(workflow::stage_word).collect()
}

/// Every review report in the artifact directory: the Markdown files that
/// record a draft sha256, by path.
fn review_reports(ws: &Workspace) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(ws.root.join(&ws.dir)) else { return out };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".md") || !e.path().is_file() {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(e.path()) {
            if design::reviewed_sha(&text).is_some() {
                out.push((format!("{}/{name}", ws.dir), text));
            }
        }
    }
    out.sort();
    out
}

fn design_state(ws: &Workspace, draft: &str) -> DesignState {
    DesignState {
        draft: Some(draft.to_string()),
        draft_sha256: std::fs::read(ws.root.join(draft)).ok().map(|b| sha256_hex(&b)),
        reviews: review_reports(ws).into_iter().map(|(_, t)| t).collect(),
        committed: committed(&ws.root, draft),
    }
}

/// The command `hexa go` suggests for the recorded draft (§5), if any.
pub fn next_design_command(ws: &Workspace, draft: Option<&str>) -> Option<String> {
    let state = match draft {
        Some(d) => design_state(ws, d),
        None => DesignState { draft: None, draft_sha256: None, reviews: Vec::new(), committed: false },
    };
    match workflow::next_design_action(&state) {
        DesignNext::Nothing => None,
        DesignNext::Review(d) => Some(format!("hexa design review {d}")),
        DesignNext::Commit(d) => Some(format!("git commit {d}")),
        DesignNext::ToAdr(d) => Some(format!("hexa design adr {d}")),
    }
}

/// `draft` relative to the project root, if it is a file inside the project
/// (`hexa loop design`, §1).
pub fn draft_in_project(ws: &Workspace, draft: &str) -> Res<String> {
    let refuse = || format!("{draft} is not a file inside the project");
    let rel = ws.relative(draft).map_err(|_| refuse())?;
    if !ws.root.join(&rel).is_file() {
        return Err(refuse());
    }
    Ok(rel)
}

/// `hexa design adr` (§3): a reviewed, committed draft compiled into a
/// proposed ADR that cites it. Returns the new ADR's id and path. The ADR's
/// decisions are the design's sections, numbered; a person edits them
/// before accepting. No model is called and no spec is written.
pub fn design_adr(ws: &Workspace, draft: &str, now: chrono::NaiveDateTime, say: &dyn Fn(&str)) -> Res<(String, String)> {
    let rel = ws.relative(draft)?;
    if !tracked(&ws.root, &rel) {
        return Err(format!("{rel} is not tracked by git: commit the design first, so the decision cites what was reviewed"));
    }
    if !committed(&ws.root, &rel) {
        return Err(format!("{rel} has uncommitted changes: commit it, so the decision cites what was reviewed"));
    }
    let bytes = std::fs::read(ws.root.join(&rel)).map_err(|e| format!("reading {rel}: {e}"))?;
    let sha = sha256_hex(&bytes);
    let reports = review_reports(ws);
    let Some((report, _)) = reports.iter().find(|(_, t)| design::review_covers(t, &sha).is_ok()) else {
        let why: Vec<String> = reports
            .iter()
            .map(|(p, t)| format!("{p}: {}", design::review_covers(t, &sha).err().unwrap_or_default()))
            .collect();
        return Err(format!(
            "no review covers {rel} as it is now (sha256 {sha}); run `hexa design review {rel}`{}",
            if why.is_empty() { String::new() } else { format!(". {}", why.join("; ")) }
        ));
    };
    let text = String::from_utf8_lossy(&bytes);
    let text = strip_header(&text);
    let Compiled { title, context, decisions, files } = compile_design(text, &rel);
    let adr_dir = "docs/adrs";
    let mut stamp = now;
    let id = loop {
        let id = format!("ADR-{}", stamp.format("%y%m%d%H%M"));
        let taken = std::fs::read_dir(ws.root.join(adr_dir))
            .map(|rd| rd.flatten().any(|e| e.file_name().to_string_lossy().starts_with(&format!("{id}-"))))
            .unwrap_or(false);
        if !taken {
            break id;
        }
        stamp += chrono::Duration::minutes(1);
    };
    let slug = slug(&title);
    let path = format!("{adr_dir}/{id}-{slug}.md");
    let date = now.format("%Y-%m-%d");
    let numbered: String = decisions.iter().enumerate().map(|(i, d)| format!("{}. {d}\n", i + 1)).collect();
    let implementation = if files.is_empty() {
        "To be written before acceptance: name the port and domain files each decision changes.".to_string()
    } else {
        format!(
            "The port and domain files the design names, which `hexa spec draft` shows the spec author: {}. \
             Write their new signatures before drafting the spec.",
            files.iter().map(|f| format!("`{f}`")).collect::<Vec<_>>().join(", ")
        )
    };
    let body = format!(
        "---\nid: {id}\nstatus: proposed\ndate: {date}\n---\n# {id}: {title}\n\n**Status:** Proposed\n**Date:** {date}\n\
         **Drivers:** The design `{rel}`, reviewed by the goldfish at sha256 `{sha}` (`{report}`).\n**Design:** {rel}\n\n\
         ## Context\n\n{context}\n\n## Decision\n\n{numbered}\n## Consequences\n\nTo be written before acceptance.\n\n\
         ## Implementation\n\n{implementation}\n\n\
         ## Gate\n\nTo be written before acceptance: the command that must exit 0.\n\n\
         ## References\n\n- `{rel}`: the reviewed design.\n- `{report}`: its review.\n"
    );
    let writer = Writer { root: ws.root.clone(), allowed: vec![adr_dir.to_string()] };
    writer.write(&path, &body, say)?;
    Ok((id, path))
}

/// What a design compiles to.
struct Compiled {
    title: String,
    context: String,
    decisions: Vec<String>,
    /// The `domain/` and `ports/` files the design names, each once.
    files: Vec<String>,
}

/// Sections of a design that describe the problem or the plan around a
/// decision rather than a decision: EGM's draft template names them so.
const NOT_DECISIONS: &[&str] = &[
    "overview", "the problem", "problem", "goals", "non-goals", "open questions", "critical user journeys",
    "alternatives considered", "non-functional requirements", "additional resources", "future improvements",
    "detailed implementation plan", "references",
];

/// A design compiled into an ADR's parts. The decisions are the `###`
/// subsections of its technical plan (a `##` section whose heading starts
/// `Technical Plan`, `Design` or `Architecture`), each as its heading in
/// bold and its first paragraph. A design without such a section gives one
/// decision per `##` section that is not one of [`NOT_DECISIONS`]. Context
/// is the opening paragraph and the problem's first paragraph.
fn compile_design(text: &str, rel: &str) -> Compiled {
    struct Section<'a> {
        level: usize,
        heading: String,
        body: Vec<&'a str>,
    }
    let mut title = None;
    let mut opening: Vec<&str> = Vec::new();
    let mut sections: Vec<Section> = Vec::new();
    let mut in_fence = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("```") {
            in_fence = !in_fence;
        }
        let level = if in_fence { 0 } else { t.chars().take_while(|c| *c == '#').count() };
        if level == 1 && title.is_none() {
            title = Some(t.trim_start_matches('#').trim().trim_start_matches("Design:").trim().to_string());
        } else if (2..=3).contains(&level) && t.chars().nth(level) == Some(' ') {
            sections.push(Section { level, heading: t[level..].trim().to_string(), body: Vec::new() });
        } else if let Some(s) = sections.last_mut() {
            s.body.push(line);
        } else {
            opening.push(line);
        }
    }
    // The first run of prose lines: fenced blocks and `**Key:** value`
    // metadata lines are not prose.
    let first_paragraph = |lines: &[&str]| -> String {
        let mut out: Vec<&str> = Vec::new();
        let mut fence = false;
        for l in lines.iter().map(|l| l.trim()) {
            if l.starts_with("```") || l.starts_with("~~~") {
                fence = !fence;
                if !out.is_empty() {
                    break;
                }
                continue;
            }
            let item = l.strip_prefix("* ").or_else(|| l.strip_prefix("- ")).unwrap_or(l).trim_start();
            let metadata = item.starts_with("**") && item.split("**").nth(1).is_some_and(|k| k.ends_with(':'));
            let rule = l.len() >= 3 && l.chars().all(|c| matches!(c, '-' | '*' | '_' | ' '));
            if fence || (out.is_empty() && (l.is_empty() || metadata || rule)) {
                continue;
            }
            if l.is_empty() {
                break;
            }
            out.push(l);
        }
        out.join(" ")
    };
    let decision = |s: &Section| match first_paragraph(&s.body) {
        p if p.is_empty() => format!("**{}.**", s.heading.trim_end_matches('.')),
        p => format!("**{}.** {p}", s.heading.trim_end_matches('.')),
    };
    let named = |s: &Section, names: &[&str]| {
        let h = s.heading.to_lowercase();
        names.iter().any(|n| h == *n || h.starts_with(&format!("{n} ")) || h.starts_with(&format!("{n}(")))
    };
    let title = title.unwrap_or_else(|| {
        Path::new(rel).file_stem().map(|s| s.to_string_lossy().replace(['-', '_'], " ")).unwrap_or_default()
    });

    // The technical plan's subsections, when the design has one.
    let plan = sections.iter().position(|s| s.level == 2 && named(s, &["technical plan", "design", "architecture"]));
    let mut decisions: Vec<String> = match plan {
        Some(i) => sections[i + 1..].iter().take_while(|s| s.level == 3).map(decision).collect(),
        None => Vec::new(),
    };
    if decisions.is_empty() {
        decisions = sections.iter().filter(|s| s.level == 2 && !named(s, NOT_DECISIONS)).map(decision).collect();
    }
    if decisions.is_empty() {
        decisions.push(format!("**Adopt the design.** As `{rel}` describes it."));
    }

    let mut context = first_paragraph(&opening);
    if context.is_empty() {
        if let Some(o) = sections.iter().find(|s| named(s, &["overview"])) {
            context = first_paragraph(&o.body);
        }
    }
    if let Some(p) = sections.iter().find(|s| named(s, &["the problem", "problem"])) {
        let p = first_paragraph(&p.body);
        if !p.is_empty() {
            context = if context.is_empty() { p } else { format!("{context}\n\n{p}") };
        }
    }
    let context = if context.is_empty() {
        format!("See the design `{rel}`.")
    } else {
        format!("{context}\n\nThe design is `{rel}`.")
    };

    let mut files: Vec<String> = Vec::new();
    for span in text.split('`').skip(1).step_by(2) {
        let layer = span.contains("domain/") || span.contains("ports/");
        if layer && design::is_inside_project(span) && !span.contains(char::is_whitespace) && span.contains('.')
            && !files.iter().any(|f| f == span)
        {
            files.push(span.to_string());
        }
    }
    Compiled { title, context, decisions, files }
}

fn slug(title: &str) -> String {
    let s: String = title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let s = s.split('-').filter(|w| !w.is_empty()).collect::<Vec<_>>().join("-");
    if s.is_empty() { "design".to_string() } else { s.chars().take(60).collect::<String>().trim_end_matches('-').to_string() }
}

#[cfg(test)]
mod compile_tests {
    use super::compile_design;

    #[test]
    fn a_technical_plan_gives_the_decisions_and_the_problem_is_context() {
        let d = "# Habits\n\n## Overview\n\nA tracker.\n\n## The Problem\n\nStreaks are hard.\n\n## Goals\n\n- g\n\n\
                 ## Technical Plan (Architecture)\n\n### Composition root\n\n`src/lib.rs` wires it.\n\n### Ports\n\n\
                 `src/ports/store.rs` holds `HabitStore`.\n\n## Alternatives Considered\n\n### Alt A\n\nNo.\n";
        let c = compile_design(d, "docs/designs/d.md");
        assert_eq!(c.title, "Habits");
        assert_eq!(c.decisions.len(), 2, "{:?}", c.decisions);
        assert!(c.decisions[0].starts_with("**Composition root.**"));
        assert!(c.context.contains("A tracker.") && c.context.contains("Streaks are hard."));
        assert_eq!(c.files, ["src/ports/store.rs"]);
    }

    #[test]
    fn without_a_plan_every_section_but_the_framing_ones_is_a_decision() {
        let d = "# Offline sync\n\nA design.\n\n## Proposal\n\nQueue writes.\n\n## Goals\n\nFast.\n";
        let c = compile_design(d, "docs/design/idea.md");
        assert_eq!(c.decisions, ["**Proposal.** Queue writes."]);
    }

    #[test]
    fn metadata_and_fenced_blocks_are_not_paragraphs() {
        let d = "# H\n\n*   **Author:** a\n*   **Date:** d\n\nThe opening.\n\n## Technical Plan\n\n### Layer map\n\n\
                 ```text\n+--+\n```\n\nThe layers, top down.\n";
        let c = compile_design(d, "x.md");
        assert!(c.context.starts_with("The opening."), "{}", c.context);
        assert_eq!(c.decisions, ["**Layer map.** The layers, top down."]);
    }
}
