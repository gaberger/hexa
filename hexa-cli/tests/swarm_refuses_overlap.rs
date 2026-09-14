//! A swarm that fans out is easy. A swarm that refuses to is the decision.
//!
//! ADR-2609140927. Two workers on one file both pass their own gate, and then
//! one overwrites the other. Nothing errors — the evidence outlives the state
//! it was evidence for. So the overlap is computed before any worker starts,
//! and a non-empty intersection stops the run.
//!
//! The exit code is the weak half of that. The assertion that matters is that
//! nothing was created: a swarm that refuses after forking three worktrees has
//! already done the damage it was refusing.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn hexa_bin() -> PathBuf {
    let mut p = std::env::current_exe().expect("test exe");
    p.pop();
    p.pop();
    p.push("hexa");
    p
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("workspace root").to_path_buf()
}

/// Every worktree git currently knows about.
fn worktrees() -> BTreeSet<String> {
    let out = Command::new("git")
        .current_dir(workspace_root())
        .args(["worktree", "list", "--porcelain"])
        .output()
        .expect("git worktree list");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.strip_prefix("worktree ").map(str::to_string))
        .collect()
}

fn swarm(args: &[&str]) -> Output {
    Command::new(hexa_bin())
        .current_dir(workspace_root())
        .arg("swarm")
        .args(args)
        .output()
        .expect("run hexa swarm")
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

/// The gate this ADR was written for.
#[test]
fn overlapping_slices_are_refused_before_anything_is_created() {
    let before = worktrees();

    // `hexa-cli/src/commands` lives inside `hexa-cli/src`. The two strings look
    // independent; the file sets are not.
    let out = swarm(&[
        "add a module doc comment",
        "--over",
        "hexa-cli/src",
        "--over",
        "hexa-cli/src/commands",
        "--gate",
        "true",
    ]);

    assert!(!out.status.success(), "an overlapping swarm must not succeed:\n{}", text(&out));

    let t = text(&out);
    assert!(t.contains("refused"), "the refusal must say so:\n{t}");
    assert!(
        t.contains("hexa-cli/src/commands/"),
        "the refusal must name a shared file so the operator can re-slice:\n{t}"
    );

    assert_eq!(
        worktrees(),
        before,
        "the swarm created a worktree before refusing; the refusal came too late"
    );
}

/// A slice that matches nothing is refused, not reported as zero workers all
/// passing. That is `evidence_is_vacuous` in a new shape.
#[test]
fn a_slice_that_matches_nothing_is_refused() {
    let before = worktrees();
    let out = swarm(&["x", "--over", "no/such/place", "--gate", "true"]);
    assert!(!out.status.success(), "an empty slice must not succeed:\n{}", text(&out));
    let t = text(&out);
    assert!(t.contains("matches no tracked file"), "{t}");
    assert_eq!(worktrees(), before, "an empty slice created a worktree");
}

/// Disjoint slices plan cleanly, and `--plan` stops before any worker.
#[test]
fn disjoint_slices_plan_and_plan_alone_starts_nothing() {
    let before = worktrees();
    let out = swarm(&[
        "x",
        "--over",
        "hexa-git/src",
        "--over",
        "hexa-parser/src",
        "--gate",
        "true",
        "--plan",
    ]);
    assert!(out.status.success(), "a disjoint plan must succeed:\n{}", text(&out));

    let t = text(&out);
    assert!(t.contains("2 slice(s), no overlap"), "{t}");
    for spec in ["hexa-git/src", "hexa-parser/src"] {
        assert!(t.contains(spec), "the plan must name every slice; `{spec}` is missing:\n{t}");
    }
    assert_eq!(worktrees(), before, "--plan created a worktree");
}

/// One slice runs, and says the simpler verb exists (decision 6).
#[test]
fn one_slice_names_the_simpler_verb() {
    let out = swarm(&["x", "--over", "hexa-parser/src", "--gate", "true", "--plan"]);
    assert!(out.status.success(), "{}", text(&out));
    let t = text(&out);
    assert!(t.contains("hexa do run"), "a single-slice swarm should point at `hexa do run`:\n{t}");
}

/// The refusal is the same whichever order the slices are given in. An operator
/// must not be able to get past the rule by reordering their arguments.
#[test]
fn the_refusal_does_not_depend_on_argument_order() {
    let a = swarm(&["x", "--over", "hexa-cli/src", "--over", "hexa-cli/src/commands", "--gate", "true"]);
    let b = swarm(&["x", "--over", "hexa-cli/src/commands", "--over", "hexa-cli/src", "--gate", "true"]);
    assert!(!a.status.success() && !b.status.success(), "both orders must refuse");
}
