//! `hexa loop` records where a project's work stands, in `.hexa/loop.json`,
//! so it travels with the branch. The ADR it points at must exist.

use std::process::Command;

fn hexa() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hexa"))
}

#[test]
fn the_loop_lives_in_the_repo_and_points_at_a_real_adr() {
    let proj = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(proj.path().join(".hexa")).unwrap();
    std::fs::create_dir_all(proj.path().join("docs/adrs")).unwrap();
    std::fs::write(proj.path().join("docs/adrs/ADR-0001-first.md"), "# ADR-0001: first\n").unwrap();
    let run = |args: &[&str]| {
        let out = hexa().args(args).current_dir(proj.path()).output().expect("run hexa");
        (out.status.success(), String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr))
    };
    assert!(run(&["loop"]).1.contains("nothing recorded"));

    let (ok, text) = run(&["loop", "adr", "ADR-0009"]);
    assert!(!ok && text.contains("no ADR-0009 in docs/adrs/"), "{text}");

    assert!(run(&["loop", "adr", "ADR-0001"]).0);
    assert!(run(&["loop", "gate", "cargo test --test add"]).0);
    let file = proj.path().join(".hexa/loop.json");
    assert!(file.is_file(), "the loop is a file in the repo");
    let state: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(state["adr"], "ADR-0001");
    assert_eq!(state["gate"], "cargo test --test add");
    assert_eq!(state["stage"], "gate");

    let shown = run(&["loop"]).1;
    assert!(shown.contains("ADR ADR-0001") && shown.contains("gate cargo test --test add"), "{shown}");
    assert!(run(&["loop", "clear"]).0);
    assert!(!file.exists());
}

#[test]
fn outside_a_hexa_project_nothing_is_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let out = hexa().args(["loop", "gate", "make test"]).current_dir(dir.path()).output().unwrap();
    assert!(!out.status.success());
    assert!(!dir.path().join(".hexa").exists());
}

#[test]
fn the_checklist_is_checked_off_and_advances() {
    let proj = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(proj.path().join(".hexa")).unwrap();
    let run = |args: &[&str]| {
        let out = hexa().args(args).current_dir(proj.path()).output().expect("run hexa");
        (out.status.success(), String::from_utf8_lossy(&out.stdout).to_string())
    };
    assert!(run(&["loop", "task", "add", "IOS classic parser"]).0);
    assert!(run(&["loop", "task", "add", "NX-OS parser"]).0);
    assert!(run(&["loop", "task", "add", "JunOS braces parser"]).0);
    let (ok, shown) = run(&["loop"]);
    assert!(ok);
    assert!(shown.contains("[>] 1 IOS classic parser"), "{shown}");
    assert!(shown.contains("[ ] 2 NX-OS parser"), "{shown}");
    assert!(shown.contains("0 of 3 done"), "{shown}");
    assert!(shown.contains("tasks 0/3") && shown.contains("doing 1 IOS classic parser"), "{shown}");

    run(&["loop", "task", "done", "1"]);
    let shown = run(&["loop"]).1;
    assert!(shown.contains("[x] 1 IOS classic parser") && shown.contains("[>] 2 NX-OS parser"), "{shown}");
    assert!(shown.contains("1 of 3 done"), "{shown}");

    run(&["loop", "task", "start", "3"]);
    let shown = run(&["loop"]).1;
    assert!(shown.contains("[ ] 2 NX-OS parser") && shown.contains("[>] 3 JunOS braces parser"), "{shown}");

    let (ok, text) = run(&["loop", "task", "done", "9"]);
    assert!(!ok || text.contains("no step 9"));

    let state: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(proj.path().join(".hexa/loop.json")).unwrap()).unwrap();
    assert_eq!(state["tasks"].as_array().unwrap().len(), 3);
    assert_eq!(state["tasks"][0]["status"], "done");
}
