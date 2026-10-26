//! `--grade` is a letter floor, and a letter can hide a violation (issue #20,
//! ADR-2610261420).
//!
//! One boundary violation costs 10 points: 100 − 10 = 90, which is an A. So
//! `hexa analyze . --grade A` passed a tree with a violation in it, silently,
//! while the README showed it as the gate. The floor keeps its meaning — a
//! reader who wants C to pass says so — but a pass that let violations through
//! now says so, and names the flag that fails on any: `--exit-code`.
//!
//! The gate drives the binary, because the silence was in what the binary
//! printed, not in the scoring function.

use std::process::{Command, Output};

fn hexa() -> std::path::PathBuf {
    let mut p = std::env::current_exe().expect("test exe");
    p.pop();
    p.pop();
    p.push("hexa");
    p
}

/// A minimal Rust tree. With `violate`, a secondary adapter imports the domain
/// directly — one boundary violation, nothing else.
fn tree(violate: bool) -> tempfile::TempDir {
    let d = tempfile::tempdir().expect("tempdir");
    let r = d.path();
    for dir in ["src/domain", "src/ports", "src/adapters/secondary"] {
        std::fs::create_dir_all(r.join(dir)).unwrap();
    }
    std::fs::write(r.join("Cargo.toml"), "[package]\nname=\"gt\"\nversion=\"0.1.0\"\nedition=\"2021\"\n").unwrap();
    std::fs::write(r.join("src/domain/mod.rs"), "pub struct Order;\n").unwrap();
    std::fs::write(r.join("src/ports/mod.rs"), "pub use crate::domain::Order;\npub trait Store { fn put(&self, o: Order); }\n")
        .unwrap();
    let import = if violate { "use crate::domain::Order;" } else { "use crate::ports::{Order, Store};" };
    let body = if violate {
        "pub struct Pg;\nimpl crate::ports::Store for Pg { fn put(&self, _o: Order) {} }\n"
    } else {
        "pub struct Pg;\nimpl Store for Pg { fn put(&self, _o: Order) {} }\n"
    };
    std::fs::write(r.join("src/adapters/secondary/mod.rs"), format!("{import}\n{body}")).unwrap();
    d
}

fn analyze(dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(hexa()).arg("analyze").arg(".").args(args).current_dir(dir).output().expect("run hexa")
}

fn text(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string() + &String::from_utf8_lossy(&o.stderr)
}

#[test]
fn a_floor_that_passes_a_violation_says_so_and_names_exit_code() {
    let t = tree(true);
    let out = analyze(t.path(), &["--grade", "A"]);
    let s = text(&out);
    assert!(s.contains("score 90/100"), "the fixture must hold exactly one violation:\n{s}");
    assert!(out.status.success(), "--grade keeps its meaning: A is not below the floor A\n{s}");
    assert!(
        s.contains("passes the floor A with 1 boundary violation") && s.contains("--exit-code"),
        "a pass that let a violation through must say so:\n{s}"
    );
}

#[test]
fn grade_with_exit_code_fails_on_one_violation() {
    let t = tree(true);
    let out = analyze(t.path(), &["--grade", "A", "--exit-code"]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
}

#[test]
fn a_clean_tree_passes_the_floor_without_a_warning() {
    let t = tree(false);
    let out = analyze(t.path(), &["--grade", "A"]);
    let s = text(&out);
    assert!(out.status.success(), "{s}");
    assert!(s.contains("score 100/100"), "the clean fixture must grade 100:\n{s}");
    assert!(!s.contains("boundary violation") || s.contains("0 boundary violations"), "{s}");
    assert!(!s.contains("passes the floor"), "no warning on a clean tree:\n{s}");
}
