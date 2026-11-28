//! ADR-2611270420: hexa grades JavaScript as it grades TypeScript.
//!
//! One test per scenario of the accepted spec, written from the scenarios and
//! run against the built binary on small projects. A negative scenario that
//! already holds today (a non-literal require makes no edge) carries a control
//! that only holds once JavaScript is graded, so it fails before the code exists
//! (the guard-test-is-a-mirror guide, fix 1).
#![allow(clippy::all)]

use std::path::Path;
use std::process::Command;

use serde_json::Value;

const RULES: &str = include_str!("../assets/templates/ADR-rules.toml");

/// A project: package.json, the given files, a git repo with everything committed.
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
        let o = Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .current_dir(r)
            .output()
            .unwrap();
        assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    };
    git(&["init", "-q"]);
    git(&["add", "-A"]);
    git(&["commit", "-qm", "project"]);
    d
}

fn hexa(r: &Path, args: &[&str]) -> (i32, String, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(args)
        .current_dir(r)
        .env("HOME", r.join(".home"))
        .output()
        .expect("run hexa");
    (
        o.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&o.stdout).to_string(),
        String::from_utf8_lossy(&o.stderr).to_string(),
    )
}

fn analyze(d: &tempfile::TempDir) -> Value {
    let (_, out, err) = hexa(d.path(), &["analyze", ".", "--json"]);
    serde_json::from_str(&out).unwrap_or_else(|e| panic!("analyze --json is not JSON ({e}):\n{out}\n{err}"))
}

/// (from, to) of every boundary violation in the graded analysis.
fn boundary(v: &Value) -> Vec<(String, String)> {
    v["boundary_violations"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|b| (b["from_file"].as_str().unwrap_or("").to_string(), b["to_file"].as_str().unwrap_or("").to_string()))
        .collect()
}
fn has(v: &[(String, String)], from: &str, to: &str) -> bool {
    v.iter().any(|(f, t)| f == from && t == to)
}
fn from(v: &[(String, String)], from: &str) -> bool {
    v.iter().any(|(f, _)| f == from)
}
/// (file, name) of every dead export.
fn dead(v: &Value) -> Vec<(String, String)> {
    v["dead_exports"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|b| (b["file"].as_str().unwrap_or("").to_string(), b["export_name"].as_str().unwrap_or("").to_string()))
        .collect()
}
fn is_dead(v: &[(String, String)], file: &str, name: &str) -> bool {
    v.iter().any(|(f, n)| f == file && n == name)
}
fn graded(v: &Value) -> u64 {
    v["coverage"]["total"].as_u64().unwrap_or(0)
}

const HTTP: (&str, &str) = ("src/adapters/secondary/http.js", "export const h = 1;\n");

/// S01: A JavaScript file in domain/ that imports an adapter is a boundary violation in the grade.
#[test]
fn s01_a_js_file_in_domain_importing_an_adapter_is_a_boundary_violation() {
    let d = project(&[("src/domain/order.js", "import { h } from \"../adapters/secondary/http.js\";\nexport const o = h;\n"), HTTP]);
    let v = analyze(&d);
    assert!(has(&boundary(&v), "src/domain/order.js", "src/adapters/secondary/http.js"), "{}", v["boundary_violations"]);
    assert_eq!(graded(&v), 2, "both JavaScript files are graded and in a layer: {}", v["coverage"]);
    assert_eq!(v["score_components"]["violations"].as_u64(), Some(1), "the grade counts it: {}", v["score_components"]);
}

/// S02: A .mjs and a .cjs file are graded source.
#[test]
fn s02_a_mjs_and_a_cjs_file_are_graded_source() {
    let d = project(&[
        ("src/domain/a.mjs", "import { h } from \"../adapters/secondary/http.js\";\nexport const a = h;\n"),
        ("src/domain/b.cjs", "import(\"../adapters/secondary/http.js\");\nmodule.exports = { b: 1 };\n"),
        HTTP,
    ]);
    let v = analyze(&d);
    let b = boundary(&v);
    assert!(from(&b, "src/domain/a.mjs"), "{b:?}");
    assert!(from(&b, "src/domain/b.cjs"), "{b:?}");
    assert_eq!(graded(&v), 3, "{}", v["coverage"]);
}

