//! `hexa harden --fast-gate`: each fix is gated fast, each batch in full, and
//! a fix that breaks the batch is found by bisection (ADR-2610160756).
//!
//! Written from the ADR's Implementation section, before the code. Every case
//! here is one bullet of its gate.
//!
//! Observed 2026-10-15/16 in weave: the full gate takes 1m27s–1m39s, and
//! ADR-2610141733 §6 ran it after every fix — a pass that kept 38 fixes spent
//! about an hour on gates alone.
//!
//! The frontier is a scripted fake: it reports one finding per scripted fix,
//! confirms each, and "fixes" bug `n` by writing `src/fix_n.txt` and a
//! regression test `tests/bug_n.txt`. Each fix writes its own files so that a
//! fix can be dropped and the later ones still cherry-pick cleanly.
//!
//! The gates are one scripted shell command, `gate.sh`, kept outside the
//! repository so it is never part of a fix:
//!
//! - `gate.sh fast {touched}` checks only the files it is given — the fix's
//!   own tests. A file carrying `BROKEN` fails it; a file saying
//!   `needs fix N` fails it unless `src/fix_N.txt` exists.
//! - `gate.sh full` runs the same checks over every file, plus an
//!   "integration test" that fails when any file carries `POISON`. A fix
//!   that writes `POISON` breaks a test in a file it did not touch: the fast
//!   gate cannot see it and the full gate can.
//! - `gate.sh vacuous …` passes having run no tests.
//!
//! Every run appends its mode to `runs.log`, so the number of full gates is
//! counted by the gate itself, not taken from the report.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use hexa_exec::adversarial::{run_review_gated, HardenGates, ReviewReport};
use hexa_exec::ports::{Frontier, FrontierError};

const TARGET: &str = "src/target.txt";

/// What the fake agent does when asked to fix bug `n`.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Fix {
    /// Writes `src/fix_n.txt` and its regression test. Passes every gate.
    Good,
    /// Writes a file that breaks the integration test: the fast gate passes
    /// it, the full gate does not.
    Poisons,
    /// Writes a file whose own test passes only once fix `m` is in.
    Needs(usize),
    /// Writes `src/my file n.txt` — a path with a space.
    SpacedPath,
}

struct FakeFrontier {
    fixes: Vec<Fix>,
    /// Each finding's title; `bug-n` unless a test names them alike.
    titles: Vec<String>,
    handed_out: AtomicBool,
}

impl FakeFrontier {
    fn new(fixes: Vec<Fix>, titles: Vec<String>) -> Arc<Self> {
        Arc::new(Self { fixes, titles, handed_out: AtomicBool::new(false) })
    }
}

