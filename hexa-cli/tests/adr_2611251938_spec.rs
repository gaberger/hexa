#![allow(unused, clippy::all)]
mod part_1 {
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    const MANIFESTS: [&str; 4] = [
        "pyproject.toml",
        "setup.py",
        "manage.py",
        "requirements.txt",
    ];

    fn project() -> tempfile::TempDir {
        // A visible prefix, so that a file walker does not skip the root as hidden.
        tempfile::Builder::new()
            .prefix("hexaproj-")
            .tempdir()
            .expect("temp project")
    }

    fn write(root: &Path, rel: &str, body: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(path, body).expect("write");
    }

    fn remove(root: &Path, rel: &str) {
        fs::remove_file(root.join(rel)).expect("remove");
    }

    fn rust_base(root: &Path) {
        write(
            root,
            "Cargo.toml",
            "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        write(root, "src/main.rs", "mod domain;\n\nfn main() {}\n");
        write(root, "src/domain/mod.rs", "fn _private_rule() {}\n");
    }

    fn layers(root: &Path, entries: &[(&str, &str)]) {
        let map: serde_json::Map<String, serde_json::Value> = entries
            .iter()
            .map(|(k, v)| (k.to_string(), serde_json::Value::String(v.to_string())))
            .collect();
        let config = serde_json::json!({ "analyze": { "layers": map } });
        write(
            root,
            ".hexa/project.json",
            &serde_json::to_string_pretty(&config).expect("json"),
        );
    }

    fn analyze(root: &Path) -> String {
        let home = tempfile::tempdir().expect("home");
        let out = Command::new(env!("CARGO_BIN_EXE_hexa"))
            .args(["analyze", "."])
            .current_dir(root)
            .env("HOME", home.path())
            .env("NO_COLOR", "1")
            .env("CLICOLOR", "0")
            .env_remove("CLICOLOR_FORCE")
            .output()
            .expect("run hexa analyze");
        let raw = format!(
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let ansi = regex::Regex::new("\x1b\\[[0-9;]*[A-Za-z]").expect("regex");
        let text = ansi.replace_all(&raw, "").into_owned();
        let lower = text.to_lowercase();
        assert!(
            lower.contains("score") || lower.contains("grade"),
            "hexa analyze did not produce a graded report:\n{text}"
        );
        text
    }

    /// The report without the ungraded-Python notice and without timings.
    fn comparable(report: &str) -> Vec<String> {
        let timing = regex::Regex::new(r"\b\d+(\.\d+)?\s*(ms|µs|s)\b").expect("regex");
        report
            .lines()
            .filter(|line| {
                let l = line.to_lowercase();
                !(l.contains("python")
                    || l.contains("pyproject")
                    || l.contains("analyze.layers")
                    || l.contains(".pyi"))
            })
            .map(|line| timing.replace_all(line.trim(), "<time>").into_owned())
            .filter(|line| !line.is_empty())
            .collect()
    }

    fn assert_ungraded_notice(report: &str, count: usize) {
        let number =
            regex::Regex::new(&format!(r"(^|[^\w.\-]){count}($|[^\w.\-])")).expect("regex");
        let has_count = report.lines().any(|line| {
            let l = line.to_lowercase();
            l.contains("python")
                && (l.contains("not graded")
                    || l.contains("ungraded")
                    || l.contains("not been graded"))
                && number.is_match(&l)
        });
        assert!(
            has_count,
            "expected a notice that {count} Python file(s) were not graded:\n{report}"
        );
        let lower = report.to_lowercase();
        assert!(
            [
                "pyproject.toml",
                "setup.py",
                "requirements.txt",
                "manifest",
                "analyze.layers"
            ]
            .iter()
            .any(|hint| lower.contains(hint)),
            "expected the notice to say how to have the files graded:\n{report}"
        );
    }

    fn assert_absent(report: &str, needles: &[&str]) {
        for needle in needles {
            assert!(
                !report.contains(needle),
                "`{needle}` must not appear in the report:\n{report}"
            );
        }
    }

    fn assert_present(report: &str, needle: &str) {
        assert!(
            report.contains(needle),
            "`{needle}` must appear in the report:\n{report}"
        );
    }

    /// S01: Helper scripts in a project with no Python manifest and no layer covering them are left ungraded.
    #[test]
    fn s01_helper_scripts_without_manifest_are_ungraded() {
        let dir = project();
        let root = dir.path();
        rust_base(root);
        write(
            root,
            "package.json",
            "{\"name\":\"fixture\",\"version\":\"0.1.0\"}\n",
        );
        write(root, "web/domain/value.ts", "const hidden = 1;\n");
        let scripts = [
            (
                "scripts/zq_release.py",
                "def zq_publish():\n    return 1\n\nZQ_TARGET = 'x'\n",
            ),
            (
                "scripts/zq_cleanup.py",
                "class ZqSweeper:\n    pass\n\ndef zq_sweep():\n    return 2\n",
            ),
            ("tools/zq_report.py", "def zq_render():\n    return 3\n"),
        ];
        for (path, body) in scripts {
            write(root, path, body);
        }

        let with_python = analyze(root);
        for (path, _) in scripts {
            remove(root, path);
        }
        let without_python = analyze(root);

        assert_absent(
            &with_python,
            &[
                "zq_release",
                "zq_cleanup",
                "zq_report",
                "zq_publish",
                "ZQ_TARGET",
                "ZqSweeper",
                "zq_sweep",
                "zq_render",
            ],
        );
        assert_eq!(comparable(&with_python), comparable(&without_python));
    }

    /// S02: The output says how many Python files were not graded and how to have them graded.
    #[test]
    fn s02_output_counts_ungraded_python_files() {
        let dir = project();
        let root = dir.path();
        rust_base(root);
        write(root, "scripts/one.py", "def zq_one():\n    return 1\n");
        write(root, "scripts/two.py", "def zq_two():\n    return 2\n");
        write(root, "scripts/three.pyi", "def zq_three() -> int: ...\n");

        let report = analyze(root);
        assert_ungraded_notice(&report, 3);
    }

    /// S03: Any one of the Python manifests causes Python to be graded.
    #[test]
    fn s03_each_manifest_enables_python_grading() {
        for manifest in MANIFESTS {
            let dir = project();
            let root = dir.path();
            rust_base(root);
            write(root, "pkg/zq_module.py", "def zq_orphan():\n    return 1\n");

            let before = analyze(root);
            assert_absent(&before, &["zq_orphan"]);

            let body = match manifest {
                "pyproject.toml" => "[project]\nname = \"fixture\"\nversion = \"0.1.0\"\n",
                "requirements.txt" => "requests\n",
                _ => "# project entry\n",
            };
            write(root, manifest, body);
            let after = analyze(root);
            assert!(
                after.contains("zq_orphan"),
                "with {manifest}, zq_orphan must be a dead export:\n{after}"
            );
        }
    }

    /// S04: A layer key covering one Python file opts the project's Python into grading without a manifest.
    #[test]
    fn s04_layer_key_covering_one_file_grades_all_python() {
        let dir = project();
        let root = dir.path();
        rust_base(root);
        write(
            root,
            "covered/zq_first.py",
            "def zq_unused_first():\n    return 1\n",
        );
        write(
            root,
            "loose/zq_second.py",
            "def zq_unused_second():\n    return 2\n",
        );

        let before = analyze(root);
        assert_absent(&before, &["zq_unused_first", "zq_unused_second"]);

        layers(root, &[("covered/", "domain")]);
        let after = analyze(root);
        assert_present(&after, "zq_unused_first");
        assert_present(&after, "zq_unused_second");

        // With no public name left, only layer coverage can still name the second file.
        write(root, "loose/zq_second.py", "_hidden = 2\n");
        let coverage = analyze(root);
        assert_present(&coverage, "zq_second");
    }

    /// S05: A layer key covering only a .pyi file also opts Python into grading.
    #[test]
    fn s05_layer_key_covering_only_pyi_enables_grading() {
        let dir = project();
        let root = dir.path();
        rust_base(root);
        write(root, "stubs/zq_types.pyi", "def zq_stubbed() -> int: ...\n");
        write(
            root,
            "loose/zq_worker.py",
            "def zq_unused_worker():\n    return 1\n",
        );

        let before = analyze(root);
        assert_absent(&before, &["zq_unused_worker"]);

        layers(root, &[("stubs/", "domain")]);
        let after = analyze(root);
        assert_present(&after, "zq_unused_worker");
        assert_present(&after, "zq_stubbed");
    }

    /// S06: Ungraded Python produces no import, cycle, policy or coverage findings and does not cap the score.
    #[test]
    fn s06_ungraded_python_produces_no_findings_and_no_cap() {
        let dir = project();
        let root = dir.path();
        rust_base(root);
        let files = [
        (
            "src/domain/zq_cycle_a.py",
            "from .zq_cycle_b import zq_b\n\ndef zq_a():\n    return zq_b\n",
        ),
        (
            "src/domain/zq_cycle_b.py",
            "from .zq_cycle_a import zq_a\n\ndef zq_b():\n    return zq_a\n",
        ),
        (
            "src/domain/zq_policy.py",
            "from ..adapters.secondary.zq_store import zq_save\n\ndef zq_rule():\n    return zq_save\n",
        ),
        (
            "src/adapters/secondary/zq_store.py",
            "def zq_save():\n    return 1\n",
        ),
        (
            "src/domain/zq_broken.py",
            "from .zq_missing import zq_nothing\n",
        ),
        ("zq_loose_one.py", "def zq_loose_one_fn():\n    pass\n"),
        ("misc/zq_loose_two.py", "def zq_loose_two_fn():\n    pass\n"),
        ("other/zq_loose_three.py", "ZQ_LOOSE = 3\n"),
    ];
        for (path, body) in files {
            write(root, path, body);
        }

        let with_python = analyze(root);
        for (path, _) in files {
            remove(root, path);
        }
        let without_python = analyze(root);

        assert_absent(&with_python, &["zq_", "ZQ_"]);
        assert_eq!(comparable(&with_python), comparable(&without_python));
        let full = regex::Regex::new(r"\b100\b").expect("regex");
        assert!(
            comparable(&with_python).iter().any(|l| full.is_match(l)),
            "the score must stay full:\n{with_python}"
        );
    }

    /// S07: A layer key that covers only non-Python files does not enable Python grading.
    #[test]
    fn s07_layer_key_covering_no_python_does_not_enable_grading() {
        let dir = project();
        let root = dir.path();
        rust_base(root);
        write(root, "engine/rules.rs", "fn _rule() {}\n");
        layers(root, &[("engine/", "domain")]);
        write(
            root,
            "scripts/zq_script.py",
            "def zq_unused_script():\n    return 1\n",
        );

        let report = analyze(root);
        assert_absent(&report, &["zq_unused_script", "zq_script"]);
        assert_ungraded_notice(&report, 1);
    }

    /// S08: Python affects the grade only when there is a Python manifest or a layer key covering at least one Python file.
    #[test]
    fn s08_python_without_manifest_or_layer_never_affects_grade() {
        type Files = &'static [(&'static str, &'static str)];
        let cases: &[(Files, bool)] = &[
            (&[("a.py", "")], false),
            (&[("stubs/only.pyi", "def zq_f() -> None: ...\n")], false),
            (
                &[
                    ("scripts/broken.py", "def (((:\n  ???\n"),
                    ("scripts/text.py", "this is not python at all {{{\n"),
                ],
                false,
            ),
            (
                &[
                    ("src/domain/x.py", "from .y import zq_y\nzq_x = 1\n"),
                    ("src/domain/y.py", "from .x import zq_x\nzq_y = 1\n"),
                    ("src/domain/y.pyi", "zq_y: int\n"),
                    (
                        "deep/er/est/z.py",
                        "__all__ = ['zq_z']\ndef zq_z():\n    pass\n",
                    ),
                    ("top.pyi", "class ZqTop: ...\n"),
                ],
                true,
            ),
        ];

        for (files, with_rust_layer) in cases {
            let dir = project();
            let root = dir.path();
            rust_base(root);
            if *with_rust_layer {
                write(root, "engine/rules.rs", "fn _rule() {}\n");
                layers(root, &[("engine/", "domain")]);
            }
            for (path, body) in files.iter() {
                write(root, path, body);
            }

            let with_python = analyze(root);
            for (path, _) in files.iter() {
                remove(root, path);
            }
            let without_python = analyze(root);

            assert_eq!(
                comparable(&with_python),
                comparable(&without_python),
                "Python changed the report for {files:?}"
            );
            assert_ungraded_notice(&with_python, files.len());
        }
    }

    /// S09: A public function called elsewhere in its own module is not dead.
    #[test]
    fn s09_locally_called_function_is_not_dead() {
        let dir = project();
        let root = dir.path();
        rust_base(root);
        write(root, "pyproject.toml", "[project]\nname = \"fixture\"\n");
        write(
        root,
        "pkg/zq_module.py",
        "def zq_helper():\n    return 1\n\n\ndef _run():\n    return zq_helper()\n\n\ndef zq_orphan():\n    return 2\n",
    );

        let report = analyze(root);
        // Control: dead exports are being reported for this module.
        assert_present(&report, "zq_orphan");
        assert_absent(&report, &["zq_helper"]);
    }

    /// S10: Local use counts for public constants and classes as well as functions.
    #[test]
    fn s10_locally_used_constant_and_class_are_not_dead() {
        let dir = project();
        let root = dir.path();
        rust_base(root);
        write(root, "pyproject.toml", "[project]\nname = \"fixture\"\n");
        write(
        root,
        "pkg/zq_module.py",
        "ZQ_LIMIT = 10\n\n\nclass ZqRecord:\n    pass\n\n\ndef _build():\n    if ZQ_LIMIT > 0:\n        return ZqRecord()\n    return None\n\n\ndef zq_orphan():\n    return 2\n",
    );

        let report = analyze(root);
        // Control: dead exports are being reported for this module.
        assert_present(&report, "zq_orphan");
        assert_absent(&report, &["ZQ_LIMIT", "ZqRecord"]);
    }
}

mod part_2 {
    use std::fs;
    use std::path::Path;
    use std::process::{Command, Output};

    const SENTINEL: &str = "sentinel_dead_zq";
    const CONTROL: &str = "control_local_zq";

    fn write(root: &Path, rel: &str, content: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().expect("parent")).expect("create dirs");
        fs::write(path, content).expect("write file");
    }

    fn run_hexa(root: &Path, home: &Path, extra: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_hexa"))
            .arg("analyze")
            .arg(root)
            .args(extra)
            .current_dir(root)
            .env("HOME", home)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("XDG_DATA_HOME", home.join(".local/share"))
            .env("NO_COLOR", "1")
            .output()
            .expect("run hexa analyze")
    }

    fn strip_ansi(s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\u{1b}' {
                for d in chars.by_ref() {
                    if d.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    /// Collects the arrays found under any key that mentions "dead".
    fn dead_arrays(value: &serde_json::Value, found: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, child) in map {
                    if key.to_lowercase().contains("dead") && child.is_array() {
                        found.push(child.to_string());
                    } else {
                        dead_arrays(child, found);
                    }
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    dead_arrays(item, found);
                }
            }
            _ => {}
        }
    }