/// S03: A JavaScript file under node_modules, or one removed by analyze.exclude, is not graded.
#[test]
fn s03_node_modules_and_excluded_javascript_are_not_graded() {
    let d = project(&[
        ("node_modules/pkg/x.js", "import { h } from \"../../src/adapters/secondary/http.js\";\nexport const x = 1;\n"),
        ("src/domain/legacy.js", "import { h } from \"../adapters/secondary/http.js\";\nexport const l = 1;\n"),
        ("src/domain/order.js", "import { h } from \"../adapters/secondary/http.js\";\nexport const o = 1;\n"),
        (".hexa/project.json", "{\"analyze\":{\"exclude\":[\"src/domain/legacy.js\"]}}\n"),
        HTTP,
    ]);
    let v = analyze(&d);
    let b = boundary(&v);
    assert!(from(&b, "src/domain/order.js"), "control: a JavaScript file that is graded is reported: {b:?}");
    assert!(!from(&b, "src/domain/legacy.js"), "an excluded file is not reported: {b:?}");
    assert!(!b.iter().any(|(f, _)| f.starts_with("node_modules/")), "{b:?}");
    assert_eq!(graded(&v), 2, "only order.js and http.js are graded: {}", v["coverage"]);
}

/// S04: A CommonJS require with a string literal is an import edge.
#[test]
fn s04_a_commonjs_require_with_a_string_literal_is_an_import_edge() {
    let d = project(&[("src/domain/order.js", "const h = require(\"../adapters/secondary/http.js\");\nmodule.exports = { o: h };\n"), HTTP]);
    let v = analyze(&d);
    assert!(has(&boundary(&v), "src/domain/order.js", "src/adapters/secondary/http.js"), "{}", v["boundary_violations"]);
}

/// S05: A require whose argument is not a string literal makes no edge and is not guessed.
#[test]
fn s05_a_non_literal_require_makes_no_edge() {
    let d = project(&[
        ("src/domain/dyn.js", "const name = \"http\";\nconst dir = \"adapters\";\nconst h = require(name);\nconst k = require(\"./\" + dir + \"/x\");\nmodule.exports = { h, k };\n"),
        ("src/domain/lit.js", "const h = require(\"../adapters/secondary/http.js\");\nmodule.exports = { h };\n"),
        HTTP,
    ]);
    let (code, _, err) = hexa(d.path(), &["analyze", ".", "--json"]);
    assert_ne!(code, 2, "hexa must not fail on a dynamic require: {err}");
    let v = analyze(&d);
    let b = boundary(&v);
    assert!(has(&b, "src/domain/lit.js", "src/adapters/secondary/http.js"), "control: the literal require is an edge: {b:?}");
    assert!(!from(&b, "src/domain/dyn.js"), "the computed requires make no edge: {b:?}");
}

/// S06: A relative specifier without an extension resolves across the six extensions and to a folder's index.
#[test]
fn s06_an_extensionless_specifier_resolves_across_extensions_and_index() {
    let d = project(&[
        ("src/domain/order.js", "import { i } from \"./item\";\nimport { p } from \"./pricing\";\nexport const o = 1;\n"),
        ("src/domain/item.mjs", "import { o } from \"./order.js\";\nexport const i = o;\n"),
        ("src/domain/pricing/index.cjs", "import(\"../order.js\");\nmodule.exports = { p: 1 };\n"),
    ]);
    let v = analyze(&d);
    let cycles = v["circular_deps"].to_string();
    assert!(cycles.contains("src/domain/item.mjs"), "./item resolves to item.mjs: {cycles}");
    assert!(cycles.contains("src/domain/pricing/index.cjs"), "./pricing resolves to pricing/index.cjs: {cycles}");
}

