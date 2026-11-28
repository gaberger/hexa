//! hexa reads Python: the grade, the evidence and the test runner.
//!
//! Written from the accepted scenarios of `docs/specs/ADR-2611211324.json` and the
//! signatures in `hexa-analysis/src/domain.rs`, `hexa-core/src/domain/evidence.rs` and
//! `hexa-core/src/domain/test_runner.rs`, before the code. The grade cases run the
//! built binary on a fixture in a temp directory, with its layers declared in
//! `.hexa/project.json` where the folder names would not place them: an import
//! is seen when it turns into a boundary violation from a file declared
//! `domain` to a file declared `adapters/secondary`.
//!
//! S46 (hexa's own grade stays A+ 100/100) is held by the CI grade step and
//! `hexa_grades_itself_by_ratchet`; it passes before the change, so it is not
//! here as a red test.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use hexa_analysis::domain::Language;
use hexa_analysis::ports::{ArchAnalysisPort, AstPort};
use hexa_analysis::treesitter_adapter::TreeSitterAdapter;
use hexa_core::domain::evidence::{read_evidence, read_gate_evidence, Evidence, KNOWN_RUNNERS};
use hexa_core::domain::test_runner::{is_test_file, runner_for, Runner};
use hexa_exec::test_exec::{judge, plan_test, TestConfig, TestRun};
use hexa_graph::extract::code::Language as GraphLanguage;

// ── harness ──────────────────────────────────────────────────────────────────

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body).unwrap();
}

fn layers(root: &Path, json: &str) {
    write(root, ".hexa/project.json", &format!("{{\"analyze\":{{\"layers\":{json}}}}}"));
}

struct Analysis {
    code: Option<i32>,
    text: String,
}

fn analyze_args(root: &Path, args: &[&str]) -> Analysis {
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .arg("analyze")
        .arg(".")
        .args(args)
        .current_dir(root)
        .env("HOME", home.path())
        .env("HEXA_HOME", home.path().join(".hexa"))
        .env("NO_COLOR", "1")
        .output()
        .expect("run hexa analyze");
    Analysis {
        code: out.status.code(),
        text: format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)),
    }
}

fn analyze_json(root: &Path) -> serde_json::Value {
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(["analyze", ".", "--json"])
        .current_dir(root)
        .env("HOME", home.path())
        .env("HEXA_HOME", home.path().join(".hexa"))
        .env("NO_COLOR", "1")
        .output()
        .expect("run hexa analyze --json");
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!("analyze --json printed no JSON ({e}):\n{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
    })
}

