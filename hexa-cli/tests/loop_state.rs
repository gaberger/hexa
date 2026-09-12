//! `hexa loop` records where a project's work stands, in hexa memory.

use std::process::Command;

fn hexa() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hexa"))
}

#[test]
fn the_loop_is_recorded_shown_and_cleared() {
    let home = tempfile::tempdir().unwrap();
    let proj = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        let out = hexa()
            .args(args)
            .env("HEXA_HOME", home.path())
            .current_dir(proj.path())
            .output()
            .expect("run hexa");
        String::from_utf8_lossy(&out.stdout).to_string()
    };
    assert!(run(&["loop"]).contains("nothing recorded"));
    run(&["loop", "gate", "cargo test --test add"]);
    run(&["loop", "adr", "ADR-2609121400"]);
    let shown = run(&["loop"]);
    assert!(shown.contains("gate cargo test --test add"), "{shown}");
    assert!(shown.contains("ADR ADR-2609121400"), "{shown}");
    assert!(shown.contains("stage gate"), "{shown}");
    run(&["loop", "stage", "build"]);
    assert!(run(&["loop"]).contains("stage build"));
    run(&["loop", "clear"]);
    assert!(run(&["loop"]).contains("nothing recorded"));
}
