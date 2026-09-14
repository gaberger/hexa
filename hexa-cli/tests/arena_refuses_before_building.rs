//! An arena that builds is expensive. An arena that refuses is free, and it
//! has to be right first (ADR-2609140926).
//!
//! Every check in this file runs before an entrant exists. The assertion that
//! matters is not the exit code but that nothing was created: an arena that
//! refuses after forking three worktrees has already spent the thing it was
//! refusing to spend.

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

fn arena(args: &[&str]) -> Output {
    Command::new(hexa_bin())
        .current_dir(workspace_root())
        .arg("arena")
        .args(args)
        .output()
        .expect("run hexa arena")
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

/// One entrant is `hexa build`. The arena says so rather than running a
/// one-horse race and calling the result a winner.
#[test]
fn a_single_entrant_is_refused_and_names_the_simpler_verb() {
    let before = worktrees();
    let out = arena(&[
        "a thing",
        "--target",
        "target/arena-never-created",
        "--gate",
        "true",
        "--entrants",
        "1",
    ]);
    assert!(!out.status.success(), "one entrant must be refused:\n{}", text(&out));
    let t = text(&out);
    assert!(t.contains("at least 2 entrants"), "{t}");
    assert!(t.contains("hexa build"), "the refusal should name the simpler verb:\n{t}");
    assert_eq!(worktrees(), before, "a refused arena forked a worktree");
}

/// Zero entrants is the vacuous arena: no builds, no eliminations, and a
/// report that would say every entrant held the gate.
#[test]
fn zero_entrants_is_refused() {
    let out = arena(&["a thing", "--target", "target/arena-never", "--gate", "true", "--entrants", "0"]);
    assert!(!out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("at least 2 entrants"));
}

/// The winner is merged whole, so a target with content in it cannot receive
/// one. A half-overwritten target is a mixture no gate ever ran.
#[test]
fn a_target_that_already_has_content_is_refused() {
    let before = worktrees();
    // `hexa-git` is a real, non-empty directory in this repository.
    let out = arena(&["a thing", "--target", "hexa-git", "--gate", "true", "--entrants", "2"]);
    assert!(!out.status.success(), "a non-empty target must be refused:\n{}", text(&out));
    let t = text(&out);
    assert!(t.contains("not empty"), "{t}");
    assert!(t.contains("no gate ever ran"), "the refusal should say why it matters:\n{t}");
    assert_eq!(worktrees(), before, "a refused arena forked a worktree");
}

/// A valid arena plans, and `--plan` stops before any entrant is forked.
#[test]
fn plan_alone_forks_nothing() {
    let before = worktrees();
    let out = arena(&[
        "a thing",
        "--target",
        "target/arena-plan-only",
        "--gate",
        "cargo test",
        "--entrants",
        "3",
        "--plan",
    ]);
    assert!(out.status.success(), "a valid plan must succeed:\n{}", text(&out));
    assert!(text(&out).contains("3 entrant(s)"), "{}", text(&out));
    assert_eq!(worktrees(), before, "--plan forked a worktree");
    assert!(
        !workspace_root().join("target/arena-plan-only").exists(),
        "--plan created the target directory"
    );
}