/// The files each boundary violation from `from` reaches.
fn reached_from(v: &serde_json::Value, from: &str) -> BTreeSet<String> {
    v["boundary_violations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|b| b["from_file"].as_str().is_some_and(|f| f.ends_with(from)))
        .filter_map(|b| b["to_file"].as_str().map(str::to_string))
        .collect()
}

fn lines_from(v: &serde_json::Value, from: &str, to: &str) -> BTreeSet<u64> {
    v["boundary_violations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|b| b["from_file"].as_str().is_some_and(|f| f.ends_with(from)))
        .filter(|b| b["to_file"].as_str().is_some_and(|f| f.ends_with(to)))
        .filter_map(|b| b["line"].as_u64())
        .collect()
}

/// Files of `lang` in the layer inventory, over every layer.
fn inventory_files(v: &serde_json::Value, lang: &str) -> u64 {
    v["layer_inventory"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["language"] == lang)
        .map(|r| r["files"].as_u64().unwrap_or(0))
        .sum()
}

fn unclassified(v: &serde_json::Value) -> Vec<String> {
    v["coverage"]["unclassified"].as_array().into_iter().flatten().filter_map(|s| s.as_str().map(str::to_string)).collect()
}

fn graded(v: &serde_json::Value) -> bool {
    v["graded"] == serde_json::json!(true)
}

/// The `language:` line of the text report.
fn language_line(text: &str) -> String {
    text.lines().find(|l| l.trim_start().starts_with("language:")).unwrap_or("").to_string()
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

fn py_exports(src: &str) -> Vec<String> {
    let ast = TreeSitterAdapter::new();
    let ex = ast.extract_exports(Path::new("pkg/m.py"), src, Language::Python).expect("extract Python exports");
    sorted(ex.into_iter().map(|e| e.name).collect())
}

async fn dead_names(root: &Path) -> Vec<(String, String)> {
    let ast = std::sync::Arc::new(TreeSitterAdapter::new());
    let r = hexa_analysis::analyzer::ArchAnalyzer::new(ast).analyze(root).await.expect("analyze");
    r.dead_exports.into_iter().map(|d| (d.file, d.export_name)).collect()
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn this_adr() -> String {
    let dir = repo_root().join("docs/adrs");
    let id = format!("ADR-{}", "2611211324");
    let f = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_string_lossy().starts_with(&id))
        .expect("this ADR's file");
    std::fs::read_to_string(f.path()).unwrap()
}

/// Everything from the ADR's first `**Built:**` line on; empty when it has none.
fn built_section(adr: &str) -> String {
    adr.find("\n**Built:**").map(|i| adr[i + 1..].to_string()).unwrap_or_default()
}

fn py_cfg(runners: &[(&str, &str)]) -> TestConfig {
    TestConfig { runners: runners.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(), remote: None }
}

fn no_runners() -> BTreeMap<String, String> {
    BTreeMap::new()
}

// ── §1 Language::Python ──────────────────────────────────────────────────────

#[test]
fn s01_a_py_or_pyi_path_is_python() {
    for p in ["pkg/ports/repo.py", "pkg/ports/repo.pyi"] {
        assert_eq!(Language::from_path(p), Language::Python, "hexa-analysis: {p}");
        assert_eq!(GraphLanguage::from_path(p), Some(GraphLanguage::Python), "hexa-graph: {p}");
    }
}

#[test]
fn s02_near_miss_extensions_are_not_python() {
    // The positive case first: a negative that nothing could fail is vacuous.
    assert_eq!(Language::from_path("pkg/a.py"), Language::Python);
    for p in ["notes.pyc", "script.py.bak", "pyproject.toml", "README.py.md"] {
        assert_ne!(Language::from_path(p), Language::Python, "hexa-analysis: {p}");
        assert_ne!(GraphLanguage::from_path(p), Some(GraphLanguage::Python), "hexa-graph: {p}");
    }
}

#[test]
fn s03_only_the_two_analysis_crates_name_the_python_grammar() {
    let root = repo_root();
    let mut naming = BTreeSet::new();
    for e in std::fs::read_dir(&root).unwrap().filter_map(|e| e.ok()) {
        let toml = e.path().join("Cargo.toml");
        if let Ok(text) = std::fs::read_to_string(&toml) {
            if text.contains("tree-sitter-python") {
                naming.insert(e.file_name().to_string_lossy().to_string());
            }
        }
    }
    let expected: BTreeSet<String> = ["hexa-analysis", "hexa-graph"].iter().map(|s| s.to_string()).collect();
    assert_eq!(naming, expected, "the crates whose Cargo.toml names tree-sitter-python");
}

#[test]
fn s04_the_extension_decides_python() {
    let cases = [
        ("a.py", Language::Python),
        ("a/b/c.pyi", Language::Python),
        ("a.ts", Language::TypeScript),
        ("a.tsx", Language::TypeScript),
        ("a.js", Language::TypeScript),
        ("a.go", Language::Go),
        ("a.rs", Language::Rust),
        ("a.pyc", Language::Unknown),
        ("a.py.txt", Language::Unknown),
        ("python", Language::Unknown),
    ];
    for (p, want) in cases {
        assert_eq!(Language::from_path(p), want, "hexa-analysis: {p}");
        let g = GraphLanguage::from_path(p);
        assert_eq!(g == Some(GraphLanguage::Python), want == Language::Python, "hexa-graph: {p}");
    }
}

// ── §2 imports resolve to project files ──────────────────────────────────────

#[test]
fn s05_absolute_from_and_relative_imports_resolve_to_project_files() {
    // Each form alone: two imports of one module from one file are one
    // crossing, and the grade counts one edge per target (`one_edge_per_target`).
    for import in ["import nautobot_ssot.ports", "from ..ports import Repo"] {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        write(r, "pyproject.toml", "[project]\nname = \"fx\"\n");
        write(r, "nautobot_ssot/__init__.py", "");
        write(r, "nautobot_ssot/ports.py", "class Repo:\n    pass\n");
        write(r, "nautobot_ssot/usecases/__init__.py", "");
        write(r, "nautobot_ssot/usecases/sync.py", &format!("{import}\n\n\ndef run():\n    return 1\n"));
        layers(
            r,
            r#"{"nautobot_ssot/usecases": "domain", "nautobot_ssot/ports.py": "adapters/secondary", "nautobot_ssot/__init__.py": "adapters/secondary"}"#,
        );
        let v = analyze_json(r);
        let reached = reached_from(&v, "nautobot_ssot/usecases/sync.py");
        assert!(reached.contains("nautobot_ssot/ports.py"), "{import}: no edge to ports.py: {reached:?}\n{v:#}");
        assert_eq!(lines_from(&v, "usecases/sync.py", "nautobot_ssot/ports.py"), BTreeSet::from([1]), "{import}:\n{v:#}");
    }
}

#[test]
fn s06_from_a_import_b_prefers_the_submodule_else_the_package() {
    // A: a/b.py exists.
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "pyproject.toml", "[project]\nname = \"fx\"\n");
    write(r, "a/__init__.py", "");
    write(r, "a/b.py", "def f():\n    return 1\n");
    write(r, "x.py", "from a import b\n\n\ndef g():\n    return b.f()\n");
    layers(r, r#"{"x.py": "domain", "a": "adapters/secondary"}"#);
    let v = analyze_json(r);
    assert_eq!(reached_from(&v, "x.py"), BTreeSet::from(["a/b.py".to_string()]), "A:\n{v:#}");

    // B: only the package, which defines b.
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "pyproject.toml", "[project]\nname = \"fx\"\n");
    write(r, "a/__init__.py", "def b():\n    return 1\n");
    write(r, "x.py", "from a import b\n\n\ndef g():\n    return b()\n");
    layers(r, r#"{"x.py": "domain", "a": "adapters/secondary"}"#);
    let v = analyze_json(r);
    assert_eq!(reached_from(&v, "x.py"), BTreeSet::from(["a/__init__.py".to_string()]), "B:\n{v:#}");
}

#[test]
fn s07_a_package_under_src_resolves() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "pyproject.toml", "[project]\nname = \"fx\"\n");
    write(r, "src/app/__init__.py", "");
    write(r, "src/app/domain.py", "class Order:\n    pass\n");
    write(r, "src/app/use.py", "from app.domain import Order\n\n\ndef make():\n    return Order()\n");
    layers(r, r#"{"src/app/use.py": "domain", "src/app/domain.py": "adapters/secondary", "src/app/__init__.py": "adapters/secondary"}"#);
    let v = analyze_json(r);
    assert_eq!(reached_from(&v, "src/app/use.py"), BTreeSet::from(["src/app/domain.py".to_string()]), "{v:#}");
}

#[test]
fn s08_an_import_under_type_checking_is_an_edge() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "pyproject.toml", "[project]\nname = \"fx\"\n");
    write(r, "app/__init__.py", "");
    write(r, "app/domain/__init__.py", "");
    write(
        r,
        "app/domain/order.py",
        "from typing import TYPE_CHECKING\n\nif TYPE_CHECKING:\n    from app.adapters.db import Session\n\n\nclass Order:\n    session: \"Session\"\n",
    );
    write(r, "app/adapters/__init__.py", "");
    write(r, "app/adapters/db.py", "class Session:\n    pass\n");
    layers(r, r#"{"app/__init__.py": "domain", "app/domain": "domain", "app/adapters": "adapters/secondary"}"#);
    let v = analyze_json(r);
    assert!(reached_from(&v, "app/domain/order.py").contains("app/adapters/db.py"), "the TYPE_CHECKING import is no violation:\n{v:#}");
}

#[test]
fn s09_stdlib_and_third_party_imports_are_neither_edges_nor_violations() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "pyproject.toml", "[project]\nname = \"fx\"\n");
    write(r, "pkg/__init__.py", "");
    write(r, "pkg/domain/__init__.py", "");
    write(r, "pkg/domain/x.py", "import os\nfrom django.db import models\nimport requests\n\n\ndef f():\n    return os, models, requests\n");
    layers(r, r#"{"pkg": "domain"}"#);
    let v = analyze_json(r);
    // The project was read: a clean result over files never parsed proves nothing.
    assert!(graded(&v), "not graded:\n{v:#}");
    assert!(inventory_files(&v, "python") >= 1, "no Python file in the inventory:\n{v:#}");
    let b = v["boundary_violations"].as_array().cloned().unwrap_or_default();
    assert!(b.is_empty(), "an import of no project file was a violation: {b:?}");
}

