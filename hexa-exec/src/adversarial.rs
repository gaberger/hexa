//! Adversarial review pipeline — the hexa-native cooperative+adversarial harness.
//!
//! Distilled from a 25-agent workflow that built a concurrent job queue: the build's
//! own passing tests still hid 6 real bugs; an *independent adversarial* pass found
//! them. This is that pass, made a first-class hexa capability:
//!
//!   hunt (parallel lenses) → skeptical verify (parallel, default-refute) → fix-loop
//!   (sequential, each fix gated by a ground-truth test command)
//!
//! Each agent is a `claude -p` worker (hexa's frontier path — no API key, no VRAM).
//! Findings flow between phases as structured JSON. The gate (a shell command that
//! must exit 0) is the only authority on whether a fix counts — the same
//! evidence-gate discipline as the do-loop.

use serde::Deserialize;
use std::path::Path;
use std::time::Duration;

fn claude_binary() -> String {
    std::env::var("HEXA_CLAUDE_BINARY").unwrap_or_else(|_| "claude".to_string())
}

/// One adversarial lens — a focused failure class a reviewer hunts.
struct Lens {
    key: &'static str,
    focus: &'static str,
}

const LENSES: &[Lens] = &[
    Lens { key: "correctness", focus: "logic errors, wrong results, broken invariants, off-by-one" },
    Lens { key: "concurrency", focus: "data races, deadlocks, non-atomic multi-step state transitions, TOCTOU, double-processing" },
    Lens { key: "durability-safety", focus: "data loss, crash-safety, partial/torn writes, integer overflow, panics, unwrap on external input" },
    Lens { key: "edges", focus: "boundary conditions, empty/zero/duplicate inputs, error paths, terminal-state operations" },
];

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Finding {
    pub title: String,
    #[serde(default)]
    pub location: String,
    pub description: String,
    #[serde(default)]
    pub lens: String,
}

#[derive(Deserialize, Debug, PartialEq)]
struct FindingsEnvelope {
    findings: Vec<Finding>,
}

#[derive(Deserialize, Debug, PartialEq)]
struct VerdictEnvelope {
    is_real: bool,
    #[serde(default)]
    reasoning: String,
}

/// Outcome of a review run.
#[derive(Debug, Default)]
pub struct ReviewReport {
    pub candidate: usize,
    pub confirmed: Vec<Finding>,
    pub fixed: Vec<String>,
    pub gate_passed: bool,
    /// Whether the final gate was run at all. A pass that returned before
    /// it did not fail the gate; it never asked (ADR-2609131646).
    pub gate_run: bool,
    pub notes: Vec<String>,
    /// Who produced the findings, distinct, in first-use order
    /// (ADR-2609131702 §2).
    pub reviewed_by: Vec<String>,
    /// Whether the local reviewer saw a truncated target (§3).
    pub truncated: bool,
    /// How many lenses were asked (ADR-2609131646).
    pub lenses: usize,
    /// How many replied with the envelope that was asked for. Zero means
    /// nothing was reviewed, whatever the counts say.
    pub answered: usize,
}

impl ReviewReport {
    /// Did any lens answer? A pass where none did reviewed nothing and may
    /// not report a clean file (ADR-2609131646 §2).
    pub fn reviewed(&self) -> bool {
        self.answered > 0
    }

    /// The gate, in words: it ran and passed, ran and failed, or was never
    /// reached. "FAIL" for a gate nobody ran is the same lie as a clean
    /// review nobody performed.
    pub fn gate_line(&self) -> &'static str {
        match (self.gate_run, self.gate_passed) {
            (false, _) => "not run",
            (true, true) => "PASS",
            (true, false) => "FAIL",
        }
    }

    /// The headline: what this pass is entitled to claim.
    pub fn verdict_line(&self) -> String {
        if !self.reviewed() {
            return format!("{} of {} lenses answered — nothing was reviewed", self.answered, self.lenses);
        }
        let scope = if self.answered < self.lenses {
            format!(" ({} of {} lenses answered)", self.answered, self.lenses)
        } else {
            String::new()
        };
        let by = if self.reviewed_by.is_empty() {
            String::new()
        } else {
            format!(" · reviewed by {}", self.reviewed_by.join(", "))
        };
        let cut = if self.truncated { " · target truncated" } else { "" };
        format!(
            "{} candidate(s) → {} confirmed real → {} fixed (gate-passed){}{}{}",
            self.candidate,
            self.confirmed.len(),
            self.fixed.len(),
            scope,
            by,
            cut
        )
    }

    /// Record who answered, once each, in the order they first did.
    fn note_reviewer(&mut self, who: &Reviewer) {
        let name = who.name();
        if !matches!(who, Reviewer::None) && !self.reviewed_by.contains(&name) {
            self.reviewed_by.push(name);
        }
    }
}

/// What came back from one agent asked for a JSON envelope.
///
/// `claude -p` exits 0 when it declines: a spend limit, an expired login and
/// a refusal are all successful processes carrying prose. Parsing is the
/// only thing that separates an answer from a non-answer, so it is the thing
/// that decides (ADR-2609131646 §1).
#[derive(Debug, Clone, PartialEq)]
pub enum Answer<T> {
    Answered(T),
    /// The call returned, and what it returned was not the envelope. The
    /// string is the first line of it, for the operator.
    NotAnswered(String),
    /// The call itself failed: a timeout, a spawn error, a panicked task.
    Failed(String),
}

/// The first line of a reply, trimmed for a status line.
fn first_line(text: &str) -> String {
    let line = text.trim().lines().next().unwrap_or("").trim();
    if line.is_empty() {
        return "empty reply".to_string();
    }
    let mut out: String = line.chars().take(120).collect();
    if line.chars().count() > 120 {
        out.push('…');
    }
    out
}

