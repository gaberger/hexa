//! ADR-2611271955: a package.json project is JavaScript when its sources are.
//!
//! One test per scenario of the accepted spec, run against the built binary on
//! small projects. A negative that already holds today (a TypeScript project
//! stays typescript) carries a control that only holds once the rule is built,
//! so it fails first instead of mirroring the old behaviour.
#![allow(unused, clippy::all)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::Value;

fn hexa(dir: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_hexa"));
    c.current_dir(dir)
        .env("NO_COLOR", "1")
        .env("HEXA_NO_CLAUDE_MODS", "1")
        .env("HOME", dir.join(".home"))
        .env("CLAUDE_PROJECT_DIR", dir);
    c
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

/// A project of exactly these files.
fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    for (rel, body) in files {
        let p = d.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }
    d
}

const PKG: (&str, &str) = ("package.json", "{\"name\":\"x\",\"version\":\"0.1.0\"}\n");
const JS: (&str, &str) = ("src/index.js", "export const a = 1;\n");
const TS: (&str, &str) = ("src/a.ts", "export const a = 1;\n");

/// A scaffold of `lang` named my-app.
fn scaffold(lang: &str) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("proj");
    std::fs::create_dir_all(&dir).unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .arg("init")
        .arg(&dir)
        .args(["--name", "my-app", "--scaffold", "--lang", lang, "--no-claude-md"])
        .env("HEXA_NO_CLAUDE_MODS", "1")
        .output()
        .unwrap();
    assert!(o.status.success(), "init --lang {lang}: {}", text(&o));
    (tmp, dir)
}

/// What `hexa analyze` prints after `language:`.
fn analyze_lang(dir: &Path) -> String {
    let o = hexa(dir).args(["analyze", "."]).output().unwrap();
    text(&o)
        .lines()
        .find_map(|l| l.trim_start().strip_prefix("language:").map(|r| r.trim().to_string()))
        .unwrap_or_else(|| panic!("no language line:\n{}", text(&o)))
}

/// What `hexa doctor` prints after `type:`.
fn doctor_type(dir: &Path) -> String {
    let o = hexa(dir).arg("doctor").output().unwrap();
    text(&o)
        .lines()
        .find_map(|l| l.trim_start().strip_prefix("type:").map(|r| r.trim().to_string()))
        .unwrap_or_else(|| panic!("no type line:\n{}", text(&o)))
}

