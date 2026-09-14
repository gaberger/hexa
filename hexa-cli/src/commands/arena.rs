//! `hexa arena` — N implementations, judged by the gate (ADR-2609140926).
//!
//! `hexa build` proposes N designs, red-teams each, synthesizes one, and builds
//! to the gate. The divergence stops at the design boundary. Everything after
//! it — the part that produces the code the gate actually judges — happens
//! once, which is backwards relative to where the variance is. A design is a
//! paragraph; two competent designs for one challenge differ less than two
//! implementations of the same design.
//!
//! The arena moves the divergence past that boundary and lets the two gates
//! decide. "Best" is not a judgement call when a command exits 0 or does not,
//! and a graph property falls out as a number.
//!
//! What it deliberately will not do is assemble a composite from the best parts
//! of several entrants. The parts were each proven inside a whole that no
//! longer exists, so the composite is a fourth artefact no gate ever ran. That
//! is proof transferred from the thing that was measured to a thing that was
//! not — the mirror test in a third hat.

use std::path::{Path, PathBuf};
use std::process::Command;

use clap::Args;
use colored::Colorize;

#[derive(Args, Debug)]
pub struct ArenaArgs {
    /// The challenge: a plain-language description of the system to build.
    pub challenge: String,
    /// Directory to build the winner into (repo-relative). Must be empty or absent.
    #[arg(long)]
    pub target: String,
    /// Ground-truth gate: a shell command that must exit 0 inside each entrant.
    #[arg(long)]
    pub gate: String,
    /// How many implementations to build. Two is the smallest arena.
    #[arg(long, default_value_t = 3)]
    pub entrants: usize,
    /// Divergent designs inside each entrant's own build.
    #[arg(long, default_value_t = 3)]
    pub designs: usize,
    /// Per-call timeout in seconds, passed to each entrant's build.
    #[arg(long, default_value_t = 600)]
    pub timeout: u64,
    /// Max attempts per inference call inside each entrant's build.
    #[arg(long, default_value_t = 3)]
    pub retries: u32,
    /// Refuse and report the plan without building anything.
    #[arg(long)]
    pub plan: bool,
}

// ── judging ─────────────────────────────────────────────────────────────────

/// What became of one entrant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Held the gate, and the gate proved something.
    Survived { score: u32, grade: String, diff_lines: usize },
    /// Out. There is no partial credit and no repair pass: an entrant that
    /// needed help did not win.
    Eliminated { reason: String },
}

#[derive(Debug, Clone)]
pub struct Outcome {
    pub entrant: usize,
    pub branch: String,
    pub verdict: Verdict,
}

/// Why an arena will not start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// One entrant is `hexa build`. Zero is nothing.
    TooFewEntrants(usize),
    /// A target with content in it cannot receive a whole winner, and a
    /// half-overwritten target is the worst outcome available.
    TargetNotEmpty(String),
}

impl Refusal {
    fn message(&self) -> String {
        match self {
            Refusal::TooFewEntrants(n) => format!(
                "an arena needs at least 2 entrants; {n} given. One entrant is `hexa build`."
            ),
            Refusal::TargetNotEmpty(p) => format!(
                "target `{p}` is not empty. The winner is merged whole, so the target must be \
                 empty or absent — otherwise the result is a mixture no gate ever ran."
            ),
        }
    }
}

/// Everything that can be refused before an entrant exists.
fn check(entrants: usize, target: &Path) -> Result<(), Refusal> {
    if entrants < 2 {
        return Err(Refusal::TooFewEntrants(entrants));
    }
    if target.exists() {
        let empty = std::fs::read_dir(target).map(|mut d| d.next().is_none()).unwrap_or(false);
        if !empty {
            return Err(Refusal::TargetNotEmpty(target.display().to_string()));
        }
    }
    Ok(())
}

/// Judge one entrant from what its run produced.
///
/// A gate that exits 0 having run nothing is not evidence. `cargo test` on a
/// crate with no tests passes, and an arena that crowned that entrant would be
/// reporting a winner chosen by a coin toss. `tests_observed` is the same guard
/// `hexa do` applies to its own evidence.
fn judge(
    gate_ok: bool,
    gate_output: &str,
    score: u32,
    grade: &str,
    diff_lines: usize,
) -> Verdict {
    if !gate_ok {
        return Verdict::Eliminated { reason: "the gate did not exit 0".into() };
    }
    if hexa_exec::direct_exec::tests_observed(gate_output) == Some(0) {
        return Verdict::Eliminated {
            reason: "the gate exited 0 having run zero tests; that is not evidence".into(),
        };
    }
    if diff_lines == 0 {
        return Verdict::Eliminated {
            reason: "the gate passed on an empty diff; nothing was built".into(),
        };
    }
    Verdict::Survived { score, grade: grade.to_string(), diff_lines }
}

