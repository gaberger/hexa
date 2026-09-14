//! `hexa swarm` — fan out by file boundary, or refuse (ADR-2609140927).
//!
//! CLAUDE.md has carried "parallelize by file boundary, serialize by file
//! overlap" since the adversarial review that produced it. Nothing in the
//! binary implemented it, checked it, or could compute it.
//!
//! The rule matters because of how it fails. Two agents editing disjoint files
//! finish and merge cleanly. Two agents editing the same file both pass their
//! own gate, and then one overwrites the other — and the gate that proved each
//! of them was measuring a state that no longer exists. Nothing errors. The
//! evidence outlives the thing it was evidence for.
//!
//! It is also mechanically checkable, which is why leaving it as prose was the
//! wrong choice. A slice is a set of files. Two slices intersect or they do
//! not.
//!
//! The planning half of this module is pure: slices in, a plan or a refusal
//! out. That is deliberate. The decision this ADR makes is testable without
//! spending a token, and `swarm_refuses_overlap.rs` spends none.

use std::collections::BTreeSet;
use std::process::Command;

use clap::Args;
use colored::Colorize;

#[derive(Args, Debug)]
pub struct SwarmArgs {
    /// What every worker is asked to do, in plain language.
    pub task: String,
    /// A slice: a path, a directory, or a glob. Repeat for each worker.
    #[arg(long = "over", value_name = "SLICE", required = true, num_args = 1..)]
    pub over: Vec<String>,
    /// Ground-truth gate: a shell command that must exit 0 in each worker.
    #[arg(long)]
    pub gate: String,
    /// Show the slices and the overlap verdict, then stop.
    #[arg(long)]
    pub plan: bool,
}

// ── slices ──────────────────────────────────────────────────────────────────

/// One worker's share of the work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slice {
    /// What the operator typed.
    pub spec: String,
    /// The repository files it expands to, sorted.
    pub files: Vec<String>,
}

/// Why a swarm will not start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// No slices, so no workers, so nothing to report but success.
    NoSlices,
    /// A slice that matches nothing. Reporting "0 workers, all passed" for it
    /// is the vacuous-evidence failure with a new face.
    EmptySlice { spec: String },
    /// The rule this command exists for.
    Overlap { a: String, b: String, shared: Vec<String> },
}

impl Refusal {
    fn message(&self) -> String {
        match self {
            Refusal::NoSlices => "no slices given; there is nothing to fan out over".to_string(),
            Refusal::EmptySlice { spec } => {
                format!("slice `{spec}` matches no tracked file; a worker with no files proves nothing")
            }
            Refusal::Overlap { a, b, shared } => {
                let shown: Vec<&str> = shared.iter().take(5).map(|s| s.as_str()).collect();
                let more = shared.len().saturating_sub(shown.len());
                let tail = if more > 0 { format!(" (and {more} more)") } else { String::new() };
                format!(
                    "slices `{a}` and `{b}` share {} file(s): {}{tail}\n  \
                     Two workers on one file each pass their own gate, and then one \
                     overwrites the other. Re-slice so they do not touch.",
                    shared.len(),
                    shown.join(", ")
                )
            }
        }
    }
}

/// Does `path` fall inside `spec`?
///
/// Three shapes, and no shell. Glob expansion happens here rather than in the
/// caller's shell because a glob that expands differently on two machines
/// produces a different overlap verdict, and the verdict is the whole point.
///
/// - `src/lib.rs` — that exact file.
/// - `src/commands` — that file, or anything beneath it.
/// - `src/**/*.rs` — `*` matches within a path segment, `**` across segments.
fn matches(spec: &str, path: &str) -> bool {
    let spec = spec.trim_end_matches('/');
    if !spec.contains('*') {
        return path == spec || path.starts_with(&format!("{spec}/"));
    }
    glob_match(
        &spec.split('/').collect::<Vec<_>>(),
        &path.split('/').collect::<Vec<_>>(),
    )
}