/// S07: A .js specifier resolves to the .ts file the compiler would map it to when that file exists, else to the .js file.
#[test]
fn s07_a_js_specifier_maps_to_the_ts_file_else_the_js_file() {
    let d = project(&[
        ("src/usecases/a.ts", "import { b } from \"./b.js\";\nimport { c } from \"./c.js\";\nexport const a = 1;\n"),
        ("src/usecases/b.ts", "import { a } from \"./a.js\";\nexport const b = 1;\n"),
        ("src/usecases/c.js", "import { a } from \"./a.js\";\nexport const c = 1;\n"),
    ]);
    let v = analyze(&d);
    let cycles = v["circular_deps"].to_string();
    assert!(cycles.contains("src/usecases/b.ts"), "./b.js maps to b.ts: {cycles}");
    assert!(cycles.contains("src/usecases/c.js"), "./c.js, with no c.ts, is c.js: {cycles}");
}

/// S08: A specifier that names no project file is unchanged, and stays external.
#[test]
fn s08_a_package_specifier_stays_external() {
    let d = project(&[
        ("src/domain/pkgs.js", "const _ = require(\"lodash\");\nimport lp from \"left-pad\";\nexport const p = lp;\n"),
        ("src/domain/ctl.js", "import { h } from \"../adapters/secondary/http.js\";\nexport const c = h;\n"),
        HTTP,
    ]);
    let b = boundary(&analyze(&d));
    assert!(from(&b, "src/domain/ctl.js"), "control: JavaScript is graded: {b:?}");
    assert!(!from(&b, "src/domain/pkgs.js"), "a package name is not a project edge: {b:?}");
}

/// S09: A jsconfig.json paths map is read as a tsconfig.json one is.
#[test]
fn s09_a_jsconfig_paths_map_is_read() {
    let d = project(&[
        ("jsconfig.json", "{\"compilerOptions\":{\"paths\":{\"@/*\":[\"./src/*\"]}}}\n"),
        ("src/domain/order.js", "import { h } from \"@/adapters/secondary/http\";\nexport const o = h;\n"),
        HTTP,
    ]);
    let v = analyze(&d);
    assert!(has(&boundary(&v), "src/domain/order.js", "src/adapters/secondary/http.js"), "{}", v["boundary_violations"]);
}

/// S10: A paths alias that matches a pattern but names no project file is left as written.
#[test]
fn s10_an_alias_naming_no_project_file_is_left_as_written() {
    let d = project(&[
        ("jsconfig.json", "{\"compilerOptions\":{\"paths\":{\"@/*\":[\"./src/*\"]}}}\n"),
        ("src/domain/miss.js", "import { t } from \"@/missing/thing\";\nexport const m = t;\n"),
        ("src/domain/hit.js", "import { h } from \"@/adapters/secondary/http\";\nexport const x = h;\n"),
        HTTP,
    ]);
    let b = boundary(&analyze(&d));
    assert!(from(&b, "src/domain/hit.js"), "control: an alias that names a file resolves: {b:?}");
    assert!(!from(&b, "src/domain/miss.js"), "an alias naming nothing makes no edge: {b:?}");
}

/// S11: module.exports and exports.x define the module's exports for dead-export detection.
#[test]
fn s11_commonjs_exports_are_exports() {
    let d = project(&[
        ("src/usecases/util.js", "const used = 1;\nconst unused = 2;\nmodule.exports = { used, unused };\nexports.extra = 1;\n"),
        ("src/usecases/app.js", "const { used } = require(\"./util\");\nmodule.exports = { app: used };\n"),
    ]);
    let dd = dead(&analyze(&d));
    assert!(is_dead(&dd, "src/usecases/util.js", "unused"), "{dd:?}");
    assert!(is_dead(&dd, "src/usecases/util.js", "extra"), "{dd:?}");
    assert!(!is_dead(&dd, "src/usecases/util.js", "used"), "{dd:?}");
}

