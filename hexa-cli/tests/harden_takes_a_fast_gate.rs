//! `hexa harden --fast-gate <cmd>` and `--batch <N>` exist, and say what they
//! do (ADR-2610160756 §1–§3). Written before the flags.

use std::process::Command;

fn help() -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(["harden", "--help"])
        .output()
        .expect("run hexa harden --help");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn harden_offers_a_fast_gate_with_touched_paths() {
    let h = help();
    assert!(h.contains("--fast-gate <FAST_GATE>"), "{h}");
    assert!(h.contains("{touched}"), "the help says how to name the fix's paths: {h}");
}

#[test]
fn harden_offers_a_batch_size() {
    let h = help();
    assert!(h.contains("--batch <BATCH>"), "{h}");
}

#[test]
fn a_batch_without_a_fast_gate_is_refused() {
    // --batch sizes the batches a fast gate creates; alone it would be a
    // flag that silently does nothing.
    // In an empty directory with no model reachable, so a missing refusal
    // fails for an unrelated reason instead of starting a real hunt.
    let tmp = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(["harden", "src/x.rs", "--gate", "true", "--batch", "3"])
        .current_dir(tmp.path())
        .env("HOME", tmp.path())
        .env("HEXA_PROJECT_ROOT", tmp.path())
        .env("HEXA_CLAUDE_BINARY", "/bin/false")
        .output()
        .expect("run hexa harden");
    assert!(!out.status.success());
    let text = String::from_utf8_lossy(&out.stderr).into_owned() + &String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("--fast-gate"), "{text}");
}