/// Segment-wise glob. `**` consumes any number of segments, including none.
fn glob_match(pat: &[&str], path: &[&str]) -> bool {
    match (pat.first(), path.first()) {
        (None, None) => true,
        (None, Some(_)) => false,
        (Some(&"**"), _) => {
            // Zero segments, or one segment then try again.
            glob_match(&pat[1..], path) || (!path.is_empty() && glob_match(pat, &path[1..]))
        }
        (Some(_), None) => false,
        (Some(p), Some(s)) => segment_match(p, s) && glob_match(&pat[1..], &path[1..]),
    }
}

/// One path segment against one pattern segment, where `*` matches any run of
/// characters that are not `/`.
fn segment_match(pat: &str, seg: &str) -> bool {
    let parts: Vec<&str> = pat.split('*').collect();
    if parts.len() == 1 {
        return pat == seg;
    }
    let mut rest = seg;
    // The pattern's leading literal must be a prefix.
    if !parts[0].is_empty() {
        if !rest.starts_with(parts[0]) {
            return false;
        }
        rest = &rest[parts[0].len()..];
    }
    // The trailing literal must be a suffix, and must not overlap the prefix.
    let last = parts[parts.len() - 1];
    if !last.is_empty() {
        if !rest.ends_with(last) || rest.len() < last.len() {
            return false;
        }
        rest = &rest[..rest.len() - last.len()];
    }
    // Every middle literal must appear, in order.
    for mid in &parts[1..parts.len() - 1] {
        if mid.is_empty() {
            continue;
        }
        match rest.find(mid) {
            Some(i) => rest = &rest[i + mid.len()..],
            None => return false,
        }
    }
    true
}

/// Expand one slice over a file list.
fn expand(spec: &str, repo_files: &[String]) -> Vec<String> {
    let mut out: Vec<String> =
        repo_files.iter().filter(|f| matches(spec, f)).cloned().collect();
    out.sort();
    out.dedup();
    out
}

/// The plan, or the reason there is not one.
///
/// Every check here runs before a worktree exists. A swarm that refuses after
/// creating three worktrees has already done the damage it was refusing.
fn plan(specs: &[String], repo_files: &[String]) -> Result<Vec<Slice>, Refusal> {
    if specs.is_empty() {
        return Err(Refusal::NoSlices);
    }
    let mut slices = Vec::new();
    for spec in specs {
        let files = expand(spec, repo_files);
        if files.is_empty() {
            return Err(Refusal::EmptySlice { spec: spec.clone() });
        }
        slices.push(Slice { spec: spec.clone(), files });
    }
    for i in 0..slices.len() {
        for j in (i + 1)..slices.len() {
            let a: BTreeSet<&String> = slices[i].files.iter().collect();
            let shared: Vec<String> =
                slices[j].files.iter().filter(|f| a.contains(f)).cloned().collect();
            if !shared.is_empty() {
                return Err(Refusal::Overlap {
                    a: slices[i].spec.clone(),
                    b: slices[j].spec.clone(),
                    shared,
                });
            }
        }
    }
    Ok(slices)
}

// ── the report ──────────────────────────────────────────────────────────────

/// What happened to one worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The gate exited 0 inside the worker's worktree.
    ///
    /// `DirectResult` reports the commit, not the branch it landed on, so the
    /// branch is looked up from the commit. Reporting "(unnamed)" for every
    /// passing worker would make decision 5 — the swarm reports branches —
    /// true on paper and useless in the terminal.
    Passed { commit: String, branch: String },
    /// The worker ran and the gate did not exit 0.
    Failed { reason: String },
    /// The worker never ran. Distinct from failing, and never omitted.
    NotRun { reason: String },
}

/// One row of the report.
#[derive(Debug, Clone)]
pub struct WorkerResult {
    pub spec: String,
    pub files: usize,
    pub verdict: Verdict,
}