    /// The text in which dead exports are named.
    fn analyze(files: &[(&str, String)]) -> String {
        let project = tempfile::tempdir().expect("project dir");
        let home = tempfile::tempdir().expect("home dir");
        let root = project.path();

        write(
            root,
            "pyproject.toml",
            "[project]\nname = \"fixture\"\nversion = \"0.1.0\"\n",
        );
        write(root, "src/__init__.py", "");
        write(root, "src/domain/__init__.py", "");
        write(
            root,
            "src/domain/sentinel_mod.py",
            &format!("def {SENTINEL}():\n    return 0\n"),
        );
        write(
            root,
            "src/domain/control_mod.py",
            &format!("def {CONTROL}():\n    return 1\n\n\n_VALUE = {CONTROL}()\n"),
        );
        for (rel, content) in files {
            write(root, rel, content);
        }

        let json_run = run_hexa(root, home.path(), &["--json"]);
        let stdout = String::from_utf8_lossy(&json_run.stdout).into_owned();
        if let Some(start) = stdout.find('{') {
            let mut stream = serde_json::Deserializer::from_str(&stdout[start..])
                .into_iter::<serde_json::Value>();
            if let Some(Ok(value)) = stream.next() {
                let mut found = Vec::new();
                dead_arrays(&value, &mut found);
                if !found.is_empty() {
                    return found.join("\n");
                }
            }
        }

        let text_run = run_hexa(root, home.path(), &[]);
        strip_ansi(&format!(
            "{}\n{}",
            String::from_utf8_lossy(&text_run.stdout),
            String::from_utf8_lossy(&text_run.stderr)
        ))
    }