#[test]
fn s10_an_edge_exists_exactly_when_the_import_names_a_project_file() {
    // (import line, files beside it, the files it must reach)
    let cases: [(&str, &[&str], &[&str]); 5] = [
        ("import pkg.a", &["pkg/a.py"], &["pkg/a.py"]),
        ("from pkg import a", &["pkg/a.py"], &["pkg/a.py"]),
        ("from pkg.sub import thing", &["pkg/sub/__init__.py"], &["pkg/sub/__init__.py"]),
        ("from . import sib", &["pkg/use/sib.py"], &["pkg/use/sib.py"]),
        ("import pkg.missing", &["pkg/a.py"], &[]),
    ];
    for (line, files, want) in cases {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        write(r, "pyproject.toml", "[project]\nname = \"fx\"\n");
        write(r, "pkg/__init__.py", "");
        write(r, "pkg/use/__init__.py", "");
        for f in files {
            write(r, f, "def thing():\n    return 1\n");
        }
        write(r, "pkg/use/x.py", &format!("{line}\n\n\ndef g():\n    return 1\n"));
        let mut decl = serde_json::Map::new();
        decl.insert("pkg/use/x.py".into(), "domain".into());
        decl.insert("pkg/use/__init__.py".into(), "adapters/secondary".into());
        decl.insert("pkg/__init__.py".into(), "adapters/secondary".into());
        for f in files {
            decl.insert(f.to_string(), "adapters/secondary".into());
        }
        layers(r, &serde_json::Value::Object(decl).to_string());
        let v = analyze_json(r);
        assert!(graded(&v), "{line}: not graded:\n{v:#}");
        let got = reached_from(&v, "pkg/use/x.py");
        let want: BTreeSet<String> = want.iter().map(|s| s.to_string()).collect();
        assert_eq!(got, want, "{line}");
    }
}