/// The aggregated report.
///
/// A slice missing from this is a defect in the report, not a quiet success:
/// the operator will assume the slices it does not mention were fine.
/// `every_slice_appears_in_the_report` holds that.
fn report(results: &[WorkerResult]) -> String {
    let mut o = String::new();
    let passed = results.iter().filter(|r| matches!(r.verdict, Verdict::Passed { .. })).count();
    o.push_str(&format!(
        "  {} slice(s) · {} passed · {} did not\n\n",
        results.len(),
        passed,
        results.len() - passed
    ));
    for r in results {
        let (mark, detail) = match &r.verdict {
            Verdict::Passed { commit, branch } => ("✓", format!("{branch} @ {commit}")),
            Verdict::Failed { reason } => ("✗", format!("gate failed — {reason}")),
            Verdict::NotRun { reason } => ("–", format!("did not run — {reason}")),
        };
        o.push_str(&format!("    {mark} {:<32} {} file(s)  {}\n", r.spec, r.files, detail));
    }
    o.push_str("\n  Nothing was merged. Review the branches, then merge them yourself.\n");
    o
}

// ── the impure edge ─────────────────────────────────────────────────────────

/// Every file git tracks, repo-relative.
fn tracked_files() -> anyhow::Result<Vec<String>> {
    let out = Command::new("git").args(["ls-files"]).output()?;
    if !out.status.success() {
        anyhow::bail!("not inside a git repository, or git failed");
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect())
}

/// Which `hexa/auto/*` branch holds this commit.
///
/// Asked of the commit rather than recorded at fork time, because the workers
/// run concurrently and a branch list sampled around one of them would pick up
/// the others.
fn branch_for(commit: &str) -> String {
    let out = Command::new("git").args(["branch", "--contains", commit, "--format=%(refname:short)"]).output();
    let Ok(o) = out else { return "(unknown)".into() };
    String::from_utf8_lossy(&o.stdout)
        .lines()
        .map(str::trim)
        .find(|b| b.starts_with("hexa/auto/"))
        .unwrap_or("(unknown)")
        .to_string()
}

/// Read a worker's verdict out of a `DirectResult`.
///
/// Pure, because the interesting part is a judgement and judgements need
/// tests. A live two-slice swarm reported "2 passed" while leaving one branch:
/// the first worker changed nothing, `cargo check` passed on the unchanged
/// tree, and `evidence_passed` came back true. The run had already been marked
/// failed and its worktree removed, and the swarm still called it a pass.
///
/// A gate passing is not proof a worker did anything. `cargo check` passes on
/// an empty diff. So a pass requires both: the gate exited 0 **and** something
/// landed that an operator can go and read.
fn verdict_from(r: &serde_json::Value) -> Verdict {
    let passed = r.get("evidence_passed").and_then(|v| v.as_bool()).unwrap_or(false);
    let commit = r.get("committed").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();

    if passed && !commit.is_empty() {
        return Verdict::Passed { branch: branch_for(&commit), commit };
    }
    if passed {
        return Verdict::Failed {
            reason: "the gate passed but nothing was committed; the gate does not test this slice"
                .into(),
        };
    }
    let reason = r
        .get("error")
        .and_then(|v| v.as_str())
        .filter(|e| !e.trim().is_empty())
        .unwrap_or("the gate did not exit 0")
        .to_string();
    Verdict::Failed { reason }
}

/// Run one slice's worker: the agent loop, isolated, gated.
async fn run_worker(task: &str, slice: &Slice, gate: &str) -> Verdict {
    // The anchor is the first file of the slice. The loop may patch any file in
    // the repository, so the instruction names the whole slice; the anchor is
    // where it starts reading.
    let listing = slice.files.join("\n");
    let instruction = format!(
        "{task}\n\nWork only inside this slice. These are the only files you may change:\n{listing}"
    );
    let body = serde_json::json!({
        "instruction": instruction,
        "file": slice.files[0],
        "evidence": gate,
        "isolate": true,
    });
    let Ok(t) = serde_json::from_value::<hexa_exec::direct_exec::DirectTask>(body) else {
        return Verdict::NotRun { reason: "could not build the task".into() };
    };
    let r = serde_json::to_value(hexa_exec::direct_exec::execute_direct(t).await)
        .unwrap_or(serde_json::Value::Null);
    verdict_from(&r)
}

