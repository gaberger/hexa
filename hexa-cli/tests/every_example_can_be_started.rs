//! A game you cannot play is not a game.
//!
//! hexa ships a skill, `hexa-project-output`, whose first line is: "Every hexa
//! project created by an agent must include README.md and a startup script."
//! It is prose, and prose cannot fail.
//!
//! When this test was written, all six examples in this repository passed their
//! tests and **not one of them had an entry point**. Three are named `game-*`.
//! `cargo test` was green for every one and there was no way to play any of
//! them, because each is a library and a test suite wearing a game's name.
//!
//! That is this project's own headline lesson, failed by its own output:
//! "It compiles" is not "it works" — can a user actually start the thing?

use std::path::{Path, PathBuf};

fn examples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("workspace root").join("examples")
}

fn examples() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(examples_dir())
        .expect("examples dir")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    out.sort();
    out
}

fn name(dir: &Path) -> String {
    dir.file_name().unwrap().to_string_lossy().to_string()
}

/// Is there anything here a person could run?
///
/// A Rust binary, a Go `main`, or a script/`package.json` entry that starts
/// something. A library with passing tests is not a thing you can run.
fn has_entry_point(dir: &Path) -> bool {
    if dir.join("src/main.rs").is_file() || dir.join("src/bin").is_dir() {
        return true;
    }
    if std::fs::read_to_string(dir.join("Cargo.toml")).is_ok_and(|t| t.contains("[[bin]]")) {
        return true;
    }
    // Go's own layout puts commands under `cmd/<name>/main.go`, so this walks
    // rather than reading one directory. The first version did not, and called
    // a game unplayable that had just been played.
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                let skip = p.file_name().is_some_and(|n| n == "target" || n == "node_modules");
                if !skip {
                    stack.push(p);
                }
            } else if p.extension().is_some_and(|x| x == "go")
                && std::fs::read_to_string(&p).is_ok_and(|t| t.contains("func main"))
            {
                return true;
            }
        }
    }
    if let Ok(pkg) = std::fs::read_to_string(dir.join("package.json")) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&pkg) {
            if v.get("scripts").and_then(|s| s.get("start")).is_some() {
                return true;
            }
        }
    }
    false
}

/// A file that starts it, for someone who does not want to read the README.
fn has_start_script(dir: &Path) -> bool {
    ["run.sh", "start.sh", "justfile", "Makefile"].iter().any(|f| dir.join(f).is_file())
}

/// Every example says what it is.
#[test]
fn every_example_has_a_readme() {
    let found = examples();
    assert!(found.len() >= 3, "found only {} examples; the walker is broken", found.len());
    let missing: Vec<String> =
        found.iter().filter(|d| !d.join("README.md").is_file()).map(|d| name(d)).collect();
    assert!(
        missing.is_empty(),
        "{} example(s) ship no README.md, so nobody can tell what they are:\n  {}",
        missing.len(),
        missing.join("\n  ")
    );
}

/// A `game-*` example must be playable. This is not a style rule.
#[test]
fn every_game_example_can_actually_be_played() {
    let games: Vec<PathBuf> =
        examples().into_iter().filter(|d| name(d).starts_with("game-")).collect();
    assert!(games.len() >= 3, "found only {} game examples; the filter is broken", games.len());

    let unplayable: Vec<String> = games
        .iter()
        .filter(|d| !has_entry_point(d))
        .map(|d| format!("{} — library and tests, no way to start it", name(d)))
        .collect();
    assert!(
        unplayable.is_empty(),
        "{} game(s) cannot be played:\n  {}\n\
         Passing tests are not the same as a game. Add a binary entry point.",
        unplayable.len(),
        unplayable.join("\n  ")
    );
}

/// And a one-command way in, for every game.
#[test]
fn every_game_example_ships_a_start_script() {
    let games: Vec<PathBuf> =
        examples().into_iter().filter(|d| name(d).starts_with("game-")).collect();
    assert!(games.len() >= 3, "found only {} game examples; the filter is broken", games.len());
    let missing: Vec<String> =
        games.iter().filter(|d| !has_start_script(d)).map(|d| name(d)).collect();
    assert!(
        missing.is_empty(),
        "{} game(s) have no run.sh, start.sh, justfile or Makefile:\n  {}",
        missing.len(),
        missing.join("\n  ")
    );
}
