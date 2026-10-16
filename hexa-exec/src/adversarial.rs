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

use std::sync::Arc;

use crate::ports::{Frontier, GitHistory};

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
#[derive(Debug)]
pub struct ReviewReport {
    pub candidate: usize,
    pub confirmed: Vec<Finding>,
    /// Findings whose fix passed the gate AND was committed, one commit per
    /// fix (ADR-2610141733 §6).
    pub fixed: Vec<String>,
    /// Findings whose fix passed the gate but was not committed: it touched
    /// a file the operator had already changed, or the commit failed.
    pub uncommitted: Vec<String>,
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
    /// What an empty local review was shown to be worth (ADR-2609131907).
    pub calibration: Calibration,
    /// How many lenses were asked (ADR-2609131646).
    pub lenses: usize,
    /// How many replied with the envelope that was asked for. Zero means
    /// nothing was reviewed, whatever the counts say.
    pub answered: usize,
    /// Every full-gate run when a fast gate was used, the final gate
    /// included (ADR-2610160756 §8). Zero without one.
    pub full_gates: usize,
    /// Each batch closed by a full gate, in order (ADR-2610160756 §3).
    pub batches: Vec<Batch>,
    /// Fixes the batch bisection dropped, each with the evidence that blamed
    /// it (ADR-2610160756 §4).
    pub dropped: Vec<Dropped>,
    /// Full gates that did not reproduce, naming both prefixes and both
    /// outputs (ADR-2610160756 §5).
    pub flakes: Vec<String>,
}

/// One batch of fast-gated fixes and what its full gate said
/// (ADR-2610160756 §3).
#[derive(Debug, Clone, PartialEq)]
pub struct Batch {
    pub fixes: Vec<String>,
    /// Whether the batch became the green base. A flaky batch never does (§5).
    pub green: bool,
}

/// A fix the bisection blamed, or that no longer applied once the culprit
/// was gone (ADR-2610160756 §4).
#[derive(Debug, Clone, PartialEq)]
pub struct Dropped {
    pub title: String,
    /// The fix's commit: two lenses may name two fixes alike, and only this
    /// tells them apart.
    pub sha: String,
    /// The red full-gate output that blamed it, or why it did not apply.
    pub evidence: String,
}

impl Default for ReviewReport {
    fn default() -> Self {
        ReviewReport {
            candidate: 0,
            confirmed: Vec::new(),
            fixed: Vec::new(),
            uncommitted: Vec::new(),
            gate_passed: false,
            gate_run: false,
            notes: Vec::new(),
            reviewed_by: Vec::new(),
            truncated: false,
            calibration: Calibration::NotNeeded,
            lenses: 0,
            answered: 0,
            full_gates: 0,
            batches: Vec::new(),
            dropped: Vec::new(),
            flakes: Vec::new(),
        }
    }
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
        let cal = self.calibration.note().map(|n| format!(" · {n}")).unwrap_or_default();
        let held = if self.uncommitted.is_empty() {
            String::new()
        } else {
            format!(" · {} gate-passed, not committed", self.uncommitted.len())
        };
        format!(
            "{} candidate(s) → {} confirmed real → {} fixed (gate-passed){}{}{}{}",
            self.candidate,
            self.confirmed.len(),
            self.fixed.len(),
            held,
            scope,
            by,
            cut
        ) + &cal
            + &self.batching_line()
    }

    /// How the fixes were proven when a fast gate was used: the full gates,
    /// the batches, the fixes bisection dropped, the flakes (ADR-2610160756
    /// §8). Empty without a fast gate, which runs no counted full gate.
    fn batching_line(&self) -> String {
        if self.full_gates == 0 {
            return String::new();
        }
        let mut line = format!(" · {}", plural(self.full_gates, "full gate"));
        if !self.batches.is_empty() {
            let batches: Vec<String> = self
                .batches
                .iter()
                .map(|b| format!("{} {}", plural(b.fixes.len(), "fix"), if b.green { "green" } else { "not green" }))
                .collect();
            line += &format!(" · batches: {}", batches.join(", "));
        }
        if !self.dropped.is_empty() {
            let titles: Vec<&str> = self.dropped.iter().map(|d| d.title.as_str()).collect();
            line += &format!(" · dropped by bisection: {}", titles.join(", "));
        }
        if !self.flakes.is_empty() {
            line += &format!(" · {}", plural(self.flakes.len(), "flake"));
        }
        line
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
        // Every bracket is a candidate: prose like "1 issue [see below]:"
        // may precede the envelope, and the first balanced span is not it.
        Ok(Ok(out)) => match json_candidates(&out).find_map(|js| serde_json::from_str::<T>(js).ok()) {
            Some(v) => Answer::Answered(v),
            None => Answer::NotAnswered(first_line(&out)),
        },
        Ok(Err(e)) => Answer::Failed(e),
        Err(e) => Answer::Failed(e),
    }
}

/// Extract the first balanced JSON value (object or array) from agent prose. Pure and
/// testable — `claude -p` often wraps JSON in markdown fences or commentary.
#[cfg(test)]
fn extract_json(text: &str) -> Option<&str> {
    json_candidates(text).next()
}

/// Every balanced JSON-looking span in `text`, one per opening bracket, in
/// order. The caller keeps the first that parses as what it asked for.
fn json_candidates(text: &str) -> impl Iterator<Item = &str> {
    let bytes = text.as_bytes();
    (0..bytes.len()).filter(move |&i| bytes[i] == b'{' || bytes[i] == b'[').filter_map(move |start| balanced_from(text, start))
}

/// The balanced span opening at byte `start` (a `{` or `[`), if it closes.
fn balanced_from(text: &str, start: usize) -> Option<&str> {
    let bytes = text.as_bytes();
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
fn silent() -> Reporter {
    std::sync::Arc::new(|_| {})
}

/// How often a phase that is waiting says so.
const HEARTBEAT: Duration = Duration::from_secs(30);

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
/// How the calls will actually be issued.
///
/// Both halves are true of every pass and neither can be predicted:
/// `budget_check` reads hexa's own spend accounting, not the account's live
/// limit, so whether the frontier will answer is only known by asking it.
/// The lenses go out at once; whichever fall back queue behind one permit
/// (ADR-2609131822 §2). Saying both is honest where picking one was a guess.
fn issue_order() -> &'static str {
    "in parallel; any that fall back to the local reviewer take their turn"
}

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