/// Read one agent's reply as the envelope it was asked for.
fn read_answer<T: serde::de::DeserializeOwned>(result: Result<Result<String, String>, String>) -> Answer<T> {
    match result {
        Ok(Ok(out)) => match extract_json(&out).and_then(|js| serde_json::from_str::<T>(js).ok()) {
            Some(v) => Answer::Answered(v),
            None => Answer::NotAnswered(first_line(&out)),
        },
        Ok(Err(e)) => Answer::Failed(e),
        Err(e) => Answer::Failed(e),
    }
}

/// Extract the first balanced JSON value (object or array) from agent prose. Pure and
/// testable — `claude -p` often wraps JSON in markdown fences or commentary.
fn extract_json(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    let start = bytes.iter().position(|&b| b == b'{' || b == b'[')?;
    let (open, close) = if bytes[start] == b'{' { (b'{', b'}') } else { (b'[', b']') };
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    for i in start..bytes.len() {
        let b = bytes[i];
        if in_str {
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            x if x == open => depth += 1,
            x if x == close => {
                depth -= 1;
                if depth == 0 {
                    return std::str::from_utf8(&bytes[start..=i]).ok();
                }
            }
            _ => {}
        }
    }
    None
}

/// One line of progress from the harness, for whoever is watching
/// (ADR-2609131427): which phase, what just happened, how long the phase
/// has run. A phase that is waiting on a model repeats itself every
/// [`HEARTBEAT`], so silence never means anything.
#[derive(Debug, Clone)]
pub struct Progress {
    pub phase: &'static str,
    pub message: String,
    pub elapsed: Duration,
}

pub type Reporter = std::sync::Arc<dyn Fn(Progress) + Send + Sync>;

/// A reporter that says nothing, for callers that only want the report.
pub fn silent() -> Reporter {
    std::sync::Arc::new(|_| {})
}

/// How often a phase that is waiting says so.
pub const HEARTBEAT: Duration = Duration::from_secs(30);

/// A phase in flight: announced on start, heartbeat while it runs, reported
/// on finish with its elapsed time. Dropping it stops the heartbeat.
pub struct Phase {
    name: &'static str,
    started: std::time::Instant,
    reporter: Reporter,
    ticker: tokio::task::JoinHandle<()>,
}

impl Phase {
    pub fn start(reporter: &Reporter, name: &'static str, message: impl Into<String>, heartbeat: Duration) -> Phase {
        let started = std::time::Instant::now();
        let message = message.into();
        reporter(Progress { phase: name, message: message.clone(), elapsed: Duration::ZERO });
        let r = reporter.clone();
        let ticker = tokio::spawn(async move {
            loop {
                tokio::time::sleep(heartbeat).await;
                r(Progress { phase: name, message: format!("still {message}"), elapsed: started.elapsed() });
            }
        });
        Phase { name, started, reporter: reporter.clone(), ticker }
    }

    pub fn note(&self, message: impl Into<String>) {
        (self.reporter)(Progress { phase: self.name, message: message.into(), elapsed: self.started.elapsed() });
    }

    pub fn finish(self, message: impl Into<String>) {
        self.ticker.abort();
        (self.reporter)(Progress { phase: self.name, message: message.into(), elapsed: self.started.elapsed() });
    }
}

impl Drop for Phase {
    fn drop(&mut self) {
        self.ticker.abort();
    }
}

/// `1 lens`, `4 lenses`, `3 designs`. A sibilant takes `es`; everything
/// else this harness counts takes `s`.
fn plural(n: usize, one: &str) -> String {
    let suffix = if n == 1 {
        ""
    } else if ["s", "x", "z", "ch", "sh"].iter().any(|e| one.ends_with(e)) {
        "es"
    } else {
        "s"
    };
    format!("{n} {one}{suffix}")
}

/// Spawn one `claude -p` agent in `cwd`, return stdout.
async fn claude_run(prompt: &str, cwd: &Path, timeout_secs: u64) -> Result<String, String> {
    crate::frontier::budget_check()?;
    let fut = tokio::process::Command::new(claude_binary())
        .arg("-p")
        .args(crate::frontier::OUTPUT_JSON)
        // hexa's own prompt. The project\'s hooks run inside this claude and must
        // not treat it as a person's work: `route` once drafted workplans from the
        // harden reviewer prompts. `hexa hook` returns early when this is set.
        .env("HEXA_INTERNAL", "1")
        .arg("--dangerously-skip-permissions")
        .arg(prompt)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();
    match tokio::time::timeout(Duration::from_secs(timeout_secs), fut).await {
        Ok(Ok(o)) => Ok(crate::frontier::take_answer(&String::from_utf8_lossy(&o.stdout), "harden")),
        Ok(Err(e)) => Err(format!("spawn claude: {e}")),
        Err(_) => Err("claude -p timed out".to_string()),
    }
}

/// Who produced a reply (ADR-2609131702 §2). A finding from an agent that
/// walked the repository is not the same claim as one from a completion
/// shown an excerpt, and a reader deciding what to trust needs to know.
#[derive(Debug, Clone, PartialEq)]
pub enum Reviewer {
    /// The frontier CLI: reads files, follows calls, can edit.
    Frontier,
    /// A tier-mapped model, shown only what the prompt carried.
    Local(String),
    /// Nothing answered.
    None,
}

impl Reviewer {
    pub fn name(&self) -> String {
        match self {
            Reviewer::Frontier => "frontier".to_string(),
            Reviewer::Local(m) => m.clone(),
            Reviewer::None => "nothing".to_string(),
        }
    }
}

/// How much of a target the local reviewer is shown. A completion has no
/// way to fetch more, so the cap is the review's field of view and §3
/// requires it be stated when it bites.
pub const LOCAL_CONTEXT_CAP: usize = 120_000;

