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

#[derive(Debug, Clone, Deserialize)]
pub struct Finding {
    pub title: String,
    #[serde(default)]
    pub location: String,
    pub description: String,
    #[serde(default)]
    pub lens: String,
}

#[derive(Deserialize)]
struct FindingsEnvelope {
    findings: Vec<Finding>,
}

#[derive(Deserialize)]
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
    pub notes: Vec<String>,
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

/// Default attempts for `claude_run` call sites that don't take an explicit retry
/// count from the caller (e.g. `run_review`, whose CLI surface isn't parametrized).
const DEFAULT_RETRIES: u32 = 3;

/// Retry-wrapped [`claude_run`]: on timeout or spawn error, retry up to `attempts`
/// times (clamped 1-6), feeding the prior failure back into the next attempt's prompt
/// for context. No sleep/backoff — failures here are inference-latency timeouts on a
/// single long call, not rate limits, so immediate retry is the right shape (mirrors
/// the bounded-attempt loop in direct_exec.rs rather than time-based backoff).
/// The phases of a cooperative build, in the order they run.
///
/// `run_build` used to be silent. Four phases, up to three retries each, a
/// build phase with a timeout four times the others', and 566 lines that
/// printed nothing at all. An operator watching `hexa build` saw two lines of
/// header and then nothing for as long as it took — with no way to tell a
/// working run from a hung one. That is not a cosmetic gap: the only available
/// response to silence is to kill the run and lose everything it has spent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Diverge,
    RedTeam,
    Synthesize,
    Build,
    Gate,
    Commit,
}

impl Phase {
    /// Every phase, in order. The list is the contract the coverage test holds.
    pub fn all() -> &'static [Phase] {
        &[Phase::Diverge, Phase::RedTeam, Phase::Synthesize, Phase::Build, Phase::Gate, Phase::Commit]
    }

    /// What this phase is doing, in words an operator does not have to decode.
    pub fn label(&self) -> &'static str {
        match self {
            Phase::Diverge => "diverge — proposing designs",
            Phase::RedTeam => "red-team — attacking each design",
            Phase::Synthesize => "synthesize — one spec from the survivors",
            Phase::Build => "build — writing code until the gate passes",
            Phase::Gate => "gate — running the command that must exit 0",
            Phase::Commit => "commit — recording what passed",
        }
    }

    /// Its position, counting from one, so "3/6" means something.
    pub fn ordinal(&self) -> usize {
        Phase::all().iter().position(|p| p == self).unwrap_or(0) + 1
    }
}

/// The phases of an adversarial pass, in the order they run.
///
/// `hexa harden` was silent for the same reason `hexa build` was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewPhase {
    Hunt,
    Verify,
    Fix,
}

impl ReviewPhase {
    pub fn all() -> &'static [ReviewPhase] {
        &[ReviewPhase::Hunt, ReviewPhase::Verify, ReviewPhase::Fix]
    }

    pub fn label(&self) -> &'static str {
        match self {
            ReviewPhase::Hunt => "hunt — looking for bugs, one lens at a time",
            ReviewPhase::Verify => "verify — trying to refute each finding",
            ReviewPhase::Fix => "fix — repairing what survived, under the gate",
        }
    }

    pub fn ordinal(&self) -> usize {
        ReviewPhase::all().iter().position(|p| p == self).unwrap_or(0) + 1
    }
}

/// Announce a review phase.
fn announce_review(phase: ReviewPhase, detail: &str, started: std::time::Instant) {
    let secs = started.elapsed().as_secs();
    let when = if secs >= 60 { format!("{}m{:02}s", secs / 60, secs % 60) } else { format!("{secs}s") };
    let tail = if detail.is_empty() { String::new() } else { format!(" · {detail}") };
    tracing::info!(
        "[{}/{}] {}{tail} · {when} elapsed",
        phase.ordinal(),
        ReviewPhase::all().len(),
        phase.label()
    );
}

/// One progress line: which phase, how far in, how long so far.
fn phase_line(phase: Phase, detail: &str, elapsed: std::time::Duration) -> String {
    let secs = elapsed.as_secs();
    let when = if secs >= 60 { format!("{}m{:02}s", secs / 60, secs % 60) } else { format!("{secs}s") };
    let tail = if detail.is_empty() { String::new() } else { format!(" · {detail}") };
    format!("[{}/{}] {}{tail} · {when} elapsed", phase.ordinal(), Phase::all().len(), phase.label())
}

