//! A file at a crate's `src/` root is checked like any other.
//!
//! Two holes let a use case import an adapter unseen, and hexa graded itself
//! A+ through both:
//!
//! 1. The "a module declaring its own submodule" exemption (for
//!    `adapters/mod.rs` → `pub mod secondary;`) skipped every edge whose
//!    target sat under the importing file's *directory*. For a file directly
//!    in `src/`, that directory is the whole crate — so every import it made
//!    inside its crate was exempt.
//! 2. A layer declared for a file (`src/store.rs`) did not match the module
//!    path an import resolves to (`src/store/Disk`), so the import's target
//!    had no layer and the edge was skipped.

use std::path::Path;
use std::process::Command;

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn violations(root: &Path) -> Vec<(String, String)> {
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(["analyze", ".", "--json"])
        .current_dir(root)
        .env("HOME", home.path())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}:\n{text}"));
    v["boundary_violations"]
        .as_array()
        .unwrap_or_else(|| panic!("{text}"))
        .iter()
        .map(|x| (x["from_file"].as_str().unwrap_or("").into(), x["rule"].as_str().unwrap_or("").into()))
        .collect()
}

/// A flat crate: every module a file directly under `src/`, layers declared
/// per file. `engine.rs`, a use case, reaches straight into `store.rs`, a
/// secondary adapter.
fn flat_crate() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "Cargo.toml", "[package]\nname = \"flat\"\nversion = \"0.1.0\"\nedition = \"2021\"\n");
    write(r, "src/lib.rs", "pub mod engine;\npub mod store;\npub mod ports;\n");
    write(r, "src/ports.rs", "pub trait StorePort {}\n");
    write(r, "src/store.rs", "use crate::ports::StorePort;\npub struct Disk;\nimpl StorePort for Disk {}\n");
    write(r, "src/engine.rs", "use crate::store::Disk;\npub fn run(_d: &Disk) {}\n");
    write(
        r,
        ".hexa/project.json",
        r#"{"analyze":{"layers":{"src/engine.rs":"usecases","src/store.rs":"adapters/secondary","src/ports.rs":"ports"}}}"#,
    );
    d
}

#[test]
fn a_use_case_at_the_crate_root_importing_an_adapter_is_a_violation() {
    let d = flat_crate();
    let v = violations(d.path());
    assert!(v.iter().any(|(f, _)| f == "src/engine.rs"), "engine.rs → store.rs must be reported: {v:?}");
}

#[test]
fn an_adapter_at_the_crate_root_importing_its_port_is_allowed() {
    // The control: checking flat files must not invent violations.
    let d = flat_crate();
    let v = violations(d.path());
    assert!(!v.iter().any(|(f, _)| f == "src/store.rs"), "{v:?}");
}

#[test]
fn a_module_declaring_its_own_submodule_is_still_exempt() {
    // What the exemption is for: `adapters/mod.rs` and `adapters.rs` naming
    // their children is structure, not one adapter reaching another.
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "Cargo.toml", "[package]\nname = \"nest\"\nversion = \"0.1.0\"\nedition = \"2021\"\n");
    write(r, "src/lib.rs", "pub mod adapters;\npub mod ports;\n");
    write(r, "src/ports/mod.rs", "pub trait P {}\n");
    write(r, "src/adapters/mod.rs", "pub mod primary;\npub mod secondary;\n");
    write(r, "src/adapters/primary/mod.rs", "pub mod cli;\n");
    write(r, "src/adapters/primary/cli.rs", "use crate::ports::P;\n");
    write(r, "src/adapters/secondary.rs", "pub mod db;\n");
    write(r, "src/adapters/secondary/db.rs", "use crate::ports::P;\npub struct Db;\nimpl P for Db {}\n");
    assert!(violations(r).is_empty());
}