/// S12: An export another file imports is not dead, in any spelling.
#[test]
fn s12_an_imported_export_is_not_dead_in_any_spelling() {
    let d = project(&[
        ("src/usecases/util.js", "const a = 1;\nmodule.exports = { a };\nexport const b = 1;\n"),
        ("src/usecases/app.js", "const { a } = require(\"./util\");\nmodule.exports = { app: a };\n"),
        ("src/usecases/main.js", "import { b } from \"./util.js\";\nexport const m = b;\n"),
        ("src/usecases/stray.js", "module.exports = { stray: 1 };\n"),
    ]);
    let dd = dead(&analyze(&d));
    assert!(is_dead(&dd, "src/usecases/stray.js", "stray"), "control: a CommonJS export nothing imports is dead: {dd:?}");
    assert!(!is_dead(&dd, "src/usecases/util.js", "a"), "{dd:?}");
    assert!(!is_dead(&dd, "src/usecases/util.js", "b"), "{dd:?}");
}

/// S13: Test files in the four extensions and under __tests__ are test files, and entry points are not dead.
#[test]
fn s13_test_files_and_entry_points() {
    let bad = "import { h } from \"../adapters/secondary/http.js\";\nexport const t = h;\n";
    let d = project(&[
        ("src/usecases/x.test.js", bad),
        ("src/usecases/y.spec.mjs", bad),
        ("src/usecases/__tests__/z.cjs", "import { h } from \"../../adapters/secondary/http.js\";\nmodule.exports = { z: h };\n"),
        ("src/usecases/index.js", "export const entry = 1;\n"),
        ("src/usecases/other.js", "import { h } from \"../adapters/secondary/http.js\";\nexport const unused = h;\n"),
        HTTP,
    ]);
    let v = analyze(&d);
    let b = boundary(&v);
    assert!(from(&b, "src/usecases/other.js"), "control: ordinary JavaScript source is graded: {b:?}");
    for t in ["src/usecases/x.test.js", "src/usecases/y.spec.mjs", "src/usecases/__tests__/z.cjs"] {
        assert!(!from(&b, t), "{t} is a test file, not graded source: {b:?}");
    }
    assert!(!is_dead(&dead(&v), "src/usecases/index.js", "entry"), "an entry point's export is not dead");
}

/// S14: A file whose name merely contains test is not a test file, and an ordinary file's unused export is dead.
#[test]
fn s14_a_name_containing_test_is_not_a_test_file() {
    let bad = "import { h } from \"../adapters/secondary/http.js\";\nexport const v = h;\n";
    let d = project(&[
        ("src/usecases/latest.js", bad),
        ("src/usecases/contest.js", bad),
        ("src/usecases/helper.js", "export const nothing = 1;\n"),
        HTTP,
    ]);
    let v = analyze(&d);
    let b = boundary(&v);
    assert!(from(&b, "src/usecases/latest.js"), "{b:?}");
    assert!(from(&b, "src/usecases/contest.js"), "{b:?}");
    assert!(is_dead(&dead(&v), "src/usecases/helper.js", "nothing"), "{:?}", dead(&v));
}

fn policy_violations(v: &Value, file: &str) -> usize {
    v["adr_compliance"]["violations"].as_array().map(|a| a.iter().filter(|x| x["file"] == file).count()).unwrap_or(0)
}

/// S15: An fs or node: import or require in domain/ is a policy violation under the shipped deny list.
#[test]
fn s15_fs_and_node_imports_in_the_domain_are_policy_violations() {
    let d = project(&[
        (".hexa/ADR-rules.toml", RULES),
        ("src/domain/order.js", "const fs = require(\"fs\");\nimport cp from \"node:child_process\";\nexport const o = 1;\n"),
    ]);
    let v = analyze(&d);
    assert!(policy_violations(&v, "src/domain/order.js") >= 2, "both are reported: {}", v["adr_compliance"]);
    assert_eq!(graded(&v), 1, "and the file is graded and in a layer, not only scanned: {}", v["coverage"]);
}

