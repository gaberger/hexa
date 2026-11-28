#![allow(unused, clippy::all)]
mod part_1 {
    #![allow(dead_code)]

    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    const GATE: &str = "python3 -m unittest discover -s tests -t .";

    fn expected_files() -> BTreeSet<String> {
        [
            "pyproject.toml",
            "my_app/domain/count.py",
            "my_app/ports/store.py",
            "my_app/usecases/increment.py",
            "my_app/adapters/secondary/memory.py",
            "my_app/composition_root.py",
            "tests/__init__.py",
            "tests/test_increment.py",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    /// Scaffold a Python project named `my-app` into a fresh directory.
    fn scaffold_python() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path().join("proj");
        std::fs::create_dir_all(&dir).expect("create project dir");
        let out = Command::new(env!("CARGO_BIN_EXE_hexa"))
            .arg("init")
            .arg(&dir)
            .args([
                "--name",
                "my-app",
                "--scaffold",
                "--lang",
                "python",
                "--no-claude-md",
            ])
            .output()
            .expect("run hexa");
        assert!(
            out.status.success(),
            "hexa init --scaffold --lang python failed:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        (tmp, dir)
    }

    fn walk(root: &Path) -> Vec<String> {
        fn go(root: &Path, dir: &Path, out: &mut Vec<String>) {
            let Ok(rd) = std::fs::read_dir(dir) else {
                return;
            };
            for e in rd.flatten() {
                let p = e.path();
                let name = e.file_name().to_string_lossy().to_string();
                if name == ".git" || name == "__pycache__" {
                    continue;
                }
                if p.is_dir() {
                    go(root, &p, out);
                } else {
                    let rel = p
                        .strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/");
                    out.push(rel);
                }
            }
        }
        let mut out = Vec::new();
        go(root, root, &mut out);
        out.sort();
        out
    }

    /// The files that belong to the Python scaffold: the root manifest, the
    /// package, the tests, and any Python file anywhere.
    fn python_scaffold_files(root: &Path) -> BTreeSet<String> {
        walk(root)
            .into_iter()
            .filter(|f| {
                f == "pyproject.toml"
                    || f.starts_with("my_app/")
                    || f.starts_with("tests/")
                    || f.ends_with(".py")
                    || f.contains("{{")
            })
            .collect()
    }

    fn read(root: &Path, rel: &str) -> String {
        std::fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
    }

    fn python3_ok() -> bool {
        Command::new("python3")
            .args([
                "-c",
                "import sys; sys.exit(0 if sys.version_info >= (3, 10) else 1)",
            ])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn run_gate(dir: &Path) -> (bool, String) {
        let out = Command::new("python3")
            .args(["-m", "unittest", "discover", "-s", "tests", "-t", "."])
            .current_dir(dir)
            .env("PYTHONNOUSERSITE", "1")
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env("PIP_NO_INDEX", "1")
            .env("http_proxy", "http://127.0.0.1:9")
            .env("https_proxy", "http://127.0.0.1:9")
            .output()
            .expect("run python3");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        (out.status.success(), text)
    }

    
    /// S02: pyproject.toml declares the project, a version and the minimum Python
    #[test]
    fn s02_pyproject_declares_project_version_and_python() {
        let (_t, dir) = scaffold_python();
        let text = read(&dir, "pyproject.toml");
        let value: toml::Value = toml::from_str(&text).expect("pyproject.toml is TOML");
        let project = value
            .get("project")
            .and_then(|p| p.as_table())
            .expect("[project] table");
        assert_eq!(project.get("name").and_then(|v| v.as_str()), Some("my-app"));
        let version = project
            .get("version")
            .and_then(|v| v.as_str())
            .expect("version");
        assert!(!version.trim().is_empty(), "version is non-empty");
        assert_eq!(
            project.get("requires-python").and_then(|v| v.as_str()),
            Some(">=3.10")
        );
    }

    /// S03: The port re-exports the domain's Count, and the test exercises the composition root
    #[test]
    fn s03_port_reexports_count_and_test_uses_composition_root() {
        let (_t, dir) = scaffold_python();
        let store = read(&dir, "my_app/ports/store.py");
        let reexport =
            regex::Regex::new(r"(?m)^\s*from\s+\S*domain\S*\s+import\s+[^\n]*\bCount\b").unwrap();
        assert!(
            reexport.is_match(&store),
            "ports/store.py imports Count from the domain:\n{store}"
        );

        let test = read(&dir, "tests/test_increment.py");
        assert!(test.contains("unittest"), "test uses unittest:\n{test}");
        assert!(
            test.contains("TestCase"),
            "test defines a TestCase:\n{test}"
        );
        let root = regex::Regex::new(
            r"my_app\.composition_root|from\s+my_app\s+import\s+[^\n]*\bcomposition_root\b",
        )
        .unwrap();
        assert!(
            root.is_match(&test),
            "test obtains its use case through my_app.composition_root:\n{test}"
        );

        if python3_ok() {
            let out = Command::new("python3")
            .args([
                "-c",
                "from my_app.ports.store import Count as P\nfrom my_app.domain.count import Count as D\nassert P is D",
            ])
            .current_dir(&dir)
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .output()
            .expect("python3");
            assert!(
                out.status.success(),
                "Count is importable from the port: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }

    /// S04: The in-memory adapter does not import the domain or the use cases
    #[test]
    fn s04_memory_adapter_imports_only_the_port() {
        let (_t, dir) = scaffold_python();
        let src = read(&dir, "my_app/adapters/secondary/memory.py");
        let mut internal = Vec::new();
        for line in src.lines() {
            let l = line.trim();
            let module = if let Some(rest) = l.strip_prefix("from ") {
                rest.split_whitespace().next().unwrap_or("").to_string()
            } else if let Some(rest) = l.strip_prefix("import ") {
                rest.split([',', ' ']).next().unwrap_or("").to_string()
            } else {
                continue;
            };
            if module.starts_with('.') {
                internal.push((module.trim_start_matches('.').to_string(), l.to_string()));
            } else if module == "my_app" || module.starts_with("my_app.") {
                internal.push((
                    module
                        .trim_start_matches("my_app")
                        .trim_start_matches('.')
                        .to_string(),
                    l.to_string(),
                ));
            }
            assert!(
                !l.contains("domain") && !l.contains("usecases"),
                "memory.py imports the domain or use cases: {l}"
            );
        }
        assert!(!internal.is_empty(), "memory.py imports the port:\n{src}");
        for (m, l) in &internal {
            assert!(
                m == "ports" || m.starts_with("ports."),
                "non-port internal import: {l}"
            );
        }
    }

    /// S05: No placeholder or template suffix survives in the Python output
    #[test]
    fn s05_no_placeholder_or_tmpl_survives() {
        let (_t, dir) = scaffold_python();
        for f in walk(&dir) {
            assert!(
                !f.contains("{{name}}") && !f.contains("{{name_snake}}"),
                "placeholder in path: {f}"
            );
            assert!(!f.ends_with(".tmpl"), "template suffix survives: {f}");
        }
        for f in python_scaffold_files(&dir) {
            let text = read(&dir, &f);
            assert!(
                !text.contains("{{name}}") && !text.contains("{{name_snake}}"),
                "placeholder in content of {f}"
            );
        }
        assert!(!python_scaffold_files(&dir).is_empty());
    }

    /// S06: The Python gate is unittest discovery and passes on a fresh scaffold
    #[test]
    fn s06_python_gate_passes_on_fresh_scaffold() {
        let (_t, dir) = scaffold_python();
        assert!(
            dir.join("tests/test_increment.py").is_file(),
            "scaffold wrote the test"
        );
        if !python3_ok() {
            eprintln!("python3 >= 3.10 not available; skipping gate run");
            return;
        }
        let (ok, text) = run_gate(&dir);
        assert!(ok, "gate exits 0:\n{text}");
        let ran = text
            .find("Ran 1 test")
            .unwrap_or_else(|| panic!("reports Ran 1 test:\n{text}"));
        assert!(
            text[ran..].contains("OK"),
            "reports OK after Ran 1 test:\n{text}"
        );
    }

    /// S07: SCAFFOLD_LANGS names python with its gate command
    #[test]
    fn s07_scaffold_langs_names_python_gate() {
        let entry = hexa_cli::commands::init::SCAFFOLD_LANGS
            .iter()
            .find(|(l, _)| *l == "python")
            .expect("python is a scaffold language");
        assert_eq!(entry.1, GATE);
        assert_eq!(
            hexa_cli::commands::init::scaffold_gate("python"),
            Some(GATE)
        );
    }

    /// S08: The Python gate needs no install step and no network
    #[test]
    fn s08_python_gate_needs_no_install_or_network() {
        let gate = hexa_cli::commands::init::scaffold_gate("python").expect("python gate");
        for bad in ["pip", "install", "venv", "&&"] {
            assert!(!gate.contains(bad), "gate contains {bad}: {gate}");
        }
        let (_t, dir) = scaffold_python();
        if !python3_ok() {
            eprintln!("python3 >= 3.10 not available; skipping gate run");
            return;
        }
        let (ok, text) = run_gate(&dir);
        assert!(ok, "gate passes without network or packages:\n{text}");
    }

    /// S09: A failing unittest run does not count as a passing gate
    #[test]
    fn s09_failing_unittest_run_is_not_a_pass() {
        let (_t, dir) = scaffold_python();
        let path = dir.join("tests/test_increment.py");
        let mut text = read(&dir, "tests/test_increment.py");
        text.push_str(
        "\n\nimport unittest as _u\n\nclass ForcedFailure(_u.TestCase):\n    def test_fails(self):\n        self.assertEqual(1, 2)\n",
    );
        std::fs::write(&path, text).expect("edit test");
        if !python3_ok() {
            eprintln!("python3 >= 3.10 not available; skipping gate run");
            return;
        }
        let (ok, out) = run_gate(&dir);
        assert!(!ok, "a failing test makes the gate exit non-zero:\n{out}");
        assert!(out.contains("FAILED"), "output reports FAILED:\n{out}");
        assert!(
            !out.lines().any(|l| l.trim() == "OK"),
            "output does not report OK:\n{out}"
        );
    }

    /// S10: Placeholders in an asset's path are rendered
    #[test]
    fn s10_placeholders_in_asset_path_are_rendered() {
        let (_t, dir) = scaffold_python();
        assert!(
            dir.join("my_app/domain/count.py").is_file(),
            "count.py written under my_app/"
        );
        assert!(
            !dir.join("{{name_snake}}").exists(),
            "no literal placeholder directory"
        );
        assert!(!dir.join("my_app/domain/count.py.tmpl").exists());
    }
}

mod part_2 {
    #![allow(dead_code, unused_imports)]

    use hexa_analysis::domain::HexLayer;
    use hexa_core::domain::project_floor::{gitignore_lines, merged_gitignore};
    use serde_json::Value;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output, Stdio};

    fn hexa() -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_hexa"));
        c.stdin(Stdio::null())
            .env("GIT_AUTHOR_NAME", "hexa-test")
            .env("GIT_AUTHOR_EMAIL", "hexa-test@example.com")
            .env("GIT_COMMITTER_NAME", "hexa-test")
            .env("GIT_COMMITTER_EMAIL", "hexa-test@example.com")
            .env("NO_COLOR", "1");
        c
    }

    fn describe(out: &Output) -> String {
        format!(
            "status: {:?}\nstdout:\n{}\nstderr:\n{}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    }

    /// Run `hexa init <dir> --name <name> --scaffold --lang <lang>`.
    fn scaffold(dir: &Path, lang: &str, name: &str) -> Output {
        fs::create_dir_all(dir).unwrap();
        hexa()
            .current_dir(dir)
            .arg("init")
            .arg(dir)
            .arg("--name")
            .arg(name)
            .arg("--scaffold")
            .arg("--lang")
            .arg(lang)
            .output()
            .expect("run hexa init")
    }

    fn scaffold_ok(dir: &Path, lang: &str, name: &str) {
        let out = scaffold(dir, lang, name);
        assert!(
            out.status.success(),
            "hexa init --scaffold --lang {lang} failed\n{}",
            describe(&out)
        );
    }

    /// Every file and directory under `root`, relative, `/`-separated, skipping `.git`.
    fn all_paths(root: &Path) -> Vec<String> {
        fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
            let Ok(rd) = fs::read_dir(dir) else { return };
            for e in rd.flatten() {
                let p = e.path();
                let rel = p
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                if rel == ".git" || rel.starts_with(".git/") {
                    continue;
                }
                out.push(rel);
                if p.is_dir() {
                    walk(root, &p, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(root, root, &mut out);
        out.sort();
        out
    }

    fn parse_json_loose(text: &str) -> Option<Value> {
        if let Ok(v) = serde_json::from_str::<Value>(text.trim()) {
            return Some(v);
        }
        let start = text.find('{')?;
        let end = text.rfind('}')?;
        serde_json::from_str::<Value>(&text[start..=end]).ok()
    }

    /// The analysis of `dir`, as JSON.
    fn analyze_json(dir: &Path) -> Value {
        let attempts: [&[&str]; 2] = [
            &["analyze", ".", "--json"],
            &["analyze", ".", "--format", "json"],
        ];
        let mut last = String::new();
        for args in attempts {
            let out = hexa()
                .current_dir(dir)
                .args(args)
                .output()
                .expect("run hexa analyze");
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            if let Some(v) = parse_json_loose(&stdout) {
                return v;
            }
            last = describe(&out);
        }
        panic!("hexa analyze produced no JSON\n{last}");
    }

    /// The object that holds `health_score` (the analysis result).
    fn find_result(v: &Value) -> Option<&Value> {
        match v {
            Value::Object(m) => {
                if m.contains_key("health_score") {
                    return Some(v);
                }
                m.values().find_map(find_result)
            }
            Value::Array(a) => a.iter().find_map(find_result),
            _ => None,
        }
    }

    fn find_key<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
        match v {
            Value::Object(m) => {
                if let Some(x) = m.get(key) {
                    return Some(x);
                }
                m.values().find_map(|x| find_key(x, key))
            }
            Value::Array(a) => a.iter().find_map(|x| find_key(x, key)),
            _ => None,
        }
    }

    fn path_matches(path: &str, suffix: &str) -> bool {
        path == suffix || path.ends_with(&format!("/{suffix}"))
    }

    /// The layer the analysis gives the file ending in `suffix`, wherever the
    /// JSON names it.
    fn find_layer(v: &Value, suffix: &str) -> Option<String> {
        match v {
            Value::Object(m) => {
                for (fk, lk) in [
                    ("file", "layer"),
                    ("path", "layer"),
                    ("from_file", "from_layer"),
                    ("to_file", "to_layer"),
                ] {
                    if let (Some(f), Some(l)) = (
                        m.get(fk).and_then(Value::as_str),
                        m.get(lk).and_then(Value::as_str),
                    ) {
                        if path_matches(f, suffix) {
                            return Some(l.to_string());
                        }
                    }
                }
                for (k, val) in m {
                    if path_matches(k, suffix) {
                        if let Some(s) = val.as_str() {
                            return Some(s.to_string());
                        }
                        if let Some(s) = val.get("layer").and_then(Value::as_str) {
                            return Some(s.to_string());
                        }
                    }
                }
                m.values().find_map(|x| find_layer(x, suffix))
            }
            Value::Array(a) => a.iter().find_map(|x| find_layer(x, suffix)),
            _ => None,
        }
    }

    /// The layer the grade's classifier gives the file under `root` whose path
    /// ends with `suffix` (`hexa analyze --json` names a file's layer only on a
    /// violation edge, and a clean scaffold has none).
    fn layer_in(root: &Path, suffix: &str) -> Option<String> {
        fn walk(d: &Path, root: &Path, out: &mut Vec<String>) {
            for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    if p.file_name().is_some_and(|n| n != ".git") {
                        walk(&p, root, out);
                    }
                } else if let Ok(r) = p.strip_prefix(root) {
                    out.push(r.to_string_lossy().replace('\\', "/"));
                }
            }
        }
        let mut files = Vec::new();
        walk(root, root, &mut files);
        let rel = files.into_iter().find(|f| f == suffix || f.ends_with(&format!("/{suffix}")))?;
        let map = hexa_analysis::layer_classifier::LayerMap::from_project(root).ok()?;
        Some(map.classify(&rel).to_string())
    }

    fn is_layer(found: &str, expected: HexLayer) -> bool {
        serde_json::from_value::<HexLayer>(Value::String(found.to_string())).ok() == Some(expected)
            || found == expected.to_string()
    }

    fn assert_a_plus(result: &Value) {
        if let Some(g) = find_key(result, "grade").and_then(Value::as_str) {
            assert_eq!(g, "A+", "grade: {result}");
        }
        let score = result["health_score"].as_u64().expect("health_score");
        assert!(score >= 95, "health score {score} is not A+: {result}");
    }

    fn empty_array(result: &Value, key: &str) -> bool {
        match result.get(key) {
            None => true,
            Some(Value::Array(a)) => a.is_empty(),
            Some(Value::Null) => true,
            _ => false,
        }
    }

    fn proj(tmp: &tempfile::TempDir) -> PathBuf {
        tmp.path().join("proj")
    }

    /// S11: {{name}} in a path is rendered to the name as given
    #[test]
    fn s11_name_placeholder_in_a_path_is_rendered() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = proj(&tmp);
        scaffold_ok(&dir, "python", "my-app");
        let paths = all_paths(&dir);
        for p in &paths {
            assert!(
                !p.contains("{{") && !p.contains("}}"),
                "a placeholder was left unrendered in the path {p}"
            );
        }
        // The package directory carries the project's name, rendered.
        assert!(
            dir.join("my_app").is_dir(),
            "the package directory named after the project is missing; paths: {paths:?}"
        );
        assert!(
            dir.join("my_app/domain/count.py").is_file(),
            "paths: {paths:?}"
        );
    }

    /// S13: Path rendering does not overwrite an existing file
    #[test]
    fn s13_path_rendering_does_not_overwrite_an_existing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = proj(&tmp);
        fs::create_dir_all(dir.join("my_app/domain")).unwrap();
        let custom = "# my own count, do not touch\nCOUNT = 42\n";
        fs::write(dir.join("my_app/domain/count.py"), custom).unwrap();
        let out = scaffold(&dir, "python", "my-app");
        assert!(out.status.success(), "{}", describe(&out));
        assert_eq!(
            fs::read_to_string(dir.join("my_app/domain/count.py")).unwrap(),
            custom,
            "the existing file was overwritten"
        );
        assert!(
            dir.join("my_app/ports/store.py").is_file(),
            "the rest of the Python scaffold was not written"
        );
    }

    /// S14: An asset's path is rendered by the same substitution as its contents
    #[test]
    fn s14_asset_path_rendered_by_same_substitution_as_contents() {
        for (name, snake) in [
            ("my-app", "my_app"),
            ("counter", "counter"),
            ("big-red-dog", "big_red_dog"),
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let dir = proj(&tmp);
            scaffold_ok(&dir, "python", name);
            let paths = all_paths(&dir);
            for p in &paths {
                assert!(!p.contains("{{"), "{name}: unrendered path {p}");
                assert!(!p.ends_with(".tmpl"), "{name}: .tmpl suffix kept on {p}");
            }
            for rel in [
                "domain/count.py",
                "ports/store.py",
                "usecases/increment.py",
                "adapters/secondary/memory.py",
                "composition_root.py",
            ] {
                let p = dir.join(snake).join(rel);
                assert!(
                    p.is_file(),
                    "{name}: expected {snake}/{rel}; paths: {paths:?}"
                );
            }
            // The contents name the package by the same substitution as the path.
            let mut found_snake = false;
            for p in &paths {
                let full = dir.join(p);
                if full.is_file() && p.ends_with(".py") {
                    let text = fs::read_to_string(&full).unwrap_or_default();
                    assert!(!text.contains("{{"), "{name}: unrendered contents in {p}");
                    if text.contains(snake) {
                        found_snake = true;
                    }
                }
            }
            assert!(
                found_snake,
                "{name}: no Python file's contents name the package {snake}"
            );
        }
    }

    /// S15: Python's ignore lines include __pycache__/ and .venv/
    #[test]
    fn s15_python_ignore_lines() {
        let lines = gitignore_lines("python");
        for l in [
            "__pycache__/",
            ".venv/",
            ".hexa/loop.json",
            ".hexa/loop.lock",
            ".env*",
            "!.env.example",
        ] {
            assert!(lines.contains(&l), "missing {l} in {lines:?}");
        }
    }

    /// S16: Python's ignore lines are Python's only
    #[test]
    fn s16_python_ignore_lines_are_pythons_only() {
        let py = gitignore_lines("python");
        assert!(
            py.contains(&"__pycache__/") && py.contains(&".venv/"),
            "{py:?}"
        );
        for l in ["target/", "node_modules/", "dist/"] {
            assert!(!py.contains(&l), "python holds {l}: {py:?}");
        }
        for lang in ["rust", "ts", "go"] {
            let ls = gitignore_lines(lang);
            for l in ["__pycache__/", ".venv/"] {
                assert!(!ls.contains(&l), "{lang} holds {l}: {ls:?}");
            }
        }
    }

    /// S17: A merged Python .gitignore always holds each Python line exactly once
    #[test]
    fn s17_merged_python_gitignore_holds_each_line_once() {
        let inputs: Vec<Option<&str>> = vec![
            None,
            Some(""),
            Some("\n"),
            Some("*.log"),
            Some("*.log\nbuild/\n"),
            Some("__pycache__/\n"),
            Some("  .venv/  \nfoo\n"),
            Some("__pycache__/\n.venv/\n.hexa/loop.json\n.hexa/loop.lock\n.env*\n!.env.example\n"),
            Some("# comment\n\n.venv/\nsecret.txt"),
            Some("target/\nnode_modules/\n"),
        ];
        for existing in inputs {
            let merged = merged_gitignore(existing, "python");
            let text = merged
                .clone()
                .unwrap_or_else(|| existing.unwrap_or("").to_string());
            for l in ["__pycache__/", ".venv/"] {
                let n = text.lines().filter(|x| x.trim() == l).count();
                assert_eq!(n, 1, "{l} appears {n} times for {existing:?}: {text:?}");
            }
            let old: Vec<&str> = existing.unwrap_or("").lines().collect();
            let new: Vec<&str> = text.lines().collect();
            assert!(new.len() >= old.len(), "lines lost for {existing:?}");
            assert_eq!(
                &new[..old.len()],
                &old[..],
                "existing lines not kept in order for {existing:?}"
            );
        }
    }

    /// S18: hexa analyze grades the fresh Python scaffold A+ with full coverage
    #[test]
    fn s18_analyze_grades_python_scaffold_a_plus() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = proj(&tmp);
        scaffold_ok(&dir, "python", "my-app");
        let json = analyze_json(&dir);
        let result = find_result(&json).unwrap_or_else(|| panic!("no analysis result in {json}"));
        assert_a_plus(result);
        let cov = &result["coverage"];
        assert_eq!(cov["classified"], cov["total"], "coverage: {cov}");
        assert!(
            cov["total"].as_u64().unwrap_or(0) > 0,
            "nothing graded: {cov}"
        );
        assert!(empty_array(cov, "unclassified"), "unclassified: {cov}");
        for key in ["violations", "circular_deps", "config_errors"] {
            assert!(empty_array(result, key), "{key} not empty: {}", result[key]);
        }
        let layer = layer_in(&dir, "composition_root.py")
            .unwrap_or_else(|| panic!("no layer for composition_root.py in {json}"));
        assert!(
            is_layer(&layer, HexLayer::CompositionRoot),
            "composition_root.py is {layer}"
        );
    }

    /// S19: The Python scaffold satisfies the domain policy it ships
    #[test]
    fn s19_python_scaffold_satisfies_its_domain_policy() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = proj(&tmp);
        scaffold_ok(&dir, "python", "my-app");
        assert!(
            dir.join("my_app/domain/count.py").is_file(),
            "no Python scaffold written"
        );
        let out = hexa()
            .current_dir(&dir)
            .args(["analyze", ".", "--exit-code"])
            .output()
            .expect("run hexa analyze");
        assert!(
            out.status.success(),
            "the domain policy reported an error-severity finding\n{}",
            describe(&out)
        );
    }

    /// S20: No Python scaffold file is left unclassified or in the wrong layer
    #[test]
    fn s20_no_python_file_unclassified_or_misplaced() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = proj(&tmp);
        scaffold_ok(&dir, "python", "my-app");
        let json = analyze_json(&dir);
        let result = find_result(&json).unwrap_or_else(|| panic!("no analysis result in {json}"));
        let cov = &result["coverage"];
        assert!(
            empty_array(cov, "unclassified"),
            "unclassified files: {cov}"
        );
        assert_eq!(cov["classified"], cov["total"], "coverage: {cov}");
        for (file, expected) in [
            ("domain/count.py", HexLayer::Domain),
            ("ports/store.py", HexLayer::Ports),
            ("usecases/increment.py", HexLayer::Usecases),
            ("adapters/secondary/memory.py", HexLayer::AdaptersSecondary),
            ("composition_root.py", HexLayer::CompositionRoot),
        ] {
            let layer =
                layer_in(&dir, file).unwrap_or_else(|| panic!("no layer for {file} in {json}"));
            assert!(!is_layer(&layer, HexLayer::Unknown), "{file} is unknown");
            assert!(
                is_layer(&layer, expected),
                "{file} is {layer}, expected {expected}"
            );
        }
    }
}

mod part_3 {
    #![allow(dead_code, clippy::all)]

    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output, Stdio};

