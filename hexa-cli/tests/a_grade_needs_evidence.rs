//! A grade is a claim about files that were read (ADR-2609140020).
//!
//! hexa parses Rust, Go and TypeScript. Pointed at a tree in any other
//! language it scanned the files, parsed none of them, computed every
//! boundary result from an empty import graph, and awarded **A+ — 100/100**.
//! The release this test was written against does exactly that on a
//! twenty-seven file Python project.
//!
//! A score of 100 from an empty graph is not a lenient grade. It is the
//! absence of evidence printed in the shape of evidence, and `--grade A`
//! passed on it.

use std::process::Command;

fn hexa() -> &'static str {
    env!("CARGO_BIN_EXE_hexa")
}

fn scratch(tag: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "hexa-grade-{}-{}-{}",
        tag,
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn write(base: &std::path::Path, rel: &str, body: &str) {
    let p = base.join(rel);
    std::fs::create_dir_all(p.parent().expect("a parent")).expect("mkdir");
    std::fs::write(p, body).expect("write");
}

/// A hexagonal-looking tree in a language hexa has no grammar for.
fn python_tree() -> std::path::PathBuf {
    let dir = scratch("py");
    write(&dir, "src/domain/board.py", "class Board:\n    pass\n");
    write(&dir, "src/ports/store.py", "class Store:\n    pass\n");
    write(&dir, "src/usecases/play.py", "from src.adapters.db import Db\n");
    write(&dir, "src/adapters/db.py", "class Db:\n    pass\n");
    write(&dir, "src/composition/main.py", "from src.usecases.play import *\n");
    dir
}

/// The same shape in a language it does parse, so a failure to grade
/// anything at all cannot pass this file.
fn rust_tree() -> std::path::PathBuf {
    let dir = scratch("rs");
    write(&dir, "Cargo.toml", "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"2021\"\n");
    write(&dir, "src/domain/mod.rs", "pub struct Board;\n");
    write(&dir, "src/ports/mod.rs", "use crate::domain::Board;\npub trait Store { fn put(&self, b: Board); }\n");
    write(&dir, "src/usecases/mod.rs", "use crate::ports::Store;\npub fn play<S: Store>(_s: &S) {}\n");
    write(&dir, "src/lib.rs", "pub mod domain;\npub mod ports;\npub mod usecases;\n");
    dir
}

fn analyze(dir: &std::path::Path, extra: &[&str]) -> (String, bool) {
    let out = Command::new(hexa()).arg("analyze").arg(dir).args(extra).output().expect("hexa analyze ran");
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    (text, out.status.success())
}

/// The point. Files scanned, none parsed, no grade.
#[test]
fn a_tree_that_parses_to_nothing_is_not_graded() {
    let dir = python_tree();
    let (text, _) = analyze(&dir, &[]);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(text.contains("NOT GRADED"), "a tree hexa cannot parse was graded anyway:\n{text}");
    assert!(
        !text.contains("Architecture grade"),
        "NOT GRADED was printed beside a grade, which states both halves of a contradiction:\n{text}"
    );
    assert!(
        !text.contains("0 boundary violations"),
        "zero violations is a claim about files that were read:\n{text}"
    );
}

/// And the gate does not pass on it. `--grade` compared a floor against a
/// number nothing produced.
#[test]
fn the_grade_gate_fails_on_a_tree_that_parses_to_nothing() {
    let dir = python_tree();
    let (text, ok) = analyze(&dir, &["--grade", "F"]);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!ok, "`--grade F` passed on a tree that parsed to nothing:\n{text}");
}

/// The control. A tree hexa does parse is still graded, so a total failure
/// to analyse cannot make the two tests above pass.
#[test]
fn a_tree_that_parses_is_still_graded() {
    let dir = rust_tree();
    let (text, _) = analyze(&dir, &[]);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(text.contains("Architecture grade"), "a Rust tree was not graded:\n{text}");
    assert!(!text.contains("NOT GRADED"), "a Rust tree hexa parses was refused a grade:\n{text}");
}