#[async_trait]
impl Frontier for FakeFrontier {
    async fn run(&self, prompt: &str, cwd: &Path, _t: Duration, _s: &str) -> Result<String, FrontierError> {
        if prompt.contains("adversarial code reviewer") {
            // One lens reports every finding; the others report none.
            if self.handed_out.swap(true, Ordering::SeqCst) {
                return Ok(r#"{"findings":[]}"#.to_string());
            }
            let findings: Vec<String> = (1..=self.fixes.len())
                .map(|n| {
                    let title = &self.titles[n - 1];
                    format!(r#"{{"title":"{title}","location":"{TARGET}:{n}","description":"defect {n}","lens":"correctness"}}"#)
                })
                .collect();
            return Ok(format!(r#"{{"findings":[{}]}}"#, findings.join(",")));
        }
        if prompt.contains("SKEPTICALLY verify") {
            return Ok(r#"{"is_real":true,"reasoning":"it is"}"#.to_string());
        }
        if prompt.contains("Fix this CONFIRMED bug") {
            let n = (1..=self.fixes.len())
                .find(|n| prompt.contains(&format!("location: {TARGET}:{n}\n")))
                .expect("the fix prompt names its bug's location");
            let (path, body) = match self.fixes[n - 1] {
                Fix::Good => (format!("src/fix_{n}.txt"), format!("fix {n}\n")),
                Fix::Poisons => (format!("src/fix_{n}.txt"), format!("fix {n}\nPOISON\n")),
                Fix::Needs(m) => (format!("src/fix_{n}.txt"), format!("fix {n}\nneeds fix {m}\n")),
                Fix::SpacedPath => (format!("src/my file {n}.txt"), format!("fix {n}\n")),
            };
            std::fs::write(cwd.join(path), body).unwrap();
            std::fs::create_dir_all(cwd.join("tests")).unwrap();
            std::fs::write(cwd.join(format!("tests/bug_{n}.txt")), format!("regression test for bug {n}\n")).unwrap();
            return Ok(format!("fixed bug-{n}"));
        }
        Ok(String::new())
    }
}

const GATE_SH: &str = r#"#!/usr/bin/env bash
# gate.sh full | gate.sh fast <touched…> | gate.sh vacuous <touched…>
ctl="$(cd "$(dirname "$0")" && pwd)"
mode="$1"; shift
echo "$mode" >> "$ctl/runs.log"
files=()
case "$mode" in
  vacuous)
    echo "running 0 tests"
    echo "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out"
    exit 0
    ;;
  fast)
    for a in "$@"; do printf '%s\n' "$a" >> "$ctl/args.log"; files+=("$a"); done
    ;;
  full)
    if [ -e "$ctl/flake-once" ]; then
      rm -f "$ctl/flake-once"
      echo "flaked: a one-off failure"
      exit 1
    fi
    while IFS= read -r -d '' f; do files+=("$f"); done < <(find src tests -type f -print0 2>/dev/null)
    for f in "${files[@]}"; do
      if grep -q POISON "$f"; then echo "integration test failed: $f carries POISON"; exit 1; fi
    done
    ;;
  *)
    echo "unknown mode $mode"; exit 2
    ;;
esac
for f in "${files[@]}"; do
  [ -f "$f" ] || continue
  if grep -q BROKEN "$f"; then echo "test failed: $f is BROKEN"; exit 1; fi
  need=$(sed -n 's/^needs fix \([0-9][0-9]*\)$/\1/p' "$f")
  if [ -n "$need" ] && [ ! -e "src/fix_$need.txt" ]; then echo "test failed: $f needs fix $need"; exit 1; fi
done
echo "ok: ${#files[@]} file(s) checked"
"#;

/// A scratch repository with one target file, and a control directory
/// beside it holding the gate script and its logs.
struct Scratch {
    _tmp: tempfile::TempDir,
    dir: PathBuf,
    ctl: PathBuf,
}

impl Scratch {
    fn new() -> Scratch {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("repo");
        let ctl = tmp.path().join("ctl");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(&ctl).unwrap();
        std::fs::write(ctl.join("gate.sh"), GATE_SH).unwrap();
        git(&dir, &["init", "-q"]);
        std::fs::write(dir.join(TARGET), "original\n").unwrap();
        std::fs::write(dir.join("README.md"), "readme\n").unwrap();
        git(&dir, &["add", "-A"]);
        git(&dir, &["-c", "user.name=t", "-c", "user.email=t@x", "commit", "-qm", "init"]);
        Scratch { _tmp: tmp, dir, ctl }
    }

    fn full(&self) -> String {
        format!("bash '{}' full", self.ctl.join("gate.sh").display())
    }

    fn fast(&self) -> String {
        format!("bash '{}' fast {{touched}}", self.ctl.join("gate.sh").display())
    }

    fn vacuous(&self) -> String {
        format!("bash '{}' vacuous {{touched}}", self.ctl.join("gate.sh").display())
    }

    /// The full gate fails once, on its next run, and then never again.
    fn flake_once(&self) {
        std::fs::write(self.ctl.join("flake-once"), "").unwrap();
    }

    /// How many times the gate ran in `mode`, as the gate itself logged it.
    fn runs(&self, mode: &str) -> usize {
        std::fs::read_to_string(self.ctl.join("runs.log"))
            .unwrap_or_default()
            .lines()
            .filter(|l| *l == mode)
            .count()
    }