// ── §3 exports ───────────────────────────────────────────────────────────────

#[test]
fn s11_without_all_public_top_level_names_are_exports() {
    let src = "def load():\n    def inner():\n        pass\n    return inner\n\n\nclass Repo:\n    pass\n\n\nLIMIT = 5\n\n\ndef _helper():\n    pass\n\n\n_cache = {}\n";
    assert_eq!(py_exports(src), vec!["LIMIT", "Repo", "load"]);
}

#[test]
fn s12_all_defines_the_exports() {
    let src = "__all__ = [\"Repo\"]\n\n\nclass Repo:\n    pass\n\n\ndef load():\n    pass\n";
    assert_eq!(py_exports(src), vec!["Repo"]);
}

#[tokio::test]
async fn s13_a_name_re_exported_through_init_is_not_dead() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "pyproject.toml", "[project]\nname = \"fx\"\n");
    write(r, "pkg/domain/__init__.py", "from .x import y\n");
    write(r, "pkg/domain/x.py", "def y():\n    return 1\n\n\ndef z():\n    return 2\n");
    let dead = dead_names(r).await;
    // The control: an export nothing names is found, so the finder read the files.
    assert!(dead.iter().any(|(f, n)| f.ends_with("pkg/domain/x.py") && n == "z"), "z is dead and was not reported: {dead:?}");
    assert!(!dead.iter().any(|(f, n)| f.ends_with("pkg/domain/x.py") && n == "y"), "y is re-exported and was reported dead: {dead:?}");
}

#[tokio::test]
async fn s14_private_and_nested_names_are_never_exports() {
    let src = "_a = 1\n\n\ndef _b():\n    def c():\n        pass\n    return c\n";
    assert!(py_exports(src).is_empty());
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "pyproject.toml", "[project]\nname = \"fx\"\n");
    write(r, "pkg/domain/private.py", src);
    let dead = dead_names(r).await;
    assert!(!dead.iter().any(|(f, _)| f.ends_with("private.py")), "a dead export in a module with none: {dead:?}");
}

#[test]
fn s15_the_export_set_is_all_or_the_public_top_level_names() {
    let cases: [(&str, &[&str]); 4] = [
        ("x = 1\n_y = 2\n", &["x"]),
        ("__all__ = ['b']\na = 1\nb = 2\n", &["b"]),
        ("__all__ = []\ndef f():\n    pass\n", &[]),
        ("class A:\n    def m(self):\n        pass\n\n\ndef _p():\n    pass\n", &["A"]),
    ];
    for (src, want) in cases {
        let want: Vec<String> = want.iter().map(|s| s.to_string()).collect();
        assert_eq!(py_exports(src), want, "{src}");
    }
}

// ── §4 hexa analyze reads a Python project ───────────────────────────────────

#[test]
fn s16_a_django_project_with_a_tooling_only_package_json_is_read_as_python() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "manage.py", "import sys\n");
    write(r, "pyproject.toml", "[project]\nname = \"fx\"\n");
    write(r, "package.json", "{\"name\":\"browser-tests\",\"private\":true}\n");
    write(r, "netbox_plugin/__init__.py", "");
    write(r, "netbox_plugin/domain/__init__.py", "");
    write(r, "netbox_plugin/domain/site.py", "class Site:\n    pass\n");
    let a = analyze_args(r, &[]);
    let lang = language_line(&a.text);
    assert!(lang.contains("python"), "language line: {lang:?}\n{}", a.text);
    assert!(!lang.contains("typescript"), "read as TypeScript: {lang:?}");
    assert!(!a.text.contains("unsupported"), "{}", a.text);
    let v = analyze_json(r);
    assert!(inventory_files(&v, "python") >= 1, "the package's .py files were not parsed:\n{v:#}");
}

#[test]
fn s17_several_languages_are_all_named_and_graded() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "pyproject.toml", "[project]\nname = \"fx\"\n");
    write(r, "package.json", "{\"name\":\"fx\",\"type\":\"module\"}\n");
    write(r, "pkg/domain/a.py", "class A:\n    pass\n");
    write(r, "src/domain/b.ts", "export interface B { x: number }\n");
    let a = analyze_args(r, &[]);
    let lang = language_line(&a.text);
    assert!(lang.contains("python") && lang.contains("typescript"), "language line: {lang:?}\n{}", a.text);
    let v = analyze_json(r);
    assert!(inventory_files(&v, "python") >= 1, "no Python graded:\n{v:#}");
    assert!(inventory_files(&v, "typescript") >= 1, "no TypeScript graded:\n{v:#}");
}