/// S16: The same fs import in an adapter is allowed.
#[test]
fn s16_an_fs_import_in_an_adapter_is_allowed() {
    let d = project(&[
        (".hexa/ADR-rules.toml", RULES),
        ("src/adapters/secondary/store.js", "const fs = require(\"fs\");\nmodule.exports = { s: fs };\n"),
        ("src/domain/order.js", "const fs = require(\"fs\");\nexport const o = 1;\n"),
    ]);
    let v = analyze(&d);
    assert!(policy_violations(&v, "src/domain/order.js") >= 1, "control: the domain's require is a violation: {}", v["adr_compliance"]);
    assert_eq!(policy_violations(&v, "src/adapters/secondary/store.js"), 0, "{}", v["adr_compliance"]);
    assert_eq!(graded(&v), 2, "both files are graded and in a layer: {}", v["coverage"]);
}

/// S17: @hexa:public keeps a JavaScript export out of the dead-export check.
#[test]
fn s17_hexa_public_keeps_a_javascript_export_alive() {
    let d = project(&[(
        "src/usecases/api.js",
        "const run = () => 1;\n// @hexa:public\nexports.entry = run;\nexports.hidden = run;\n",
    )]);
    let dd = dead(&analyze(&d));
    assert!(is_dead(&dd, "src/usecases/api.js", "hidden"), "control: an untagged export is dead: {dd:?}");
    assert!(!is_dead(&dd, "src/usecases/api.js", "entry"), "{dd:?}");
}

/// S18: Without the tag the same export is dead.
#[test]
fn s18_without_the_tag_the_export_is_dead() {
    let d = project(&[("src/usecases/api.js", "const run = () => 1;\nexports.entry = run;\n")]);
    let dd = dead(&analyze(&d));
    assert!(is_dead(&dd, "src/usecases/api.js", "entry"), "{dd:?}");
}

/// S19: hexa analyze --file checks one JavaScript file.
#[test]
fn s19_analyze_file_checks_one_javascript_file() {
    let d = project(&[("src/domain/order.js", "import { h } from \"../adapters/secondary/http.js\";\nexport const o = h;\n"), HTTP]);
    let (code, out, err) = hexa(d.path(), &["analyze", ".", "--file", "src/domain/order.js"]);
    let all = format!("{out}{err}");
    assert_eq!(code, 1, "{all}");
    assert!(all.contains("src/domain/order.js"), "it names the JavaScript file, not another extension: {all}");
    assert!(all.contains("../adapters/secondary/http.js"), "{all}");
}

/// S20: A clean JavaScript file passes --file, and an excluded JavaScript file is not graded.
#[test]
fn s20_a_clean_file_passes_and_an_excluded_one_is_not_graded() {
    let d = project(&[
        ("src/domain/clean.js", "import { i } from \"./item.js\";\nexport const c = i;\n"),
        ("src/domain/item.js", "export const i = 1;\n"),
        ("src/domain/legacy.js", "import { h } from \"../adapters/secondary/http.js\";\nexport const l = h;\n"),
        (".hexa/project.json", "{\"analyze\":{\"exclude\":[\"src/domain/legacy.js\"]}}\n"),
        HTTP,
    ]);
    let (code, out, err) = hexa(d.path(), &["analyze", ".", "--file", "src/domain/clean.js"]);
    assert_eq!(code, 0, "{out}{err}");
    let v = analyze(&d);
    assert_eq!(graded(&v), 3, "clean.js, item.js and http.js are graded; legacy.js is not: {}", v["coverage"]);
    assert!(!from(&boundary(&v), "src/domain/legacy.js"), "an excluded file is not reported");
}
