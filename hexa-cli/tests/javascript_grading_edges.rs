//! Edges of ADR-2611270420 the accepted scenarios do not pin, derived from its
//! decisions (§2 a require is an edge wherever it is written, and only a literal
//! one; §3 a .js specifier names the .ts file when there is one, and a project
//! file keeps its own name; §2 TypeScript files alike).
#![allow(clippy::all)]

use std::process::Command;

use serde_json::Value;

fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    std::fs::write(r.join("package.json"), "{\"name\":\"x\",\"type\":\"module\"}\n").unwrap();
    for (rel, text) in files {
        let p = r.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    let git = |args: &[&str]| {
        let o = Command::new("git").args(["-c", "user.name=t", "-c", "user.email=t@t"]).args(args).current_dir(r).output().unwrap();
        assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    };
    git(&["init", "-q"]);
    git(&["add", "-A"]);
    git(&["commit", "-qm", "project"]);
    d
}

/// (from, to) of each boundary violation of the graded analysis.
fn boundary(d: &tempfile::TempDir) -> Vec<(String, String)> {
    let o = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(["analyze", ".", "--json"])
        .current_dir(d.path())
        .env("HOME", d.path().join(".home"))
        .output()
        .expect("run hexa");
    let v: Value = serde_json::from_slice(&o.stdout).unwrap_or_else(|e| panic!("not JSON ({e}): {}", String::from_utf8_lossy(&o.stdout)));
    v["boundary_violations"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|b| (b["from_file"].as_str().unwrap_or("").to_string(), b["to_file"].as_str().unwrap_or("").to_string()))
        .collect()
}
fn edge(b: &[(String, String)], from: &str, to: &str) -> bool {
    b.iter().any(|(f, t)| f == from && t == to)
}
fn from_file(b: &[(String, String)], from: &str) -> bool {
    b.iter().any(|(f, _)| f == from)
}

const HTTP: (&str, &str) = ("src/adapters/secondary/http.js", "export const h = 1;\n");

#[test]
fn a_require_inside_a_function_is_an_edge() {
    let d = project(&[
        ("src/domain/order.js", "function load() {\n  return require(\"../adapters/secondary/http.js\");\n}\nmodule.exports = { load };\n"),
        HTTP,
    ]);
    assert!(edge(&boundary(&d), "src/domain/order.js", "src/adapters/secondary/http.js"));
}

#[test]
fn a_literal_inside_a_larger_expression_is_not_a_require_of_that_file() {
    let d = project(&[
        ("src/domain/cat.js", "const name = \"x\";\nconst h = require(\"../adapters/secondary/http\" + name);\nmodule.exports = { h };\n"),
        ("src/domain/two.js", "const h = require(\"../adapters/secondary/http.js\", extra);\nmodule.exports = { h };\n"),
        ("src/domain/lit.js", "const h = require(\"../adapters/secondary/http.js\");\nmodule.exports = { h };\n"),
        HTTP,
    ]);
    let b = boundary(&d);
    assert!(from_file(&b, "src/domain/lit.js"), "control: the plain literal is an edge: {b:?}");
    assert!(!from_file(&b, "src/domain/cat.js"), "a concatenation is not a literal: {b:?}");
    assert!(!from_file(&b, "src/domain/two.js"), "a call with two arguments is not require(\"x\"): {b:?}");
}

#[test]
fn a_dot_js_specifier_names_the_ts_file_when_both_exist() {
    let d = project(&[
        ("src/domain/order.ts", "import { h } from \"../adapters/secondary/http.js\";\nexport const o = h;\n"),
        ("src/adapters/secondary/http.ts", "export const h = 1;\n"),
        ("src/adapters/secondary/http.js", "export const h = 2;\n"),
    ]);
    let b = boundary(&d);
    assert!(edge(&b, "src/domain/order.ts", "src/adapters/secondary/http.ts"), "the TypeScript file wins the tie: {b:?}");
    assert!(!edge(&b, "src/domain/order.ts", "src/adapters/secondary/http.js"), "{b:?}");
}

#[test]
fn a_typescript_file_that_requires_is_graded_alike() {
    let d = project(&[
        ("src/domain/order.ts", "const h = require(\"../adapters/secondary/http.js\");\nexport const o = h;\n"),
        HTTP,
    ]);
    assert!(edge(&boundary(&d), "src/domain/order.ts", "src/adapters/secondary/http.js"));
}

#[test]
fn a_typescript_file_importing_a_javascript_file_reaches_it_by_its_own_name() {
    let d = project(&[
        ("src/domain/order.ts", "import { h } from \"../adapters/secondary/http.js\";\nexport const o = h;\n"),
        HTTP,
    ]);
    let b = boundary(&d);
    assert!(edge(&b, "src/domain/order.ts", "src/adapters/secondary/http.js"), "{b:?}");
}