#[test]
fn s18_python_files_outside_the_convention_need_a_declared_layer() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "pyproject.toml", "[project]\nname = \"fx\"\n");
    write(r, "pkg/models.py", "class Site:\n    pass\n");
    write(r, "pkg/jobs.py", "from pkg.models import Site\n\n\ndef sync():\n    return Site()\n");
    write(r, "pkg/views.py", "from pkg.jobs import sync\n\n\ndef page():\n    return sync()\n");
    let v = analyze_json(r);
    let un = unclassified(&v);
    for f in ["pkg/models.py", "pkg/jobs.py", "pkg/views.py"] {
        assert!(un.iter().any(|u| u.ends_with(f)), "{f} is not listed unclassified: {un:?}\n{v:#}");
    }
    assert!(graded(&v), "not graded:\n{v:#}");
    assert_ne!(v["grade"], "A+", "unclassified files did not cap the grade:\n{v:#}");

    layers(r, r#"{"pkg/models.py": "domain", "pkg/jobs.py": "usecases", "pkg/views.py": "adapters/primary"}"#);
    let v = analyze_json(r);
    let un = unclassified(&v);
    for f in ["pkg/models.py", "pkg/jobs.py", "pkg/views.py"] {
        assert!(!un.iter().any(|u| u.ends_with(f)), "{f} declared and still unclassified: {un:?}");
    }
}

#[test]
fn s19_excluded_directories_and_test_code_are_not_graded_as_layers() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "pyproject.toml", "[project]\nname = \"fx\"\n");
    write(r, "pkg/domain/core.py", "class Core:\n    pass\n");
    let outside = [
        "pkg/migrations/0001_initial.py",
        ".venv/lib/site.py",
        "venv/lib/site2.py",
        "site-packages/dep/mod.py",
        "node_modules/m/w.py",
        "pkg/__pycache__/c.py",
        "tests/test_a.py",
        "pkg/test_sync.py",
    ];
    for f in outside {
        write(r, f, "from pkg.domain.core import Core\n\n\ndef unused_name():\n    return Core\n");
    }
    let v = analyze_json(r);
    assert!(graded(&v), "not graded:\n{v:#}");
    assert_eq!(inventory_files(&v, "python"), 1, "only pkg/domain/core.py is a layered file:\n{v:#}");
    let un = unclassified(&v);
    for f in outside {
        assert!(!un.iter().any(|u| u.ends_with(f)), "{f} reported unclassified: {un:?}");
    }
    assert_eq!(v["coverage"]["total"], 1, "the graded file count:\n{v:#}");
    let dead = v["dead_exports"].to_string();
    assert!(!dead.contains("unused_name"), "an excluded file's export was graded: {dead}");
}

#[test]
fn s20_the_language_is_not_chosen_by_manifest_order_alone() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "requirements.txt", "django\n");
    write(r, "package.json", "{\"name\":\"tools\",\"private\":true}\n");
    write(r, "pkg/domain/a.py", "class A:\n    pass\n");
    let a = analyze_args(r, &[]);
    let lang = language_line(&a.text);
    assert!(lang.contains("python"), "language line: {lang:?}\n{}", a.text);
    let v = analyze_json(r);
    assert!(graded(&v), "not graded:\n{v:#}");
    assert!(inventory_files(&v, "python") >= 1, "the Python files were not graded:\n{v:#}");
}

#[test]
fn s21_every_detected_language_is_graded_and_the_scan_covers_package_directories() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "pyproject.toml", "[project]\nname = \"fx\"\n");
    write(r, "pkg/domain/a.py", "class A:\n    pass\n");
    write(r, "pkg/ports/b.py", "from pkg.domain.a import A\n\n\nclass Store:\n    def load(self) -> A: ...\n");
    write(r, "venv/lib/c.py", "x = 1\n");
    let v = analyze_json(r);
    assert!(graded(&v), "not graded:\n{v:#}");
    assert_eq!(inventory_files(&v, "python"), 2, "pkg/ at the root is scanned, venv/ is not:\n{v:#}");
    let a = analyze_args(r, &[]);
    assert!(language_line(&a.text).contains("python"), "{}", a.text);
}

// ── §5 pytest evidence ───────────────────────────────────────────────────────

#[test]
fn s22_pytests_summary_is_counted() {
    for out in ["============================== 5 passed in 0.12s ==============================\n", "5 passed in 0.12s\n"] {
        assert_eq!(read_evidence(true, out, false), Evidence::Passed { tests: 5 }, "{out:?}");
    }
}

#[test]
fn s23_a_pytest_failure_line_fails() {
    let out = "==================== 3 failed, 5 passed, 1 skipped in 0.31s ====================\n";
    match read_evidence(true, out, false) {
        Evidence::Failed { reason } => assert!(reason.contains('3'), "{reason}"),
        e => panic!("{e:?}"),
    }
}

#[test]
fn s24_zero_test_and_skip_only_pytest_runs_are_vacuous() {
    for out in [
        "no tests ran in 0.01s\n",
        "collected 0 items\n\n============================ no tests ran in 0.01s ============================\n",
        "collected 0 items\n",
        "============================== 4 skipped in 0.02s ==============================\n",
        "============================== 2 xfailed in 0.02s ==============================\n",
    ] {
        assert_eq!(read_evidence(true, out, false), Evidence::Vacuous, "{out:?}");
    }
}

