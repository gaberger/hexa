//! ADR-2611251938 §1: ungraded Python has no findings from the project's rules
//! file either: neither an `[[import_policy]]` nor a text rule.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

fn analyze(root: &Path) -> String {
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(["analyze", "."])
        .current_dir(root)
        .env("HOME", home.path())
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn an_import_policy_does_not_judge_ungraded_python() {
    let dir = tempfile::Builder::new().prefix("hexaproj-").tempdir().unwrap();
    let root = dir.path();
    write(root, "Cargo.toml", "[package]\nname = \"p\"\nversion = \"0.1.0\"\n");
    write(root, "src/lib.rs", "mod domain;\n");
    write(root, "src/domain/mod.rs", "fn _rule() {}\n");
    write(
        root,
        ".hexa/ADR-rules.toml",
        "[[import_policy]]\nadr = \"ADR-1\"\nid = \"domain-is-pure\"\nlayer = \"domain\"\nseverity = \"error\"\nmessage = \"the domain does no I/O\"\ndeny = [\"socket\"]\n",
    );
    write(root, "src/domain/zq_wire.py", "import socket\n");

    let report = analyze(root);
    assert!(!report.contains("zq_wire"), "{report}");

    write(root, "requirements.txt", "requests\n");
    let report = analyze(root);
    assert!(report.contains("zq_wire"), "the policy applies once Python is graded:\n{report}");
}

#[test]
fn a_text_rule_does_not_judge_ungraded_python() {
    let dir = tempfile::Builder::new().prefix("hexaproj-").tempdir().unwrap();
    let root = dir.path();
    write(root, "Cargo.toml", "[package]\nname = \"p\"\nversion = \"0.1.0\"\n");
    write(root, "src/lib.rs", "mod domain;\n");
    write(root, "src/domain/mod.rs", "fn _rule() {}\n");
    write(
        root,
        ".hexa/ADR-rules.toml",
        "[[adr_rules]]\nadr = \"ADR-1\"\nid = \"no-zq\"\nmessage = \"no zq paths\"\nseverity = \"error\"\nfile_patterns = [\".py\"]\nviolation_patterns = [\"/zq/home\"]\n",
    );
    write(root, "src/tools/zq_gen.py", "OUT = \"/zq/home\"\n");

    let report = analyze(root);
    assert!(!report.contains("zq_gen"), "{report}");
    assert!(report.contains("score 100/100"), "{report}");

    write(root, "requirements.txt", "requests\n");
    let report = analyze(root);
    assert!(report.contains("zq_gen"), "the rule applies once Python is graded:\n{report}");
}