/// The target's source for a prompt, and whether it was truncated.
/// A directory contributes its files in name order until the cap.
pub fn read_target(target: &str, repo_root: &Path) -> (String, bool) {
    let path = repo_root.join(target);
    let mut out = String::new();
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    if path.is_dir() {
        let mut stack = vec![path.clone()];
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            let mut entries: Vec<std::path::PathBuf> = rd.flatten().map(|e| e.path()).collect();
            entries.sort();
            for e in entries {
                if e.is_dir() {
                    stack.push(e);
                } else if e.extension().is_some_and(|x| x == "rs") {
                    files.push(e);
                }
            }
        }
        files.sort();
    } else {
        files.push(path);
    }
    let mut truncated = false;
    for f in files {
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        let name = f.strip_prefix(repo_root).unwrap_or(&f).display().to_string();
        let header = format!("\n// ── {name} ──\n");
        if out.len() + header.len() + text.len() > LOCAL_CONTEXT_CAP {
            let room = LOCAL_CONTEXT_CAP.saturating_sub(out.len() + header.len());
            if room > 0 {
                out.push_str(&header);
                out.push_str(&text[..room.min(text.len())]);
            }
            truncated = true;
            break;
        }
        out.push_str(&header);
        out.push_str(&text);
    }
    (out, truncated)
}

/// The tier-mapped model for review work, strongest tier first. `None` when
/// nothing is configured, which §4 makes a non-answer rather than a silent
/// skip.
fn review_model() -> Option<String> {
    ["t2.5", "t2", "t1"].iter().find_map(|t| hexa_infer::tier_model(t))
}

/// Ask the tier-mapped model, with the code in the prompt because a
/// completion cannot go and read it.
async fn local_run(prompt: &str, code: &str, truncated: bool) -> Result<String, String> {
    let model = review_model().ok_or_else(|| {
        "no tier model configured — set inference.tier_models in .hexa/project.json".to_string()
    })?;
    let note = if truncated {
        format!("\n\n(The excerpt below is the first {LOCAL_CONTEXT_CAP} bytes of the target; it is truncated.)")
    } else {
        String::new()
    };
    let user = format!("{prompt}{note}\n\nTHE CODE UNDER REVIEW:\n```rust\n{code}\n```");
    hexa_infer::complete_text(
        &model,
        "You are a meticulous reviewer. Answer with the exact JSON object asked for and nothing else.",
        &user,
        4096,
    )
    .await
    .map_err(|e| format!("{model}: {e}"))
}

/// Ask the frontier; on any non-answer, ask the tier-mapped model with the
/// code inline (ADR-2609131702 §1). Returns what came back and who said it.
async fn ask<T: serde::de::DeserializeOwned>(
    prompt: &str,
    code: &str,
    truncated: bool,
    cwd: &Path,
    timeout_secs: u64,
    attempts: u32,
) -> (Answer<T>, Reviewer) {
    let frontier = read_answer::<T>(Ok(claude_run_retry(prompt, cwd, timeout_secs, attempts).await));
    if let Answer::Answered(v) = frontier {
        return (Answer::Answered(v), Reviewer::Frontier);
    }
    let model = review_model().unwrap_or_else(|| "no tier model".to_string());
    let local = read_answer::<T>(Ok(local_run(prompt, code, truncated).await));
    match local {
        Answer::Answered(v) => (Answer::Answered(v), Reviewer::Local(model)),
        other => (other, Reviewer::None),
    }
}

/// Default attempts for `claude_run` call sites that don't take an explicit retry
/// count from the caller (e.g. `run_review`, whose CLI surface isn't parametrized).
const DEFAULT_RETRIES: u32 = 3;

/// Retry-wrapped [`claude_run`]: on timeout or spawn error, retry up to `attempts`
/// times (clamped 1-6), feeding the prior failure back into the next attempt's prompt
/// for context. No sleep/backoff — failures here are inference-latency timeouts on a
/// single long call, not rate limits, so immediate retry is the right shape (mirrors
/// the bounded-attempt loop in direct_exec.rs rather than time-based backoff).
async fn claude_run_retry(
    prompt: &str,
    cwd: &Path,
    timeout_secs: u64,
    attempts: u32,
) -> Result<String, String> {
    let attempts = attempts.clamp(1, 6);
    let mut last_err = String::new();
    for attempt in 1..=attempts {
        let this_prompt = retry_prompt(prompt, attempt, attempts, &last_err);
        match claude_run(&this_prompt, cwd, timeout_secs).await {
            Ok(out) => return Ok(out),
            Err(e) => last_err = e,
        }
    }
    Err(last_err)
}

/// Pure prompt-formatting for a retry attempt — split out from [`claude_run_retry`]
/// so the retry framing logic is unit-testable without spawning a real subprocess.
fn retry_prompt(prompt: &str, attempt: u32, attempts: u32, last_err: &str) -> String {
    if attempt == 1 {
        prompt.to_string()
    } else {
        format!("{prompt}\n\n(retry {attempt}/{attempts} — the previous attempt failed: {last_err})")
    }
}