    fn reported(report: &str, name: &str) -> bool {
        report
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .any(|word| word == name)
    }

    fn assert_sentinel(report: &str) {
        assert!(
            reported(report, SENTINEL),
            "an entirely unreferenced name must be reported:\n{report}"
        );
    }

    fn assert_control(report: &str) {
        assert!(
            !reported(report, CONTROL),
            "a name used by its own module (no __all__) must not be reported:\n{report}"
        );
    }

    /// S11: A name that nothing at all refers to is still reported.
    #[test]
    fn s11_unreferenced_name_is_reported() {
        let report = analyze(&[(
            "src/domain/subject_mod.py",
            "def orphan_zq():\n    return 1\n".to_string(),
        )]);
        assert_sentinel(&report);
        assert!(reported(&report, "orphan_zq"), "{report}");
        assert_control(&report);
    }

    /// S12: Mentions in comments and strings are not uses.
    #[test]
    fn s12_comments_and_strings_are_not_uses() {
        let source = r#""""Module notes: ghost_zq() does things."""


def ghost_zq():
    return 1


# ghost_zq is mentioned here: ghost_zq()
def _other():
    """Call ghost_zq() to get a value."""
    return "ghost_zq()"


_LABEL = 'ghost_zq'
"#;
        let report = analyze(&[("src/domain/subject_mod.py", source.to_string())]);
        assert_sentinel(&report);
        assert!(reported(&report, "ghost_zq"), "{report}");
        assert_control(&report);
    }