    fn hexa() -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_hexa"));
        c.stdin(Stdio::null());
        c.env("NO_COLOR", "1");
        c
    }

    fn run(mut c: Command) -> Output {
        c.output().expect("failed to spawn the hexa binary")
    }

    fn text(o: &Output) -> String {
        format!(
            "{}\n{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        )
    }

    /// Every file under `root`, by relative path with `/` separators, with its bytes.
    fn files(root: &Path) -> BTreeMap<String, Vec<u8>> {
        fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for e in entries.flatten() {
                let p = e.path();
                let ft = e.file_type().expect("file type");
                if ft.is_dir() {
                    walk(root, &p, out);
                } else if ft.is_file() {
                    let rel = p
                        .strip_prefix(root)
                        .expect("under root")
                        .to_string_lossy()
                        .replace('\\', "/");
                    out.insert(rel, std::fs::read(&p).expect("read file"));
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(root, root, &mut out);
        out
    }

    fn snake(name: &str) -> String {
        name.replace('-', "_").to_lowercase()
    }

    /// `hexa init <dir> --name <name> --scaffold --lang <lang>`.
    fn init_scaffold(dir: &Path, name: &str, lang: &str) -> Output {
        let mut c = hexa();
        c.current_dir(dir).arg("init").arg(dir).args([
            "--name",
            name,
            "--scaffold",
            "--lang",
            lang,
        ]);
        run(c)
    }

    /// The files that belong to the Python scaffold for `name`.
    fn python_scaffold_files(
        all: &BTreeMap<String, Vec<u8>>,
        name: &str,
    ) -> BTreeMap<String, Vec<u8>> {
        let pkg = format!("{}/", snake(name));
        all.iter()
            .filter(|(p, _)| {
                p.as_str() == "pyproject.toml" || p.starts_with(&pkg) || p.starts_with("tests/")
            })
            .filter(|(p, _)| !p.contains("__pycache__"))
            .map(|(p, b)| (p.clone(), b.clone()))
            .collect()
    }

    /// The files a scaffold of `lang` writes, excluding what `hexa init` adds around it.
    fn scaffold_only(all: &BTreeMap<String, Vec<u8>>) -> BTreeMap<String, Vec<u8>> {
        all.iter()
            .filter(|(p, _)| {
                !(p.starts_with(".hexa/")
                    || p.starts_with(".claude/")
                    || p.starts_with(".git/")
                    || p.starts_with("docs/")
                    || p.as_str() == "CLAUDE.md"
                    || p.as_str() == ".gitignore"
                    || p.as_str() == ".mcp.json")
            })
            .map(|(p, b)| (p.clone(), b.clone()))
            .collect()
    }

    /// S21: A fresh Python scaffold needs no edit before its gate passes
    #[test]
    fn s21_fresh_python_scaffold_gate_passes_without_edit() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let name = "counter_app";
        let o = init_scaffold(dir, name, "python");
        assert!(
            o.status.success(),
            "hexa init --scaffold --lang python failed:\n{}",
            text(&o)
        );

        let pkg = snake(name);
        for f in [
            "pyproject.toml".to_string(),
            format!("{pkg}/domain/count.py"),
            format!("{pkg}/ports/store.py"),
            format!("{pkg}/usecases/increment.py"),
            format!("{pkg}/adapters/secondary/memory.py"),
            format!("{pkg}/composition_root.py"),
            "tests/__init__.py".to_string(),
            "tests/test_increment.py".to_string(),
        ] {
            assert!(dir.join(&f).is_file(), "scaffold did not write {f}");
        }

        let gate = Command::new("python3")
            .current_dir(dir)
            .args(["-m", "unittest", "discover", "-s", "tests", "-t", "."])
            .stdin(Stdio::null())
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .output()
            .expect("python3 must be available to run the Python gate");
        let out = text(&gate);
        assert!(gate.status.success(), "Python gate did not exit 0:\n{out}");
        assert!(
            out.contains("Ran 1 test"),
            "gate output lacks `Ran 1 test`:\n{out}"
        );
        assert!(out.contains("OK"), "gate output lacks `OK`:\n{out}");
    }

    /// S22: The Python scaffold is deterministic
    #[test]
    fn s22_python_scaffold_is_deterministic() {
        for name in ["counter", "my-app", "demo_proj"] {
            let a = tempfile::tempdir().unwrap();
            let b = tempfile::tempdir().unwrap();
            let oa = init_scaffold(a.path(), name, "python");
            assert!(
                oa.status.success(),
                "first run for {name} failed:\n{}",
                text(&oa)
            );
            let ob = init_scaffold(b.path(), name, "python");
            assert!(
                ob.status.success(),
                "second run for {name} failed:\n{}",
                text(&ob)
            );

            let fa = python_scaffold_files(&files(a.path()), name);
            let fb = python_scaffold_files(&files(b.path()), name);
            assert!(
                fa.contains_key("pyproject.toml"),
                "no pyproject.toml for {name}"
            );
            let pkg = snake(name);
            assert!(
                fa.contains_key(&format!("{pkg}/composition_root.py")),
                "no {pkg}/composition_root.py for {name}; got {:?}",
                fa.keys().collect::<Vec<_>>()
            );
            assert_eq!(
                fa.keys().collect::<Vec<_>>(),
                fb.keys().collect::<Vec<_>>(),
                "the two runs for {name} wrote different paths"
            );
            for (p, bytes) in &fa {
                assert_eq!(
                    Some(bytes),
                    fb.get(p),
                    "{p} differs between runs for {name}"
                );
            }
        }
    }

    /// S23: --lang still defaults to rust
    #[test]
    fn s23_lang_defaults_to_rust() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("demo");
        std::fs::create_dir(&dir).unwrap();
        let mut c = hexa();
        // `hexa init --scaffold` takes `--lang` offline; `hexa scaffold` also
        // needs a description, `--target` and a model call.
        c.current_dir(&dir).args(["init", "--scaffold", "."]);
        let o = run(c);
        assert!(o.status.success(), "hexa scaffold failed:\n{}", text(&o));

        let all = files(&dir);
        assert!(
            all.contains_key("Cargo.toml"),
            "no Cargo.toml written; got {:?}",
            all.keys().collect::<Vec<_>>()
        );
        assert!(
            all.keys().any(|p| p.ends_with(".rs")),
            "no Rust source written"
        );
        let py: Vec<_> = all
            .keys()
            .filter(|p| p.ends_with(".py") || p.ends_with(".pyi") || p.as_str() == "pyproject.toml")
            .collect();
        assert!(
            py.is_empty(),
            "the default scaffold wrote Python files: {py:?}"
        );
    }

    /// S24: An unknown --lang is refused and names python among the known ones
    #[test]
    fn s24_unknown_lang_refused_naming_python() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("demo");
        std::fs::create_dir(&dir).unwrap();
        let mut c = hexa();
        // `hexa scaffold` checks the language before it writes anything or
        // calls a model; it needs a description and `--target`.
        c.current_dir(&dir).args(["scaffold", "a counter", "--target", ".", "--lang", "cobol"]);
        let o = run(c);
        let msg = text(&o);
        assert!(!o.status.success(), "`--lang cobol` was accepted:\n{msg}");
        let written = files(&dir);
        assert!(
            written.is_empty(),
            "files were written: {:?}",
            written.keys().collect::<Vec<_>>()
        );
        for lang in ["python", "rust", "go", "ts"] {
            assert!(msg.contains(lang), "refusal does not name `{lang}`:\n{msg}");
        }
    }

    /// S25: The Rust, Go and TypeScript scaffolds are byte-for-byte unchanged
    #[test]
    fn s25_other_scaffolds_unchanged() {
        // `python` is now a scaffold language as well, and the others keep their gates.
        assert!(hexa_cli::commands::init::scaffold_gate("python").is_some());
        assert_eq!(
            hexa_cli::commands::init::scaffold_gate("rust"),
            Some("cargo test")
        );
        assert_eq!(
            hexa_cli::commands::init::scaffold_gate("go"),
            Some("go test ./...")
        );
        assert_eq!(
            hexa_cli::commands::init::scaffold_gate("ts"),
            Some("npm install && npm test")
        );

        for name in ["counter", "my-app"] {
            for (lang, manifest) in [
                ("rust", "Cargo.toml"),
                ("go", "go.mod"),
                ("ts", "package.json"),
            ] {
                let a = tempfile::tempdir().unwrap();
                let b = tempfile::tempdir().unwrap();
                let oa = init_scaffold(a.path(), name, lang);
                assert!(
                    oa.status.success(),
                    "{lang} scaffold for {name} failed:\n{}",
                    text(&oa)
                );
                let ob = init_scaffold(b.path(), name, lang);
                assert!(
                    ob.status.success(),
                    "{lang} scaffold for {name} failed:\n{}",
                    text(&ob)
                );

                let fa = scaffold_only(&files(a.path()));
                let fb = scaffold_only(&files(b.path()));
                assert!(
                    fa.contains_key(manifest),
                    "{lang} scaffold lacks {manifest}"
                );
                assert_eq!(
                    fa.keys().collect::<Vec<_>>(),
                    fb.keys().collect::<Vec<_>>(),
                    "{lang} scaffold paths differ between runs"
                );
                for (p, bytes) in &fa {
                    assert_eq!(Some(bytes), fb.get(p), "{lang}: {p} differs between runs");
                    assert!(!p.contains("{{"), "{lang}: unrendered path {p}");
                    assert!(
                        !p.ends_with(".py") && p.as_str() != "pyproject.toml",
                        "{lang} scaffold wrote a Python file {p}"
                    );
                    assert!(!p.ends_with(".tmpl"), "{lang}: .tmpl suffix kept on {p}");
                    let s = String::from_utf8_lossy(bytes);
                    assert!(
                        !s.contains("{{name}}") && !s.contains("{{name_snake}}"),
                        "{lang}: {p} holds an unrendered placeholder"
                    );
                }
            }
        }
    }
}