/// Announce a phase to the operator.
fn announce(phase: Phase, detail: &str, started: std::time::Instant) {
    tracing::info!("{}", phase_line(phase, detail, started.elapsed()));
}

/// Run `f`, logging a heartbeat every 30 seconds until it finishes.
///
/// This is the half that answers "is it hung". A phase line at the start tells
/// you what began; only a heartbeat tells you it is still going.
async fn with_heartbeat<T>(what: &str, f: impl std::future::Future<Output = T>) -> T {
    let label = what.to_string();
    let beat = tokio::spawn(async move {
        let start = std::time::Instant::now();
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(30));
        tick.tick().await; // the first tick is immediate; skip it
        loop {
            tick.tick().await;
            let s = start.elapsed().as_secs();
            tracing::info!("      … {label} still running · {}m{:02}s", s / 60, s % 60);
        }
    });
    let out = f.await;
    beat.abort();
    out
}

async fn claude_run_retry(
    prompt: &str,
    cwd: &Path,
    timeout_secs: u64,
    attempts: u32,
) -> Result<String, String> {
    let attempts = attempts.clamp(1, 6);
    let mut last_err = String::new();
    for attempt in 1..=attempts {
        if attempt > 1 {
            // A silent retry is indistinguishable from a hang that lasts three
            // times as long.
            tracing::info!("      retry {attempt}/{attempts} — previous attempt failed: {last_err}");
        }
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
    let mut report = ReviewReport::default();
    let started = std::time::Instant::now();
    // What the operator had already changed. Anything dirty after the pass and
    // not in this set is the pass's own work, tests included.
    let dirty_before = dirty_paths(repo_root).await;

    // ── Phase 1: hunt (parallel lenses) ──────────────────────────────────────
    announce_review(ReviewPhase::Hunt, &format!("{} lens(es), in parallel", LENSES.len()), started);
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
        hunts.push(tokio::spawn(async move { claude_run_retry(&prompt, &root, 600, DEFAULT_RETRIES).await }));
    }
    for h in with_heartbeat("hunt", futures_all(hunts)).await {
        if let Ok(Ok(out)) = h {
            if let Some(js) = extract_json(&out) {
                if let Ok(env) = serde_json::from_str::<FindingsEnvelope>(js) {
                    report.candidate += env.findings.len();
                    report.confirmed.extend(env.findings); // staged; pruned by verify below
                }
            }
        }
    }
    let candidates = std::mem::take(&mut report.confirmed);

    // ── Phase 2: skeptical verify (parallel, default-refute) ─────────────────
    announce_review(
        ReviewPhase::Verify,
        &format!("{} candidate(s) to refute", report.candidate),
        started,
    );
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
        checks.push((f, tokio::spawn(async move { claude_run_retry(&prompt, &root, 600, DEFAULT_RETRIES).await })));
    }
    for (f, c) in checks {
        if let Ok(Ok(out)) = c.await {
            if let Some(js) = extract_json(&out) {
                if let Ok(v) = serde_json::from_str::<VerdictEnvelope>(js) {
                    if v.is_real {
                        report.confirmed.push(f);
                    } else {
                        report.notes.push(format!("refuted: {} — {}", f.title, v.reasoning));
                    }
                }
            }
        }
    }

    // ── Phase 3: fix-loop (sequential, each fix gated) ───────────────────────
    announce_review(
        ReviewPhase::Fix,
        &format!("{} confirmed finding(s), one gated fix each", report.confirmed.len()),
        started,
    );
    for f in &report.confirmed {
        tracing::info!("      fixing: {} [{}] {}", f.title, f.lens, f.location);
        let prompt = format!(
            "Fix this CONFIRMED bug in the code under `{target}`, then add a regression test that \
             fails without the fix. Make a minimal, correct change. Bug:\ntitle: {title}\nlocation: {loc}\ndescription: {desc}",
            target = target, title = f.title, loc = f.location, desc = f.description
        );
        if claude_run_retry(&prompt, repo_root, 900, DEFAULT_RETRIES).await.is_ok() {
            let (passed, _) = crate::direct_exec::run_evidence(gate, repo_root).await;
            if passed {
                report.fixed.push(f.title.clone());
            } else {
                report.notes.push(format!("fix for '{}' did not pass the gate", f.title));
            }
        }
    }

    // ── Final gate ───────────────────────────────────────────────────────────
    let (passed, _) = crate::direct_exec::run_evidence(gate, repo_root).await;
    report.gate_passed = passed;
    if passed {
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
            &format!("fix: adversarial review pass on {target}"),
            "hexa-harden",
            &mut report.notes,
        )
        .await;
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
/// Await every spawned task, in order, and hand back the results.
async fn futures_all<T>(tasks: Vec<tokio::task::JoinHandle<T>>) -> Vec<Result<T, tokio::task::JoinError>> {
    let mut out = Vec::with_capacity(tasks.len());
    for t in tasks {
        out.push(t.await);
    }
    out
}