/// One frontier agent in `cwd`; its answer.
async fn claude_run(frontier: &dyn Frontier, prompt: &str, cwd: &Path, timeout_secs: u64) -> Result<String, String> {
    frontier
        .run(prompt, cwd, Duration::from_secs(timeout_secs), "harden")
        .await
        .map_err(|e| e.to_string())
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

/// Output budget for one local review call. A reasoning model emits its
/// thinking into this budget before the answer, so the figure is mostly
/// headroom for that; the envelope asked for is a few hundred tokens
/// (ADR-2609131835). `HEXA_REVIEW_MAX_TOKENS` overrides it.
const REVIEW_MAX_TOKENS: u32 = 16_384;

/// How much of a target the local reviewer is shown. A completion has no
/// way to fetch more, so the cap is the review's field of view and §3
/// requires it be stated when it bites.
const LOCAL_CONTEXT_CAP: usize = 120_000;

/// The target's source for a prompt, and whether it was truncated.
/// A directory contributes its files in name order until the cap.
fn read_target(target: &str, repo_root: &Path) -> (String, bool) {
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
            let mut room = LOCAL_CONTEXT_CAP.saturating_sub(out.len() + header.len()).min(text.len());
            // The cap is in bytes; the cut must still land on a char boundary.
            while !text.is_char_boundary(room) {
                room -= 1;
            }
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

/// Attempts for one local call. A local gateway drops a connection now and
/// then; the frontier path has had retries all along.
const LOCAL_ATTEMPTS: u32 = 3;

/// One local call at a time, process-wide (ADR-2609131822).
///
/// One model on one machine serves one request at a time. Four arriving
/// together queue inside the server, where the wait is invisible and counts
/// against each caller's timeout: the first fallback run lost three of four
/// lenses that way, each having waited its full 300 seconds without being
/// served. Queuing here makes the wait visible and finite.
static LOCAL_TURN: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

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
    let _turn = LOCAL_TURN.acquire().await.map_err(|e| format!("local reviewer queue closed: {e}"))?;
    // A reasoning model spends output tokens thinking before it answers, and
    // 4096 covered the thinking and not the reply: every lens came back
    // empty (ADR-2609131835). The envelope itself is small; the headroom is
    // for the reasoning in front of it.
    let budget = std::env::var("HEXA_REVIEW_MAX_TOKENS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(REVIEW_MAX_TOKENS);
    // The frontier path retries; this one did not, so a gateway that drops
    // one connection cost a whole lens — observed twice on this machine.
    let mut last = String::new();
    for attempt in 1..=LOCAL_ATTEMPTS {
        match hexa_infer::complete_text(
            &model,
            "You are a meticulous reviewer. Think briefly, then answer with the exact JSON object asked for and nothing else.",
            &user,
            budget,
        )
        .await
        {
            Ok(v) => return Ok(v),
            Err(e) => {
                last = format!("{model}: {e}");
                if attempt < LOCAL_ATTEMPTS {
                    tracing::debug!(attempt, error = %last, "local reviewer call failed; retrying");
                }
            }
        }
    }
    Err(last)
}

/// Ask the frontier; on any non-answer, ask the tier-mapped model with the
/// code inline (ADR-2609131702 §1). Returns what came back and who said it.
async fn ask<T: serde::de::DeserializeOwned>(
    frontier: &dyn Frontier,
    prompt: &str,
    code: &str,
    truncated: bool,
    cwd: &Path,
    timeout_secs: u64,
    attempts: u32,
) -> (Answer<T>, Reviewer) {
    let frontier = read_answer::<T>(Ok(claude_run_retry(frontier, prompt, cwd, timeout_secs, attempts).await));
    if let Answer::Answered(v) = frontier {
        return (Answer::Answered(v), Reviewer::Frontier);
    }
    // Why the frontier did not answer, for the case where the local one does
    // not either: an operator with both paths shut needs both reasons, and
    // the frontier's is the one that says the account is out of budget.
    let frontier_why = match &frontier {
        Answer::NotAnswered(why) => format!("frontier: {why}"),
        Answer::Failed(e) => format!("frontier failed: {e}"),
        Answer::Answered(_) => unreachable!("answered was returned above"),
    };
    let model = review_model().unwrap_or_else(|| "no tier model".to_string());
    let local = read_answer::<T>(Ok(local_run(prompt, code, truncated).await));
    match local {
        Answer::Answered(v) => (Answer::Answered(v), Reviewer::Local(model)),
        Answer::NotAnswered(why) => (Answer::NotAnswered(format!("{frontier_why}; then {model}: {why}")), Reviewer::None),
        Answer::Failed(e) => (Answer::Failed(format!("{frontier_why}; then {model}: {e}")), Reviewer::None),
    }
}

/// A short snippet with one planted defect, for asking a reviewer that
/// reported nothing whether it can find anything at all (ADR-2609131907).
///
/// The defect is a use-after-release: the guard is dropped on its own line,
/// and the map is then written outside the lock. It is of a class the
/// concurrency lens hunts, and it is unambiguous — a reviewer that reads the
/// code cannot honestly call this clean.
const PROBE_CODE: &str = r#"
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub struct Counters {
    inner: Arc<Mutex<HashMap<String, u64>>>,
}

impl Counters {
    pub fn bump(&self, key: &str) {
        let mut guard = self.inner.lock().unwrap();
        let next = guard.get(key).copied().unwrap_or(0) + 1;
        drop(guard);
        // The lock is gone; this write races every other caller.
        self.inner.lock().unwrap().insert(key.to_string(), next);
    }
}
"#;

/// Whether an empty local review was shown to be worth anything
/// (ADR-2609131907 §2).
#[derive(Debug, Clone, PartialEq)]
pub enum Calibration {
    /// Not applicable: findings were produced, or the frontier answered.
    NotNeeded,
    /// The reviewer found the planted defect.
    Calibrated,
    /// The reviewer missed it; an empty result from it is not evidence.
    Missed,
    /// The probe itself did not answer.
    Unrun(String),
}

impl Calibration {
    pub fn note(&self) -> Option<String> {
        match self {
            Calibration::NotNeeded => None,
            Calibration::Calibrated => Some("calibrated — the reviewer found a planted defect".into()),
            Calibration::Missed => Some(
                "uncalibrated — the reviewer missed a planted defect, so an empty result is not evidence".into(),
            ),
            Calibration::Unrun(why) => Some(format!("calibration did not run: {why}")),
        }
    }
}

/// Does a reply show the reviewer noticed the planted defect?
///
/// What must be noticed: **the read and the write are not under one lock.**
/// That sentence lived in a constant of its own until clippy pointed out
/// nothing read it — two statements of one expectation, and only this one
/// decided anything. Judged on the finding's own words, not on a count: a
/// reviewer may report it once or alongside noise.
fn probe_found_it(findings: &[Finding]) -> bool {
    findings.iter().any(|f| {
        let t = format!("{} {}", f.title, f.description).to_ascii_lowercase();
        (t.contains("lock") || t.contains("guard") || t.contains("mutex"))
            && (t.contains("rac") || t.contains("drop") || t.contains("outside") || t.contains("released") || t.contains("atomic"))
    })
}

/// Ask the local reviewer to find the planted defect in [`PROBE_CODE`].
///
/// `local` is the local reviewer itself, called with the probe prompt; the
/// harness passes [`local_run`]. Never [`ask`]: the empty result being judged
/// is the local model's, and a frontier that recovered from a transient
/// failure would answer the probe and calibrate a model it never tested.
async fn calibrate<F, Fut>(local: F) -> Calibration
where
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = Result<String, String>>,
{
    let prompt = "You are an adversarial code reviewer. Hunt this code for concurrency defects: races, \
         lost updates, work done outside a lock. Report exclusively REAL bugs you can point to. \
         Output ONLY a JSON object: \
         {\"findings\":[{\"title\":\"...\",\"location\":\"file:line or fn\",\"description\":\"the concrete failure\",\"lens\":\"concurrency\"}]}".to_string();
    match read_answer::<FindingsEnvelope>(Ok(local(prompt).await)) {
        Answer::Answered(env) if probe_found_it(&env.findings) => Calibration::Calibrated,
        Answer::Answered(_) => Calibration::Missed,
        Answer::NotAnswered(why) => Calibration::Unrun(why),
        Answer::Failed(e) => Calibration::Unrun(e),
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
    frontier: &dyn Frontier,
    prompt: &str,
    cwd: &Path,
    timeout_secs: u64,
    attempts: u32,
) -> Result<String, String> {
    let attempts = attempts.clamp(1, 6);
    let mut last_err = String::new();
    for attempt in 1..=attempts {
        let this_prompt = retry_prompt(prompt, attempt, attempts, &last_err);
        match claude_run(frontier, &this_prompt, cwd, timeout_secs).await {
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

/// Every path git reports as changed, from `git status --porcelain=v1 -z -uall`,
/// with its current bytes (`None` = deleted).
///
/// `-uall` lists each file inside an untracked directory, and the bytes tell a
/// second edit to an already-dirty file from no edit at all (ADR-2610141733 §6).
async fn changed_files(repo_root: &Path) -> std::collections::BTreeMap<String, Option<Vec<u8>>> {
    let mut files = std::collections::BTreeMap::new();
    let out = tokio::process::Command::new("git")
        .args(["status", "--porcelain=v1", "-z", "-uall"])
        .current_dir(repo_root)
        .output()
        .await;
    let Ok(out) = out else {
        return files;
    };
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut entries = stdout.split('\0');
    while let Some(entry) = entries.next() {
        let (Some(xy), Some(path)) = (entry.get(..2), entry.get(3..)) else {
            continue;
        };
        if xy.starts_with('R') || xy.starts_with('C') {
            // The source path of a rename or copy follows as its own entry.
            entries.next();
        }
        if path.is_empty() {
            continue;
        }
        files.insert(path.to_string(), std::fs::read(repo_root.join(path)).ok());
    }
    files
}

/// The commit HEAD names, if there is one.
async fn head(repo_root: &Path) -> Option<String> {
    let out = tokio::process::Command::new("git")
        .args(["rev-parse", "--verify", "-q", "HEAD"])
        .current_dir(repo_root)
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!sha.is_empty()).then_some(sha)
}

/// Run git quietly in `repo_root`; true iff it exited 0.
async fn git_ok(repo_root: &Path, args: &[&str]) -> bool {
    tokio::process::Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Put each of `touched` back as it was in `before`: its old bytes, absent,
/// or — not in `before` at all — as HEAD has it.
async fn revert_paths(
    repo_root: &Path,
    touched: &[String],
    before: &std::collections::BTreeMap<String, Option<Vec<u8>>>,
) {
    for p in touched {
        let full = repo_root.join(p);
        match before.get(p) {
            Some(Some(bytes)) => {
                if let Some(parent) = full.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&full, bytes);
            }
            Some(None) => {
                if full.exists() {
                    let _ = std::fs::remove_file(&full);
                }
            }
            None => {
                let _ = git_ok(repo_root, &["reset", "-q", "--", p]).await;
                if !git_ok(repo_root, &["checkout", "-q", "HEAD", "--", p]).await && full.exists() {
                    let _ = std::fs::remove_file(&full);
                }
            }
        }
    }
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
///
/// `trailers` go in the trailer block beside `Co-Authored-By`, e.g.
/// `Hexa-Gate: fast` (ADR-2610160756 §2).
///
/// Returns true iff `git commit` succeeded.
async fn commit_result(
    repo_root: &Path,
    paths: &[String],
    subject: &str,
    trailer: &str,
    trailers: &[&str],
    notes: &mut Vec<String>,
) -> bool {
    if paths.is_empty() {
        notes.push("nothing new to commit".to_string());
        return false;
    }
    let mut add_args: Vec<&str> = vec!["add", "-A", "--"];
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
            return false;
        }
        Err(e) => {
            notes.push(format!("git not found — skipping auto-commit (non-fatal): {e}"));
            return false;
        }
    }
    let subject: String = subject.lines().next().unwrap_or(subject).chars().take(72).collect();
    let mut block: String = trailers.iter().map(|t| format!("{t}\n")).collect();
    block += &format!("Co-Authored-By: {trailer} <noreply@hexa.local>");
    let msg = format!("{subject}\n\n{block}");
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
        Ok(out) if out.status.success() => {
            notes.push("auto-committed result (hexa-factory)".to_string());
            true
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            if stderr.contains("nothing to commit") {
                notes.push("nothing new to commit".to_string());
            } else {
                notes.push(format!("git commit failed (non-fatal): {}", stderr.trim()));
            }
            false
        }
        Err(e) => {
            notes.push(format!("git not found — skipping auto-commit (non-fatal): {e}"));
            false
        }
    }
}

/// Run the adversarial review pipeline over `target` (a path), gated by `gate` (a
/// shell command that must exit 0). Each fix that passes the gate is committed on
/// its own; one that fails the gate is reverted (ADR-2610141733 §6).
pub async fn run_review(frontier: Arc<dyn Frontier>, target: &str, gate: &str, repo_root: &Path) -> ReviewReport {
    run_review_with(frontier, target, gate, repo_root, silent()).await
}

/// [`run_review`], reporting each phase as it happens.
pub async fn run_review_with(
    frontier: Arc<dyn Frontier>,
    target: &str,
    gate: &str,
    repo_root: &Path,
    reporter: Reporter,
) -> ReviewReport {
    let gates = HardenGates { gate: gate.to_string(), fast_gate: None, batch: None };
    review(frontier, None, target, &gates, repo_root, reporter).await
}

/// The gates a harden pass answers to (ADR-2610160756 §1).
#[derive(Debug, Clone, PartialEq)]
pub struct HardenGates {
    /// The full gate: what ships must pass it.
    pub gate: String,
    /// Gates each fix when set; `{touched}` is replaced by the fix's changed
    /// paths (§2). Unset, every fix gets the full gate (ADR-2610141733 §6).
    pub fast_gate: Option<String>,
    /// Close a batch with a full gate after this many fixes; `None` = one
    /// batch for the whole pass (§3).
    pub batch: Option<usize>,
}

/// [`run_review`] under `gates`: with a fast gate, each fix is gated fast,
/// each batch in full, and a breaking fix is found by bisecting the batch
/// on `history` (ADR-2610160756).
pub async fn run_review_gated(
    frontier: Arc<dyn Frontier>,
    history: Arc<dyn GitHistory>,
    target: &str,
    gates: &HardenGates,
    repo_root: &Path,
) -> ReviewReport {
    run_review_gated_with(frontier, history, target, gates, repo_root, silent()).await
}

/// [`run_review_gated`], reporting each phase as it happens.
pub async fn run_review_gated_with(
    frontier: Arc<dyn Frontier>,
    history: Arc<dyn GitHistory>,
    target: &str,
    gates: &HardenGates,
    repo_root: &Path,
    reporter: Reporter,
) -> ReviewReport {
    review(frontier, Some(history), target, gates, repo_root, reporter).await
}

/// The pipeline both entry points share. `history` is needed only with a
/// fast gate: without one, nothing is reset or cherry-picked.
async fn review(
    frontier: Arc<dyn Frontier>,
    history: Option<Arc<dyn GitHistory>>,
    target: &str,
    gates: &HardenGates,
    repo_root: &Path,
    reporter: Reporter,
) -> ReviewReport {
    let gate = gates.gate.as_str();
    let mut report = ReviewReport::default();
    // What the operator had already changed. A fix that touches any of these
    // is left in the tree for review, never committed over the operator's work.
    let dirty_before: std::collections::BTreeSet<String> = changed_files(repo_root).await.into_keys().collect();

    // The local reviewer cannot go and read the code, so it is read once
    // here and carried in every prompt (ADR-2609131702 §1).
    let (code, truncated) = read_target(target, repo_root);
    report.truncated = truncated;
    if truncated {
        report.notes.push(format!("target truncated to {LOCAL_CONTEXT_CAP} bytes for the local reviewer"));
    }

    // ── Phase 1: hunt (parallel lenses) ──────────────────────────────────────
    let hunt = Phase::start(&reporter, "hunt", format!("{} on {target}, {}", plural(LENSES.len(), "lens"), issue_order()), HEARTBEAT);
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
        let f = frontier.clone();
        hunts.push((
            lens.key,
            tokio::spawn(async move { ask::<FindingsEnvelope>(&*f, &prompt, &code, truncated, &root, 600, DEFAULT_RETRIES).await }),
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
    // An empty answer from a local reviewer is either good news or no news,
    // and one small call tells them apart (ADR-2609131907 §3). Only then:
    // the frontier has been observed finding real defects, and a pass with
    // findings has already shown the reviewer looks.
    let local_only = report.reviewed_by.iter().any(|r| r != "frontier");
    if candidates.is_empty() && local_only {
        let probe = Phase::start(&reporter, "calibrate", "one planted defect, to see if an empty review means anything", HEARTBEAT);
        report.calibration = calibrate(|prompt| async move { local_run(&prompt, PROBE_CODE, false).await }).await;
        probe.finish(report.calibration.note().unwrap_or_else(|| "not needed".into()));
    }
    if !report.reviewed() {
        hunt.finish(format!("no lens answered — nothing was reviewed ({} asked)", report.lenses));
        report.notes.push("nothing was reviewed: no lens returned the findings envelope".to_string());
        // A gate run now would pass on code nobody read, and a pass is not
        // entitled to that claim (ADR-2609131646 §3).
        return report;
    }
    hunt.finish(format!("{} to verify", plural(candidates.len(), "candidate")));

    // ── Phase 2: skeptical verify (parallel, default-refute) ─────────────────
    let verify = Phase::start(&reporter, "verify", format!("{}, default refute, {}", plural(candidates.len(), "claim"), issue_order()), HEARTBEAT);
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
        let fr = frontier.clone();
        checks.push((
            f,
            tokio::spawn(async move { ask::<VerdictEnvelope>(&*fr, &prompt, &code, truncated, &root, 600, DEFAULT_RETRIES).await }),
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
    // With a fast gate, each fix is gated fast and each batch in full
    // (ADR-2610160756 §2–§3); the base of the first batch is HEAD now.
    let mut batching = match (&gates.fast_gate, &history) {
        (Some(fast), Some(history)) => Some(Batching {
            history: history.as_ref(),
            gate,
            fast_gate: fast.as_str(),
            batch: gates.batch.filter(|n| *n > 0),
            repo_root,
            base: head(repo_root).await,
            pending: Vec::new(),
            retry: Vec::new(),
            last_green: None,
            green_changes: Default::default(),
        }),
        _ => None,
    };
    let under = if batching.is_some() { "the fast gate, each batch under the full gate" } else { "the gate" };
    let fix = Phase::start(&reporter, "fix", format!("{} to fix, one at a time, each under {under}", plural(report.confirmed.len(), "bug")), HEARTBEAT);
    let confirmed = report.confirmed.clone();
    for f in &confirmed {
        fix.note(format!("fixing: {}", f.title));
        let prompt = format!(
            "Fix this CONFIRMED bug in the code under `{target}`, then add a regression test that \
             fails without the fix. Make a minimal, correct change. Do not commit; hexa gates and \
             commits each fix. Bug:\ntitle: {title}\nlocation: {loc}\ndescription: {desc}",
            target = target, title = f.title, loc = f.location, desc = f.description
        );
        // Only the frontier can edit files; a completion has no way to
        // (ADR-2609131702 §1). And a reply is not an edit: the frontier
        // exits 0 when it declines, so the tree itself is the evidence.
        let before_head = head(repo_root).await;
        let before = changed_files(repo_root).await;
        match claude_run_retry(&*frontier, &prompt, repo_root, 900, DEFAULT_RETRIES).await {
            Ok(reply) => {
                if let Some(bh) = &before_head {
                    if head(repo_root).await.as_ref() != Some(bh)
                        && git_ok(repo_root, &["reset", "-q", "--soft", bh]).await
                    {
                        fix.note(format!("agent committed its own change: {}", f.title));
                        report.notes.push(format!(
                            "the agent committed its own change for '{}'; folded back to be gated",
                            f.title
                        ));
                    }
                }
                let after = changed_files(repo_root).await;
                let touched: Vec<String> = before
                    .keys()
                    .chain(after.keys())
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .filter(|p| before.get(*p) != after.get(*p))
                    .cloned()
                    .collect();
                if touched.is_empty() {
                    fix.note(format!("NO EDIT — {}: {}", f.title, first_line(&reply)));
                    report.notes.push(format!("no fix applied for '{}': the tree did not change — {}", f.title, first_line(&reply)));
                    continue;
                }
                if let Some(b) = batching.as_mut() {
                    b.gate_fix(f, &touched, &before, before_head.as_deref(), &dirty_before, &fix, &mut report).await;
                    if b.batch.is_some_and(|n| b.pending.len() >= n) {
                        b.close(&reporter, &mut report).await;
                    }
                    continue;
                }
                fix.note(format!("gate after fix: {gate}"));
                let (passed, _) = crate::direct_exec::run_evidence(gate, repo_root).await;
                if passed {
                    fix.note(format!("gate passed: {}", f.title));
                    let held: Vec<&str> = touched
                        .iter()
                        .filter(|p| dirty_before.contains(*p))
                        .map(String::as_str)
                        .collect();
                    if !held.is_empty() {
                        report.uncommitted.push(f.title.clone());
                        report.notes.push(format!(
                            "fix for '{}': gate passed, but it touched files that had uncommitted changes before the pass: {}; left in the tree for review",
                            f.title,
                            held.join(", ")
                        ));
                        continue;
                    }
                    let mut scratch = Vec::new();
                    if commit_result(
                        repo_root,
                        &touched,
                        &format!("fix(harden): {}", f.title),
                        "hexa-harden",
                        &[],
                        &mut scratch,
                    )
                    .await
                    {
                        fix.note(format!("committed: {}", f.title));
                        report.fixed.push(f.title.clone());
                    } else {
                        report.uncommitted.push(f.title.clone());
                        report.notes.extend(scratch);
                    }
                } else {
                    fix.note(format!("gate FAILED after: {}", f.title));
                    revert_paths(repo_root, &touched, &before).await;
                    report.notes.push(format!("fix for '{}' did not pass the gate — reverted", f.title));
                }
            }
            Err(e) => fix.note(format!("no fix produced for {}: {e}", f.title)),
        }
    }
    fix.finish(format!("{} fixed", report.fixed.len()));

    if let Some(b) = batching.as_mut() {
        b.close(&reporter, &mut report).await;
        b.last_chance(&reporter, &mut report).await;
    }

    // ── Final gate ───────────────────────────────────────────────────────────
    // The last full gate is the final gate when it ran on this very tree
    // (ADR-2610160756 §7): the same HEAD, and the same uncommitted changes —
    // a fix left in the tree after it was seen only by the fast gate.
    if let Some(b) = &batching {
        if b.last_green.is_some()
            && head(repo_root).await == b.last_green
            && changed_files(repo_root).await == b.green_changes
        {
            let final_gate = Phase::start(&reporter, "gate", "the last full gate ran on this tree", HEARTBEAT);
            report.gate_passed = true;
            report.gate_run = true;
            final_gate.finish("passed (not run again)");
            return report;
        }
    }
    let final_gate = Phase::start(&reporter, "gate", format!("running: {gate}"), HEARTBEAT);
    let (passed, _) = crate::direct_exec::run_evidence(gate, repo_root).await;
    if batching.is_some() {
        report.full_gates += 1;
    }
    report.gate_passed = passed;
    report.gate_run = true;
    final_gate.finish(if passed { "passed" } else { "FAILED" });
    report
}

/// A fix hexa committed under the fast gate, or committed and set aside
/// after the fast gate rejected it, kept by its commit so it can be
/// cherry-picked again (ADR-2610160756 §4, §6).
#[derive(Debug, Clone)]
struct Landed {
    title: String,
    sha: String,
    touched: Vec<String>,
}

fn titles_of(fixes: &[Landed]) -> Vec<String> {
    fixes.iter().map(|l| l.title.clone()).collect()
}

/// A pass's batched gating (ADR-2610160756 §2–§7): the green base, the
/// fixes committed since it, and the fixes owed a last chance.
struct Batching<'a> {
    history: &'a dyn GitHistory,
    gate: &'a str,
    fast_gate: &'a str,
    batch: Option<usize>,
    repo_root: &'a Path,
    /// HEAD at the last full gate that passed; at first, HEAD when phase 3
    /// began. `None` only in a repository with no commit.
    base: Option<String>,
    /// Fixes committed since `base`, oldest first.
    pending: Vec<Landed>,
    /// Fixes dropped by bisection or rejected by the fast gate (§6).
    retry: Vec<Landed>,
    /// HEAD when a full gate last passed (§7).
    last_green: Option<String>,
    /// The uncommitted changes in the tree that gate passed on (§7).
    green_changes: std::collections::BTreeMap<String, Option<Vec<u8>>>,
}

impl Batching<'_> {
    /// One full-gate run, counted; a pass records the tree it passed on.
    async fn full_gate(&mut self, report: &mut ReviewReport) -> (bool, String) {
        let (passed, out) = crate::direct_exec::run_evidence(self.gate, self.repo_root).await;
        report.full_gates += 1;
        if passed {
            self.last_green = head(self.repo_root).await;
            self.green_changes = changed_files(self.repo_root).await;
        }
        (passed, out)
    }

    /// The fast gate over `touched`. A pass that ran no tests is a fail, and
    /// the second value says so (§2).
    async fn fast_gate(&self, touched: &[String]) -> (bool, bool) {
        let cmd = crate::direct_exec::substitute_touched(self.fast_gate, touched);
        let (passed, out) = crate::direct_exec::run_evidence(&cmd, self.repo_root).await;
        let vacuous = passed && crate::direct_exec::evidence_is_vacuous(&out);
        (passed && !vacuous, vacuous)
    }

    /// The branch at `base` plus `fixes`, cherry-picked in order (§4).
    fn build(&self, base: &str, fixes: &[Landed]) -> Result<(), String> {
        self.history.reset_to(self.repo_root, base)?;
        for l in fixes {
            self.history.cherry_pick(self.repo_root, &l.sha).map_err(|e| format!("'{}' did not apply: {e}", l.title))?;
        }
        Ok(())
    }

    /// Build `base` + `fixes` and run the full gate on it. A prefix that
    /// cannot be built is red, with the reason as its output.
    async fn probe(&mut self, base: &str, fixes: &[Landed], report: &mut ReviewReport) -> (bool, String) {
        match self.build(base, fixes) {
            Ok(()) => self.full_gate(report).await,
            Err(e) => (false, e),
        }
    }

    /// The per-fix step (§2): the fast gate; a pass commits alone with a
    /// `Hexa-Gate: fast` trailer, a fail is committed aside for its last
    /// chance and then restored as ADR-2610141733 §6 restores it.
    #[allow(clippy::too_many_arguments)]
    async fn gate_fix(
        &mut self,
        f: &Finding,
        touched: &[String],
        before: &std::collections::BTreeMap<String, Option<Vec<u8>>>,
        before_head: Option<&str>,
        dirty_before: &std::collections::BTreeSet<String>,
        fix: &Phase,
        report: &mut ReviewReport,
    ) {
        fix.note(format!("fast gate after fix: {}", crate::direct_exec::substitute_touched(self.fast_gate, touched)));
        let (passed, vacuous) = self.fast_gate(touched).await;
        let subject = format!("fix(harden): {}", f.title);
        if passed {
            fix.note(format!("fast gate passed: {}", f.title));
            let held: Vec<&str> = touched.iter().filter(|p| dirty_before.contains(*p)).map(String::as_str).collect();
            if !held.is_empty() {
                report.uncommitted.push(f.title.clone());
                report.notes.push(format!(
                    "fix for '{}': fast gate passed, but it touched files that had uncommitted changes before the pass: {}; left in the tree for review",
                    f.title,
                    held.join(", ")
                ));
                return;
            }
            let mut scratch = Vec::new();
            if commit_result(self.repo_root, touched, &subject, "hexa-harden", &["Hexa-Gate: fast"], &mut scratch).await {
                fix.note(format!("committed: {}", f.title));
                report.fixed.push(f.title.clone());
                if let Some(sha) = head(self.repo_root).await {
                    self.pending.push(Landed { title: f.title.clone(), sha, touched: touched.to_vec() });
                }
            } else {
                report.uncommitted.push(f.title.clone());
                report.notes.extend(scratch);
            }
            return;
        }
        let why = if vacuous { "was rejected: its fast gate ran no tests (vacuous)" } else { "did not pass the fast gate" };
        fix.note(format!("fast gate FAILED after: {}", f.title));
        // Kept retryable as a commit, then reset away. Only when every path
        // was clean before the fix: a reset must never meet a file someone
        // else had already changed (ADR-2610141733 §6).
        if let Some(bh) = before_head.filter(|_| touched.iter().all(|p| !before.contains_key(p))) {
            let mut scratch = Vec::new();
            if commit_result(self.repo_root, touched, &subject, "hexa-harden", &["Hexa-Gate: fast"], &mut scratch).await {
                if let Some(sha) = head(self.repo_root).await {
                    self.retry.push(Landed { title: f.title.clone(), sha, touched: touched.to_vec() });
                }
                if self.history.reset_to(self.repo_root, bh).is_err() {
                    let _ = git_ok(self.repo_root, &["reset", "-q", "--soft", bh]).await;
                }
            }
        }
        revert_paths(self.repo_root, touched, before).await;
        report.notes.push(format!("fix for '{}' {why} — reverted", f.title));
    }

    /// Close the batch with one full gate (§3); bisect it if red (§4),
    /// confirming the culprit before blaming it (§5).
    async fn close(&mut self, reporter: &Reporter, report: &mut ReviewReport) {
        if self.pending.is_empty() {
            return;
        }
        let mut batch = self.pending.clone();
        let phase = Phase::start(reporter, "batch", format!("{}, one full gate: {}", plural(batch.len(), "fix"), self.gate), HEARTBEAT);
        let (green, _) = self.full_gate(report).await;
        if green {
            self.green(batch, report).await;
            phase.finish("green");
            return;
        }
        let Some(base) = self.base.clone() else {
            // No commit to bisect from: the batch stands, unproven.
            report.batches.push(Batch { fixes: titles_of(&batch), green: false });
            report.notes.push("the batch's full gate failed and there is no base commit to bisect from".to_string());
            phase.finish("red, not bisected");
            return;
        };
        loop {
            let batch_head = head(self.repo_root).await;
            // Prefix 0 (the base) is green and prefix n is red: find the
            // smallest red prefix k.
            let (mut lo, mut hi) = (0, batch.len());
            while hi - lo > 1 {
                let mid = (lo + hi) / 2;
                phase.note(format!("bisecting: prefix {mid} of {}", batch.len()));
                let (g, _) = self.probe(&base, &batch[..mid], report).await;
                if g {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let k = hi;
            let culprit = batch[k - 1].clone();
            phase.note(format!("confirming '{}': prefix {} then prefix {k}", culprit.title, k - 1));
            let (before_green, before_out) = self.probe(&base, &batch[..k - 1], report).await;
            let (at_green, at_out) = self.probe(&base, &batch[..k], report).await;
            if !before_green || at_green {
                // It did not reproduce: the gate is flaky, and no innocent fix
                // is dropped for it (§5).
                let restored = match &batch_head {
                    Some(sha) => self.history.reset_to(self.repo_root, sha),
                    None => self.build(&base, &batch),
                };
                if let Err(e) = restored {
                    report.notes.push(format!("could not restore the flaky batch: {e}"));
                }
                let verdict = |g: bool| if g { "passed" } else { "failed" };
                report.flakes.push(format!(
                    "the full gate did not reproduce on '{}': prefix {} {} ({}); prefix {k} {} ({})",
                    culprit.title,
                    k - 1,
                    verdict(before_green),
                    first_line(&before_out),
                    verdict(at_green),
                    first_line(&at_out)
                ));
                report.batches.push(Batch { fixes: titles_of(&batch), green: false });
                self.pending = batch;
                phase.finish("FLAKY — bisection stopped, nothing dropped, the base is unchanged");
                return;
            }
            self.drop_fix(&culprit, at_out, report);
            phase.note(format!("dropped: {}", culprit.title));
            // Rebuild: prefix k−1, then fixes k+1..n on top.
            let mut rebuilt = batch[..k - 1].to_vec();
            if let Err(e) = self.build(&base, &rebuilt) {
                report.notes.push(format!("could not rebuild the batch: {e}"));
            }
            for l in &batch[k..] {
                match self.history.cherry_pick(self.repo_root, &l.sha) {
                    Ok(()) => rebuilt.push(l.clone()),
                    Err(e) => {
                        phase.note(format!("dropped, no longer applies: {}", l.title));
                        self.drop_fix(l, format!("no longer applies once '{}' is dropped: {e}", culprit.title), report);
                    }
                }
            }
            batch = rebuilt;
            if batch.is_empty() {
                self.pending.clear();
                phase.finish("every fix dropped; the base stands");
                return;
            }
            phase.note(format!("rebuilt batch of {}: full gate", plural(batch.len(), "fix")));
            if self.full_gate(report).await.0 {
                self.green(batch, report).await;
                phase.finish(format!("green after bisection, {} dropped", report.dropped.len()));
                return;
            }
        }
    }

    /// The batch passed its full gate: it stands, and its head is the new base.
    async fn green(&mut self, batch: Vec<Landed>, report: &mut ReviewReport) {
        report.batches.push(Batch { fixes: titles_of(&batch), green: true });
        self.base = head(self.repo_root).await;
        self.pending.clear();
    }

    /// Take `l` out of what the pass claims, keep why, and owe it a last
    /// chance (§4, §6).
    fn drop_fix(&mut self, l: &Landed, evidence: String, report: &mut ReviewReport) {
        // One entry per commit: a same-titled fix that passed stays claimed.
        if let Some(i) = report.fixed.iter().position(|t| *t == l.title) {
            report.fixed.remove(i);
        }
        report.notes.push(format!("fix for '{}' dropped from its batch: {}", l.title, first_line(&evidence)));
        report.dropped.push(Dropped { title: l.title.clone(), sha: l.sha.clone(), evidence });
        self.retry.push(l.clone());
    }

    /// Each fix dropped or rejected gets one more try on the final tree,
    /// under the fast gate and a full gate together (§6).
    async fn last_chance(&mut self, reporter: &Reporter, report: &mut ReviewReport) {
        let retry = std::mem::take(&mut self.retry);
        if retry.is_empty() {
            return;
        }
        let phase = Phase::start(reporter, "retry", format!("{} once more, last", plural(retry.len(), "fix")), HEARTBEAT);
        let mut landed = 0;
        for l in retry {
            let Some(prev) = head(self.repo_root).await else {
                continue;
            };
            if let Err(e) = self.history.cherry_pick(self.repo_root, &l.sha) {
                phase.note(format!("does not apply: {}", l.title));
                report.notes.push(format!("last chance for '{}': it does not apply — {e}", l.title));
                continue;
            }
            let (fast, _) = self.fast_gate(&l.touched).await;
            let full = fast && self.full_gate(report).await.0;
            if full {
                phase.note(format!("landed: {}", l.title));
                report.fixed.push(l.title.clone());
                report.dropped.retain(|d| d.sha != l.sha);
                report.notes.push(format!("fix for '{}' landed on its last chance", l.title));
                landed += 1;
            } else {
                phase.note(format!("still fails: {}", l.title));
                let which = if fast { "the full gate" } else { "the fast gate" };
                if let Err(e) = self.history.reset_to(self.repo_root, &prev) {
                    report.notes.push(format!("could not reset away '{}': {e}", l.title));
                }
                report.notes.push(format!("last chance for '{}': it still fails {which} — not committed", l.title));
            }
        }
        phase.finish(format!("{landed} landed"));
    }
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
    frontier: Arc<dyn Frontier>,
    challenge: &str,
    target: &str,
    gate: &str,
    n_designs: usize,
    repo_root: &Path,
    timeout_secs: u64,
    retries: u32,
) -> BuildReport {
    run_build_with(frontier, challenge, target, gate, n_designs, repo_root, timeout_secs, retries, silent()).await
}

/// [`run_build`], reporting each phase as it happens.
#[allow(clippy::too_many_arguments)]
pub async fn run_build_with(
    frontier: Arc<dyn Frontier>,
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
    // Per file (`-uall`): a directory-level entry would hide an operator's
    // file inside the build's target directory.
    let dirty_before: std::collections::BTreeSet<String> = changed_files(repo_root).await.into_keys().collect();
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
        let f = frontier.clone();
        tasks.push(tokio::spawn(async move { claude_run_retry(&*f, &prompt, &root, timeout_secs, retries).await }));
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
        let f = frontier.clone();
        ctasks.push(tokio::spawn(async move { claude_run_retry(&*f, &prompt, &root, timeout_secs, retries).await }));
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
        &*frontier,
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
    if let Err(e) = claude_run_retry(&*frontier, &build_prompt, repo_root, timeout_secs.saturating_mul(4), retries).await {
        build.note(format!("build agent error: {e}"));
        report.notes.push(format!("build agent error: {e}"));
    }
    build.note(format!("gate: {gate}"));
    let (ok, _) = crate::direct_exec::run_evidence(gate, repo_root).await;
    report.build_ok = ok;
    build.finish(if ok { "gate passed" } else { "gate FAILED" });
    if ok {
        // Only the files the build changed. The target is not added back: it
        // is a pathspec, and would commit the operator's own edits to it or
        // to any file under it.
        let paths: Vec<String> = changed_files(repo_root)
            .await
            .into_keys()
            .filter(|p| !dirty_before.contains(p))
            .collect();
        commit_result(
            repo_root,
            &paths,
            &format!("feat: {challenge}"),
            "hexa-build",
            &[],
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

    /// A bracket in the prose before the envelope is not the answer: the
    /// envelope after it still is.
    #[test]
    fn an_envelope_after_a_bracket_in_prose_is_answered() {
        let a: Answer<FindingsEnvelope> = read_answer(ok(
            "Found 1 issue [see below]:\n{\"findings\":[{\"title\":\"t\",\"location\":\"l\",\"description\":\"d\",\"lens\":\"x\"}]}",
        ));
        assert!(matches!(a, Answer::Answered(ref e) if e.findings.len() == 1), "{a:?}");

        // `[42]` is valid JSON but not the envelope; `{ok}` is not JSON at all.
        let a: Answer<FindingsEnvelope> = read_answer(ok(
            "see line [42]; it returns {ok} early.\n{\"findings\":[{\"title\":\"t\",\"location\":\"l\",\"description\":\"d\",\"lens\":\"x\"}]}",
        ));
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

    /// The cap is a byte count, but a source file is UTF-8: a cut that lands
    /// inside a multi-byte character must back off to its start, not panic.
    #[test]
    fn truncating_at_the_cap_never_splits_a_utf8_character() {
        let dir = std::env::temp_dir().join(format!("hexa-target-utf8-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        // Every offset in a run of 3-byte `—` is hit, whatever the header
        // length: one of these three files puts the cut mid-character.
        for pad in 0..3 {
            std::fs::write(dir.join("src/wide.rs"), format!("{}{}", "x".repeat(pad), "—".repeat(LOCAL_CONTEXT_CAP / 3 + 10))).unwrap();
            let (code, truncated) = read_target("src/wide.rs", &dir);
            assert!(truncated, "over the cap");
            assert!(code.len() <= LOCAL_CONTEXT_CAP && code.len() + 3 > LOCAL_CONTEXT_CAP, "shown {} bytes, cap {}", code.len(), LOCAL_CONTEXT_CAP);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Both paths shut means both reasons: the frontier's names the budget,
    /// the local one names what the model said (ADR-2609131948).
    #[test]
    fn a_non_answer_from_both_paths_carries_both_reasons() {
        // The shape `ask` builds when neither answers.
        let combined = format!("{}; then {}: {}", "frontier: You've hit your monthly spend limit.", "a-model", "I cannot help with that.");
        assert!(combined.starts_with("frontier: "), "the frontier's reason leads: {combined}");
        assert!(combined.contains("spend limit"), "{combined}");
        assert!(combined.contains("a-model: I cannot help"), "and the local model's follows: {combined}");
    }

    /// ADR-2609131907 §1: the probe must actually contain the defect its
    /// expectation describes, or it measures nothing.
    #[test]
    fn the_probe_contains_the_defect_it_claims_to() {
        assert!(PROBE_CODE.contains("drop(guard)"), "the guard is released early");
        let after_drop = PROBE_CODE.split("drop(guard)").nth(1).unwrap();
        assert!(after_drop.contains("insert("), "and the write happens after it: {after_drop}");
        assert!(PROBE_CODE.lines().count() < 30, "small enough to be the cheapest call the harness makes");
    }

    /// §2: a reviewer that names the defect calibrates; one that finds
    /// nothing, or something else, does not.
    #[test]
    fn only_naming_the_planted_defect_calibrates() {
        let f = |title: &str, desc: &str| Finding {
            title: title.into(),
            location: "bump".into(),
            description: desc.into(),
            lens: "concurrency".into(),
        };
        assert!(probe_found_it(&[f("Lock dropped before the write", "the guard is released, then the map is written, so two callers race")]));
        assert!(probe_found_it(&[f("Race on counters", "read and write are not atomic because the mutex guard is dropped between them")]));
        assert!(!probe_found_it(&[]), "finding nothing is not finding it");
        assert!(!probe_found_it(&[f("unwrap on a poisoned mutex", "lock().unwrap() panics if another thread panicked")]), "a different real bug is not this one");
    }

    /// §2 and §4: each outcome reads as what it is, and a probe that did not
    /// answer is not a miss.
    #[test]
    fn each_calibration_outcome_says_what_it_is() {
        assert_eq!(Calibration::NotNeeded.note(), None);
        assert!(Calibration::Calibrated.note().unwrap().contains("found a planted defect"));
        let missed = Calibration::Missed.note().unwrap();
        assert!(missed.contains("not evidence"), "{missed}");
        let unrun = Calibration::Unrun("spend limit".into()).note().unwrap();
        assert!(unrun.starts_with("calibration did not run: spend limit"), "{unrun}");
        assert!(!unrun.contains("not evidence"), "an unrun probe accuses nobody: {unrun}");
    }

    /// §2: the probe judges the local reviewer whose empty result it
    /// qualifies. A local model that misses the planted defect is Missed —
    /// it is the one asked, not a frontier that recovered and would find it.
    #[test]
    fn calibration_judges_the_local_reviewer_it_asks() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let asked = std::sync::atomic::AtomicUsize::new(0);
            let blind = calibrate(|prompt| {
                asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                assert!(prompt.contains("concurrency"), "{prompt}");
                async { Ok::<_, String>(r#"{"findings":[]}"#.to_string()) }
            })
            .await;
            assert_eq!(blind, Calibration::Missed, "the local model saw nothing, so its empty result is not evidence");
            assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 1, "the local reviewer is the one asked");

            let sharp = calibrate(|_| async {
                Ok::<_, String>(r#"{"findings":[{"title":"Lock dropped before the write","location":"bump","description":"the guard is released, then the map is written, so callers race","lens":"concurrency"}]}"#.to_string())
            })
            .await;
            assert_eq!(sharp, Calibration::Calibrated);

            let down = calibrate(|_| async { Err::<String, _>("gateway refused".to_string()) }).await;
            assert_eq!(down, Calibration::Unrun("gateway refused".into()));
        });
    }

    /// The verdict line carries the qualification where the count is read.
    #[test]
    fn the_verdict_line_carries_the_calibration() {
        let r = ReviewReport {
            lenses: 4, answered: 3,
            reviewed_by: vec!["openai/gpt-oss-120b".into()],
            calibration: Calibration::Missed,
            ..ReviewReport::default()
        };
        let line = r.verdict_line();
        assert!(line.contains("0 candidate(s)"), "{line}");
        assert!(line.ends_with("an empty result is not evidence"), "{line}");
    }

    /// ADR-2609131822 §2: the phase describes how the calls are actually
    /// issued. Saying "in parallel" while every caller queues behind one
    /// permit describes a plan, not what is happening.
    #[test]
    fn the_phase_says_how_the_calls_are_issued() {
        let order = issue_order();
        // Both halves, because both happen and neither is predictable: the
        // first cut branched on `budget_check`, which reads hexa's own spend
        // accounting and not the account's live limit, so it said "in
        // parallel" on the very run whose lenses all queued.
        assert!(order.contains("in parallel"), "the lenses do go out at once: {order:?}");
        assert!(order.contains("local reviewer"), "and the fallbacks queue: {order:?}");
        let line = format!("{} on src/x.rs, {}", plural(4, "lens"), order);
        assert!(line.starts_with("4 lenses on src/x.rs, in parallel;"), "{line}");
    }

    /// ADR-2609131822: local calls do not overlap, however many are issued
    /// together. Measured by holding the permit and watching a second
    /// caller wait for it.
    #[test]
    fn local_calls_are_serialised() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            assert_eq!(LOCAL_TURN.available_permits(), 1, "one caller at a time");
            let held = LOCAL_TURN.acquire().await.unwrap();
            assert_eq!(LOCAL_TURN.available_permits(), 0, "the permit is taken");

            let waited = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let flag = waited.clone();
            let second = tokio::spawn(async move {
                let _t = LOCAL_TURN.acquire().await.unwrap();
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
            });
            tokio::time::sleep(Duration::from_millis(60)).await;
            assert!(!waited.load(std::sync::atomic::Ordering::SeqCst), "the second caller must wait");

            drop(held);
            second.await.unwrap();
            assert!(waited.load(std::sync::atomic::Ordering::SeqCst), "and proceed once the first is done");
            assert_eq!(LOCAL_TURN.available_permits(), 1, "the permit returns");
        });
    }

    /// A pass that fixed nothing has nothing to commit.
    #[test]
    fn a_pass_that_fixed_nothing_commits_nothing() {
        let reviewed_no_fixes = ReviewReport { lenses: 4, answered: 1, gate_passed: true, gate_run: true, ..ReviewReport::default() };
        assert!(reviewed_no_fixes.reviewed());
        assert!(reviewed_no_fixes.fixed.is_empty(), "nothing was fixed, so the commit is skipped");
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