#[test]
fn s25_known_runners_names_pytest_and_unittest() {
    assert!(KNOWN_RUNNERS.contains("pytest"), "{KNOWN_RUNNERS}");
    assert!(KNOWN_RUNNERS.contains("unittest"), "{KNOWN_RUNNERS}");
}

#[test]
fn s26_a_pytest_summary_passes_only_with_passed_tests_and_no_failures() {
    for p in 0..3u64 {
        for f in 0..2u64 {
            for s in 0..2u64 {
                for x in 0..2u64 {
                    let mut parts = Vec::new();
                    if f > 0 { parts.push(format!("{f} failed")); }
                    if p > 0 { parts.push(format!("{p} passed")); }
                    if s > 0 { parts.push(format!("{s} skipped")); }
                    if x > 0 { parts.push(format!("{x} xfailed")); }
                    if parts.is_empty() {
                        continue;
                    }
                    let out = format!("======== {} in 0.05s ========\n", parts.join(", "));
                    let got = read_evidence(true, &out, false);
                    if f > 0 {
                        assert!(matches!(got, Evidence::Failed { .. }), "{out:?} → {got:?}");
                    } else if p > 0 {
                        assert_eq!(got, Evidence::Passed { tests: p }, "{out:?}");
                    } else {
                        assert_eq!(got, Evidence::Vacuous, "{out:?}");
                    }
                }
            }
        }
    }
}

// ── §6 a grade over no parsed source ─────────────────────────────────────────

/// Sources hexa lists, none of which it grades: a Python package whose only
/// module is a migration, beside a tooling-only package.json.
fn nothing_parsed(r: &Path) {
    write(r, "pyproject.toml", "[project]\nname = \"fx\"\n");
    write(r, "package.json", "{\"name\":\"tools\",\"private\":true}\n");
    write(r, "pkg/migrations/0001_initial.py", "operations = []\n");
}

fn no_letter(text: &str) -> bool {
    !text.contains("/100") && !text.contains("Architecture grade:")
}

#[test]
fn s27_zero_parsed_files_is_not_graded() {
    let d = tempfile::tempdir().unwrap();
    nothing_parsed(d.path());
    let a = analyze_args(d.path(), &[]);
    assert!(a.text.to_lowercase().contains("not graded"), "{}", a.text);
    assert!(no_letter(&a.text), "a letter or score was printed:\n{}", a.text);
}

#[test]
fn s28_an_empty_grade_never_exits_green_under_a_gate() {
    let d = tempfile::tempdir().unwrap();
    nothing_parsed(d.path());
    let e = analyze_args(d.path(), &["--exit-code"]);
    assert_ne!(e.code, Some(0), "--exit-code exited 0:\n{}", e.text);
    let g = analyze_args(d.path(), &["--grade", "A"]);
    assert_ne!(g.code, Some(0), "--grade A exited 0:\n{}", g.text);
}

#[test]
fn s29_no_letter_without_parsed_files_in_any_language() {
    let fixtures: [(&str, &[(&str, &str)]); 4] = [
        ("python beside package.json", &[("pyproject.toml", "[project]\nname = \"fx\"\n"), ("package.json", "{\"name\":\"t\"}\n"), ("pkg/migrations/0001.py", "x = 1\n")]),
        ("rust", &[("Cargo.toml", "[package]\nname = \"fx\"\nversion = \"0.1.0\"\n"), ("README.md", "x\n")]),
        ("go", &[("go.mod", "module fx\n\ngo 1.22\n"), ("README.md", "x\n")]),
        ("none", &[("README.md", "x\n")]),
    ];
    for (what, files) in fixtures {
        let d = tempfile::tempdir().unwrap();
        for (f, body) in files {
            write(d.path(), f, body);
        }
        let a = analyze_args(d.path(), &[]);
        assert!(a.text.to_lowercase().contains("not graded"), "{what}: {}", a.text);
        assert!(no_letter(&a.text), "{what}: a letter or score:\n{}", a.text);
        assert_ne!(analyze_args(d.path(), &["--exit-code"]).code, Some(0), "{what}: --exit-code exited 0");
        assert_ne!(analyze_args(d.path(), &["--grade", "A"]).code, Some(0), "{what}: --grade A exited 0");
    }
}

// ── §7 unittest and Django counts ────────────────────────────────────────────

#[test]
fn s30_unittests_ok_counts_the_run_minus_the_skipped() {
    let out = "...\n----------------------------------------------------------------------\nRan 40 tests in 1.200s\n\nOK (skipped=33)\n";
    assert_eq!(read_evidence(true, out, false), Evidence::Passed { tests: 7 });
}

#[test]
fn s31_a_plain_unittest_ok_counts_every_test() {
    let out = ".....\n----------------------------------------------------------------------\nRan 5 tests in 0.002s\n\nOK\n";
    assert_eq!(read_evidence(true, out, false), Evidence::Passed { tests: 5 });
}