    /// S13: A definition that refers only to itself does not use itself.
    #[test]
    fn s13_self_recursion_is_not_a_use() {
        let source =
            "def walk_zq(n):\n    if n <= 0:\n        return 0\n    return walk_zq(n - 1)\n";
        let report = analyze(&[("src/domain/subject_mod.py", source.to_string())]);
        assert_sentinel(&report);
        assert!(reported(&report, "walk_zq"), "{report}");
        assert_control(&report);
    }

    /// S14: Local use does not excuse a name listed in __all__.
    #[test]
    fn s14_local_use_does_not_excuse_listed_name() {
        let source = "__all__ = [\"listed_zq\"]\n\n\ndef listed_zq():\n    return 1\n\n\ndef _caller():\n    return listed_zq()\n\n\n_RESULT = listed_zq()\n";
        let report = analyze(&[("src/domain/subject_mod.py", source.to_string())]);
        assert_sentinel(&report);
        assert!(reported(&report, "listed_zq"), "{report}");
        assert_control(&report);
    }

    /// S15: In a module with __all__, only the listed names are its exports.
    #[test]
    fn s15_only_listed_names_are_exports() {
        let subject = "__all__ = [\"api_zq\"]\n\n\ndef api_zq():\n    return 1\n\n\ndef unlisted_zq():\n    return 2\n";
        let consumer = "from .subject_mod import api_zq\n\n_RESULT = api_zq()\n";
        let report = analyze(&[
            ("src/domain/subject_mod.py", subject.to_string()),
            ("src/domain/consumer_mod.py", consumer.to_string()),
        ]);
        assert_sentinel(&report);
        assert!(!reported(&report, "api_zq"), "{report}");
        assert!(!reported(&report, "unlisted_zq"), "{report}");
        assert_control(&report);
    }

