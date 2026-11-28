//! ADR-2611271700: hexa scaffolds JavaScript, as it scaffolds Rust, Go, TypeScript and Python.
//!
//! One test per scenario of the accepted spec, written from the scenarios and
//! run against the built binary. The other four languages' scaffold bytes are
//! fixed fingerprints taken before this decision was built, so "unchanged" is
//! measured against something the change cannot have produced.
#![allow(unused, clippy::all)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use hexa_core::domain::project_floor::{gitignore_lines, merged_gitignore};
use serde_json::Value;

const EXPECTED_JS: &[&str] = &[
    "package.json",
    "src/adapters/secondary/in-memory-counter-store.js",
    "src/composition-root.js",
    "src/core/domain/count.js",
    "src/core/ports/counter-store.js",
    "src/core/usecases/increment.js",
    "src/counter.test.js",
];

fn hexa() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_hexa"));
    c.env("NO_COLOR", "1").env("HEXA_NO_CLAUDE_MODS", "1");
    c
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

fn init_scaffold(dir: &Path, name: &str, lang: &str) -> Output {
    hexa()
        .arg("init")
        .arg(dir)
        .args(["--name", name, "--scaffold", "--lang", lang, "--no-claude-md"])
        .output()
        .expect("run hexa init")
}

/// A JavaScript project named `my-app` in a fresh directory.
fn scaffold_js() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("proj");
    std::fs::create_dir_all(&dir).unwrap();
    let o = init_scaffold(&dir, "my-app", "js");
    assert!(o.status.success(), "hexa init --scaffold --lang js failed:\n{}", text(&o));
    (tmp, dir)
}