/// The winner's index in `outcomes`, or `None` when nobody survived.
///
/// Grade first, because that is the second gate. Smallest diff breaks a tie:
/// the change that solves the problem with the least added surface wins, which
/// is the rule hexa applies everywhere else. The entrant number breaks the rest,
/// so the same set of results always crowns the same entrant.
fn rank(outcomes: &[Outcome]) -> Option<usize> {
    outcomes
        .iter()
        .enumerate()
        .filter_map(|(i, o)| match &o.verdict {
            Verdict::Survived { score, diff_lines, .. } => Some((i, *score, *diff_lines, o.entrant)),
            Verdict::Eliminated { .. } => None,
        })
        .max_by(|a, b| {
            a.1.cmp(&b.1) // higher score wins
                .then_with(|| b.2.cmp(&a.2)) // then smaller diff
                .then_with(|| b.3.cmp(&a.3)) // then lower entrant number
        })
        .map(|(i, ..)| i)
}

/// The scoreboard. Every entrant appears, winner or not.
fn report(outcomes: &[Outcome], winner: Option<usize>) -> String {
    let survivors = outcomes
        .iter()
        .filter(|o| matches!(o.verdict, Verdict::Survived { .. }))
        .count();
    let mut o = String::new();
    o.push_str(&format!(
        "  {} entrant(s) · {} held the gate · {} eliminated\n\n",
        outcomes.len(),
        survivors,
        outcomes.len() - survivors
    ));
    for (i, out) in outcomes.iter().enumerate() {
        let crown = if Some(i) == winner { "★" } else { " " };
        match &out.verdict {
            Verdict::Survived { score, grade, diff_lines } => o.push_str(&format!(
                "  {crown} #{} {:<26} {} {}/100 · {} line(s) changed\n",
                out.entrant, out.branch, grade, score, diff_lines
            )),
            Verdict::Eliminated { reason } => o.push_str(&format!(
                "  {crown} #{} {:<26} out — {}\n",
                out.entrant, out.branch, reason
            )),
        }
    }
    o.push('\n');
    match winner {
        Some(i) => o.push_str(&format!(
            "  Winner: entrant #{} on {}. Merged whole — no parts were taken from the others.\n",
            outcomes[i].entrant, outcomes[i].branch
        )),
        None => o.push_str(
            "  No entrant held the gate. Nothing was merged.\n  \
             That is information about the gate or the challenge, not a build to rescue.\n",
        ),
    }
    o
}

// ── the impure edge ─────────────────────────────────────────────────────────

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").current_dir(dir).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Where entrant worktrees live: beside the repository, never inside it.
///
/// An entrant worktree is a full checkout. Put one under `.hexa/arena/` and
/// `hexa analyze` walks into it, counts the copies as source, and lets a copy
/// of a file act as a caller of the original — which erases real dead-export
/// findings and *raises* the grade. Observed: this repository graded A+ 96 with
/// two entrants running and A+ 100 without them, on identical `hexa-infer`
/// code, because the scan went from 149 files to 448.
///
/// `hexa do`'s isolated runs already live outside the repo for the same reason.
fn arena_root(repo: &Path) -> PathBuf {
    repo.parent().unwrap_or(repo).join(".hexa-arenas")
}

/// Fork a worktree for one entrant.
fn fork(repo: &Path, branch: &str) -> anyhow::Result<PathBuf> {
    let dir = arena_root(repo).join(branch.replace('/', "_"));
    if dir.exists() {
        let _ = git(repo, &["worktree", "remove", "--force", &dir.to_string_lossy()]);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).ok();
        }
    }
    let _ = git(repo, &["branch", "-D", branch]);
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let out = Command::new("git")
        .current_dir(repo)
        .args(["worktree", "add", "-b", branch, &dir.to_string_lossy(), "HEAD"])
        .output()?;
    if !out.status.success() {
        anyhow::bail!("could not fork a worktree for {branch}: {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(dir)
}

/// How many lines the entrant changed against the base.
fn diff_lines(worktree: &Path) -> usize {
    let Some(out) = git(worktree, &["add", "-A", "--dry-run"]) else { return 0 };
    let _ = out;
    let _ = git(worktree, &["add", "-A"]);
    let stat = git(worktree, &["diff", "--cached", "--numstat"]).unwrap_or_default();
    stat.lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let a: usize = f.next()?.parse().ok()?;
            let d: usize = f.next()?.parse().ok()?;
            Some(a + d)
        })
        .sum()
}

