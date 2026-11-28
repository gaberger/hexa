//! `hexa graph` reads JavaScript as `hexa analyze` does (ADR-2611270420).
//!
//! The graph is the excision oracle: `hexa graph consumers <path>` says whether a
//! file can be deleted. A JavaScript file that another file `require()`s, or a
//! `.mjs` file, was not in the graph's picture, so it answered "SAFE TO REMOVE"
//! for a file in use. That is the failure the oracle exists to prevent.
#![allow(clippy::all)]

use std::path::Path;
use std::process::Command;

fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    std::fs::write(r.join("package.json"), "{\"name\":\"g\",\"type\":\"module\"}\n").unwrap();
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
    let b = hexa(r, &["graph", "build"]);
    assert!(b.0 == 0, "graph build: {}", b.1);
    d
}

fn hexa(r: &Path, args: &[&str]) -> (i32, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_hexa")).args(args).current_dir(r).env("HOME", r.join(".home")).output().expect("run hexa");
    (o.status.code().unwrap_or(-1), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
}

/// The files `consumers <path>` lists as importers, and whether it calls the file safe to remove.
fn consumers(d: &tempfile::TempDir, path: &str) -> (Vec<String>, bool, String) {
    let (_, out) = hexa(d.path(), &["graph", "consumers", path]);
    let files = out.lines().filter_map(|l| l.trim().strip_prefix('•')).map(|l| l.trim().to_string()).collect();
    (files, out.contains("SAFE TO REMOVE"), out)
}

const TARGET: (&str, &str) = ("src/adapters/secondary/target.js", "module.exports = { t: 1 };\n");

#[test]
fn a_required_file_is_not_safe_to_remove() {
    let d = project(&[("src/domain/req.js", "const t = require(\"../adapters/secondary/target.js\");\nmodule.exports = { r: t };\n"), TARGET]);
    let (files, safe, out) = consumers(&d, TARGET.0);
    assert!(!safe, "a file another file require()s was called safe to remove:\n{out}");
    assert!(files.iter().any(|f| f == "src/domain/req.js"), "{out}");
}

#[test]
fn a_require_inside_a_function_counts() {
    let d = project(&[("src/domain/lazy.js", "function load() { return require(\"../adapters/secondary/target.js\"); }\nmodule.exports = { load };\n"), TARGET]);
    let (files, _, out) = consumers(&d, TARGET.0);
    assert!(files.iter().any(|f| f == "src/domain/lazy.js"), "{out}");
}

#[test]
fn a_dynamic_import_with_a_literal_counts() {
    let d = project(&[
        ("src/domain/lazy.js", "export async function load() { return import(\"../adapters/secondary/target.js\"); }\n"),
        TARGET,
    ]);
    let (files, safe, out) = consumers(&d, TARGET.0);
    assert!(!safe && files.iter().any(|f| f == "src/domain/lazy.js"), "an import() of a file is a use of it:\n{out}");
}

#[test]
fn a_non_literal_require_makes_no_edge() {
    let d = project(&[
        ("src/domain/dyn.js", "const n = \"target\";\nconst t = require(\"../adapters/secondary/\" + n);\nmodule.exports = { t };\n"),
        ("src/domain/two.js", "const t = require(\"../adapters/secondary/target.js\", extra);\nmodule.exports = { t };\n"),
        ("src/domain/lit.js", "const t = require(\"../adapters/secondary/target.js\");\nmodule.exports = { t };\n"),
        TARGET,
    ]);
    let (files, _, out) = consumers(&d, TARGET.0);
    assert!(files.iter().any(|f| f == "src/domain/lit.js"), "control: the literal require is an edge:\n{out}");
    assert!(!files.iter().any(|f| f == "src/domain/dyn.js"), "a concatenated require is not a literal:\n{out}");
    assert!(!files.iter().any(|f| f == "src/domain/two.js"), "require(\"x\", y) is not require(\"x\"):\n{out}");
}

#[test]
fn mjs_and_cjs_files_are_in_the_graph() {
    let d = project(&[
        ("src/adapters/secondary/mod.mjs", "export const m = 1;\n"),
        ("src/adapters/secondary/old.cjs", "module.exports = { o: 1 };\n"),
        ("src/domain/a.mjs", "import { m } from \"../adapters/secondary/mod.mjs\";\nexport const a = m;\n"),
        ("src/domain/b.cjs", "const o = require(\"../adapters/secondary/old.cjs\");\nmodule.exports = { b: o };\n"),
    ]);
    let (files, safe, out) = consumers(&d, "src/adapters/secondary/mod.mjs");
    assert!(!out.contains("not found in the graph"), "{out}");
    assert!(!safe && files.iter().any(|f| f == "src/domain/a.mjs"), "{out}");
    let (files, safe, out) = consumers(&d, "src/adapters/secondary/old.cjs");
    assert!(!safe && files.iter().any(|f| f == "src/domain/b.cjs"), "{out}");
}

#[test]
fn an_extensionless_specifier_finds_any_of_the_six_extensions_and_an_index() {
    let d = project(&[
        ("src/adapters/secondary/one.mjs", "export const one = 1;\n"),
        ("src/adapters/secondary/two/index.cjs", "module.exports = { two: 2 };\n"),
        ("src/domain/uses.js", "import { one } from \"../adapters/secondary/one\";\nimport two from \"../adapters/secondary/two\";\nexport const u = [one, two];\n"),
    ]);
    let (files, _, out) = consumers(&d, "src/adapters/secondary/one.mjs");
    assert!(files.iter().any(|f| f == "src/domain/uses.js"), "./one names one.mjs:\n{out}");
    let (files, _, out) = consumers(&d, "src/adapters/secondary/two/index.cjs");
    assert!(files.iter().any(|f| f == "src/domain/uses.js"), "./two names two/index.cjs:\n{out}");
}

#[test]
fn a_dot_js_specifier_names_the_ts_file_when_there_is_one() {
    let d = project(&[
        ("src/adapters/secondary/both.ts", "export const b = 1;\n"),
        ("src/adapters/secondary/both.js", "export const b = 2;\n"),
        ("src/domain/uses.ts", "import { b } from \"../adapters/secondary/both.js\";\nexport const u = b;\n"),
    ]);
    let (ts, _, out) = consumers(&d, "src/adapters/secondary/both.ts");
    assert!(ts.iter().any(|f| f == "src/domain/uses.ts"), "the .ts file wins the tie:\n{out}");
    let (js, _, out) = consumers(&d, "src/adapters/secondary/both.js");
    assert!(!js.iter().any(|f| f == "src/domain/uses.ts"), "{out}");
}