pub async fn run(args: SwarmArgs) -> anyhow::Result<()> {
    println!("{} {}", "⬡ swarm:".cyan().bold(), args.task);

    let files = tracked_files()?;
    let slices = match plan(&args.over, &files) {
        Ok(s) => s,
        Err(r) => {
            println!("  {} {}", "✗ refused".red().bold(), r.message());
            // Nothing was created. The refusal is the whole point of the verb.
            anyhow::bail!("swarm refused: slices are not disjoint");
        }
    };

    println!("  {} slice(s), no overlap", slices.len());
    for s in &slices {
        println!("    {:<32} {} file(s)", s.spec, s.files.len());
    }

    if slices.len() == 1 {
        println!(
            "  {} one slice is one task. `hexa do run` does this with less machinery.",
            "note:".yellow()
        );
    }
    if args.plan {
        return Ok(());
    }

    println!("\n  {} {} worker(s), each isolated\n", "→".green(), slices.len());
    // Concurrent, not sequential. A swarm that runs its workers one after
    // another is a loop with extra steps, and the whole reason the overlap
    // check exists is that the workers overlap in time.
    //
    // Each future returns a row, so a worker that fails cannot remove its slice
    // from the results — decision 4 is structural here, not a convention.
    let (task, gate) = (args.task.as_str(), args.gate.as_str());
    let futures = slices.iter().map(|slice| async move {
        let verdict = run_worker(task, slice, gate).await;
        WorkerResult { spec: slice.spec.clone(), files: slice.files.len(), verdict }
    });
    let results: Vec<WorkerResult> = futures_util::future::join_all(futures).await;
    print!("{}", report(&results));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files() -> Vec<String> {
        ["a/one.rs", "a/two.rs", "a/deep/three.rs", "b/one.rs", "b/note.md", "c/four.rs"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn a_directory_slice_takes_everything_beneath_it() {
        assert_eq!(expand("a", &files()), vec!["a/deep/three.rs", "a/one.rs", "a/two.rs"]);
    }

    #[test]
    fn a_directory_never_matches_a_sibling_with_the_same_prefix() {
        let f = vec!["ab/x.rs".to_string(), "a/x.rs".to_string()];
        assert_eq!(expand("a", &f), vec!["a/x.rs"]);
    }

    #[test]
    fn an_exact_file_matches_only_itself() {
        assert_eq!(expand("b/note.md", &files()), vec!["b/note.md"]);
    }

    #[test]
    fn a_star_stays_inside_one_segment() {
        assert!(matches("a/*.rs", "a/one.rs"));
        assert!(!matches("a/*.rs", "a/deep/three.rs"));
    }

    #[test]
    fn a_double_star_crosses_segments_and_may_match_none() {
        assert!(matches("a/**/*.rs", "a/deep/three.rs"));
        assert!(matches("a/**/*.rs", "a/one.rs"), "** must also match zero segments");
        assert!(!matches("a/**/*.md", "a/one.rs"));
    }

    #[test]
    fn no_slices_is_a_refusal_not_an_empty_success() {
        assert_eq!(plan(&[], &files()), Err(Refusal::NoSlices));
    }

    #[test]
    fn a_slice_matching_nothing_is_a_refusal() {
        let got = plan(&["nowhere".to_string()], &files());
        assert_eq!(got, Err(Refusal::EmptySlice { spec: "nowhere".into() }));
    }

    #[test]
    fn disjoint_slices_plan_cleanly() {
        let got = plan(&["a".to_string(), "b".to_string()], &files()).expect("a plan");
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].files.len(), 3);
        assert_eq!(got[1].files.len(), 2);
    }

    #[test]
    fn a_slice_nested_inside_another_is_refused() {
        // The case the rule exists for: `a` and `a/deep` are not disjoint, and
        // it is not obvious from the two strings alone.
        let got = plan(&["a".to_string(), "a/deep".to_string()], &files());
        match got {
            Err(Refusal::Overlap { a, b, shared }) => {
                assert_eq!((a.as_str(), b.as_str()), ("a", "a/deep"));
                assert_eq!(shared, vec!["a/deep/three.rs"]);
            }
            other => panic!("expected an overlap refusal, got {other:?}"),
        }
    }

    #[test]
    fn two_globs_that_reach_the_same_file_are_refused() {
        let got = plan(&["**/*.rs".to_string(), "b".to_string()], &files());
        assert!(matches!(got, Err(Refusal::Overlap { .. })), "got {got:?}");
    }

    #[test]
    fn the_refusal_names_the_shared_files() {
        let got = plan(&["a".to_string(), "a/deep".to_string()], &files()).unwrap_err();
        let m = got.message();
        assert!(m.contains("a/deep/three.rs"), "the operator must see which file: {m}");
        assert!(m.contains("overwrites"), "and why it matters: {m}");
    }

    #[test]
    fn a_gate_that_passes_on_an_unchanged_tree_is_not_a_pass() {
        // The live defect: a two-slice swarm reported "2 passed" and left one
        // branch. `cargo check` passes on an empty diff, so `evidence_passed`
        // was true for a worker whose run had already been abandoned.
        let r = serde_json::json!({ "evidence_passed": true, "committed": null });
        match verdict_from(&r) {
            Verdict::Failed { reason } => {
                assert!(reason.contains("nothing was committed"), "{reason}");
            }
            other => panic!("a worker that committed nothing must not pass: {other:?}"),
        }
    }

    #[test]
    fn an_empty_commit_string_is_the_same_as_none() {
        let r = serde_json::json!({ "evidence_passed": true, "committed": "   " });
        assert!(matches!(verdict_from(&r), Verdict::Failed { .. }));
    }

    #[test]
    fn a_failed_gate_carries_its_own_reason() {
        let r = serde_json::json!({ "evidence_passed": false, "error": "cargo check exited 101" });
        match verdict_from(&r) {
            Verdict::Failed { reason } => assert_eq!(reason, "cargo check exited 101"),
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    #[test]
    fn a_failure_with_no_error_still_says_something() {
        let r = serde_json::json!({ "evidence_passed": false });
        match verdict_from(&r) {
            Verdict::Failed { reason } => assert!(!reason.is_empty(), "an empty reason explains nothing"),
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    #[test]
    fn a_null_result_is_a_failure_not_a_pass() {
        assert!(matches!(verdict_from(&serde_json::Value::Null), Verdict::Failed { .. }));
    }

    #[test]
    fn every_slice_appears_in_the_report() {
        let results = vec![
            WorkerResult {
                spec: "a".into(),
                files: 3,
                verdict: Verdict::Passed { commit: "abc1234".into(), branch: "hexa/auto/x".into() },
            },
            WorkerResult { spec: "b".into(), files: 2, verdict: Verdict::Failed { reason: "boom".into() } },
            WorkerResult { spec: "c".into(), files: 1, verdict: Verdict::NotRun { reason: "skipped".into() } },
        ];
        let out = report(&results);
        for spec in ["a", "b", "c"] {
            assert!(out.contains(&format!(" {spec:<32}")), "slice `{spec}` is missing:\n{out}");
        }
        assert!(out.contains("1 passed · 2 did not"), "{out}");
        assert!(out.contains("Nothing was merged"), "the report must say nothing merged:\n{out}");
    }

    #[test]
    fn a_failed_worker_does_not_hide_the_others() {
        let results = vec![
            WorkerResult { spec: "a".into(), files: 1, verdict: Verdict::Failed { reason: "x".into() } },
            WorkerResult {
                spec: "b".into(),
                files: 1,
                verdict: Verdict::Passed { commit: "def5678".into(), branch: "hexa/auto/y".into() },
            },
        ];
        let out = report(&results);
        assert!(out.contains("gate failed"), "{out}");
        assert!(out.contains("hexa/auto/y"), "a passing slice after a failing one must still show:\n{out}");
    }
}
