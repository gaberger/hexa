//! An import from one workspace package into another is an edge the grade
//! checks, in every language hexa grades.
//!
//! Before this, `hexa_core::…` in Rust, a second module's path in Go, and a
//! workspace package name in TypeScript were all treated as third-party: no
//! edge, no layer, no check. A domain file importing another crate's
//! secondary adapter scored 99 with no violation — a workspace that puts
//! each layer in its own package, the layout that should grade best, had
//! its layer crossings invisible.

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
        .unwrap_or_else(|| panic!("no boundary_violations:\n{text}"))
        .iter()
        .map(|x| (x["from_file"].as_str().unwrap_or("").to_string(), x["rule"].as_str().unwrap_or("").to_string()))
        .collect()
}

/// Two Rust crates. `core`'s domain imports `infra`'s secondary adapter;
/// `infra`'s adapter imports `core`'s port, which is allowed.
fn rust_workspace() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "Cargo.toml", "[workspace]\nmembers = [\"core\", \"infra-store\"]\n");
    write(r, "core/Cargo.toml", "[package]\nname = \"fx-core\"\nversion = \"0.1.0\"\n");
    write(r, "core/src/lib.rs", "pub mod domain;\npub mod ports;\n");
    write(r, "core/src/domain/mod.rs", "pub mod order;\n");
    write(r, "core/src/domain/order.rs", "use infra_store::adapters::secondary::disk::Disk;\npub struct Order { pub d: Disk }\n");
    write(r, "core/src/ports/mod.rs", "pub mod store;\n");
    write(r, "core/src/ports/store.rs", "pub trait StorePort {}\n");
    write(r, "infra-store/Cargo.toml", "[package]\nname = \"infra-store\"\nversion = \"0.1.0\"\n");
    write(r, "infra-store/src/lib.rs", "pub mod adapters;\n");
    write(r, "infra-store/src/adapters/mod.rs", "pub mod secondary;\n");
    write(r, "infra-store/src/adapters/secondary/mod.rs", "pub mod disk;\n");
    write(r, "infra-store/src/adapters/secondary/disk.rs", "use fx_core::ports::store::StorePort;\npub struct Disk;\nimpl StorePort for Disk {}\n");
    d
}

#[test]
fn rust_a_domain_importing_another_crates_adapter_is_a_violation() {
    let d = rust_workspace();
    let v = violations(d.path());
    assert!(
        v.iter().any(|(f, r)| f == "core/src/domain/order.rs" && r.contains("domain")),
        "{v:?}"
    );
}

#[test]
fn rust_an_adapter_importing_another_crates_port_is_allowed() {
    // The control: resolving across crates must not invent violations.
    let d = rust_workspace();
    let v = violations(d.path());
    assert!(!v.iter().any(|(f, _)| f.starts_with("infra-store/")), "{v:?}");
}

#[test]
fn go_a_domain_importing_another_modules_adapter_is_a_violation() {
    // The module's layer is declared, not spelt in its import path, so only
    // resolving `example.com/store` to `store/` can find it. (An import path
    // that happens to contain `/adapters/secondary/` is classified by
    // substring and passes without any resolution — which is how the first
    // version of this test passed before the code existed.)
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "go.work", "go 1.22\n\nuse (\n\t./core\n\t./store\n)\n");
    write(r, "core/go.mod", "module example.com/core\n\ngo 1.22\n");
    write(r, "core/domain/order.go", "package domain\n\nimport \"example.com/store\"\n\ntype Order struct{ D store.Disk }\n");
    write(r, "store/go.mod", "module example.com/store\n\ngo 1.22\n");
    write(r, "store/disk.go", "package store\n\ntype Disk struct{}\n");
    write(r, ".hexa/project.json", "{\"analyze\":{\"layers\":{\"store\":\"adapters/secondary\"}}}");
    let v = violations(r);
    assert!(v.iter().any(|(f, r)| f == "core/domain/order.go" && r.contains("domain")), "{v:?}");
}

#[test]
fn typescript_a_domain_importing_another_packages_adapter_is_a_violation() {
    // As for Go: the package's layer is declared, so `@fx/store` must be
    // resolved to `packages/store/` to be seen at all.
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", "{\"name\": \"fx\", \"private\": true, \"workspaces\": [\"packages/*\"]}\n");
    write(r, "packages/core/package.json", "{\"name\": \"@fx/core\"}\n");
    write(
        r,
        "packages/core/src/domain/order.ts",
        "import { Disk } from '@fx/store/src/disk.js';\nexport interface Order { d: Disk }\n",
    );
    write(r, "packages/store/package.json", "{\"name\": \"@fx/store\"}\n");
    write(r, "packages/store/src/disk.ts", "export class Disk {}\n");
    write(r, ".hexa/project.json", "{\"analyze\":{\"layers\":{\"packages/store\":\"adapters/secondary\"}}}");
    let v = violations(r);
    assert!(
        v.iter().any(|(f, r)| f == "packages/core/src/domain/order.ts" && r.contains("domain")),
        "{v:?}"
    );
}

#[test]
fn a_third_party_package_is_still_not_an_edge() {
    // The control for all three: only the workspace's own packages resolve.
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "Cargo.toml", "[package]\nname = \"solo\"\nversion = \"0.1.0\"\n");
    write(r, "src/lib.rs", "pub mod domain;\n");
    write(r, "src/domain/mod.rs", "use serde::Serialize;\npub struct X;\n");
    assert!(violations(r).is_empty());
}
