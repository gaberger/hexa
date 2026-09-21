//! A lesson belongs to the project it was learned in.
//!
//! The gate for the scope bug: `hexa memory` wrote to one file per *user*, so
//! two unrelated repositories shared one memory and `adr:0007:why` meant
//! whatever the last project to store it meant. Keys carry no project, so the
//! collision was silent — the wrong answer looked like a normal answer.
//!
//! Written against the CLI, not the store, because the store's directory
//! resolution is exactly what is on trial. `HOME` is set per child process;
//! nothing here writes this process's environment (ADR-2609131749).

use std::path::Path;
use std::process::Command;

/// A project is a directory holding `.hexa/`.
fn project(at: &Path, name: &str) -> std::path::PathBuf {
    let dir = at.join(name);
    std::fs::create_dir_all(dir.join(".hexa")).unwrap();
    std::fs::write(
        dir.join(".hexa/project.json"),
        format!("{{ \"name\": \"{name}\" }}"),
    )
    .unwrap();
    dir
}

fn hexa(cwd: &Path, home: &Path, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(args)
        .env("HOME", home)
        .env_remove("HEXA_HOME")
        .env_remove("HEXA_PROJECT_ROOT")
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "hexa {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn keys(json: &str) -> Vec<String> {
    serde_json::from_str::<serde_json::Value>(json)
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["key"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_decision_stored_in_one_project_is_invisible_in_another() {
    let home = tempfile::tempdir().unwrap();
    let a = project(home.path(), "alpha");
    let b = project(home.path(), "beta");

    hexa(
        &a,
        home.path(),
        &["memory", "store", "adr:0007:why", "Postgres over SQLite: concurrent writers"],
    );

    assert_eq!(
        keys(&hexa(&a, home.path(), &["memory", "list", "--json"])),
        vec!["adr:0007:why".to_string()],
        "the project that learned it still reads it"
    );
    assert!(
        keys(&hexa(&b, home.path(), &["memory", "list", "--json"])).is_empty(),
        "an unrelated project must not see it"
    );
    assert!(
        hexa(&b, home.path(), &["memory", "get", "adr:0007:why"]).contains("not found"),
        "and must not resolve the key"
    );

    assert!(a.join(".hexa/memory.jsonl").is_file(), "the store lives in the project");
    assert!(
        !home.path().join(".hexa/memory.jsonl").exists(),
        "and not in the user's home"
    );
}

#[test]
fn a_subdirectory_reads_the_store_of_the_project_above_it() {
    let home = tempfile::tempdir().unwrap();
    let a = project(home.path(), "alpha");
    let nested = a.join("crates/deep/src");
    std::fs::create_dir_all(&nested).unwrap();

    hexa(&a, home.path(), &["memory", "store", "lesson:trace", "trace consumers first"]);

    assert_eq!(
        keys(&hexa(&nested, home.path(), &["memory", "list", "--json"])),
        vec!["lesson:trace".to_string()],
        "the project root is found by walking up, not by cwd alone"
    );
}

#[test]
fn outside_a_project_the_user_store_is_used_and_a_project_does_not_read_it() {
    let home = tempfile::tempdir().unwrap();
    let loose = home.path().join("elsewhere");
    std::fs::create_dir_all(&loose).unwrap();

    hexa(&loose, home.path(), &["memory", "store", "lesson:global", "no project here"]);
    assert!(
        home.path().join(".hexa/memory.jsonl").is_file(),
        "with no project, the store falls back to ~/.hexa"
    );

    let a = project(home.path(), "alpha");
    assert!(
        keys(&hexa(&a, home.path(), &["memory", "list", "--json"])).is_empty(),
        "a project does not inherit the user store"
    );
}

#[test]
fn the_global_flag_reaches_the_user_store_from_inside_a_project() {
    let home = tempfile::tempdir().unwrap();
    let a = project(home.path(), "alpha");

    hexa(&a, home.path(), &["memory", "store", "lesson:project", "mine"]);
    hexa(&a, home.path(), &["memory", "--global", "store", "lesson:shared", "everyone's"]);

    assert_eq!(
        keys(&hexa(&a, home.path(), &["memory", "list", "--json"])),
        vec!["lesson:project".to_string()],
        "the project store is still the default"
    );
    assert_eq!(
        keys(&hexa(&a, home.path(), &["memory", "list", "--json", "--global"])),
        vec!["lesson:shared".to_string()],
        "--global is the explicit shared scope, and only that"
    );
}
