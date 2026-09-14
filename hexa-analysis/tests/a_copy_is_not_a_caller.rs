//! A copy of a file is not a caller of the original (ADR-2609141030).
//!
//! `hexa analyze` is the second of the two gates. Its number is what
//! `hexa scaffold --grade A` refuses a build below, and what this project
//! quotes about itself. It could be raised by putting a copy of the repository
//! inside the repository.
//!
//! Measured on one working tree, minutes apart:
//!
//! | tree | files | edges | dead exports | grade |
//! |---|---|---|---|---|
//! | with two worktrees under `.hexa/` | 448 | 2808 | 0 | A+ 100 |
//! | the same tree, worktrees gone | 150 | 940 | 4 | A+ 96 |
//!
//! The four exports were byte-identical in both. Their copies inside the
//! worktrees counted as callers, and the findings disappeared.
//!
//! This is a differential test: grade a tree, add a nested checkout, grade it
//! again, and nothing may move.

use std::path::Path;

use std::sync::Arc;

use hexa_analysis::analyzer::ArchAnalyzer;
use hexa_analysis::treesitter_adapter::TreeSitterAdapter;
use hexa_analysis::ports::ArchAnalysisPort;

/// A project with one real dead export in it.
///
/// The vacuity guard: without a finding to lose, both runs score the same and
/// the test proves nothing.
fn fixture(root: &Path) {
    std::fs::create_dir_all(root.join("src")).expect("mkdir src");
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("Cargo.toml");
    // `helper` is exported and called only from inside this module. That is a
    // dead export, and it is what a nested copy makes disappear.
    std::fs::write(
        root.join("src/lib.rs"),
        "pub mod thing;\n",
    )
    .expect("lib.rs");
    std::fs::write(
        root.join("src/thing.rs"),
        "pub fn helper() -> u32 { 7 }\n\npub fn public_api() -> u32 { helper() + 1 }\n",
    )
    .expect("thing.rs");
}

/// Copy `src` into `dst`, and mark `dst`'s parent as a separate checkout.
fn nest_a_checkout(project: &Path, at: &str) {
    let dst = project.join(at);
    std::fs::create_dir_all(dst.join("src")).expect("mkdir nested");
    // The marker that makes it a different tree.
    std::fs::write(dst.join(".git"), "gitdir: /elsewhere\n").expect(".git marker");
    for f in ["Cargo.toml", "src/lib.rs", "src/thing.rs"] {
        std::fs::copy(project.join(f), dst.join(f)).expect("copy");
    }
}

async fn dead_export_names(root: &Path) -> Vec<String> {
    let r = ArchAnalyzer::new(Arc::new(TreeSitterAdapter::new())).analyze(root).await.expect("analysis");
    let mut names: Vec<String> = r.dead_exports.iter().map(|d| d.export_name.clone()).collect();
    names.sort();
    names
}

#[tokio::test]
async fn a_nested_checkout_changes_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = dir.path();
    fixture(project);

    let before = dead_export_names(project).await;
    assert!(
        before.iter().any(|n| n == "helper"),
        "the fixture has no dead export, so this test cannot detect losing one: {before:?}"
    );

    // A worktree parked inside the project, exactly as `hexa arena` once did.
    nest_a_checkout(project, "nested-worktree");
    let after = dead_export_names(project).await;

    assert_eq!(
        before, after,
        "a nested checkout changed the analysis. Its copies are being read as this project's source."
    );
}

/// The same, for a hidden directory — how it actually happened here.
#[tokio::test]
async fn a_checkout_under_a_hidden_directory_changes_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = dir.path();
    fixture(project);
    let before = dead_export_names(project).await;
    assert!(before.iter().any(|n| n == "helper"), "vacuous fixture: {before:?}");

    nest_a_checkout(project, ".hexa/arena/entrant-1");
    let after = dead_export_names(project).await;
    assert_eq!(before, after, "a checkout under `.hexa/` changed the analysis");
}

/// And the rule itself, directly.
#[test]
fn the_walk_rule_refuses_a_nested_tree_and_a_hidden_one() {
    use hexa_analysis::analyzer::should_descend;
    let dir = tempfile::tempdir().expect("tempdir");

    let plain = dir.path().join("src");
    std::fs::create_dir_all(&plain).expect("mkdir");
    assert!(should_descend("src", &plain, &[]), "a plain source directory must be walked");

    let hidden = dir.path().join(".hexa");
    std::fs::create_dir_all(&hidden).expect("mkdir");
    assert!(!should_descend(".hexa", &hidden, &[]), "a hidden directory is not project source");

    let nested = dir.path().join("vendored");
    std::fs::create_dir_all(&nested).expect("mkdir");
    std::fs::write(nested.join(".git"), "gitdir: /elsewhere\n").expect("marker");
    assert!(!should_descend("vendored", &nested, &[]), "a nested checkout is a different tree");

    let excluded = dir.path().join("node_modules");
    std::fs::create_dir_all(&excluded).expect("mkdir");
    assert!(!should_descend("node_modules", &excluded, &[]), "the exclude list still applies");
}