#[test]
fn s32_a_framework_wrapper_is_read_by_its_output() {
    let out = "Creating test database for alias 'default'...\n............\n----------------------------------------------------------------------\nRan 12 tests in 3.400s\n\nOK\nDestroying test database for alias 'default'...\n";
    for cmd in ["nautobot-server test", "invoke unittest"] {
        assert_eq!(read_gate_evidence(cmd, true, out, false), Evidence::Passed { tests: 12 }, "{cmd}");
    }
}

#[test]
fn s33_failures_errors_and_zero_test_runs_do_not_pass() {
    let failed = "Ran 5 tests in 0.010s\n\nFAILED (failures=1, errors=2)\n";
    assert!(matches!(read_evidence(true, failed, false), Evidence::Failed { .. }), "{:?}", read_evidence(true, failed, false));
    assert_eq!(read_evidence(true, "Ran 0 tests in 0.000s\n\nOK\n", false), Evidence::Vacuous);
    assert_eq!(read_evidence(true, "Ran 3 tests in 0.010s\n\nOK (skipped=3)\n", false), Evidence::Vacuous);
}

#[test]
fn s34_the_unittest_count_is_n_minus_skipped() {
    for n in 0..6u64 {
        for k in 0..=n {
            let ok = if k == 0 { "OK".to_string() } else { format!("OK (skipped={k})") };
            let out = format!("Ran {n} tests in 0.100s\n\n{ok}\n");
            let got = read_evidence(true, &out, false);
            if n - k > 0 {
                assert_eq!(got, Evidence::Passed { tests: n - k }, "{out:?}");
            } else {
                assert_eq!(got, Evidence::Vacuous, "{out:?}");
            }
            let bad = format!("Ran {n} tests in 0.100s\n\nFAILED (failures=1)\n");
            assert!(matches!(read_evidence(true, &bad, false), Evidence::Failed { .. }), "{bad:?}");
        }
    }
}

// ── §8 the built-in Python runner ────────────────────────────────────────────

#[test]
fn s35_py_has_a_built_in_pytest_runner() {
    let builtin = runner_for("py", &no_runners());
    assert!(builtin.is_some() && !matches!(builtin, Some(Runner::Configured(_))), "{builtin:?}");

    for marker in ["pyproject.toml", "pytest.ini", "tox.ini", "setup.cfg"] {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        write(r, &format!("pkg/{marker}"), "\n");
        write(r, "pkg/tests/test_sync.py", "def test_one():\n    assert True\n");
        let p = plan_test(r, "pkg/tests/test_sync.py::test_one", &py_cfg(&[]), "")
            .unwrap_or_else(|e| panic!("{marker}: not planned: {e:?}"));
        assert!(p.cmd.contains("cd 'pkg'"), "{marker}: does not run from pkg/: {}", p.cmd);
        assert!(p.cmd.contains("python3 -m pytest"), "{marker}: {}", p.cmd);
        assert!(p.cmd.contains("tests/test_sync.py::test_one"), "{marker}: {}", p.cmd);
        assert!(!p.cmd.contains("pkg/tests/test_sync.py"), "{marker}: the path is not seen from pkg/: {}", p.cmd);
        assert!(p.cmd.contains(" -q"), "{marker}: {}", p.cmd);
    }
}

#[test]
fn s36_python_test_files_are_recognised_for_sealing() {
    for p in ["app/tests/models.py", "app/test_sync.py", "app/sync_test.py"] {
        assert!(is_test_file(p), "{p}");
    }
}

#[test]
fn s37_a_configured_runner_wins_over_the_built_in() {
    // The built-in exists, so winning over it means something.
    assert!(matches!(runner_for("py", &no_runners()), Some(r) if !matches!(r, Runner::Configured(_))));
    let mut cfg = BTreeMap::new();
    cfg.insert("py".to_string(), "nautobot-server test {name}".to_string());
    assert_eq!(runner_for("py", &cfg), Some(Runner::Configured("nautobot-server test {name}".into())));
}

fn planned_py() -> (tempfile::TempDir, hexa_exec::test_exec::Planned) {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "pyproject.toml", "[project]\nname = \"fx\"\n");
    write(d.path(), "tests/test_sync.py", "def test_one():\n    assert True\n");
    let p = plan_test(d.path(), "tests/test_sync.py::test_one", &py_cfg(&[]), "").expect("a built-in Python plan");
    (d, p)
}

#[test]
fn s38_zero_or_several_tests_do_not_pass_a_single_test_run() {
    let (_d, p) = planned_py();
    assert!(matches!(judge(&p, true, "no tests ran in 0.01s\n"), TestRun::Vacuous), "{:?}", judge(&p, true, "no tests ran in 0.01s\n"));
    assert!(matches!(judge(&p, true, "3 passed in 0.02s\n"), TestRun::Ambiguous(3)), "{:?}", judge(&p, true, "3 passed in 0.02s\n"));
}