    /// S16: In a module without __all__, an unimported public name is dead exactly when the module's code does not use it outside its own definition.
    #[test]
    fn s16_dead_exactly_when_not_used_outside_definition() {
        const NAME: &str = "subject_zq";

        #[derive(Debug, Clone, Copy, PartialEq)]
        enum Outside {
            None,
            ModuleCall,
            PrivateBody,
            ValueReference,
        }

        for is_class in [false, true] {
            for outside in [
                Outside::None,
                Outside::ModuleCall,
                Outside::PrivateBody,
                Outside::ValueReference,
            ] {
                // bit 0: self reference, bit 1: comment, bit 2: string
                for mask in 0u8..8 {
                    let self_ref = mask & 1 != 0;
                    let comment = mask & 2 != 0;
                    let string = mask & 4 != 0;

                    let mut source = String::new();
                    if is_class {
                        source.push_str(&format!("class {NAME}:\n    def _make(self):\n"));
                        if self_ref {
                            source.push_str(&format!("        return {NAME}()\n"));
                        } else {
                            source.push_str("        return None\n");
                        }
                    } else {
                        source.push_str(&format!("def {NAME}(n=0):\n"));
                        if self_ref {
                            source.push_str(&format!(
                                "    if n > 0:\n        return {NAME}(n - 1)\n"
                            ));
                        }
                        source.push_str("    return n\n");
                    }
                    source.push_str("\n\n");
                    if comment {
                        source.push_str(&format!("# {NAME}() is described here\n"));
                    }
                    if string {
                        source.push_str(&format!(
                        "_TEXT = \"{NAME}()\"\n\n\ndef _documented():\n    \"\"\"See {NAME}.\"\"\"\n    return None\n\n\n"
                    ));
                    }
                    match outside {
                        Outside::None => {}
                        Outside::ModuleCall => source.push_str(&format!("_RESULT = {NAME}()\n")),
                        Outside::PrivateBody => {
                            source.push_str(&format!("def _user():\n    return {NAME}()\n"))
                        }
                        Outside::ValueReference => {
                            source.push_str(&format!("_TABLE = {{\"key\": {NAME}}}\n"))
                        }
                    }

                    let report = analyze(&[("src/domain/subject_mod.py", source.clone())]);
                    assert_sentinel(&report);
                    assert_eq!(
                    reported(&report, NAME),
                    outside == Outside::None,
                    "class={is_class} outside={outside:?} self={self_ref} comment={comment} string={string}\nsource:\n{source}\nreport:\n{report}"
                );
                }
            }
        }
    }

