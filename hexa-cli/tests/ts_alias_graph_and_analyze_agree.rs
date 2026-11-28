//! The two copies of the TypeScript `paths` resolver (hexa-analysis
//! `path_normalizer::TsPaths`, hexa-graph `load_ts_paths`) must reach the
//! same files (ADR-2611221645 §4), and both must follow `extends` by the same
//! rules:
//!
//! - a tsconfig outside the project root is not followed: only project files
//!   are read;
//! - `extends` (a string, or an array since TS 5.0, later entries winning) is
//!   resolved per chain, so a base reached by two branches of a diamond gives
//!   its settings to both; a cycle still terminates;
//! - a file importing itself through an alias is never a dependency: the
//!   graph lists no file as its own consumer, and analyze reports neither a
//!   violation nor a cycle for it.
//!
//! Black box: every importing file is in the domain layer and every target in
//! a secondary adapter, so each analyze edge is a reported violation, and the
//! graph is read through `graph consumers`.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn hexa(root: &Path, args: &[&str]) -> (String, String) {
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(args)
        .current_dir(root)
        .env("HOME", home.path())
        .output()
        .unwrap();
    (String::from_utf8_lossy(&out.stdout).to_string(), String::from_utf8_lossy(&out.stderr).to_string())
}

fn analyze(root: &Path) -> (Value, String) {
    let (stdout, stderr) = hexa(root, &["analyze", ".", "--json"]);
    let v: Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("analyze --json is not JSON ({e}):\n{stdout}{stderr}"));
    (v, format!("{stdout}{stderr}"))
}

/// The files analyze reports an edge from into `target`.
fn analyze_importers(v: &Value, target: &str) -> BTreeSet<String> {
    v["analysis"]["violations"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|x| &x["edge"])
        .filter(|e| e["to_file"] == target)
        .filter_map(|e| e["from_file"].as_str().map(str::to_string))
        .collect()
}

fn graph_importers(root: &Path, target: &str) -> BTreeSet<String> {
    let (bo, be) = hexa(root, &["graph", "build"]);
    assert!(root.join(".hexa/graph/graph.json").exists(), "graph build failed:\n{bo}{be}");
    let (stdout, stderr) = hexa(root, &["graph", "consumers", target, "--json"]);
    let v: Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("graph consumers --json is not JSON ({e}):\n{stdout}{stderr}"));
    v["imported_by"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

const GET: &str = "export function get(): number { return 1; }\n";
const IMPORT_X: &str = "import { get } from \"@/x\";\nexport const a = () => get();\n";

/// For each target: analyze and the graph name the same importers, and they
/// are `want`.
fn assert_agree(root: &Path, want: &[(&str, &[&str])]) {
    let (v, all) = analyze(root);
    for (target, files) in want {
        let a = analyze_importers(&v, target);
        let g = graph_importers(root, target);
        assert_eq!(g, a, "graph consumers of {target} vs analyze edges to it:\n{all}");
        assert_eq!(a, set(files), "importers of {target}:\n{all}");
    }
}

fn layers(root: &Path) {
    write(
        root,
        ".hexa/project.json",
        r#"{"analyze":{"layers":{"src/domain":"domain","src/lib":"adapters/secondary","b":"adapters/secondary","d":"adapters/secondary"}}}"#,
    );
}

/// An `extends` that climbs above the project root is not followed, so its
/// `paths` reach nothing, in analyze and in the graph alike.
#[test]
fn an_extends_outside_the_project_root_is_not_followed_by_either() {
    let outer = tempfile::tempdir().unwrap();
    write(outer.path(), "outside.json", r#"{"compilerOptions": {"paths": {"@/*": ["src/lib/*"]}}}"#);
    let r = &outer.path().join("proj");
    write(r, "tsconfig.json", r#"{"extends": "../outside.json", "compilerOptions": {"baseUrl": "."}}"#);
    layers(r);
    write(r, "src/lib/x.ts", GET);
    write(r, "src/domain/a.ts", IMPORT_X);
    assert_agree(r, &[("src/lib/x.ts", &[])]);
}

/// A diamond: the root extends [b, c], both extend d; b sets its own paths,
/// c inherits d's. c comes later, so d's paths (through c) win. With one
/// `seen` set for the whole walk, d was skipped on the c branch and b's
/// paths won instead.
#[test]
fn a_base_shared_by_two_extends_branches_gives_its_settings_to_both() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "tsconfig.json", r#"{"extends": ["./b.json", "./c.json"]}"#);
    write(r, "b.json", r#"{"extends": "./d.json", "compilerOptions": {"paths": {"@/*": ["b/*"]}}}"#);
    write(r, "c.json", r#"{"extends": "./d.json"}"#);
    write(r, "d.json", r#"{"compilerOptions": {"paths": {"@/*": ["d/*"]}}}"#);
    layers(r);
    write(r, "b/x.ts", GET);
    write(r, "d/x.ts", GET);
    write(r, "src/domain/a.ts", IMPORT_X);
    assert_agree(r, &[("d/x.ts", &["src/domain/a.ts"]), ("b/x.ts", &[])]);
}

/// An `extends` cycle terminates, and the settings on it still apply.
#[test]
fn an_extends_cycle_terminates_in_both() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "tsconfig.json", r#"{"extends": "./loop.json"}"#);
    write(r, "loop.json", r#"{"extends": "./tsconfig.json", "compilerOptions": {"paths": {"@/*": ["src/lib/*"]}}}"#);
    layers(r);
    write(r, "src/lib/x.ts", GET);
    write(r, "src/domain/a.ts", IMPORT_X);
    assert_agree(r, &[("src/lib/x.ts", &["src/domain/a.ts"])]);
}

/// A file importing itself through an alias is not its own consumer in the
/// graph, and analyze charges it neither a violation nor a cycle; another
/// file's import of it is an edge in both.
#[test]
fn a_file_importing_itself_through_an_alias_is_no_dependency_in_either() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "tsconfig.json", r#"{"compilerOptions": {"paths": {"@/*": ["src/lib/*"]}}}"#);
    layers(r);
    write(r, "src/lib/x.ts", "import { get as g } from \"@/x\";\nexport function get(): number { return 1; }\nexport const h = () => g();\n");
    write(r, "src/domain/a.ts", IMPORT_X);
    assert_agree(r, &[("src/lib/x.ts", &["src/domain/a.ts"])]);
    let (v, all) = analyze(r);
    let cycles = v["analysis"]["circular_deps"].as_array().map(|a| a.len()).unwrap_or(0);
    assert_eq!(cycles, 0, "a self-import is not a cycle:\n{all}");
}