/// Every file under `root`, relative and `/`-separated, skipping `.git` and `node_modules`.
fn files(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn go(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if name == ".git" || name == "node_modules" {
                continue;
            }
            if p.is_dir() {
                go(root, &p, out);
            } else {
                let rel = p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
                out.insert(rel, std::fs::read(&p).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    go(root, root, &mut out);
    out
}

/// The files the JavaScript scaffold owns: the manifest and every `.js` source.
/// (`hexa init` also writes a README into each layer folder; those are not the scaffold's.)
fn js_owned(all: &BTreeMap<String, Vec<u8>>) -> BTreeMap<String, Vec<u8>> {
    all.iter().filter(|(p, _)| p.as_str() == "package.json" || p.ends_with(".js")).map(|(p, b)| (p.clone(), b.clone())).collect()
}

fn read(root: &Path, rel: &str) -> String {
    std::fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// Node 22 or later. A test that needs it and finds none is a failure under
/// HEXA_NO_SKIP, not a pass nobody earned.
fn node_ok() -> bool {
    let ok = Command::new("node")
        .args(["-e", "process.exit(Number(process.versions.node.split('.')[0]) >= 22 ? 0 : 1)"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        assert!(std::env::var_os("HEXA_NO_SKIP").is_none(), "not proven here: node 22 or later is not available");
        eprintln!("node >= 22 not available; skipping the run of the gate");
    }
    ok
}

/// `node --test` in `dir`, with the network pointed nowhere.
fn run_gate(dir: &Path) -> (bool, String) {
    let out = Command::new("node")
        .arg("--test")
        .current_dir(dir)
        .env("http_proxy", "http://127.0.0.1:9")
        .env("https_proxy", "http://127.0.0.1:9")
        .env("npm_config_offline", "true")
        .output()
        .expect("run node --test");
    (out.status.success(), text(&out))
}

fn analyze_json(dir: &Path) -> Value {
    let out = hexa().current_dir(dir).args(["analyze", ".", "--json"]).output().expect("run hexa analyze");
    let s = String::from_utf8_lossy(&out.stdout).to_string();
    let start = s.find('{').unwrap_or_else(|| panic!("hexa analyze produced no JSON:\n{}", text(&out)));
    serde_json::from_str(&s[start..]).unwrap_or_else(|e| panic!("hexa analyze JSON ({e}):\n{s}"))
}

fn git(dir: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git")
}

// ── §1 what --lang js writes ─────────────────────────────────────────────────

/// S01: --lang js writes the whole JavaScript counter layout
#[test]
fn s01_lang_js_writes_the_whole_javascript_layout() {
    let (_t, dir) = scaffold_js();
    let owned: BTreeSet<String> = js_owned(&files(&dir)).into_keys().collect();
    let want: BTreeSet<String> = EXPECTED_JS.iter().map(|s| s.to_string()).collect();
    assert_eq!(owned, want, "the scaffold's files");
}

/// S02: package.json declares the project, a version, ES modules, the minimum Node and the test script
#[test]
fn s02_package_json_declares_project_modules_node_and_test() {
    let (_t, dir) = scaffold_js();
    let v: Value = serde_json::from_str(&read(&dir, "package.json")).expect("package.json is JSON");
    assert_eq!(v["name"], "my-app", "{v}");
    assert_eq!(v["version"], "0.1.0", "{v}");
    assert_eq!(v["type"], "module", "{v}");
    assert_eq!(v["engines"]["node"], ">=22", "{v}");
    assert_eq!(v["scripts"]["test"], "node --test", "{v}");
}

/// S03: The port re-exports the domain's Count and documents the store, and the test exercises the composition root
#[test]
fn s03_port_reexports_count_and_test_uses_the_composition_root() {
    let (_t, dir) = scaffold_js();
    let port = read(&dir, "src/core/ports/counter-store.js");
    assert!(port.contains("export { Count }") || port.contains("export {Count}"), "the port re-exports Count:\n{port}");
    assert!(port.contains("@typedef") && port.contains("CounterStore"), "the contract is a typedef:\n{port}");
    assert!(port.contains("load") && port.contains("save"), "{port}");
    let test = read(&dir, "src/counter.test.js");
    assert!(test.contains("composition-root.js"), "the test imports the composition root:\n{test}");
    assert!(test.contains("incrementOnce") || test.contains("counter()"), "and calls it:\n{test}");
}

/// S04: The in-memory adapter does not import the domain or the use cases
#[test]
fn s04_the_adapter_imports_only_the_port() {
    let (_t, dir) = scaffold_js();
    let a = read(&dir, "src/adapters/secondary/in-memory-counter-store.js");
    let imports: Vec<&str> = a.lines().filter(|l| l.trim_start().starts_with("import ")).collect();
    assert_eq!(imports.len(), 1, "one import expected:\n{a}");
    assert!(imports[0].contains("core/ports/counter-store.js"), "{imports:?}");
    assert!(!a.contains("core/domain") && !a.contains("core/usecases"), "{a}");
}

/// S05: No placeholder or template suffix survives in the JavaScript output
#[test]
fn s05_no_placeholder_or_tmpl_survives() {
    let (_t, dir) = scaffold_js();
    let all = files(&dir);
    let owned = js_owned(&all);
    assert!(!owned.is_empty(), "nothing was written");
    for (p, b) in &owned {
        assert!(!p.ends_with(".tmpl") && !p.contains("{{"), "{p}");
        let s = String::from_utf8_lossy(b);
        assert!(!s.contains("{{") && !s.contains("}}"), "{p} keeps a placeholder");
    }
    assert!(!all.keys().any(|p| p.ends_with(".tmpl")), "a .tmpl file was written");
    let v: Value = serde_json::from_str(&read(&dir, "package.json")).unwrap();
    assert_eq!(v["name"], "my-app");
}

// ── §2 the gate ──────────────────────────────────────────────────────────────

/// S06: The JavaScript gate is node --test and passes on a fresh scaffold
#[test]
fn s06_the_gate_passes_on_a_fresh_scaffold() {
    let (_t, dir) = scaffold_js();
    if !node_ok() {
        return;
    }
    let (ok, out) = run_gate(&dir);
    assert!(ok, "node --test failed on a fresh scaffold:\n{out}");
    assert!(out.contains("pass 4") && out.contains("fail 0"), "4 passed, 0 failed expected:\n{out}");
}

/// S07: SCAFFOLD_LANGS names js with its gate command
#[test]
fn s07_scaffold_langs_names_js_with_its_gate() {
    let entry = hexa_cli::commands::init::SCAFFOLD_LANGS.iter().find(|(l, _)| *l == "js");
    assert_eq!(entry.map(|(_, g)| *g), Some("node --test"), "{:?}", hexa_cli::commands::init::SCAFFOLD_LANGS);
    assert_eq!(hexa_cli::commands::init::scaffold_gate("js"), Some("node --test"));
}

/// S08: The JavaScript gate needs no install step and no network
#[test]
fn s08_the_gate_needs_no_install_or_network() {
    let (_t, dir) = scaffold_js();
    let v: Value = serde_json::from_str(&read(&dir, "package.json")).unwrap();
    assert!(v.get("dependencies").is_none() && v.get("devDependencies").is_none(), "dependencies declared: {v}");
    if !node_ok() {
        return;
    }
    let (ok, out) = run_gate(&dir);
    assert!(ok, "the gate needs something it has not got:\n{out}");
    assert!(!dir.join("node_modules").exists(), "the run created node_modules");
}

/// S09: A failing or empty node --test run does not count as a passing gate
#[test]
fn s09_a_failing_or_empty_run_is_not_a_pass() {
    // The scaffold, committed, then judged by hexa's own evidence reader.
    let gate = |edit: &dyn Fn(&Path)| -> (i32, String, String) {
        // `hexa init` makes the project's first commit itself (ADR-2610281552 §3).
        let (_t, dir) = scaffold_js();
        assert!(git(&dir, &["rev-parse", "--verify", "HEAD"]).status.success(), "the scaffold has no first commit");
        let before = String::from_utf8_lossy(&git(&dir, &["rev-parse", "HEAD"]).stdout).trim().to_string();
        edit(&dir);
        let o = hexa()
            .current_dir(&dir)
            .env("HOME", dir.join(".home"))
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .args(["do", "gate", "--files", "src/counter.test.js", "-e", "node --test", "-m", "edit", "--no-land"])
            .output()
            .expect("run hexa do gate");
        let after = String::from_utf8_lossy(&git(&dir, &["rev-parse", "HEAD"]).stdout).trim().to_string();
        (o.status.code().unwrap_or(-1), text(&o), format!("{}", before == after))
    };
    if !node_ok() {
        return;
    }
    let (code, out, unmoved) = gate(&|d| {
        let mut t = read(d, "src/counter.test.js");
        t.push_str("\ntest('forced failure', () => { assert.equal(1, 2); });\n");
        std::fs::write(d.join("src/counter.test.js"), t).unwrap();
    });
    assert_ne!(code, 0, "a failing test passed the gate:\n{out}");
    assert_eq!(unmoved, "true", "a failing gate committed:\n{out}");
    // No test file at all. (An emptied test file is not this: Node counts a file that
    // registers no test as one passing test, and the gate reads what Node reports.)
    let (code, out, unmoved) = gate(&|d| {
        std::fs::remove_file(d.join("src/counter.test.js")).unwrap();
    });
    assert_ne!(code, 0, "a run that executed no test passed the gate:\n{out}");
    assert_eq!(unmoved, "true", "an empty gate committed:\n{out}");
}

// ── §3 imports ───────────────────────────────────────────────────────────────

fn import_specifiers(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in src.lines() {
        let t = line.trim();
        if t.starts_with("import ") || t.starts_with("export ") {
            if let Some(i) = t.find(" from ") {
                let rest = t[i + 6..].trim().trim_end_matches(';');
                out.push(rest.trim_matches(|c| c == '\'' || c == '"').to_string());
            }
        }
    }
    out
}

/// S10: Every import is a relative path with its .js extension that names a scaffolded file
#[test]
fn s10_every_import_is_relative_with_js_and_resolves() {
    let (_t, dir) = scaffold_js();
    let owned = js_owned(&files(&dir));
    let mut seen = 0;
    for (p, b) in owned.iter().filter(|(p, _)| p.ends_with(".js")) {
        for spec in import_specifiers(&String::from_utf8_lossy(b)) {
            seen += 1;
            if spec.starts_with("node:") {
                continue;
            }
            assert!(spec.starts_with("./") || spec.starts_with("../"), "{p}: {spec} is not relative");
            assert!(spec.ends_with(".js"), "{p}: {spec} has no .js extension");
            let target = Path::new(p).parent().unwrap().join(&spec);
            let joined = target.to_string_lossy().to_string();
            let mut parts: Vec<&str> = Vec::new();
            for c in joined.split('/') {
                match c {
                    "." | "" => {}
                    ".." => {
                        parts.pop();
                    }
                    c => parts.push(c),
                }
            }
            let resolved = parts.join("/");
            assert!(owned.contains_key(&resolved), "{p}: {spec} resolves to {resolved}, which is not scaffolded");
        }
    }
    assert!(seen >= 6, "expected the scaffold to import across its layers, saw {seen}");
}

/// S11: No import names a package from a registry
#[test]
fn s11_no_import_names_a_registry_package() {
    let (_t, dir) = scaffold_js();
    let owned = js_owned(&files(&dir));
    let mut seen = 0;
    for (p, b) in owned.iter().filter(|(p, _)| p.ends_with(".js")) {
        let src = String::from_utf8_lossy(b);
        for spec in import_specifiers(&src) {
            seen += 1;
            assert!(spec.starts_with('.') || spec.starts_with("node:"), "{p}: {spec} is a bare package name");
        }
        assert!(!src.contains("require("), "{p} uses require");
    }
    assert!(seen > 0, "no import was read; the check proved nothing");
}

// ── §4 ignore lines ──────────────────────────────────────────────────────────

/// S12: The JavaScript ignore lines include node_modules/
#[test]
fn s12_js_ignore_lines_include_node_modules() {
    let lines = gitignore_lines("js");
    for l in ["node_modules/", ".hexa/loop.json", ".hexa/loop.lock", ".env*", "!.env.example"] {
        assert!(lines.contains(&l), "missing {l} in {lines:?}");
    }
}

/// S13: JavaScript's ignore lines are JavaScript's only
#[test]
fn s13_js_ignore_lines_are_javascripts_only() {
    let js = gitignore_lines("js");
    assert!(js.contains(&"node_modules/"), "{js:?}");
    for l in ["dist/", "target/", "__pycache__/", ".venv/"] {
        assert!(!js.contains(&l), "js holds {l}: {js:?}");
    }
}

/// S14: A merged JavaScript .gitignore always holds each JavaScript line exactly once
#[test]
fn s14_merged_js_gitignore_holds_each_line_once() {
    let inputs: Vec<Option<&str>> = vec![
        None,
        Some(""),
        Some("\n"),
        Some("*.log"),
        Some("*.log\nbuild/\n"),
        Some("node_modules/\n"),
        Some("  node_modules/  \nfoo\n"),
        Some("node_modules/\n.hexa/loop.json\n.hexa/loop.lock\n.env*\n!.env.example\n"),
        Some("# comment\n\nnode_modules/\nsecret.txt"),
        Some("target/\n__pycache__/\n"),
    ];
    assert!(gitignore_lines("js").contains(&"node_modules/"), "js has no node_modules/ line to hold once");
    for existing in inputs {
        let once = merged_gitignore(existing, "js");
        let first = once.clone().unwrap_or_else(|| existing.unwrap_or("").to_string());
        let twice = merged_gitignore(Some(&first), "js");
        let text = twice.clone().unwrap_or_else(|| first.clone());
        for l in gitignore_lines("js") {
            let n = text.lines().filter(|x| x.trim() == l).count();
            assert_eq!(n, 1, "{l} appears {n} times for {existing:?}: {text:?}");
        }
        let old: Vec<&str> = existing.unwrap_or("").lines().collect();
        let new: Vec<&str> = text.lines().collect();
        assert!(new.len() >= old.len(), "lines lost for {existing:?}");
        assert_eq!(&new[..old.len()], &old[..], "existing lines not kept in order for {existing:?}");
    }
}

// ── §5 the promises ──────────────────────────────────────────────────────────

fn assert_a_plus(result: &Value) {
    assert_eq!(result["grade"], "A+", "grade: {result}");
    assert_eq!(result["score"].as_u64(), Some(100), "score: {result}");
}

/// S15: hexa analyze grades the fresh JavaScript scaffold A+ with full coverage
#[test]
fn s15_analyze_grades_the_scaffold_a_plus() {
    let (_t, dir) = scaffold_js();
    let v = analyze_json(&dir);
    assert_a_plus(&v);
    let cov = &v["coverage"];
    assert_eq!(cov["classified"], cov["total"], "coverage: {cov}");
    assert!(cov["total"].as_u64().unwrap_or(0) >= 5, "the scaffold's sources were not graded: {cov}");
    for key in ["violations", "boundary_violations", "dead_exports", "unused_ports"] {
        assert!(v[key].as_array().map(|a| a.is_empty()).unwrap_or(true), "{key} not empty: {}", v[key]);
    }
    assert_eq!(v["score_components"]["circular_deps"].as_u64().unwrap_or(0), 0, "{}", v["score_components"]);
}

/// S16: The JavaScript scaffold satisfies the domain policy it ships
#[test]
fn s16_the_scaffold_satisfies_its_domain_policy() {
    let (_t, dir) = scaffold_js();
    let v = analyze_json(&dir);
    assert_eq!(v["adr_compliance"]["checked"], true, "the shipped policy was not checked: {}", v["adr_compliance"]);
    assert_eq!(v["adr_compliance"]["violation_count"].as_u64(), Some(0), "{}", v["adr_compliance"]);
    assert!(read(&dir, "src/core/domain/count.js").lines().all(|l| !l.trim_start().starts_with("import ")), "the domain imports something");
}

/// S17: No JavaScript scaffold file is left unclassified or in the wrong layer
#[test]
fn s17_every_file_is_in_the_layer_its_folder_names() {
    let (_t, dir) = scaffold_js();
    let v = analyze_json(&dir);
    assert!(v["coverage"]["unclassified"].as_array().map(|a| a.is_empty()).unwrap_or(false), "{}", v["coverage"]);
    let inv = v["layer_inventory"].as_array().cloned().unwrap_or_default();
    let files_in = |layer: &str| inv.iter().filter(|r| r["layer"] == layer).map(|r| r["files"].as_u64().unwrap_or(0)).sum::<u64>();
    for layer in ["domain", "ports", "usecases", "adapters/secondary"] {
        assert_eq!(files_in(layer), 1, "layer {layer} should hold one scaffold file: {inv:?}");
    }
    let text = text(&hexa().current_dir(&dir).args(["analyze", "."]).output().unwrap());
    assert!(text.contains("Composition root") && text.contains("(detected)"), "the composition root was not found:\n{text}");
}

/// S18: A fresh JavaScript scaffold needs no edit before its gate passes
#[test]
fn s18_a_fresh_scaffold_needs_no_edit() {
    let (_t, dir) = scaffold_js();
    if !node_ok() {
        return;
    }
    let before = files(&dir);
    let (ok, out) = run_gate(&dir);
    assert!(ok, "the gate failed on a scaffold nobody edited:\n{out}");
    assert_eq!(before, files(&dir), "running the gate changed a file");
}

/// S19: The JavaScript scaffold is deterministic
#[test]
fn s19_the_scaffold_is_deterministic() {
    for name in ["counter", "my-app"] {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        for d in [a.path(), b.path()] {
            let o = init_scaffold(d, name, "js");
            assert!(o.status.success(), "{}", text(&o));
        }
        let (fa, fb) = (js_owned(&files(a.path())), js_owned(&files(b.path())));
        assert!(fa.len() >= 7, "{:?}", fa.keys().collect::<Vec<_>>());
        assert_eq!(fa, fb, "two runs for {name} differ");
    }
}

// ── §6 nothing else changes ──────────────────────────────────────────────────

/// S20: --lang still defaults to rust
#[test]
fn s20_lang_defaults_to_rust() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("proj");
    std::fs::create_dir_all(&dir).unwrap();
    let o = hexa().arg("init").arg(&dir).args(["--name", "my-app", "--scaffold", "--no-claude-md"]).output().unwrap();
    assert!(o.status.success(), "{}", text(&o));
    let f = files(&dir);
    assert!(f.contains_key("Cargo.toml"), "no Rust project: {:?}", f.keys().collect::<Vec<_>>());
    assert!(!f.contains_key("package.json"), "a JavaScript project was written by default");
    assert!(hexa_cli::commands::init::SCAFFOLD_LANGS.iter().any(|(l, _)| *l == "js"), "js is not a known scaffold language");
}

/// S21: An unknown --lang is refused and names js among the known ones
#[test]
fn s21_an_unknown_lang_is_refused_naming_js() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("proj");
    std::fs::create_dir_all(&dir).unwrap();
    let o = init_scaffold(&dir, "my-app", "cobol");
    let msg = text(&o);
    assert!(!o.status.success(), "--lang cobol was accepted:\n{msg}");
    let scaffold_files: Vec<String> = files(&dir).into_keys().filter(|p| p == "package.json" || p == "Cargo.toml" || p.starts_with("src/")).collect();
    assert!(scaffold_files.is_empty(), "files were written: {scaffold_files:?}");
    // Read the list of known languages itself: a temp path in the message may hold "js".
    let known: Vec<&str> = msg
        .lines()
        .find_map(|l| l.split_once("expected one of:").map(|(_, rest)| rest))
        .map(|rest| rest.split(',').map(str::trim).collect())
        .unwrap_or_default();
    for lang in ["rust", "go", "ts", "python", "js"] {
        assert!(known.contains(&lang), "the refusal's list of known languages {known:?} lacks {lang}:\n{msg}");
    }
}

const GOLD_RUST: &[(&str, &str)] = &[
        ("Cargo.toml", "8e07281642ddb5819df158867f3eb579eb88721a384e1ad990c2edd23857371f"),
        ("src/adapters/README.md", "6a8b41eefa6a95d98a1f7eaa810bd7f756b2b1c7e90b8ff68c87faeb706c5fd6"),
        ("src/adapters/mod.rs", "ff8b12b033ef3359d4588d0691cde9128f8c73ce9f442aaa5ed1469d4c6ecc87"),
        ("src/adapters/secondary/README.md", "647355a9a3d688b0be9f668d227ce0c66123436b8c525f164f72bca2af2adac6"),
        ("src/adapters/secondary/mod.rs", "3652d9480bc92406b371f106486958529f06b91d8ed3ea1e2216beda9a442d88"),
        ("src/domain/README.md", "706cacfd0bc3148668568080b4122d91176711e0bad18d4cb5f20223fe26b579"),
        ("src/domain/mod.rs", "816b8567e73041040e66a2285fedb06437e0a765926acc67be5529fb97065602"),
        ("src/lib.rs", "003995ac525b34b45d60605570debb5d136adde6439fa0dbf0e36e8463cc91eb"),
        ("src/ports/README.md", "587b8bd1af0159e6b6f0b3ccc9f9ccc7f12e33ab9a78a14fdd57f55c4ebec0da"),
        ("src/ports/mod.rs", "08b6d89bbafd4d56cc228322936c82fc57e83006b0abc7378e4dee45838682ea"),
        ("src/usecases/README.md", "41bc4160507634af6b6c980142797c14f921baa5cc18ecfc1c1fb90438c7bf64"),
        ("src/usecases/mod.rs", "41948103f6edbf190f0f6b0b64c9b6026add4f64247167d1bd1ffc594cec8831"),
        ("tests/counter.rs", "99955978b71abadb29d0a426a5b2a445e5d85fbc7918d1a575f843c18d03c5a9"),
];

const GOLD_GO: &[(&str, &str)] = &[
        ("adapters/secondary/memory.go", "1951f0d96f18dde839cbd96815dfb73786289edef0e61c0e94d8eb7d1bfa8694"),
        ("composition-root.go", "80502548e751bd49922e6515557df8dd023a9e51578fd3fbb998f64f34c36969"),
        ("composition-root_test.go", "a44c2a041a1db278d6aee841b14b1ac949f7a9fe8346b3c31c0d5388453ef5a9"),
        ("go.mod", "5bc5954fc975fea4c9c05115dece82cfca2a7df266288d26986205beac0d5c26"),
        ("internal/domain/count.go", "b7258a362b29a8b4f06331ef1706f8b53e2a21646a7a508b5d3a03a31da7395e"),
        ("internal/ports/store.go", "5183e6f50b902dbda9a1c937d963804f2f8990e45d70a3c845aaa80b29ba8d97"),
        ("internal/usecases/increment.go", "6e16bd0b739e097ee13dbaacf8767cafb990295a631c083cf147ab2597c10bda"),
];

const GOLD_TS: &[(&str, &str)] = &[
        ("package.json", "3c70bd878008199c28852dd16cb0963d434116b9bd431cba2c032ab62667f536"),
        ("src/adapters/README.md", "79a5f65179aa77209e809cd9db0f45f5efb74641d739348709b43e66bf3e0b0d"),
        ("src/adapters/secondary/README.md", "5bfb5c288008c6acd9dc66faa1986493d5f1f76d7d7fdfa122253b7044df4eb0"),
        ("src/adapters/secondary/in-memory-counter-store.ts", "08ab9d256dba9171f62236d92ed64b9e7494362f9b9f2afd5a857e12352b6ad6"),
        ("src/composition-root.ts", "0bcb57092864cda8838180d146a6513ff8d074af5e3f9a04bbb936c9edc0c2fb"),
        ("src/core/domain/README.md", "78a38ac0de2493b050232634d3c5503ad44d9f8e970380c37f8391879477db06"),
        ("src/core/domain/count.ts", "3775b21f268eb947a1ddbfee6c2e60a2db31ac21eeb0479358c4d96f154c29fd"),
        ("src/core/ports/README.md", "8724b387f04cc0fa166281a14cd7090eb2111d44b32205db5ef093c4968141e0"),
        ("src/core/ports/counter-store.ts", "07e6be0d18399d836391116875d05a705903e8a22f4843e01603e49dd5965407"),
        ("src/core/usecases/README.md", "6fb5cf7af6744a9a9ae4dcf4409d39bff5dd8dc9ac08f575474cb66e47c08e61"),
        ("src/core/usecases/increment.ts", "81fc1d145c03d062410267d4b273440b660a0535320c9cde4cee38eb9d875776"),
        ("src/counter.test.ts", "1b042adb469a6ebbe6a6d146998be31321fbc626a1dfbebc660aa16d0e4fd251"),
        ("tsconfig.json", "e0f519fb9278cbdda98c01311343b886a4d56edae15d0ec64d7f89acd0ef649a"),
];

const GOLD_PYTHON: &[(&str, &str)] = &[
        ("my_app/__init__.py", "45ce5790e581fa489595b6eacb9ee7391831d5669450ce961ea9fa2debd35752"),
        ("my_app/adapters/README.md", "bce02a1cf10df307ad75b6ba7546782d16e73301e6c199bf4b043f014bd8a785"),
        ("my_app/adapters/secondary/README.md", "12baf756f0ca825ce31a002a74b259f99d58df40867f1f45495cd8c997b9ed9d"),
        ("my_app/adapters/secondary/memory.py", "e9a50b026e2c130a4f5679457e286f2dfc05140c1be0e7dd230d99bab1a9e3c9"),
        ("my_app/composition_root.py", "fb8c73fbd5a00ef31ed78abfbd6b2892ce6933ba7233a9692b3a30504edf3e22"),
        ("my_app/domain/README.md", "64c00e7faac88b3ca10d5f226fd2b807f72f01cf2a95635e509e7aedf4ebb9be"),
        ("my_app/domain/count.py", "bba3867f351810320b09b17f549828a89ee41d979a45d9bc66e0f523c9897485"),
        ("my_app/ports/README.md", "d2e44fd7182fd7a14707ece2ee5f854505b229ad2288c2949ebfc8e8467e36c6"),
        ("my_app/ports/store.py", "339899bd0714089108b56b04c057962b495c7aa17f2ac812141a9aeeeb4f21e2"),
        ("my_app/usecases/README.md", "48605da1e6ebd6c5c876321349606d98c4ae8082dbb36c8aa3a6007324315217"),
        ("my_app/usecases/increment.py", "6732b96ffe341910a9d4c9907e7d01bd03c82b34d788a3dee99051673e9f0017"),
        ("pyproject.toml", "30c04b623b09345ce7fcf9abaf0d4da1d8c45874cde3c274b4aea771f1545801"),
        ("tests/__init__.py", "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
        ("tests/test_increment.py", "75067b6f156a3921a32f45514646db536ec233987029598196028e9f9e70bfd0"),
];

fn owned_by(lang: &str, rel: &str) -> bool {
    match lang {
        "rust" => rel == "Cargo.toml" || rel.starts_with("src/") || rel.starts_with("tests/"),
        "go" => rel == "go.mod" || rel.ends_with(".go"),
        "ts" => rel == "package.json" || rel == "tsconfig.json" || rel.starts_with("src/"),
        "python" => rel == "pyproject.toml" || rel.starts_with("my_app/") || rel.starts_with("tests/"),
        _ => false,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    use std::io::Write;
    let mut c = Command::new("sha256sum").stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().expect("sha256sum");
    c.stdin.take().unwrap().write_all(bytes).unwrap();
    String::from_utf8_lossy(&c.wait_with_output().unwrap().stdout).split_whitespace().next().unwrap().to_string()
}

/// S22: The Rust, Go, TypeScript and Python scaffolds are byte-for-byte unchanged
#[test]
fn s22_the_other_scaffolds_are_unchanged() {
    for (lang, gold) in [("rust", GOLD_RUST), ("go", GOLD_GO), ("ts", GOLD_TS), ("python", GOLD_PYTHON)] {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("proj");
        std::fs::create_dir_all(&dir).unwrap();
        let o = init_scaffold(&dir, "my-app", lang);
        assert!(o.status.success(), "{lang}: {}", text(&o));
        let got: Vec<(String, String)> = files(&dir)
            .into_iter()
            .filter(|(p, _)| owned_by(lang, p))
            .map(|(p, b)| (p, sha256_hex(&b)))
            .collect();
        let want: Vec<(String, String)> = gold.iter().map(|(p, h)| (p.to_string(), h.to_string())).collect();
        assert_eq!(got, want, "the {lang} scaffold changed");
    }
    // And the new language is the only addition: a JavaScript file in none of them.
    assert!(hexa_cli::commands::init::SCAFFOLD_LANGS.iter().any(|(l, _)| *l == "js"));
}

/// S23: A JavaScript scaffold never overwrites an existing file
#[test]
fn s23_an_existing_file_is_never_overwritten() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("proj");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("package.json"), "{\"name\":\"mine\",\"private\":true}\n").unwrap();
    let o = init_scaffold(&dir, "my-app", "js");
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(read(&dir, "package.json"), "{\"name\":\"mine\",\"private\":true}\n", "the existing package.json was overwritten");
    for rel in EXPECTED_JS.iter().filter(|p| **p != "package.json") {
        assert!(dir.join(rel).is_file(), "{rel} was not written beside the existing file");
    }
}