#[test]
fn s39_ordinary_python_modules_are_not_test_files() {
    // The control: a test module is recognised, so refusing these means something.
    assert!(is_test_file("app/test_sync.py"));
    for p in ["app/models.py", "app/contest.py", "app/testing_utils.py"] {
        assert!(!is_test_file(p), "{p}");
    }
}

#[test]
fn s40_a_python_test_passes_only_when_exactly_one_ran_and_passed() {
    let (_d, p) = planned_py();
    assert!(matches!(judge(&p, true, "1 passed in 0.01s\n"), TestRun::Passed));
    assert!(matches!(judge(&p, false, "1 failed in 0.01s\n"), TestRun::Failed(_)));
    assert!(matches!(judge(&p, false, "1 failed, 1 passed in 0.01s\n"), TestRun::Failed(_)));
    assert!(matches!(judge(&p, true, "1 skipped in 0.01s\n"), TestRun::Vacuous));
    assert!(matches!(judge(&p, true, "2 passed in 0.01s\n"), TestRun::Ambiguous(2)));
    assert!(!matches!(judge(&p, true, "all good\n"), TestRun::Passed), "unreadable output passed");
}

// ── §9 out of scope, named as not done ───────────────────────────────────────

#[test]
fn s41_the_out_of_scope_items_are_named_as_not_done() {
    let adr = this_adr();
    assert!(!built_section(&adr).is_empty(), "the ADR has no **Built:** line yet");
    let scope = adr.lines().find(|l| l.contains("Out of scope")).unwrap_or("");
    for item in ["hexa spec tests", "hexa scaffold", "lifecycle_enforcement"] {
        assert!(scope.contains(item), "the out-of-scope decision does not name {item}: {scope}");
    }
}

#[test]
fn s42_this_adr_is_not_cited_as_having_delivered_the_out_of_scope_items() {
    let built = built_section(&this_adr());
    assert!(!built.is_empty(), "the ADR has no **Built:** line yet");
    for line in built.lines() {
        let l = line.to_lowercase();
        let names_one = l.contains("spec tests") || l.contains("scaffold") || l.contains("lifecycle_enforcement");
        if names_one {
            assert!(l.contains("out of scope") || l.contains("not built") || l.contains("not delivered"), "the Built record claims an out-of-scope item: {line}");
        }
    }
}

// ── §10 measured ─────────────────────────────────────────────────────────────

fn copy_dir(from: &Path, to: &Path) {
    for e in std::fs::read_dir(from).unwrap().filter_map(|e| e.ok()) {
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if src.is_dir() {
            if e.file_name() == "__pycache__" {
                continue;
            }
            std::fs::create_dir_all(&dst).unwrap();
            copy_dir(&src, &dst);
        } else {
            std::fs::copy(&src, &dst).unwrap();
        }
    }
}

#[test]
fn s43_the_python_example_is_graded() {
    let example = repo_root().join("examples/stress/game-wordle-py");
    let d = tempfile::tempdir().unwrap();
    copy_dir(&example, d.path());
    let v = analyze_json(d.path());
    assert!(graded(&v), "not graded:\n{v:#}");
    let from: BTreeSet<String> = v["boundary_violations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|b| b["from_file"].as_str().map(|f| f.trim_start_matches("./").to_string()))
        .collect();
    let planted: BTreeSet<String> = ["src/domain/game.py", "src/usecases/start_game.py"].iter().map(|s| s.to_string()).collect();
    assert_eq!(from, planted, "the violations are not exactly the two planted ones:\n{v:#}");
    let readme = std::fs::read_to_string(example.join("README.md")).unwrap();
    assert!(!readme.contains("a tree it cannot read"), "the README still says hexa cannot read it");
}

#[test]
fn s44_the_two_target_repositories_are_measured_and_recorded() {
    let built = built_section(&this_adr());
    for repo in ["netbox-plugin", "nautobot-ssot-plugin"] {
        let at = built.find(repo).unwrap_or_else(|| panic!("the Built record does not name {repo}"));
        let rest = &built[at..];
        let entry = &rest[..rest.find("\n\n").unwrap_or(rest.len())];
        assert!(entry.contains("grade"), "{repo}: no grade recorded: {entry}");
        assert!(entry.contains("with a layer"), "{repo}: no layered-file count recorded: {entry}");
        assert!(entry.contains("analyze.layers"), "{repo}: no analyze.layers recorded: {entry}");
    }
}

#[test]
fn s45_not_built_while_the_readme_or_built_line_is_unchanged() {
    let readme = std::fs::read_to_string(repo_root().join("examples/stress/game-wordle-py/README.md")).unwrap();
    assert!(!readme.contains("a tree it cannot read"), "the README is unchanged");
    let built = built_section(&this_adr());
    for repo in ["netbox-plugin", "nautobot-ssot-plugin"] {
        assert!(built.contains(repo), "the Built record lacks {repo}");
    }
}