    /// Every argument the fast gate received, one per line, in order.
    fn fast_args(&self) -> Vec<String> {
        std::fs::read_to_string(self.ctl.join("args.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    async fn harden(&self, fixes: Vec<Fix>, fast_gate: Option<String>) -> ReviewReport {
        let titles = titles(&(1..=fixes.len()).collect::<Vec<_>>());
        self.harden_titled(fixes, titles, fast_gate).await
    }

    /// [`Scratch::harden`], with each finding's title given.
    async fn harden_titled(&self, fixes: Vec<Fix>, titles: Vec<String>, fast_gate: Option<String>) -> ReviewReport {
        let gates = HardenGates { gate: self.full(), fast_gate, batch: None };
        run_review_gated(FakeFrontier::new(fixes, titles), hexa_exec::default_git_history(), TARGET, &gates, &self.dir).await
    }

    /// Fix commits on HEAD since `init`, oldest first: (subject, Hexa-Gate trailer).
    fn commits(&self) -> Vec<(String, String)> {
        git(&self.dir, &["log", "--reverse", "--format=%s%x1f%(trailers:key=Hexa-Gate,valueonly,separator=%x2c)%x1e", "HEAD"])
            .split('\x1e')
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .map(|r| {
                let (s, t) = r.split_once('\x1f').unwrap_or((r, ""));
                (s.to_string(), t.trim().to_string())
            })
            .filter(|(s, _)| s != "init")
            .collect()
    }

    fn subjects(&self) -> Vec<String> {
        self.commits().into_iter().map(|(s, _)| s).collect()
    }

    fn status(&self) -> String {
        git(&self.dir, &["status", "--porcelain"])
    }

    fn exists(&self, path: &str) -> bool {
        self.dir.join(path).exists()
    }
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").args(args).current_dir(dir).output().expect("git runs");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn titles(ns: &[usize]) -> Vec<String> {
    ns.iter().map(|n| format!("bug-{n}")).collect()
}

fn subjects(ns: &[usize]) -> Vec<String> {
    ns.iter().map(|n| format!("fix(harden): bug-{n}")).collect()
}

/// ⌈log₂ n⌉, for the bisection bound.
fn ceil_log2(n: usize) -> usize {
    let mut k = 0;
    while (1usize << k) < n {
        k += 1;
    }
    k
}

/// Five clean fixes: five commits, each with `Hexa-Gate: fast`, exactly one
/// full-gate run, and a final gate that is not run again (§2, §3, §7).
#[tokio::test]
async fn five_clean_fixes_cost_one_full_gate() {
    let s = Scratch::new();
    let report = s.harden(vec![Fix::Good; 5], Some(s.fast())).await;

    assert_eq!(report.fixed, titles(&[1, 2, 3, 4, 5]), "notes: {:?}", report.notes);
    let commits = s.commits();
    assert_eq!(commits.len(), 5, "one commit per fix: {commits:?}");
    for (subject, gate) in &commits {
        assert_eq!(gate, "fast", "'{subject}' carries Hexa-Gate: fast as a trailer: {commits:?}");
    }
    assert_eq!(s.runs("fast"), 5, "each fix is gated fast");
    assert_eq!(s.runs("full"), 1, "one full gate for the batch, and the final gate is that run");
    assert_eq!(report.full_gates, 1, "the report counts the full gates that ran");
    assert!(report.gate_run && report.gate_passed, "the batch gate is the final gate: {}", report.gate_line());
    assert_eq!(report.batches.len(), 1, "one batch: {:?}", report.batches);
    assert!(report.batches[0].green, "{:?}", report.batches);
    assert_eq!(report.batches[0].fixes, titles(&[1, 2, 3, 4, 5]));
    assert!(report.dropped.is_empty() && report.flakes.is_empty());
    assert_eq!(s.status(), "", "nothing is left in the tree");
    assert!(report.verdict_line().contains("1 full gate"), "{}", report.verdict_line());
}

/// Fix 3 breaks a test in a file it did not touch. The fast gate passes it,
/// the batch is red, bisection blames fix 3 within the bound, and fixes 1, 2,
/// 4 and 5 are committed on a green head (§4, §5). The operator's own
/// uncommitted work is never touched by the resets and cherry-picks.
#[tokio::test]
async fn a_fix_that_breaks_an_untouched_file_is_found_by_bisection() {
    let s = Scratch::new();
    std::fs::write(s.dir.join("README.md"), "readme\noperator edit\n").unwrap();
    std::fs::write(s.dir.join("scratch.txt"), "operator notes\n").unwrap();

    let report = s.harden(vec![Fix::Good, Fix::Good, Fix::Poisons, Fix::Good, Fix::Good], Some(s.fast())).await;

    assert!(s.runs("fast") >= 5, "the fast gate saw every fix, and passed fix 3");
    assert_eq!(report.fixed, titles(&[1, 2, 4, 5]), "notes: {:?}", report.notes);
    assert_eq!(s.subjects(), subjects(&[1, 2, 4, 5]), "fix 3 is not on the branch");
    assert!(!s.exists("src/fix_3.txt") && !s.exists("tests/bug_3.txt"), "fix 3 and its test are gone");
    for n in [1, 2, 4, 5] {
        assert!(s.exists(&format!("src/fix_{n}.txt")), "fix {n} is in the tree");
    }

    let dropped: Vec<&str> = report.dropped.iter().map(|d| d.title.as_str()).collect();
    assert_eq!(dropped, vec!["bug-3"], "bisection names the culprit");
    assert!(
        report.dropped[0].evidence.contains("POISON"),
        "the culprit's evidence is the full gate's output: {:?}",
        report.dropped[0].evidence
    );
    assert!(report.flakes.is_empty(), "a reproducible red is not a flake: {:?}", report.flakes);

    // The batch gate (1), the prefix search (⌈log₂ 5⌉), and the confirmation
    // of prefixes k−1 and k (2) — the ADR's bound for blaming fix 3. Then one
    // full gate for the rebuilt batch, and one for fix 3's last-chance retry
    // on the final tree (§6), which it fails again.
    let bound = ceil_log2(5) + 1 + 2 + 1 + 1;
    assert!(s.runs("full") <= bound, "{} full gates, bound {bound}", s.runs("full"));
    assert_eq!(report.full_gates, s.runs("full"), "the report counts the full gates that ran");

    assert!(report.gate_run && report.gate_passed, "the head is green: {}", report.gate_line());
    let (head_green, out) = run(&s.dir, &s.full());
    assert!(head_green, "the committed head passes the full gate: {out}");

    assert_eq!(std::fs::read_to_string(s.dir.join("README.md")).unwrap(), "readme\noperator edit\n");
    assert_eq!(std::fs::read_to_string(s.dir.join("scratch.txt")).unwrap(), "operator notes\n");
    let mut status: Vec<String> = s.status().lines().map(str::to_string).collect();
    status.sort();
    assert_eq!(status, vec![" M README.md", "?? scratch.txt"], "only the operator's work is uncommitted");
    assert!(report.verdict_line().contains("bug-3"), "the verdict names the dropped fix: {}", report.verdict_line());
}

/// Two lenses name two fixes alike. Bisection drops the poisoned one; the
/// other passed its full gate and stays committed, so it stays claimed.
#[tokio::test]
async fn dropping_a_fix_keeps_a_same_titled_fix_that_landed() {
    let s = Scratch::new();
    let dup = || "missing bounds check".to_string();
    let report = s.harden_titled(vec![Fix::Good, Fix::Poisons], vec![dup(), dup()], Some(s.fast())).await;

    assert!(s.exists("src/fix_1.txt") && !s.exists("src/fix_2.txt"), "fix 1 is in, fix 2 is out");
    assert_eq!(s.subjects(), vec!["fix(harden): missing bounds check"], "one fix is committed");
    assert_eq!(report.fixed, vec![dup()], "the committed fix is still claimed; notes: {:?}", report.notes);
    let dropped: Vec<&str> = report.dropped.iter().map(|d| d.title.as_str()).collect();
    assert_eq!(dropped, vec![dup().as_str()], "the poisoned fix is dropped");
}

/// A fix lands on its last chance while a same-titled fix stays dropped:
/// only the one that landed leaves `dropped`.
#[tokio::test]
async fn a_last_chance_landing_keeps_a_same_titled_fix_dropped() {
    let s = Scratch::new();
    let dup = || "missing bounds check".to_string();
    let report = s
        .harden_titled(vec![Fix::Needs(3), Fix::Poisons, Fix::Good], vec![dup(), dup(), "bug-3".to_string()], Some(s.fast()))
        .await;

    assert!(s.exists("src/fix_1.txt") && !s.exists("src/fix_2.txt"), "fix 1 landed last, fix 2 is out");
    let mut fixed = report.fixed.clone();
    fixed.sort();
    assert_eq!(fixed, vec!["bug-3".to_string(), dup()], "notes: {:?}", report.notes);
    let dropped: Vec<&str> = report.dropped.iter().map(|d| d.title.as_str()).collect();
    assert_eq!(dropped, vec![dup().as_str()], "the poisoned fix is still out: notes: {:?}", report.notes);
    assert!(report.verdict_line().contains("dropped by bisection"), "{}", report.verdict_line());
}

/// Fix 2 passes only once fix 4 is in. The fast gate rejects it, it is
/// retried last on the final tree, and it lands (§6).
#[tokio::test]
async fn a_fix_that_needed_a_later_fix_is_retried_last_and_committed() {
    let s = Scratch::new();
    let report = s.harden(vec![Fix::Good, Fix::Needs(4), Fix::Good, Fix::Good, Fix::Good], Some(s.fast())).await;

    let mut fixed = report.fixed.clone();
    fixed.sort();
    assert_eq!(fixed, titles(&[1, 2, 3, 4, 5]), "every fix lands; notes: {:?}", report.notes);
    assert_eq!(report.fixed.last().map(String::as_str), Some("bug-2"), "fix 2 lands last: {:?}", report.fixed);
    assert_eq!(s.subjects(), subjects(&[1, 3, 4, 5, 2]));
    assert!(s.exists("src/fix_2.txt") && s.exists("tests/bug_2.txt"));
    assert_eq!(s.status(), "");
    assert!(report.gate_run && report.gate_passed, "{}", report.gate_line());
    assert!(s.runs("full") <= 3, "the batch, the retry, at most one more: {}", s.runs("full"));
}

/// A full gate that fails once and never again is a flake. It is reported,
/// no fix is dropped, and the batch is not counted as green (§5).
#[tokio::test]
async fn a_flaky_full_gate_is_reported_and_drops_no_fix() {
    let s = Scratch::new();
    s.flake_once();
    let report = s.harden(vec![Fix::Good; 3], Some(s.fast())).await;

    assert_eq!(report.flakes.len(), 1, "the flake is reported: {:?}; notes: {:?}", report.flakes, report.notes);
    assert!(
        report.flakes[0].to_lowercase().contains("prefix"),
        "the flake names the prefixes it reran: {}",
        report.flakes[0]
    );
    assert!(report.dropped.is_empty(), "an innocent fix is not blamed: {:?}", report.dropped);
    assert_eq!(report.fixed, titles(&[1, 2, 3]), "notes: {:?}", report.notes);
    assert_eq!(s.subjects(), subjects(&[1, 2, 3]), "no fix is reverted");
    assert!(!report.batches.is_empty(), "the batch is reported");
    assert!(report.batches.iter().all(|b| !b.green), "a flaky batch is not a green base: {:?}", report.batches);
    assert!(report.verdict_line().contains("flake"), "{}", report.verdict_line());
}

/// A fast gate that ran no tests did not pass: the fix is rejected as
/// vacuous and nothing is committed (§2).
#[tokio::test]
async fn a_vacuous_fast_gate_rejects_the_fix() {
    let s = Scratch::new();
    let report = s.harden(vec![Fix::Good], Some(s.vacuous())).await;

    assert!(report.fixed.is_empty(), "a gate that ran nothing proves nothing: {:?}", report.fixed);
    assert!(s.subjects().is_empty(), "nothing is committed: {:?}", s.subjects());
    assert!(!s.exists("src/fix_1.txt"), "the rejected fix is restored away");
    assert_eq!(s.status(), "");
    assert!(
        report.notes.iter().any(|n| n.contains("bug-1") && n.contains("vacuous")),
        "the rejection says why: {:?}",
        report.notes
    );
}

/// `{touched}` is replaced by the fix's changed paths, each shell-quoted: a
/// path with a space reaches the command as one argument (§2).
#[tokio::test]
async fn a_touched_path_with_a_space_is_one_argument() {
    let s = Scratch::new();
    let report = s.harden(vec![Fix::SpacedPath], Some(s.fast())).await;

    let mut args = s.fast_args();
    args.sort();
    assert_eq!(args, vec!["src/my file 1.txt".to_string(), "tests/bug_1.txt".to_string()]);
    assert_eq!(report.fixed, titles(&[1]), "notes: {:?}", report.notes);
}

/// With `--batch 1`, fix 1 closes a green batch. Fix 2 poisons the build but
/// touches a file the operator had already changed, so it is left in the tree
/// uncommitted, seen only by the fast gate. HEAD is still the green head, yet
/// the tree is not the one the full gate passed on: the final gate runs, and
/// it fails (§7).
#[tokio::test]
async fn an_uncommitted_fast_gated_fix_is_not_covered_by_the_last_green_gate() {
    let s = Scratch::new();
    std::fs::create_dir_all(s.dir.join("tests")).unwrap();
    std::fs::write(s.dir.join("tests/bug_2.txt"), "operator draft\n").unwrap();
    let gates = HardenGates { gate: s.full(), fast_gate: Some(s.fast()), batch: Some(1) };
    let report = run_review_gated(
        FakeFrontier::new(vec![Fix::Good, Fix::Poisons], titles(&[1, 2])),
        hexa_exec::default_git_history(),
        TARGET,
        &gates,
        &s.dir,
    )
    .await;

    assert_eq!(report.fixed, titles(&[1]), "notes: {:?}", report.notes);
    assert_eq!(report.uncommitted, titles(&[2]), "fix 2 is left in the tree: {:?}", report.notes);
    assert!(s.exists("src/fix_2.txt"), "fix 2's poison is in the tree");
    assert!(report.gate_run, "{}", report.gate_line());
    assert!(!report.gate_passed, "the tree holds POISON the full gate never saw: {}", report.gate_line());
    assert_eq!(s.runs("full"), 2, "the batch's full gate, then the final gate on the changed tree");
}

/// Without `--fast-gate` nothing changes: the full gate runs after each fix
/// and once more at the end, and no commit claims a fast gate (§1).
#[tokio::test]
async fn without_a_fast_gate_every_fix_gets_the_full_gate() {
    let s = Scratch::new();
    let report = s.harden(vec![Fix::Good; 3], None).await;

    assert_eq!(report.fixed, titles(&[1, 2, 3]), "notes: {:?}", report.notes);
    assert_eq!(s.runs("full"), 3 + 1, "a full gate per fix, then the final gate");
    assert_eq!(s.runs("fast"), 0);
    for (subject, gate) in s.commits() {
        assert!(gate.is_empty(), "'{subject}' claims Hexa-Gate: {gate}");
    }
    assert!(report.gate_passed);
}

fn run(dir: &Path, cmd: &str) -> (bool, String) {
    let out = Command::new("bash").arg("-c").arg(cmd).current_dir(dir).output().expect("bash runs");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr),
    )
}