    /// S17: In a module with __all__, local references never change which listed names are dead.
    #[test]
    fn s17_local_references_do_not_affect_listed_names() {
        const NAMES: [&str; 3] = ["first_zq", "second_zq", "third_zq"];

        for imported in 0u8..8 {
            // Which listed names are referenced locally: none, each alone, or all.
            for local in [0u8, 1, 2, 4, 7] {
                let mut subject =
                    String::from("__all__ = [\"first_zq\", \"second_zq\", \"third_zq\"]\n\n\n");
                for name in NAMES {
                    subject.push_str(&format!("def {name}():\n    return 1\n\n\n"));
                }
                for (i, name) in NAMES.iter().enumerate() {
                    if local & (1 << i) != 0 {
                        subject.push_str(&format!(
                            "def _use_{i}():\n    return {name}()\n\n\n_REF_{i} = {name}\n\n\n"
                        ));
                    }
                }

                let mut files = vec![("src/domain/subject_mod.py", subject)];
                let mut consumer = String::new();
                for (i, name) in NAMES.iter().enumerate() {
                    if imported & (1 << i) != 0 {
                        consumer.push_str(&format!(
                            "from .subject_mod import {name}\n\n_RESULT_{i} = {name}()\n"
                        ));
                    }
                }
                if !consumer.is_empty() {
                    files.push(("src/domain/consumer_mod.py", consumer));
                }

                let report = analyze(&files);
                assert_sentinel(&report);
                for (i, name) in NAMES.iter().enumerate() {
                    assert_eq!(
                        reported(&report, name),
                        imported & (1 << i) == 0,
                        "name={name} imported={imported:03b} local={local:03b}\nreport:\n{report}"
                    );
                }
                assert_control(&report);
            }
        }
    }
}
