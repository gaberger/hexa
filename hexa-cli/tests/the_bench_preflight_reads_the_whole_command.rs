//! The bench pre-flight (ADR-2611221646 §2) checks every program an evidence
//! command runs: leading `NAME=value` assignments are not programs, shell
//! builtins are always present, and each command after `&&`, `||`, `;` or `|`
//! is checked too.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};

fn write_fixture(corpus: &Path, id: &str, evidence: &str) {
    // The format hexa bench reads: <corpus>/<id>.json, every field present.
    let fx = serde_json::json!({
        "id": id,
        "status": "verified",
        "tier": "T1",
        "instruction": "Make the failing test pass.",
        "target_file": "src/lib.ts",
        "oracle": { "command": evidence }
    });
    fs::write(corpus.join(format!("{id}.json")), serde_json::to_string_pretty(&fx).unwrap()).unwrap();
}

#[test]
fn every_command_in_the_chain_is_checked_and_assignments_and_builtins_are_not_programs() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("project");
    let corpus = project.join("fixtures");
    let bin = tmp.path().join("bin");
    let home = tmp.path().join("home");
    for d in [&project, &corpus, &bin, &home] {
        fs::create_dir_all(d).unwrap();
    }
    let marker = tmp.path().join("ran");
    let stub = bin.join("present-runner");
    fs::write(&stub, format!("#!/bin/sh\n: > '{}'\nexit 0\n", marker.display())).unwrap();
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();

    write_fixture(&corpus, "fx-cd", "cd sub && export X=1 && present-runner check");
    write_fixture(&corpus, "fx-env", "RUST_LOG=/tmp/x FOO=bar present-runner check 2>&1 | present-runner");
    write_fixture(&corpus, "fx-test", "[ -d . ] && test -n x; true");
    write_fixture(&corpus, "fx-chain", "present-runner a && chainmissingqq b");
    write_fixture(&corpus, "fx-pipe", "present-runner a | pipemissingqq");
    write_fixture(&corpus, "fx-semi", "true; semimissingqq");

    let out = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(["bench", "agentic", "--model", "test-model-does-not-exist", "--corpus"])
        .arg(&corpus)
        .args(["--arms", "react,fast"])
        .current_dir(&project)
        .env("PATH", &bin)
        .env("HOME", &home)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let all = format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(!out.status.success(), "bench should refuse:\n{all}");
    for (prog, fx) in [("chainmissingqq", "fx-chain"), ("pipemissingqq", "fx-pipe"), ("semimissingqq", "fx-semi")] {
        assert!(all.contains(prog), "refusal does not name {prog}:\n{all}");
        assert!(all.contains(fx), "refusal does not name {fx}:\n{all}");
    }
    for fx in ["fx-cd", "fx-env", "fx-test"] {
        assert!(!all.contains(fx), "refusal names {fx}, whose programs all resolve:\n{all}");
    }
    assert!(!all.contains("RUST_LOG"), "an assignment was taken for a program:\n{all}");
    assert!(!marker.exists(), "a fixture ran its evidence before the refusal");
}