/// The `Language:` entry of the session-start block.
fn session_lang(dir: &Path) -> String {
    let mut c = hexa(dir)
        .args(["hook", "session-start"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(&mut c.stdin.take().unwrap(), b"{\"session_id\":\"adr-2611271955\"}").unwrap();
    let o = c.wait_with_output().unwrap();
    text(&o)
        .lines()
        .find_map(|l| l.strip_prefix("Language: ").map(|r| r.split('|').next().unwrap().trim().to_string()))
        .unwrap_or_else(|| panic!("no Language entry:\n{}", text(&o)))
}

fn analyze_json(dir: &Path) -> Value {
    let o = hexa(dir).args(["analyze", ".", "--json"]).output().unwrap();
    let s = String::from_utf8_lossy(&o.stdout).to_string();
    let start = s.find('{').unwrap_or_else(|| panic!("no JSON:\n{}", text(&o)));
    serde_json::from_str(&s[start..]).unwrap_or_else(|e| panic!("JSON ({e}):\n{s}"))
}

const JAVASCRIPT: &str = "javascript (package.json)";
const TYPESCRIPT: &str = "typescript (package.json)";

/// The control of every negative below: a JavaScript-only project is named javascript.
fn assert_js_is_named() {
    let d = project(&[PKG, JS]);
    assert_eq!(analyze_lang(d.path()), JAVASCRIPT, "control: a JavaScript project is not named javascript");
}

/// S01: A package.json project with only JavaScript sources is javascript
#[test]
fn s01_javascript_only_is_javascript() {
    let d = project(&[PKG, JS]);
    assert_eq!(analyze_lang(d.path()), JAVASCRIPT);
}

/// S02: A project with any TypeScript source stays typescript
#[test]
fn s02_any_typescript_source_keeps_typescript() {
    assert_js_is_named();
    let d = project(&[PKG, TS, ("src/b.js", "export const b = 2;\n")]);
    assert_eq!(analyze_lang(d.path()), TYPESCRIPT);
    let tsx = project(&[PKG, ("src/a.tsx", "export const a = 1;\n"), JS]);
    assert_eq!(analyze_lang(tsx.path()), TYPESCRIPT, "a .tsx source counts as TypeScript");
}

/// S03: A tsconfig.json at the root keeps a project typescript before it has a source
#[test]
fn s03_tsconfig_keeps_typescript() {
    assert_js_is_named();
    let d = project(&[PKG, ("tsconfig.json", "{\"compilerOptions\":{}}\n"), JS]);
    assert_eq!(analyze_lang(d.path()), TYPESCRIPT);
}

/// S04: A package.json with no source is typescript, as before
#[test]
fn s04_package_json_alone_is_typescript() {
    assert_js_is_named();
    // `hexa analyze` refuses a project with no source ("not graded"), so it prints no
    // language line here; `hexa doctor` and the session-start block always name one.
    let d = project(&[PKG]);
    assert_eq!(doctor_type(d.path()), TYPESCRIPT);
    assert_eq!(session_lang(d.path()), "typescript");
}

/// S05: hexa doctor names the same language
#[test]
fn s05_doctor_names_the_same_language() {
    let js = project(&[PKG, JS]);
    let ts = project(&[PKG, TS]);
    assert_eq!(doctor_type(js.path()), JAVASCRIPT);
    assert_eq!(doctor_type(ts.path()), TYPESCRIPT);
}

/// S06: The session-start fingerprint names the same language (both code paths)
#[test]
fn s06_session_start_names_the_same_language() {
    // A directory that is not a hexa project: the hook's own reading of the manifest.
    let bare = project(&[PKG, JS]);
    assert_eq!(session_lang(bare.path()), "javascript", "the manifest path");
    // A hexa project: the fingerprint extractor's reading.
    let (_t, dir) = scaffold("js");
    assert_eq!(session_lang(&dir), "javascript", "the fingerprint path");
}

/// S07: The session-start fingerprint still names typescript for a TypeScript project
#[test]
fn s07_session_start_still_names_typescript() {
    let js = project(&[PKG, JS]);
    assert_eq!(session_lang(js.path()), "javascript", "control: a JavaScript project is not named javascript");
    let bare = project(&[PKG, TS]);
    assert_eq!(session_lang(bare.path()), "typescript", "the manifest path");
    let (_t, dir) = scaffold("ts");
    assert_eq!(session_lang(&dir), "typescript", "the fingerprint path");
}

/// S08: Beside a Python manifest, JavaScript sources list javascript
#[test]
fn s08_beside_a_python_manifest_javascript_is_listed() {
    let d = project(&[PKG, JS, ("pyproject.toml", "[project]\nname = \"x\"\nversion = \"0.1.0\"\n")]);
    assert_eq!(analyze_lang(d.path()), "javascript (package.json), python (pyproject.toml)");
}

/// S09: Beside a Python manifest, a package.json that is tooling only is not listed
#[test]
fn s09_a_tooling_package_json_is_not_listed() {
    let py = ("pyproject.toml", "[project]\nname = \"x\"\nversion = \"0.1.0\"\n");
    // Control: with JavaScript sources it is listed.
    let with = project(&[PKG, JS, py]);
    assert!(analyze_lang(with.path()).contains(JAVASCRIPT), "control: JavaScript beside Python is not listed");
    // A Python source gives `hexa analyze` something to grade (it refuses an empty project).
    let tooling = project(&[PKG, py, ("app/mod.py", "x = 1\n")]);
    assert_eq!(analyze_lang(tooling.path()), "python (pyproject.toml)");
}

/// S10: JavaScript in node_modules or in analyze.exclude does not make a project javascript
#[test]
fn s10_excluded_javascript_does_not_count() {
    assert_js_is_named();
    let d = project(&[
        PKG,
        ("node_modules/dep/index.js", "module.exports = {};\n"),
        ("vendor/lib.js", "export const v = 1;\n"),
        (".hexa/project.json", "{\"analyze\":{\"exclude\":[\"vendor\"]}}\n"),
    ]);
    // No graded source is left, so `hexa analyze` refuses the project; doctor names it.
    assert_eq!(doctor_type(d.path()), TYPESCRIPT);
    assert_eq!(session_lang(d.path()), "typescript");
}

/// S11: A test file does not make a project javascript, and a source file does
#[test]
fn s11_a_test_file_does_not_count_a_source_file_does() {
    let both = project(&[PKG, JS, ("src/a.test.js", "export const t = 1;\n")]);
    assert_eq!(analyze_lang(both.path()), JAVASCRIPT);
    let only_test = project(&[PKG, ("src/a.test.js", "export const t = 1;\n")]);
    assert_eq!(analyze_lang(only_test.path()), TYPESCRIPT);
}

/// S12: The layer inventory's language column is unchanged for JavaScript files
#[test]
fn s12_the_inventory_column_is_unchanged() {
    assert_js_is_named();
    let (_t, dir) = scaffold("js");
    let v = analyze_json(&dir);
    let rows = v["layer_inventory"].as_array().cloned().unwrap_or_default();
    assert!(!rows.is_empty(), "no inventory rows: {v}");
    for r in &rows {
        assert_eq!(r["language"], "typescript", "row {r}");
    }
}

/// S13: The label change moves no grade, score or finding
#[test]
fn s13_no_grade_moves() {
    let (_t, dir) = scaffold("js");
    assert_eq!(analyze_lang(&dir), JAVASCRIPT, "control");
    let v = analyze_json(&dir);
    assert_eq!(v["grade"], "A+", "{v}");
    assert_eq!(v["score"].as_u64(), Some(100), "{v}");
    for key in ["violations", "boundary_violations", "dead_exports", "unused_ports"] {
        assert!(v[key].as_array().map(|a| a.is_empty()).unwrap_or(true), "{key}: {}", v[key]);
    }
}

/// S14: The Rust, Go and Python labels are unchanged
#[test]
fn s14_other_labels_unchanged() {
    assert_js_is_named();
    for (lang, want) in [("rust", "rust (Cargo.toml)"), ("go", "go (go.mod)"), ("python", "python (pyproject.toml)")] {
        let (_t, dir) = scaffold(lang);
        assert_eq!(analyze_lang(&dir), want, "{lang} scaffold");
    }
}

/// S15: Naming JavaScript does not depend on the order of the files
#[test]
fn s15_order_does_not_matter() {
    let files: Vec<(&str, &str)> = vec![
        PKG,
        JS,
        ("src/core/domain/count.js", "export const c = 1;\n"),
        ("src/core/usecases/go.js", "export const g = 1;\n"),
        ("src/adapters/secondary/db.js", "export const d = 1;\n"),
    ];
    for rotate in 0..files.len() {
        let mut order = files.clone();
        order.rotate_left(rotate);
        let d = project(&order);
        assert_eq!(analyze_lang(d.path()), JAVASCRIPT, "order rotated by {rotate}");
    }
}

/// S16: The TypeScript scaffold keeps its label and its grade
#[test]
fn s16_the_typescript_scaffold_is_unchanged() {
    assert_js_is_named();
    let (_t, dir) = scaffold("ts");
    assert_eq!(analyze_lang(&dir), TYPESCRIPT);
    let v = analyze_json(&dir);
    assert_eq!(v["grade"], "A+", "{v}");
}

/// S17: The inventory gains no javascript row
#[test]
fn s17_no_javascript_row() {
    assert_js_is_named();
    let d = project(&[
        PKG,
        ("src/domain/a.ts", "export const a = 1;\n"),
        ("src/domain/b.js", "export const b = 2;\n"),
    ]);
    let v = analyze_json(d.path());
    let rows = v["layer_inventory"].as_array().cloned().unwrap_or_default();
    assert!(rows.iter().all(|r| r["language"] != "javascript"), "{rows:?}");
    let domain: Vec<&Value> = rows.iter().filter(|r| r["layer"] == "domain").collect();
    assert_eq!(domain.len(), 1, "a mixed layer is one row, not one per extension: {rows:?}");
    assert_eq!(domain[0]["files"].as_u64(), Some(2), "{rows:?}");
}