/// The architecture grade of a worktree, asked of this very binary.
fn grade_of(worktree: &Path) -> (u32, String) {
    let me = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("hexa"));
    let Ok(out) = Command::new(me).current_dir(worktree).args(["analyze", ".", "--json"]).output()
    else {
        return (0, "?".into());
    };
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else {
        return (0, "?".into());
    };
    let score = v.get("score").and_then(|s| s.as_u64()).unwrap_or(0) as u32;
    let grade = v.get("grade").and_then(|g| g.as_str()).unwrap_or("?").to_string();
    (score, grade)
}

/// Run the gate inside a worktree.
fn run_gate(worktree: &Path, gate: &str) -> (bool, String) {
    let out = Command::new("sh").current_dir(worktree).arg("-c").arg(gate).output();
    match out {
        Ok(o) => (
            o.status.success(),
            format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)),
        ),
        Err(e) => (false, e.to_string()),
    }
}

/// Copy a built target out of the winner's worktree into the repository.
fn copy_tree(from: &Path, to: &Path) -> std::io::Result<usize> {
    let mut n = 0usize;
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let src = e.path();
        let dst = to.join(e.file_name());
        if src.is_dir() {
            n += copy_tree(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst)?;
            n += 1;
        }
    }
    Ok(n)
}

pub async fn run(args: ArenaArgs) -> anyhow::Result<()> {
    let repo = std::env::current_dir()?;
    let target_abs = repo.join(&args.target);

    println!("{} {} {} {}", "⬡ arena".cyan().bold(), args.target.yellow(), "←".dimmed(), args.challenge);

    if let Err(r) = check(args.entrants, &target_abs) {
        println!("  {} {}", "✗ refused".red().bold(), r.message());
        anyhow::bail!("arena refused before building anything");
    }
    println!("  {} entrant(s), same challenge, same gate", args.entrants);
    if args.plan {
        return Ok(());
    }

    let _ = crate::commands::loop_cmd::update_loop(
        &repo,
        serde_json::json!({ "gate": args.gate.clone(), "stage": "build" }),
    );

    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let mut forks = Vec::new();
    for i in 1..=args.entrants {
        let branch = format!("hexa/arena/{stamp}-{i}");
        match fork(&repo, &branch) {
            Ok(dir) => forks.push((i, branch, dir)),
            Err(e) => {
                // A fork that fails is reported, not silently skipped.
                println!("  {} entrant #{i}: {e}", "✗".red());
            }
        }
    }
    if forks.is_empty() {
        anyhow::bail!("no entrant could be forked");
    }

    println!("  {} building {} entrant(s) in parallel\n", "→".green(), forks.len());

    // Concurrent. The whole point is to sample the build step more than once,
    // and doing that in sequence just costs the same wall-clock as N builds.
    let futures = forks.iter().map(|(i, branch, dir)| {
        let (challenge, target, gate) = (&args.challenge, &args.target, &args.gate);
        let (designs, timeout, retries) = (args.designs, args.timeout, args.retries);
        async move {
            let b = hexa_exec::adversarial::run_build(
                challenge, target, gate, designs, dir, timeout, retries,
            )
            .await;
            let (gate_ok, output) = if b.build_ok { run_gate(dir, gate) } else { (false, String::new()) };
            let changed = diff_lines(dir);
            let (score, grade) = grade_of(dir);
            Outcome {
                entrant: *i,
                branch: branch.clone(),
                verdict: judge(gate_ok, &output, score, &grade, changed),
            }
        }
    });
    let outcomes: Vec<Outcome> = futures_util::future::join_all(futures).await;

    let winner = rank(&outcomes);
    print!("{}", report(&outcomes, winner));

    let Some(w) = winner else {
        anyhow::bail!("no entrant held the gate");
    };
    let src = forks
        .iter()
        .find(|(i, ..)| *i == outcomes[w].entrant)
        .map(|(.., d)| d.join(&args.target))
        .ok_or_else(|| anyhow::anyhow!("the winning worktree is gone"))?;
    let n = copy_tree(&src, &target_abs)?;
    println!("  {} {} file(s) merged into {}", "✓".green(), n, args.target.yellow());
    println!(
        "  {} losing worktrees kept for review; `hexa dev worktree cleanup` prunes them.",
        "·".dimmed()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn survived(entrant: usize, score: u32, diff: usize) -> Outcome {
        Outcome {
            entrant,
            branch: format!("hexa/arena/x-{entrant}"),
            verdict: Verdict::Survived { score, grade: "A".into(), diff_lines: diff },
        }
    }
    fn out(entrant: usize) -> Outcome {
        Outcome {
            entrant,
            branch: format!("hexa/arena/x-{entrant}"),
            verdict: Verdict::Eliminated { reason: "the gate did not exit 0".into() },
        }
    }

    #[test]
    fn entrant_worktrees_never_live_inside_the_repository() {
        // A full checkout under the repo makes `hexa analyze` walk into it and
        // treat the copies as source. That erases dead-export findings and
        // raises the grade — a gate degrading in the flattering direction.
        let repo = std::path::Path::new("/home/someone/dev/myrepo");
        let root = arena_root(repo);
        assert!(!root.starts_with(repo), "entrant worktrees are inside the repo: {}", root.display());
        assert_eq!(root, std::path::Path::new("/home/someone/dev/.hexa-arenas"));
    }

    #[test]
    fn one_entrant_is_not_an_arena() {
        let d = std::path::Path::new("/nonexistent-target");
        assert_eq!(check(1, d), Err(Refusal::TooFewEntrants(1)));
        assert_eq!(check(0, d), Err(Refusal::TooFewEntrants(0)));
        assert!(check(2, d).is_ok());
    }

    #[test]
    fn a_target_with_content_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("keep.txt"), "mine").expect("write");
        assert!(matches!(check(2, dir.path()), Err(Refusal::TargetNotEmpty(_))));
    }

    #[test]
    fn an_empty_target_is_fine_and_so_is_an_absent_one() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(check(2, dir.path()).is_ok());
        assert!(check(2, &dir.path().join("not-yet")).is_ok());
    }

    #[test]
    fn a_failing_gate_eliminates() {
        assert!(matches!(judge(false, "", 100, "A+", 40), Verdict::Eliminated { .. }));
    }

    #[test]
    fn a_gate_that_runs_zero_tests_eliminates() {
        let output = "test result: ok. 0 passed; 0 failed; 0 ignored";
        match judge(true, output, 100, "A+", 40) {
            Verdict::Eliminated { reason } => assert!(reason.contains("zero tests"), "{reason}"),
            other => panic!("a vacuous gate must not crown anyone: {other:?}"),
        }
    }

    #[test]
    fn a_gate_that_passes_on_an_empty_diff_eliminates() {
        let output = "test result: ok. 7 passed; 0 failed";
        match judge(true, output, 100, "A+", 0) {
            Verdict::Eliminated { reason } => assert!(reason.contains("empty diff"), "{reason}"),
            other => panic!("nothing was built: {other:?}"),
        }
    }

    #[test]
    fn a_real_pass_survives() {
        let output = "test result: ok. 7 passed; 0 failed";
        assert!(matches!(judge(true, output, 96, "A+", 120), Verdict::Survived { .. }));
    }

    #[test]
    fn the_higher_grade_wins() {
        let o = vec![survived(1, 90, 10), survived(2, 96, 900)];
        assert_eq!(rank(&o).map(|i| o[i].entrant), Some(2));
    }

    #[test]
    fn a_tie_on_grade_breaks_on_the_smaller_diff() {
        let o = vec![survived(1, 96, 900), survived(2, 96, 120)];
        assert_eq!(rank(&o).map(|i| o[i].entrant), Some(2));
    }

    #[test]
    fn a_tie_on_both_is_still_deterministic() {
        let o = vec![survived(3, 96, 100), survived(1, 96, 100), survived(2, 96, 100)];
        for _ in 0..5 {
            assert_eq!(rank(&o).map(|i| o[i].entrant), Some(1), "the same results crowned two entrants");
        }
    }

    #[test]
    fn an_eliminated_entrant_can_never_win_however_good_its_grade_would_be() {
        let o = vec![out(1), survived(2, 60, 5000)];
        assert_eq!(rank(&o).map(|i| o[i].entrant), Some(2));
    }

    #[test]
    fn zero_survivors_crowns_nobody() {
        let o = vec![out(1), out(2), out(3)];
        assert_eq!(rank(&o), None);
    }

    #[test]
    fn the_report_names_every_entrant_including_the_losers() {
        let o = vec![survived(1, 96, 100), out(2), survived(3, 90, 50)];
        let w = rank(&o);
        let text = report(&o, w);
        for n in ["#1", "#2", "#3"] {
            assert!(text.contains(n), "entrant {n} is missing from the report:\n{text}");
        }
        assert!(text.contains("★"), "the winner must be marked:\n{text}");
        assert!(text.contains("no parts were taken"), "{text}");
    }

    #[test]
    fn a_shut_out_report_says_so_and_crowns_nobody() {
        let o = vec![out(1), out(2)];
        let text = report(&o, None);
        assert!(text.contains("No entrant held the gate"), "{text}");
        assert!(text.contains("Nothing was merged"), "{text}");
        assert!(!text.contains("★"), "nobody won; nothing should be crowned:\n{text}");
    }
}
