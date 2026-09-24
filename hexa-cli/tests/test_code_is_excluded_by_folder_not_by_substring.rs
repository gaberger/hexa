//! What the grade leaves out is decided by whole path segments.
//!
//! The built-in exclusions were substrings: `dist` left out
//! `src/domain/distance.ts`, `tests/` left out `src/contests/`, and `test/` —
//! where TypeScript projects keep their tests — was not excluded at all. Under
//! the coverage ceiling (ADR-2609241707) that last one costs grade: test helpers
//! were counted as source with no layer, capping a real project at 84.

use std::path::Path;
use std::process::Command;

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn analyze(root: &Path) -> serde_json::Value {
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(["analyze", ".", "--json"])
        .current_dir(root)
        .env("HOME", home.path())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}:\n{text}"))
}

fn ts_project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", "{\"name\": \"fx\", \"type\": \"module\"}\n");
    write(r, "src/domain/order.ts", "export interface Order { id: string }\n");
    write(r, "src/ports/store.ts", "import type { Order } from '../domain/order.js';\nexport interface StorePort { save(o: Order): void }\n");
    d
}

fn files_graded(v: &serde_json::Value) -> Vec<String> {
    let c = &v["coverage"];
    let mut all: Vec<String> = c["unclassified"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(String::from))
        .collect();
    all.sort();
    all
}

#[test]
fn a_test_folder_is_not_graded() {
    let d = ts_project();
    write(d.path(), "test/helpers.ts", "export const h = 1;\n");
    write(d.path(), "src/__tests__/order.ts", "export const t = 1;\n");
    let v = analyze(d.path());
    assert!(files_graded(&v).is_empty(), "test code counted as source: {}", v["coverage"]);
    assert_eq!(v["coverage"]["total"], 2, "{}", v["coverage"]);
}

#[test]
fn a_name_that_merely_contains_an_excluded_word_is_graded() {
    // The controls: `distance` is not `dist/`, `contests` is not `tests/`,
    // `latest` is not `test/`.
    let d = ts_project();
    write(d.path(), "src/domain/distance.ts", "export const km = 1;\n");
    write(d.path(), "src/domain/contests/cup.ts", "export const cup = 1;\n");
    write(d.path(), "src/domain/latest/news.ts", "export const n = 1;\n");
    let v = analyze(d.path());
    assert_eq!(v["coverage"]["total"], 5, "all five source files are graded: {}", v["coverage"]);
}