pub async fn run_build(
    challenge: &str,
    target: &str,
    gate: &str,
    n_designs: usize,
    repo_root: &Path,
    timeout_secs: u64,
    retries: u32,
) -> BuildReport {
    let mut report = BuildReport::default();
    let started = std::time::Instant::now();
    let dirty_before = dirty_paths(repo_root).await;
    let n = n_designs.clamp(2, DESIGN_PRIORITIES.len());

    // ── Phase 1: diverge — N designs from competing priorities ───────────────
    announce(Phase::Diverge, &format!("{n} design(s), in parallel"), started);
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
    for t in with_heartbeat("diverge", futures_all(tasks)).await {
        if let Ok(Ok(d)) = t {
            designs.push(d);
        }
    }
    report.designs = designs.len();
    if designs.is_empty() {
        report.notes.push("no designs produced".into());
        return report;
    }

    // ── Phase 2: red-team each design (adversarial) ──────────────────────────
    announce(Phase::RedTeam, &format!("{} design(s) to attack", designs.len()), started);
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
    for t in with_heartbeat("red-team", futures_all(ctasks)).await {
        if let Ok(Ok(c)) = t {
            critiques.push(c);
        }
    }
    report.critiques = critiques.len();

    // ── Phase 3: synthesize one build spec ───────────────────────────────────
    announce(Phase::Synthesize, &format!("{} critique(s) to fold in", critiques.len()), started);
    let designs_block = designs
        .iter()
        .enumerate()
        .map(|(i, d)| format!("--- DESIGN {i} ---\n{d}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let critiques_block = critiques.join("\n\n--- next critique ---\n\n");
    let spec = match with_heartbeat("synthesize", claude_run_retry(
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
    ))
    .await
    {
        Ok(s) => s,
        Err(e) => {
            report.notes.push(format!("synthesize failed: {e}"));
            return report;
        }
    };
    report.spec_chars = spec.len();

    // ── Phase 4: build to the gate ───────────────────────────────────────────
    // This is the long one: its timeout is four times the others', so say so
    // rather than letting the operator guess how long "too long" is.
    announce(
        Phase::Build,
        &format!("spec is {} chars · up to {}s per attempt", spec.len(), timeout_secs.saturating_mul(4)),
        started,
    );
    let build_prompt = format!(
        "Implement the following spec as code under `{target}`. Write the full implementation AND a \
         comprehensive test suite per the spec's test plan. Then run the gate command `{gate}` and ITERATE \
         — fix compile errors and failing tests — until the gate exits 0. Do not stop until the gate passes.\n\n\
         CHALLENGE:\n{challenge}\n\nSPEC:\n{spec}"
    );
    if let Err(e) = with_heartbeat(
        "build",
        claude_run_retry(&build_prompt, repo_root, timeout_secs.saturating_mul(4), retries),
    )
    .await
    {
        report.notes.push(format!("build agent error: {e}"));
    }

    announce(Phase::Gate, gate, started);
    let (ok, _) = with_heartbeat("gate", crate::direct_exec::run_evidence(gate, repo_root)).await;
    report.build_ok = ok;
    tracing::info!("      gate {}", if ok { "PASSED" } else { "FAILED" });
    if ok {
        announce(Phase::Commit, "", started);
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
mod tests {
    use super::{extract_json, retry_prompt, Phase};

    /// The source of this file, read at compile time, so the coverage test
    /// below cannot drift from the code it is about.
    const SOURCE: &str = include_str!("adversarial.rs");

    #[test]
    fn every_phase_has_a_label_and_a_place_in_the_run() {
        let all = Phase::all();
        assert!(all.len() >= 4, "only {} phases; the list is broken", all.len());
        for (i, p) in all.iter().enumerate() {
            assert!(!p.label().is_empty(), "{p:?} has no label");
            assert_eq!(p.ordinal(), i + 1, "{p:?} is numbered wrong");
        }
        let mut labels: Vec<&str> = all.iter().map(|p| p.label()).collect();
        labels.sort();
        let before = labels.len();
        labels.dedup();
        assert_eq!(labels.len(), before, "two phases share a label");
    }

    #[test]
    fn a_progress_line_says_where_we_are_and_how_long_it_has_been() {
        let line = super::phase_line(Phase::Build, "spec is 900 chars", std::time::Duration::from_secs(185));
        assert!(line.contains("[4/6]"), "no position: {line}");
        assert!(line.contains("build"), "no phase: {line}");
        assert!(line.contains("spec is 900 chars"), "no detail: {line}");
        assert!(line.contains("3m05s"), "no elapsed time: {line}");
    }

    #[test]
    fn a_short_run_reads_in_seconds_not_zero_minutes() {
        let line = super::phase_line(Phase::Diverge, "", std::time::Duration::from_secs(7));
        assert!(line.contains("7s elapsed"), "{line}");
        assert!(!line.contains("0m"), "{line}");
    }

    /// A phase that is never announced is a phase the operator cannot see.
    ///
    /// This is the whole defect, held shut: `run_build` ran four silent phases
    /// for as long as it took, and the only signal available to a watching
    /// operator was the absence of output.
    #[test]
    fn every_phase_is_actually_announced_in_the_run() {
        let unannounced: Vec<String> = Phase::all()
            .iter()
            .filter(|p| {
                let packed: String = SOURCE.chars().filter(|c| !c.is_whitespace()).collect();
                !packed.contains(&format!("announce(Phase::{p:?}"))
            })
            .map(|p| format!("{p:?}"))
            .collect();
        assert!(
            unannounced.is_empty(),
            "{} phase(s) run without telling anyone:\n  {}",
            unannounced.len(),
            unannounced.join("\n  ")
        );
    }

    /// And the long calls are wrapped in a heartbeat, which is the half that
    /// distinguishes "still working" from "hung".
    ///
    /// Whitespace is stripped before the search: rustfmt wraps a long call
    /// across lines, and a test that fails on formatting teaches people to
    /// format around the test.
    #[test]
    fn the_long_calls_emit_a_heartbeat() {
        let packed: String = SOURCE.chars().filter(|c| !c.is_whitespace()).collect();
        for what in ["diverge", "red-team", "synthesize", "build", "gate"] {
            assert!(
                packed.contains(&format!("with_heartbeat(\"{what}\"")),
                "`{what}` runs with no heartbeat; a long silence there is indistinguishable from a hang"
            );
        }
    }

    #[test]
    fn every_review_phase_has_a_label_and_is_announced() {
        let packed: String = SOURCE.chars().filter(|c| !c.is_whitespace()).collect();
        let all = super::ReviewPhase::all();
        assert!(all.len() >= 3, "only {} review phases; the list is broken", all.len());
        let silent: Vec<String> = all
            .iter()
            .enumerate()
            .filter_map(|(i, p)| {
                assert!(!p.label().is_empty(), "{p:?} has no label");
                assert_eq!(p.ordinal(), i + 1, "{p:?} is numbered wrong");
                (!packed.contains(&format!("announce_review(ReviewPhase::{p:?}")))
                    .then(|| format!("{p:?}"))
            })
            .collect();
        assert!(
            silent.is_empty(),
            "{} review phase(s) run without telling anyone:\n  {}",
            silent.len(),
            silent.join("\n  ")
        );
    }

    /// A retry that says nothing looks exactly like one call taking three times
    /// as long.
    #[test]
    fn a_retry_announces_itself() {
        assert!(SOURCE.contains("retry {attempt}/{attempts}"), "retries are silent");
    }


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