/// Paths git reports as changed, from `git status --porcelain`.
///
/// Used to tell the operator's edits from the pass's own. A rename line
/// (`R  old -> new`) contributes the new path.
async fn dirty_paths(repo_root: &Path) -> std::collections::HashSet<String> {
    let out = tokio::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo_root)
        .output()
        .await;
    let Ok(out) = out else {
        return Default::default();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.get(3..))
        .map(|p| p.rsplit(" -> ").next().unwrap_or(p).trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

/// Stage and commit `paths` using a commit-local factory identity
/// (ADR-2606071323 §4) so autonomous commits are attributable and never masquerade as
/// the operator. Non-fatal by design: any failure (nothing to commit, git missing,
/// no repo) is recorded as a note, never surfaced as an error — the build/review
/// itself already succeeded per the gate by the time this runs.
///
/// `paths` is every file the pass changed, not just its target. Committing the
/// target alone shipped eleven fixes and left the five tests proving them
/// uncommitted (ADR-2609122048).
async fn commit_result(
    repo_root: &Path,
    paths: &[String],
    subject: &str,
    trailer: &str,
    notes: &mut Vec<String>,
) {
    if paths.is_empty() {
        notes.push("nothing new to commit".to_string());
        return;
    }
    let mut add_args: Vec<&str> = vec!["add", "--"];
    add_args.extend(paths.iter().map(String::as_str));
    let add = tokio::process::Command::new("git")
        .args(&add_args)
        .current_dir(repo_root)
        .output()
        .await;
    match add {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            notes.push(format!(
                "git add failed (non-fatal): {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
            return;
        }
        Err(e) => {
            notes.push(format!("git not found — skipping auto-commit (non-fatal): {e}"));
            return;
        }
    }
    let subject: String = subject.lines().next().unwrap_or(subject).chars().take(72).collect();
    let msg = format!("{subject}\n\nCo-Authored-By: {trailer} <noreply@hexa.local>");
    let mut commit_args: Vec<&str> = vec![
        "-c", "user.name=hexa-factory",
        "-c", "user.email=factory@hexa.local",
        "commit", "-m", &msg, "--",
    ];
    commit_args.extend(paths.iter().map(String::as_str));
    let commit = tokio::process::Command::new("git")
        .args(&commit_args)
        .current_dir(repo_root)
        .output()
        .await;
    match commit {
        Ok(out) if out.status.success() => notes.push("auto-committed result (hexa-factory)".to_string()),
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            if stderr.contains("nothing to commit") {
                notes.push("nothing new to commit".to_string());
            } else {
                notes.push(format!("git commit failed (non-fatal): {}", stderr.trim()));
            }
        }
        Err(e) => notes.push(format!("git not found — skipping auto-commit (non-fatal): {e}")),
    }
}

/// Run the adversarial review pipeline over `target` (a path), gated by `gate` (a
/// shell command that must exit 0). Fixes are applied to the working tree and left
/// uncommitted for operator review.
pub async fn run_review(target: &str, gate: &str, repo_root: &Path) -> ReviewReport {
    run_review_with(target, gate, repo_root, silent()).await
}

/// [`run_review`], reporting each phase as it happens.
pub async fn run_review_with(target: &str, gate: &str, repo_root: &Path, reporter: Reporter) -> ReviewReport {
    let mut report = ReviewReport::default();
    // What the operator had already changed. Anything dirty after the pass and
    // not in this set is the pass's own work, tests included.
    let dirty_before = dirty_paths(repo_root).await;

    // The local reviewer cannot go and read the code, so it is read once
    // here and carried in every prompt (ADR-2609131702 §1).
    let (code, truncated) = read_target(target, repo_root);
    report.truncated = truncated;
    if truncated {
        report.notes.push(format!("target truncated to {LOCAL_CONTEXT_CAP} bytes for the local reviewer"));
    }

    // ── Phase 1: hunt (parallel lenses) ──────────────────────────────────────
    let hunt = Phase::start(&reporter, "hunt", format!("{} on {target}, in parallel", plural(LENSES.len(), "lens")), HEARTBEAT);
    let mut hunts = Vec::new();
    for lens in LENSES {
        let prompt = format!(
            "You are an adversarial code reviewer. Read the code under `{target}` (and its tests) \
             and hunt ONLY for this failure class: {focus}. Report exclusively REAL bugs you can \
             point to in the actual code — do NOT invent issues; if the code is correct on this \
             lens, return an empty list. Do NOT modify any files. Output ONLY a JSON object: \
             {{\"findings\":[{{\"title\":\"...\",\"location\":\"file:line or fn\",\"description\":\"the concrete failure\",\"lens\":\"{key}\"}}]}}",
            target = target, focus = lens.focus, key = lens.key
        );
        let root = repo_root.to_path_buf();
        let code = code.clone();
        hunts.push((
            lens.key,
            tokio::spawn(async move { ask::<FindingsEnvelope>(&prompt, &code, truncated, &root, 600, DEFAULT_RETRIES).await }),
        ));
    }
    report.lenses = hunts.len();
    for (key, h) in hunts {
        let (answer, who) = match h.await {
            Ok(v) => v,
            Err(e) => (Answer::Failed(e.to_string()), Reviewer::None),
        };
        report.note_reviewer(&who);
        match answer {
            Answer::Answered(env) => {
                report.answered += 1;
                let n = env.findings.len();
                report.candidate += n;
                report.confirmed.extend(env.findings); // staged; pruned by verify below
                hunt.note(format!("{key}: {} ({})", plural(n, "candidate"), who.name()));
            }
            Answer::NotAnswered(why) => {
                hunt.note(format!("{key}: NO ANSWER — {why}"));
                report.notes.push(format!("{key} did not answer: {why}"));
            }
            Answer::Failed(e) => {
                hunt.note(format!("{key}: FAILED — {e}"));
                report.notes.push(format!("{key} failed: {e}"));
            }
        }
    }
    let candidates = std::mem::take(&mut report.confirmed);
    if !report.reviewed() {
        hunt.finish(format!("no lens answered — nothing was reviewed ({} asked)", report.lenses));
        report.notes.push("nothing was reviewed: no lens returned the findings envelope".to_string());
        // A gate run now would pass on code nobody read, and a pass is not
        // entitled to that claim (ADR-2609131646 §3).
        return report;
    }
    hunt.finish(format!("{} to verify", plural(candidates.len(), "candidate")));

    // ── Phase 2: skeptical verify (parallel, default-refute) ─────────────────
    let verify = Phase::start(&reporter, "verify", format!("{}, default refute, in parallel", plural(candidates.len(), "claim")), HEARTBEAT);
    let mut checks = Vec::new();
    for f in candidates {
        let prompt = format!(
            "Independently and SKEPTICALLY verify this claimed bug in the code under `{target}`. \
             Read the actual code at the cited location. Default to is_real=false unless you can \
             point to the exact wrong code and explain the concrete failure sequence; reject vague \
             or speculative claims. Do NOT modify files. Output ONLY JSON: \
             {{\"is_real\":true|false,\"reasoning\":\"...\"}}.\n\nCLAIM:\ntitle: {title}\nlocation: {loc}\ndescription: {desc}",
            target = target, title = f.title, loc = f.location, desc = f.description
        );
        let root = repo_root.to_path_buf();
        let code = code.clone();
        checks.push((
            f,
            tokio::spawn(async move { ask::<VerdictEnvelope>(&prompt, &code, truncated, &root, 600, DEFAULT_RETRIES).await }),
        ));
    }
    for (f, c) in checks {
        let (answer, who) = match c.await {
            Ok(v) => v,
            Err(e) => (Answer::Failed(e.to_string()), Reviewer::None),
        };
        report.note_reviewer(&who);
        match answer {
            Answer::Answered(v) if v.is_real => {
                verify.note(format!("real: {}", f.title));
                report.confirmed.push(f);
            }
            Answer::Answered(v) => {
                verify.note(format!("refuted: {}", f.title));
                report.notes.push(format!("refuted: {} — {}", f.title, v.reasoning));
            }
            // Unverified is not refuted: the claim stands unexamined, and
            // saying so is the point (ADR-2609131646 §1).
            Answer::NotAnswered(why) => {
                verify.note(format!("NO VERDICT: {} — {why}", f.title));
                report.notes.push(format!("unverified, no verdict: {} — {why}", f.title));
            }
            Answer::Failed(e) => {
                verify.note(format!("VERIFY FAILED: {} — {e}", f.title));
                report.notes.push(format!("unverified, call failed: {} — {e}", f.title));
            }
        }
    }
    verify.finish(format!("{} confirmed real", report.confirmed.len()));

    // ── Phase 3: fix-loop (sequential, each fix gated) ───────────────────────
    let fix = Phase::start(&reporter, "fix", format!("{} to fix, one at a time, each under the gate", plural(report.confirmed.len(), "bug")), HEARTBEAT);
    for f in &report.confirmed {
        fix.note(format!("fixing: {}", f.title));
        let prompt = format!(
            "Fix this CONFIRMED bug in the code under `{target}`, then add a regression test that \
             fails without the fix. Make a minimal, correct change. Bug:\ntitle: {title}\nlocation: {loc}\ndescription: {desc}",
            target = target, title = f.title, loc = f.location, desc = f.description
        );
        // Only the frontier can edit files; a completion has no way to
        // (ADR-2609131702 §1). And a reply is not an edit: the frontier
        // exits 0 when it declines, so the tree itself is the evidence.
        let before = dirty_paths(repo_root).await;
        match claude_run_retry(&prompt, repo_root, 900, DEFAULT_RETRIES).await {
            Ok(reply) => {
                let after = dirty_paths(repo_root).await;
                if after == before {
                    fix.note(format!("NO EDIT — {}: {}", f.title, first_line(&reply)));
                    report.notes.push(format!("no fix applied for '{}': the tree did not change — {}", f.title, first_line(&reply)));
                    continue;
                }
                fix.note(format!("gate after fix: {gate}"));
                let (passed, _) = crate::direct_exec::run_evidence(gate, repo_root).await;
                if passed {
                    fix.note(format!("gate passed: {}", f.title));
                    report.fixed.push(f.title.clone());
                } else {
                    fix.note(format!("gate FAILED after: {}", f.title));
                    report.notes.push(format!("fix for '{}' did not pass the gate", f.title));
                }
            }
            Err(e) => fix.note(format!("no fix produced for {}: {e}", f.title)),
        }
    }
    fix.finish(format!("{} fixed", report.fixed.len()));

    // ── Final gate ───────────────────────────────────────────────────────────
    let final_gate = Phase::start(&reporter, "gate", format!("running: {gate}"), HEARTBEAT);
    let (passed, _) = crate::direct_exec::run_evidence(gate, repo_root).await;
    report.gate_passed = passed;
    report.gate_run = true;
    final_gate.finish(if passed { "passed" } else { "FAILED" });
    if passed && report.reviewed() {
        let mut paths: Vec<String> = dirty_paths(repo_root)
            .await
            .into_iter()
            .filter(|p| !dirty_before.contains(p))
            .collect();
        if !paths.iter().any(|p| p == target) {
            paths.push(target.to_string());
        }
        paths.sort();
        let commit = Phase::start(&reporter, "commit", format!("{} as hexa-harden", plural(paths.len(), "path")), HEARTBEAT);
        commit_result(
            repo_root,
            &paths,
            &format!("fix: adversarial review pass on {target}"),
            "hexa-harden",
            &mut report.notes,
        )
        .await;
        commit.finish("done");
    }
    report
}

/// Outcome of a cooperative build run.
#[derive(Debug, Default)]
pub struct BuildReport {
    pub designs: usize,
    pub critiques: usize,
    pub spec_chars: usize,
    pub build_ok: bool,
    pub notes: Vec<String>,
}

/// Competing design priorities — the divergence that makes the red-team meaningful.
const DESIGN_PRIORITIES: &[&str] = &[
    "durability-and-correctness-first: crash-safety, persistence, recovery, and provable invariants are paramount",
    "concurrency-first: correct and lock-minimal under heavy parallelism; no races, no double-processing",
    "simplicity-first: the smallest design that is obviously correct; the fewest moving parts",
    "performance-first: throughput and low overhead, without sacrificing correctness",
];

/// Run the cooperative-design half of the harness: diverge (N designs from competing
/// priorities) → red-team each → synthesize one spec → build to the gate. Pairs with
/// [`run_review`] for the full cooperative+adversarial pipeline.
pub async fn run_build(
    challenge: &str,
    target: &str,
    gate: &str,
    n_designs: usize,
    repo_root: &Path,
    timeout_secs: u64,
    retries: u32,
) -> BuildReport {
    run_build_with(challenge, target, gate, n_designs, repo_root, timeout_secs, retries, silent()).await
}

/// [`run_build`], reporting each phase as it happens.
#[allow(clippy::too_many_arguments)]
pub async fn run_build_with(
    challenge: &str,
    target: &str,
    gate: &str,
    n_designs: usize,
    repo_root: &Path,
    timeout_secs: u64,
    retries: u32,
    reporter: Reporter,
) -> BuildReport {
    let mut report = BuildReport::default();
    let dirty_before = dirty_paths(repo_root).await;
    let n = n_designs.clamp(2, DESIGN_PRIORITIES.len());

    // ── Phase 1: diverge — N designs from competing priorities ───────────────
    let diverge = Phase::start(&reporter, "diverge", format!("{} from competing priorities, in parallel", plural(n, "design")), HEARTBEAT);
    let mut tasks = Vec::new();
    for prio in DESIGN_PRIORITIES.iter().take(n) {
        let prompt = format!(
            "You are a senior systems engineer. Propose a concrete design for this challenge:\n{challenge}\n\n\
             Your design PRIORITY: {prio}\n\nBe specific about the data model, the key algorithms, the \
             concurrency/atomicity strategy, and the main risks. Output your design as clear prose (no code yet)."
        );
        let root = repo_root.to_path_buf();
        tasks.push(tokio::spawn(async move { claude_run_retry(&prompt, &root, timeout_secs, retries).await }));
    }
    let mut designs = Vec::new();
    for (i, t) in tasks.into_iter().enumerate() {
        if let Ok(Ok(d)) = t.await {
            diverge.note(format!("design {i}: {} chars", d.len()));
            designs.push(d);
        } else {
            diverge.note(format!("design {i}: no answer"));
        }
    }
    report.designs = designs.len();
    diverge.finish(plural(designs.len(), "design"));
    if designs.is_empty() {
        report.notes.push("no designs produced".into());
        return report;
    }

    // ── Phase 2: red-team each design (adversarial) ──────────────────────────
    let red = Phase::start(&reporter, "red-team", format!("{} critiqued, in parallel", plural(designs.len(), "design")), HEARTBEAT);
    let mut ctasks = Vec::new();
    for (i, d) in designs.iter().enumerate() {
        let prompt = format!(
            "Adversarially review this design for the challenge:\n{challenge}\n\nBe RUTHLESS — find fatal \
             flaws, race conditions, lost-data scenarios, correctness gaps, and unhandled edge cases. List \
             them concretely.\n\nDESIGN {i}:\n{d}"
        );
        let root = repo_root.to_path_buf();
        ctasks.push(tokio::spawn(async move { claude_run_retry(&prompt, &root, timeout_secs, retries).await }));
    }
    let mut critiques = Vec::new();
    for (i, t) in ctasks.into_iter().enumerate() {
        if let Ok(Ok(c)) = t.await {
            red.note(format!("critique {i}: {} chars", c.len()));
            critiques.push(c);
        } else {
            red.note(format!("critique {i}: no answer"));
        }
    }
    report.critiques = critiques.len();
    red.finish(plural(critiques.len(), "critique"));

    // ── Phase 3: synthesize one build spec ───────────────────────────────────
    let synth = Phase::start(&reporter, "synthesize", "one spec from the designs and their critiques", HEARTBEAT);
    let designs_block = designs
        .iter()
        .enumerate()
        .map(|(i, d)| format!("--- DESIGN {i} ---\n{d}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let critiques_block = critiques.join("\n\n--- next critique ---\n\n");
    let spec = match claude_run_retry(
        &format!(
            "You are the lead architect. Given these candidate designs and their adversarial critiques for \
             the challenge:\n{challenge}\n\nSynthesize ONE concrete build spec: the public API, the internal \
             data model, the exact concurrency/correctness strategy, and a TEST PLAN that exercises the hard \
             cases the red team raised. Every fatal flaw must be designed out. Output the spec as clear prose \
             an implementer can follow.\n\nDESIGNS:\n{designs_block}\n\nCRITIQUES:\n{critiques_block}"
        ),
        repo_root,
        timeout_secs,
        retries,
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            synth.finish(format!("FAILED: {e}"));
            report.notes.push(format!("synthesize failed: {e}"));
            return report;
        }
    };
    report.spec_chars = spec.len();
    synth.finish(format!("spec of {} chars", spec.len()));

    // ── Phase 4: build to the gate ───────────────────────────────────────────
    let build = Phase::start(&reporter, "build", format!("one agent builds to the gate: {gate}"), HEARTBEAT);
    let build_prompt = format!(
        "Implement the following spec as code under `{target}`. Write the full implementation AND a \
         comprehensive test suite per the spec's test plan. Then run the gate command `{gate}` and ITERATE \
         — fix compile errors and failing tests — until the gate exits 0. Do not stop until the gate passes.\n\n\
         CHALLENGE:\n{challenge}\n\nSPEC:\n{spec}"
    );
    if let Err(e) = claude_run_retry(&build_prompt, repo_root, timeout_secs.saturating_mul(4), retries).await {
        build.note(format!("build agent error: {e}"));
        report.notes.push(format!("build agent error: {e}"));
    }
    build.note(format!("gate: {gate}"));
    let (ok, _) = crate::direct_exec::run_evidence(gate, repo_root).await;
    report.build_ok = ok;
    build.finish(if ok { "gate passed" } else { "gate FAILED" });
    if ok {
        let mut paths: Vec<String> = dirty_paths(repo_root)
            .await
            .into_iter()
            .filter(|p| !dirty_before.contains(p))
            .collect();
        if !paths.iter().any(|p| p == target) {
            paths.push(target.to_string());
        }
        paths.sort();
        commit_result(
            repo_root,
            &paths,
            &format!("feat: {challenge}"),
            "hexa-build",
            &mut report.notes,
        )
        .await;
    }
    report
}

#[cfg(test)]
mod answered_tests {
    use super::*;

    fn ok(body: &str) -> Result<Result<String, String>, String> {
        Ok(Ok(body.to_string()))
    }

    /// ADR-2609131646 §1: parsing decides. An envelope is an answer, prose
    /// is not, and the refusal text survives into the reason.
    #[test]
    fn an_envelope_is_answered_and_prose_is_not() {
        let a: Answer<FindingsEnvelope> = read_answer(ok(r#"{"findings":[]}"#));
        assert!(matches!(a, Answer::Answered(ref e) if e.findings.is_empty()), "an empty list is a real answer: {a:?}");

        let a: Answer<FindingsEnvelope> = read_answer(ok("You've hit your monthly spend limit. Switch to another model."));
        assert_eq!(a, Answer::NotAnswered("You've hit your monthly spend limit. Switch to another model.".into()));

        let a: Answer<FindingsEnvelope> = read_answer(ok("   "));
        assert_eq!(a, Answer::NotAnswered("empty reply".into()));

        let a: Answer<FindingsEnvelope> = read_answer(Ok(Err("claude -p timed out".into())));
        assert_eq!(a, Answer::Failed("claude -p timed out".into()));

        // Markdown fences still parse: that is why extract_json exists.
        let a: Answer<FindingsEnvelope> =
            read_answer(ok("Here you go:\n```json\n{\"findings\":[{\"title\":\"t\",\"location\":\"l\",\"description\":\"d\",\"lens\":\"x\"}]}\n```"));
        assert!(matches!(a, Answer::Answered(ref e) if e.findings.len() == 1), "{a:?}");
    }

    /// §2: a pass where no lens answered reviewed nothing and may not say
    /// otherwise — this is the run that produced the ADR.
    #[test]
    fn a_review_with_no_answer_is_not_a_clean_review() {
        let none = ReviewReport { lenses: 4, answered: 0, gate_passed: true, ..ReviewReport::default() };
        assert!(!none.reviewed());
        let line = none.verdict_line();
        assert_eq!(line, "0 of 4 lenses answered — nothing was reviewed");
        assert!(!line.contains("confirmed real"), "never a clean-sounding count: {line}");
    }

    /// §4: a partial review keeps its count and says how partial it was.
    #[test]
    fn a_partial_review_keeps_its_count_and_names_the_ratio() {
        let partial = ReviewReport { lenses: 4, answered: 2, candidate: 3, gate_passed: true, ..ReviewReport::default() };
        assert!(partial.reviewed());
        let line = partial.verdict_line();
        assert!(line.starts_with("3 candidate(s) → 0 confirmed real → 0 fixed (gate-passed)"), "{line}");
        assert!(line.ends_with("(2 of 4 lenses answered)"), "{line}");

        let whole = ReviewReport { lenses: 4, answered: 4, candidate: 3, gate_passed: true, ..ReviewReport::default() };
        assert!(!whole.verdict_line().contains("lenses answered"), "a full review says nothing about the ratio");
    }

    /// ADR-2609131702 §2: the verdict line names who reviewed, so a reader
    /// can tell an agent's finding from a completion's.
    #[test]
    fn the_verdict_line_names_the_reviewer_and_any_truncation() {
        let frontier = ReviewReport {
            lenses: 4, answered: 4, candidate: 2,
            reviewed_by: vec!["frontier".into()],
            ..ReviewReport::default()
        };
        assert!(frontier.verdict_line().ends_with("reviewed by frontier"), "{}", frontier.verdict_line());

        let mixed = ReviewReport {
            lenses: 4, answered: 4, candidate: 2,
            reviewed_by: vec!["frontier".into(), "openai/gpt-oss-120b".into()],
            truncated: true,
            ..ReviewReport::default()
        };
        let line = mixed.verdict_line();
        assert!(line.contains("reviewed by frontier, openai/gpt-oss-120b"), "{line}");
        assert!(line.ends_with("target truncated"), "{line}");
    }

    /// A reviewer is recorded once, in the order it first answered, and
    /// nothing is recorded for a non-answer.
    #[test]
    fn reviewers_are_recorded_once_each_and_never_for_a_non_answer() {
        let mut r = ReviewReport::default();
        r.note_reviewer(&Reviewer::Frontier);
        r.note_reviewer(&Reviewer::Local("m".into()));
        r.note_reviewer(&Reviewer::Frontier);
        r.note_reviewer(&Reviewer::None);
        assert_eq!(r.reviewed_by, vec!["frontier".to_string(), "m".to_string()]);
    }

    /// §3: a target over the cap is truncated and the fact is carried, not
    /// swallowed; one under it is whole.
    #[test]
    fn a_target_over_the_cap_is_truncated_and_says_so() {
        let dir = std::env::temp_dir().join(format!("hexa-target-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();

        std::fs::write(dir.join("src/small.rs"), "fn a() {}\n").unwrap();
        let (code, truncated) = read_target("src/small.rs", &dir);
        assert!(code.contains("fn a() {}"), "{code}");
        assert!(code.contains("src/small.rs"), "the file is named in the excerpt: {code}");
        assert!(!truncated);

        std::fs::write(dir.join("src/big.rs"), "x".repeat(LOCAL_CONTEXT_CAP + 5_000)).unwrap();
        let (code, truncated) = read_target("src/big.rs", &dir);
        assert!(truncated, "over the cap");
        assert!(code.len() <= LOCAL_CONTEXT_CAP, "shown {} bytes, cap {}", code.len(), LOCAL_CONTEXT_CAP);

        // A directory gathers its .rs files in name order, and stops at the
        // cap: here `big.rs` sorts first and fills it, so `small.rs` is
        // never reached — which is exactly what `truncated` is for.
        let (code, truncated) = read_target("src", &dir);
        assert!(truncated, "the directory exceeds the cap");
        assert!(code.contains("src/big.rs"), "first by name: {}", &code[..80.min(code.len())]);
        assert!(!code.contains("src/small.rs"), "the cap was reached before it");

        // Without the oversized file, the directory comes through whole.
        std::fs::remove_file(dir.join("src/big.rs")).unwrap();
        std::fs::write(dir.join("src/other.rs"), "fn b() {}\n").unwrap();
        let (code, truncated) = read_target("src", &dir);
        assert!(!truncated);
        assert!(code.contains("fn a() {}") && code.contains("fn b() {}"), "{code}");
        assert!(code.find("other.rs").unwrap() < code.find("small.rs").unwrap(), "name order: {code}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A gate nobody ran is reported as such, never as a failure.
    #[test]
    fn a_gate_that_never_ran_is_not_a_failed_gate() {
        let unreviewed = ReviewReport { lenses: 4, answered: 0, ..ReviewReport::default() };
        assert_eq!(unreviewed.gate_line(), "not run");
        let failed = ReviewReport { lenses: 4, answered: 4, gate_run: true, gate_passed: false, ..ReviewReport::default() };
        assert_eq!(failed.gate_line(), "FAIL");
        let passed = ReviewReport { lenses: 4, answered: 4, gate_run: true, gate_passed: true, ..ReviewReport::default() };
        assert_eq!(passed.gate_line(), "PASS");
    }

    #[test]
    fn a_first_line_is_trimmed_not_dropped() {
        assert_eq!(first_line("one\ntwo"), "one");
        assert_eq!(first_line(""), "empty reply");
        assert_eq!(first_line(&"x".repeat(200)).chars().count(), 121, "120 plus the ellipsis");
    }
}

#[cfg(test)]
mod progress_tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn a_sibilant_is_pluralised_with_es() {
        assert_eq!(plural(1, "lens"), "1 lens");
        assert_eq!(plural(4, "lens"), "4 lenses");
        assert_eq!(plural(3, "design"), "3 designs");
        assert_eq!(plural(0, "candidate"), "0 candidates");
        assert_eq!(plural(1, "bug"), "1 bug");
    }

    /// A phase announces itself, repeats itself while it waits, and reports
    /// when it finishes with its elapsed time. Silence never means anything.
    #[test]
    fn a_phase_heartbeats_while_it_waits_and_reports_when_done() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let seen: Arc<Mutex<Vec<Progress>>> = Arc::new(Mutex::new(Vec::new()));
            let sink = seen.clone();
            let reporter: Reporter = Arc::new(move |p| sink.lock().unwrap().push(p));
            let phase = Phase::start(&reporter, "hunt", "4 lenses", Duration::from_millis(20));
            tokio::time::sleep(Duration::from_millis(110)).await;
            phase.note("durability: 2 candidates");
            phase.finish("5 candidates to verify");
            tokio::time::sleep(Duration::from_millis(60)).await;
            let seen = seen.lock().unwrap();
            assert_eq!(seen[0].message, "4 lenses");
            assert_eq!(seen[0].elapsed, Duration::ZERO);
            let beats = seen.iter().filter(|p| p.message == "still 4 lenses").count();
            assert!(beats >= 3, "{} heartbeats in 110ms at 20ms", beats);
            let last = seen.last().unwrap();
            assert_eq!(last.message, "5 candidates to verify");
            assert!(last.elapsed >= Duration::from_millis(100), "{:?}", last.elapsed);
            assert!(seen.iter().all(|p| p.phase == "hunt"));
            let after_finish = seen.iter().position(|p| p.message == "5 candidates to verify").unwrap();
            assert_eq!(after_finish, seen.len() - 1, "no heartbeat after finish");
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{extract_json, retry_prompt};

    #[test]
    fn retry_prompt_first_attempt_is_unmodified() {
        assert_eq!(retry_prompt("do the thing", 1, 3, ""), "do the thing");
    }

    #[test]
    fn retry_prompt_later_attempts_include_prior_failure() {
        let p = retry_prompt("do the thing", 2, 3, "claude -p timed out");
        assert!(p.starts_with("do the thing"));
        assert!(p.contains("retry 2/3"));
        assert!(p.contains("claude -p timed out"));
    }

    #[test]
    fn extracts_object_from_markdown_fence() {
        let s = "Here are the findings:\n```json\n{\"findings\": [{\"title\": \"x\"}]}\n```\nDone.";
        assert_eq!(extract_json(s), Some("{\"findings\": [{\"title\": \"x\"}]}"));
    }

    #[test]
    fn handles_braces_inside_strings() {
        let s = "{\"reasoning\": \"the code does foo() { bar }\", \"is_real\": true}";
        assert_eq!(extract_json(s), Some(s));
    }

    #[test]
    fn extracts_array() {
        assert_eq!(extract_json("noise [1, 2, [3]] tail"), Some("[1, 2, [3]]"));
    }

    #[test]
    fn none_when_no_json() {
        assert_eq!(extract_json("no json here"), None);
    }

    #[test]
    fn ignores_close_brace_in_string_before_open() {
        let s = "prefix } then {\"a\": 1}";
        assert_eq!(extract_json(s), Some("{\"a\": 1}"));
    }
}

#[cfg(test)]
mod commit_scope_tests {

    /// The harden pass committed only its target, so the tests it wrote for
    /// the bugs it fixed were left uncommitted (ADR-2609122048). The parser
    /// that tells its work from the operator's has to read every status line
    /// shape git emits.
    #[test]
    fn porcelain_lines_parse_to_paths() {
        let sample = " M hexa-graph/src/lib.rs\n?? hexa-graph/tests/precision.rs\nA  docs/adrs/x.md\nR  old.rs -> new.rs\n";
        let got: std::collections::HashSet<String> = sample
            .lines()
            .filter_map(|l| l.get(3..))
            .map(|p| p.rsplit(" -> ").next().unwrap_or(p).trim().to_string())
            .filter(|p| !p.is_empty())
            .collect();
        assert!(got.contains("hexa-graph/src/lib.rs"));
        assert!(got.contains("hexa-graph/tests/precision.rs"), "an untracked new test must count");
        assert!(got.contains("docs/adrs/x.md"));
        assert!(got.contains("new.rs"), "a rename contributes its new path");
        assert_eq!(got.len(), 4);
    }
}
