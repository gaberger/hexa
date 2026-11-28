#![allow(unused, clippy::all)]
mod part_1 {
    #![allow(dead_code)]

    use std::collections::HashMap;

    use hexa_analysis::dead_export_finder::{find_dead_exports, FileData};
    use hexa_analysis::domain::{DeadExport, ExportDeclaration, ExportKind, HexLayer};
    use hexa_analysis::layer_classifier::{classify_layer, LayerMap};

    fn export(file: &str, name: &str, line: usize, kind: ExportKind) -> ExportDeclaration {
        ExportDeclaration {
            file: file.to_string(),
            name: name.to_string(),
            line,
            hexa_public: false,
            kind,
        }
    }

    /// A file exporting the given names, each a function, naming each once (its declaration).
    fn py_file(path: &str, names: &[(&str, ExportKind)]) -> FileData {
        let mut references = HashMap::new();
        let mut exports = Vec::new();
        for (i, (name, kind)) in names.iter().enumerate() {
            exports.push(export(path, name, i + 1, *kind));
            references.insert(name.to_string(), 1usize);
        }
        FileData {
            path: path.to_string(),
            imports: Vec::new(),
            exports,
            references,
            members: HashMap::new(),
        }
    }

    fn fn_file(path: &str, name: &str) -> FileData {
        py_file(path, &[(name, ExportKind::Function)])
    }

    fn is_dead(dead: &[DeadExport], file: &str, name: &str) -> bool {
        dead.iter().any(|d| d.file == file && d.export_name == name)
    }

    fn layer_of(path: &str) -> (HexLayer, HexLayer) {
        (classify_layer(path), LayerMap::default().classify(path))
    }

    /// S01: A `__main__.py` at the project root keeps its exports alive.
    #[test]
    fn s01_root_dunder_main_keeps_exports_alive() {
        let files = vec![
            fn_file("__main__.py", "run"),
            fn_file("app/helpers.py", "unrelated"),
        ];
        let dead = find_dead_exports(&files, &[]);
        assert!(
            !is_dead(&dead, "__main__.py", "run"),
            "`run` in __main__.py must not be dead; got {dead:?}"
        );
    }

    /// S02: Each named Python entry-point file under a nested directory keeps its exports alive.
    #[test]
    fn s02_nested_python_entry_points_keep_exports_alive() {
        let files = vec![
            fn_file("pkg/app/main.py", "start"),
            fn_file("pkg/app/cli.py", "parse_args"),
            fn_file("pkg/app/composition_root.py", "wire"),
        ];
        let dead = find_dead_exports(&files, &[]);
        for (file, name) in [
            ("pkg/app/main.py", "start"),
            ("pkg/app/cli.py", "parse_args"),
            ("pkg/app/composition_root.py", "wire"),
        ] {
            assert!(
                !is_dead(&dead, file, name),
                "`{name}` in {file} must not be dead; got {dead:?}"
            );
        }
    }

    /// S06: An exported name in a Python entry-point file is never dead, wherever the file is.
    #[test]
    fn s06_python_entry_point_exports_never_dead_anywhere() {
        let names = ["__main__.py", "main.py", "cli.py", "composition_root.py"];
        let dirs = ["", "app/", "pkg/app/", "a/b/c/d/", "src/my_pkg/sub/"];
        let export_sets: Vec<Vec<(&str, ExportKind)>> = vec![
            vec![("run", ExportKind::Function)],
            vec![
                ("main", ExportKind::Function),
                ("Config", ExportKind::Type),
                ("DEFAULT_PORT", ExportKind::Value),
            ],
            vec![("build", ExportKind::Function), ("App", ExportKind::Type)],
        ];
        for name in names {
            for dir in dirs {
                for set in &export_sets {
                    let path = format!("{dir}{name}");
                    let files = vec![
                        py_file(&path, set),
                        fn_file("other/helpers.py", "unrelated"),
                    ];
                    let dead = find_dead_exports(&files, &[]);
                    let offending: Vec<&DeadExport> =
                        dead.iter().filter(|d| d.file == path).collect();
                    assert!(
                        offending.is_empty(),
                        "no dead export may name {path}; got {offending:?}"
                    );
                }
            }
        }
    }

    /// S07: `__main__.py` takes the entry-point layer.
    #[test]
    fn s07_dunder_main_is_entry_point_layer() {
        let (builtin, mapped) = layer_of("app/__main__.py");
        assert_eq!(builtin, HexLayer::EntryPoint);
        assert_eq!(mapped, HexLayer::EntryPoint);
    }

    /// S08: `main.py` takes the entry-point layer, as `main.go` does.
    #[test]
    fn s08_main_py_is_entry_point_like_main_go() {
        for path in ["main.py", "main.go"] {
            let (builtin, mapped) = layer_of(path);
            assert_eq!(builtin, HexLayer::EntryPoint, "{path}");
            assert_eq!(mapped, HexLayer::EntryPoint, "{path}");
        }
    }
}

mod part_2 {
    #![allow(clippy::all, dead_code, unused_imports)]

    use hexa_analysis::domain::{crosses_adapters, AdapterMap, HexLayer, ImportEdge};
    use hexa_analysis::layer_classifier::{classify_layer, LayerMap};

    // ── helpers ──────────────────────────────────────────────

    /// A `LayerMap` read from a temporary project whose `.hexa/project.json`
    /// declares `layers` under `analyze.layers`.
    fn layer_map(layers: &[(&str, &str)]) -> LayerMap {
        let dir = tempfile::tempdir().expect("tempdir");
        let hexa = dir.path().join(".hexa");
        std::fs::create_dir_all(&hexa).expect("create .hexa");
        let mut obj = serde_json::Map::new();
        for (k, v) in layers {
            obj.insert(
                (*k).to_string(),
                serde_json::Value::String((*v).to_string()),
            );
        }
        let json = serde_json::json!({ "analyze": { "layers": serde_json::Value::Object(obj) } });
        std::fs::write(hexa.join("project.json"), json.to_string()).expect("write project.json");
        LayerMap::from_project(dir.path()).expect("analyze.layers is valid")
    }

    fn edge(from: &str, to: &str, layer: HexLayer) -> ImportEdge {
        ImportEdge {
            from_file: from.to_string(),
            to_file: to.to_string(),
            from_layer: layer,
            to_layer: layer,
            import_path: to.to_string(),
            line: 1,
        }
    }

    fn template_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("hexa-cli")
            .join("assets")
            .join("templates")
            .join("ADR-rules.toml")
    }

    /// The rules file `hexa init` writes, parsed.
    fn template() -> toml::Value {
        let path = template_path();
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        toml::from_str(&text).unwrap_or_else(|e| panic!("cannot parse {}: {e}", path.display()))
    }

    /// Find the table of the rule named `id`, wherever the template nests it.
    fn find_rule<'a>(v: &'a toml::Value, id: &str) -> Option<&'a toml::value::Table> {
        match v {
            toml::Value::Table(t) => {
                let named = ["id", "name"]
                    .iter()
                    .any(|k| t.get(*k).and_then(|x| x.as_str()) == Some(id));
                if named {
                    return Some(t);
                }
                if let Some(toml::Value::Table(inner)) = t.get(id) {
                    if inner.contains_key("file_patterns") {
                        return Some(inner);
                    }
                }
                t.values().find_map(|x| find_rule(x, id))
            }
            toml::Value::Array(a) => a.iter().find_map(|x| find_rule(x, id)),
            _ => None,
        }
    }

    fn rule(id: &str) -> toml::value::Table {
        let t = template();
        find_rule(&t, id)
            .unwrap_or_else(|| panic!("rule `{id}` not found in the template"))
            .clone()
    }

    fn str_array(t: &toml::value::Table, key: &str) -> Option<Vec<String>> {
        t.get(key).map(|v| {
            v.as_array()
                .unwrap_or_else(|| panic!("`{key}` is not an array"))
                .iter()
                .map(|x| {
                    x.as_str()
                        .unwrap_or_else(|| panic!("`{key}` holds a non-string"))
                        .to_string()
                })
                .collect()
        })
    }

    fn rule_reads(t: &toml::value::Table, file: &str) -> bool {
        str_array(t, "file_patterns")
            .unwrap_or_default()
            .iter()
            .any(|p| file.ends_with(p.as_str()))
    }

    // ── scenarios ────────────────────────────────────────────

    /// S11: Every `__main__.py` and every `main.py` is an entry point by name.
    #[test]
    fn s11_main_py_and_dunder_main_py_are_entry_points() {
        let dirs = ["", "app/", "tools/run/", "myproj/pkg/", "a/b/c/d/"];
        let empty = LayerMap::default();
        for dir in dirs {
            for name in ["__main__.py", "main.py"] {
                let path = format!("{dir}{name}");
                assert_eq!(
                    classify_layer(&path),
                    HexLayer::EntryPoint,
                    "classify_layer({path:?})"
                );
                assert_eq!(
                    empty.classify(&path),
                    HexLayer::EntryPoint,
                    "LayerMap::classify({path:?}) with no keys"
                );
            }
        }
    }

    /// S12: A layer key naming a `.py` file covers the package of the same name.
    #[test]
    fn s12_py_layer_key_covers_same_named_package() {
        let map = layer_map(&[("app/store.py", "adapters/secondary")]);
        for path in ["app/store/__init__.py", "app/store/disk.py"] {
            assert_eq!(
                map.classify(path),
                HexLayer::AdaptersSecondary,
                "{path} should be covered by key app/store.py"
            );
        }
        // As the Rust key does.
        let rs = layer_map(&[("app/store.rs", "adapters/secondary")]);
        assert_eq!(rs.classify("app/store/x.rs"), HexLayer::AdaptersSecondary);
    }

    /// S13: Under `analyze.adapters`, a group path naming a `.py` file covers its package.
    #[test]
    fn s13_adapters_group_py_path_covers_package() {
        let layer_of = |_: &str| HexLayer::AdaptersSecondary;
        let map = AdapterMap::new(
            vec![],
            vec![("storage".to_string(), vec!["app/store.py".to_string()])],
            &layer_of,
        )
        .expect("valid adapters declaration");
        assert_eq!(
            map.adapter_of("app/store/disk.py", HexLayer::AdaptersSecondary),
            Some("storage".to_string())
        );
    }

    /// S14: An adapter named from its `.py` file is the same adapter as its package.
    #[test]
    fn s14_py_adapter_file_and_its_package_are_one_adapter() {
        let map = AdapterMap::default();
        let a = "src/adapters/secondary/db.py";
        let b = "src/adapters/secondary/db/conn.py";
        let want = Some("src/adapters/secondary/db".to_string());
        assert_eq!(
            map.adapter_of(a, HexLayer::AdaptersSecondary),
            want,
            "adapter of {a}"
        );
        assert_eq!(
            map.adapter_of(b, HexLayer::AdaptersSecondary),
            want,
            "adapter of {b}"
        );
        assert!(
            !crosses_adapters(&edge(a, b, HexLayer::AdaptersSecondary), &map),
            "db.py -> db/conn.py stays inside one adapter"
        );
    }

    /// S15: A `.py` key covers its package on whole path segments only.
    #[test]
    fn s15_py_key_covers_whole_segments_only() {
        let map = layer_map(&[("app/store.py", "adapters/secondary")]);
        // The key is in force for its own package...
        assert_eq!(
            map.classify("app/store/disk.py"),
            HexLayer::AdaptersSecondary
        );
        // ...but not for siblings sharing a prefix.
        for path in ["app/storefront/__init__.py", "app/stores/x.py"] {
            assert_eq!(
                map.classify(path),
                classify_layer(path),
                "{path} must fall back to the built-in classification"
            );
            assert_ne!(map.classify(path), HexLayer::AdaptersSecondary, "{path}");
        }
    }

    /// S16: Two different Python adapter files are two adapters.
    #[test]
    fn s16_two_python_adapter_files_are_two_adapters() {
        let map = AdapterMap::default();
        let a = "src/adapters/secondary/db.py";
        let b = "src/adapters/secondary/cache.py";
        assert_eq!(
            map.adapter_of(a, HexLayer::AdaptersSecondary),
            Some("src/adapters/secondary/db".to_string())
        );
        assert_eq!(
            map.adapter_of(b, HexLayer::AdaptersSecondary),
            Some("src/adapters/secondary/cache".to_string())
        );
        assert!(crosses_adapters(
            &edge(a, b, HexLayer::AdaptersSecondary),
            &map
        ));
    }

    /// S17: A `.py` key covers its module directory exactly as the other languages' keys do.
    #[test]
    fn s17_py_key_covers_like_rs_key() {
        let keys = ["app/store", "lib/a/b", "pkg"];
        for k in keys {
            let py_key = format!("{k}.py");
            let rs_key = format!("{k}.rs");
            let py_map = layer_map(&[(&py_key, "adapters/secondary")]);
            let rs_map = layer_map(&[(&rs_key, "adapters/secondary")]);
            let layer_of = |_: &str| HexLayer::AdaptersSecondary;
            let py_groups = AdapterMap::new(
                vec![],
                vec![("g".to_string(), vec![py_key.clone()])],
                &layer_of,
            )
            .unwrap();
            let rs_groups = AdapterMap::new(
                vec![],
                vec![("g".to_string(), vec![rs_key.clone()])],
                &layer_of,
            )
            .unwrap();

            let stems = [
                format!("{k}/x"),
                format!("{k}/__init__"),
                format!("{k}/a/b"),
                format!("{k}x/y"),
                format!("{k}_other/y"),
                format!("other/{k}/x"),
                format!("{k}s"),
            ];
            for stem in &stems {
                let under = stem.starts_with(&format!("{k}/"));
                for suffix in [".py", ".rs"] {
                    let p = format!("{stem}{suffix}");
                    let builtin = classify_layer(&p) == HexLayer::AdaptersSecondary;
                    let by_py = py_map.classify(&p) == HexLayer::AdaptersSecondary && !builtin;
                    let by_rs = rs_map.classify(&p) == HexLayer::AdaptersSecondary && !builtin;
                    if !builtin {
                        assert_eq!(by_py, under, "layer key {py_key} covering {p}");
                        assert_eq!(by_py, by_rs, "{py_key} vs {rs_key} on {p}");
                    }
                    let g_py = py_groups.adapter_of(&p, HexLayer::AdaptersSecondary)
                        == Some("g".to_string());
                    let g_rs = rs_groups.adapter_of(&p, HexLayer::AdaptersSecondary)
                        == Some("g".to_string());
                    assert_eq!(g_py, under, "adapters group {py_key} covering {p}");
                    assert_eq!(g_py, g_rs, "group {py_key} vs {rs_key} on {p}");
                }
            }
            // The key's own file.
            assert_eq!(
                py_map.classify(&py_key),
                HexLayer::AdaptersSecondary,
                "{py_key} covers itself"
            );
            assert_eq!(
                py_groups.adapter_of(&py_key, HexLayer::AdaptersSecondary),
                Some("g".to_string())
            );
        }
    }

    /// S18: A new project's absolute-path rule reads Python.
    #[test]
    fn s18_absolute_path_rule_reads_python() {
        let r = rule("no-hardcoded-absolute-path");
        assert!(
            rule_reads(&r, "app/config.py"),
            "no-hardcoded-absolute-path must read app/config.py; file_patterns = {:?}",
            str_array(&r, "file_patterns")
        );
        let vp = str_array(&r, "violation_patterns").expect("violation_patterns present");
        assert!(
            !vp.is_empty(),
            "the rule has violation patterns to match the path with"
        );
    }

    /// S19: All three language-neutral rules in the written template list `.py`, and nothing else in them changes.
    #[test]
    fn s19_three_rules_list_py_and_keep_their_patterns() {
        let want: Vec<String> = [".rs", ".go", ".ts", ".tsx", ".py"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        for id in [
            "no-hardcoded-absolute-path",
            "no-hardcoded-host-port",
            "no-model-name-outside-inference",
        ] {
            let r = rule(id);
            assert_eq!(
                str_array(&r, "file_patterns"),
                Some(want.clone()),
                "{id} file_patterns"
            );
            let vp = str_array(&r, "violation_patterns")
                .unwrap_or_else(|| panic!("{id} violation_patterns"));
            assert!(!vp.is_empty(), "{id} keeps its violation_patterns");
            // exclude_patterns, where present, stays a list of strings.
            let _ = str_array(&r, "exclude_patterns");
        }
    }

    /// S20: Adding `.py` widens which files are read, not what counts as a violation.
    #[test]
    fn s20_py_widens_files_read_not_violations() {
        let r = rule("no-hardcoded-host-port");
        // The file is read now...
        assert!(
            rule_reads(&r, "app/client.py"),
            "no-hardcoded-host-port must read app/client.py"
        );
        // ...and the exclusions that keep excluded text from being a violation are still there.
        let excludes = str_array(&r, "exclude_patterns")
            .expect("no-hardcoded-host-port keeps exclude_patterns");
        assert!(!excludes.is_empty(), "exclude_patterns must not be emptied");
        let vp = str_array(&r, "violation_patterns").expect("violation_patterns present");
        assert!(!vp.is_empty());
    }
}

mod part_3 {
    #[allow(unused_imports)]
    use std::collections::{BTreeMap, HashMap};
    use std::path::{Path, PathBuf};

    use hexa_analysis::dead_export_finder::{find_dead_exports, find_dead_exports_with, FileData};
    use hexa_analysis::domain::{AdapterMap, ExportDeclaration, ExportKind, HexLayer};
    use hexa_analysis::layer_classifier::{adapters_from_project, classify_layer, LayerMap};

    const NEUTRAL_RULES: [&str; 3] = [
        "no-hardcoded-absolute-path",
        "no-hardcoded-host-port",
        "no-model-name-outside-inference",
    ];

    const OLD_FOUR: [&str; 4] = [".rs", ".go", ".ts", ".tsx"];

    fn template_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../hexa-cli/assets/templates/ADR-rules.toml")
    }

    /// Every table in the rules file that carries `file_patterns`, by its id or name.
    fn rules_with_patterns(text: &str) -> Vec<(String, Vec<String>)> {
        let value: toml::Value = toml::from_str(text).expect("rules file parses as TOML");
        let mut out = Vec::new();
        walk(&value, &mut out);
        out
    }

    fn walk(v: &toml::Value, out: &mut Vec<(String, Vec<String>)>) {
        match v {
            toml::Value::Table(t) => {
                if let Some(toml::Value::Array(pats)) = t.get("file_patterns") {
                    let name = ["id", "name", "rule"]
                        .iter()
                        .find_map(|k| t.get(*k).and_then(|x| x.as_str()))
                        .unwrap_or("<unnamed>")
                        .to_string();
                    let pats = pats
                        .iter()
                        .filter_map(|p| p.as_str().map(str::to_string))
                        .collect();
                    out.push((name, pats));
                }
                for (_, child) in t {
                    walk(child, out);
                }
            }
            toml::Value::Array(a) => {
                for child in a {
                    walk(child, out);
                }
            }
            _ => {}
        }
    }

    fn export(file: &str, name: &str) -> ExportDeclaration {
        ExportDeclaration {
            file: file.to_string(),
            name: name.to_string(),
            line: 1,
            hexa_public: false,
            kind: ExportKind::Function,
        }
    }

    fn file_data(path: &str, names: &[&str]) -> FileData {
        FileData {
            path: path.to_string(),
            imports: Vec::new(),
            exports: names.iter().map(|n| export(path, n)).collect(),
            references: HashMap::new(),
            members: HashMap::new(),
        }
    }

    fn project_with_layers(layers: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".hexa")).unwrap();
        let mut map = serde_json::Map::new();
        for (k, v) in layers {
            map.insert(k.to_string(), serde_json::Value::String(v.to_string()));
        }
        let json = serde_json::json!({ "analyze": { "layers": map } });
        std::fs::write(
            dir.path().join(".hexa/project.json"),
            serde_json::to_string_pretty(&json).unwrap(),
        )
        .unwrap();
        dir
    }

    /// S21: Rules other than the three language-neutral ones do not gain `.py`.
    #[test]
    fn s21_other_rules_do_not_gain_py() {
        let text = std::fs::read_to_string(template_path()).expect("read hexa init rules template");
        let rules = rules_with_patterns(&text);

        // The three neutral rules are the ones that changed: they now read `.py`
        // alongside exactly the four they had before.
        for id in NEUTRAL_RULES {
            let (_, pats) = rules
                .iter()
                .find(|(n, _)| n == id)
                .unwrap_or_else(|| panic!("template has rule {id}"));
            for ext in OLD_FOUR {
                assert!(pats.iter().any(|p| p == ext), "{id} keeps {ext}: {pats:?}");
            }
            assert!(pats.iter().any(|p| p == ".py"), "{id} reads .py: {pats:?}");
            assert_eq!(pats.len(), 5, "{id} gains only .py: {pats:?}");
        }

        let others: Vec<_> = rules
            .iter()
            .filter(|(n, _)| !NEUTRAL_RULES.contains(&n.as_str()))
            .collect();
        assert!(
            !others.is_empty(),
            "the template has rules besides the three"
        );
        for (name, pats) in others {
            let multi_language = pats.iter().any(|p| OLD_FOUR.contains(&p.as_str()));
            assert!(
                !(multi_language && pats.iter().any(|p| p == ".py")),
                "rule {name} gained .py alongside other languages: {pats:?}"
            );
        }
    }

    /// S22: Rust, Go and TypeScript files are read as before.
    #[test]
    fn s22_rust_go_typescript_read_as_before() {
        let entry_files = [
            "src/main.rs",
            "src/lib.rs",
            "main.go",
            "web/index.ts",
            "web/cli.ts",
        ];
        let mut files: Vec<FileData> = entry_files
            .iter()
            .map(|p| file_data(p, &["entry_thing"]))
            .collect();
        // The Python counterpart this ADR adds, read in the same run.
        files.push(file_data("app/__main__.py", &["entry_thing"]));

        let dead = find_dead_exports(&files, &[]);
        for f in entry_files {
            assert!(
                !dead.iter().any(|d| d.file == f),
                "export of {f} counted as dead: {dead:?}"
            );
        }
        assert!(
            !dead.iter().any(|d| d.file == "app/__main__.py"),
            "export of app/__main__.py counted as dead: {dead:?}"
        );

        assert_eq!(classify_layer("src/main.rs"), HexLayer::EntryPoint);
        assert_eq!(classify_layer("main.go"), HexLayer::EntryPoint);
        assert_eq!(classify_layer("web/main.ts"), HexLayer::EntryPoint);
        assert_eq!(classify_layer("app/__main__.py"), HexLayer::EntryPoint);

        let dir = project_with_layers(&[
            ("src/store.rs", "adapters/secondary"),
            ("app/store.py", "adapters/secondary"),
        ]);
        let layers = LayerMap::from_project(dir.path()).expect("layers parse");
        assert_eq!(
            layers.classify("src/store/x.rs"),
            HexLayer::AdaptersSecondary
        );
        assert_eq!(layers.classify("src/store.rs"), HexLayer::AdaptersSecondary);
        assert_eq!(
            layers.classify("app/store/__init__.py"),
            HexLayer::AdaptersSecondary
        );
    }

    /// S23: A rules file written before this ADR is not rewritten.
    #[test]
    fn s23_old_rules_file_is_not_rewritten() {
        let old_rules = r#"
[[rules]]
id = "no-hardcoded-absolute-path"
severity = "warning"
file_patterns = [".rs", ".go", ".ts", ".tsx"]
violation_patterns = ["\"/home/", "\"/Users/"]
exclude_patterns = []
"#;
        let dir = project_with_layers(&[]);
        std::fs::write(dir.path().join(".hexa/rules.toml"), old_rules).unwrap();
        std::fs::create_dir_all(dir.path().join("app")).unwrap();
        std::fs::write(
            dir.path().join("app/config.py"),
            "DATA = \"/home/alice/data\"\n",
        )
        .unwrap();

        // Analyse with the new hexa's layer and adapter reading.
        let layers = LayerMap::from_project(dir.path()).expect("layers parse");
        let _ = layers.classify("app/config.py");
        let _ = adapters_from_project(dir.path(), &layers);
        let _ = find_dead_exports_with(&[file_data("app/config.py", &["DATA"])], &[], &layers);

        let after = std::fs::read_to_string(dir.path().join(".hexa/rules.toml")).unwrap();
        assert_eq!(after, old_rules, "rules file is unchanged");

        let rules = rules_with_patterns(&after);
        let (_, pats) = rules
            .iter()
            .find(|(n, _)| n == "no-hardcoded-absolute-path")
            .expect("rule present");
        assert_eq!(
            pats,
            &OLD_FOUR.iter().map(|s| s.to_string()).collect::<Vec<_>>()
        );
        assert!(
            !pats.iter().any(|p| "app/config.py".ends_with(p.as_str())),
            "the old rule does not read app/config.py"
        );

        // While a new project's rules file does read `.py`.
        let template = std::fs::read_to_string(template_path()).expect("read template");
        let (_, new_pats) = rules_with_patterns(&template)
            .into_iter()
            .find(|(n, _)| n == "no-hardcoded-absolute-path")
            .expect("template has the rule");
        assert!(
            new_pats.iter().any(|p| p == ".py"),
            "template reads .py: {new_pats:?}"
        );
    }

    /// S24: No non-Python file is classified or judged differently.
    #[test]
    fn s24_non_python_files_unchanged() {
        let non_py = [
            "src/main.rs",
            "src/lib.rs",
            "src/domain/model.rs",
            "src/ports/repo.rs",
            "src/store.rs",
            "src/store/x.rs",
            "src/adapters/secondary/db.rs",
            "src/adapters/primary/http/handler.rs",
            "src/util.rs",
            "main.go",
            "internal/service.go",
            "web/index.ts",
            "web/cli.ts",
            "web/main.ts",
            "web/composition-root.ts",
            "web/view.tsx",
            "web/helpers.ts",
        ];
        let python = [
            "app/__main__.py",
            "app/main.py",
            "app/cli.py",
            "app/composition_root.py",
            "app/__init__.py",
            "app/store/__init__.py",
        ];

        // Built-in classification of a non-.py file stays what a plain read gives.
        let base_dir = project_with_layers(&[
            ("src/store.rs", "adapters/secondary"),
            ("internal", "usecases"),
        ]);
        let py_dir = project_with_layers(&[
            ("src/store.rs", "adapters/secondary"),
            ("internal", "usecases"),
            ("app/store.py", "adapters/secondary"),
        ]);
        let base = LayerMap::from_project(base_dir.path()).expect("base layers");
        let with_py = LayerMap::from_project(py_dir.path()).expect("py layers");

        let base_adapters =
            AdapterMap::new(base.keys(), Vec::new(), &|p| base.classify(p)).expect("base adapters");
        let py_adapters = AdapterMap::new(with_py.keys(), Vec::new(), &|p| with_py.classify(p))
            .expect("py adapters");

        for f in non_py {
            let l0 = base.classify(f);
            let l1 = with_py.classify(f);
            assert_eq!(l0, l1, "layer of {f}");
            assert_eq!(
                base_adapters.adapter_of(f, l0),
                py_adapters.adapter_of(f, l1),
                "adapter of {f}"
            );
        }

        // Dead-export judgement of non-.py files is the same with Python files present.
        let non_py_files = || -> Vec<FileData> {
            non_py
                .iter()
                .map(|p| file_data(p, &["thing_one", "thing_two"]))
                .collect()
        };
        let mut mixed = non_py_files();
        mixed.extend(python.iter().map(|p| file_data(p, &["py_thing"])));

        let dead_base: Vec<(String, String)> = {
            let mut v: Vec<_> = find_dead_exports_with(&non_py_files(), &[], &base)
                .into_iter()
                .map(|d| (d.file, d.export_name))
                .collect();
            v.sort();
            v
        };
        let dead_mixed_all = find_dead_exports_with(&mixed, &[], &with_py);
        let mut dead_mixed: Vec<(String, String)> = dead_mixed_all
            .iter()
            .filter(|d| !d.file.ends_with(".py"))
            .map(|d| (d.file.clone(), d.export_name.clone()))
            .collect();
        dead_mixed.sort();
        assert_eq!(dead_base, dead_mixed, "dead non-.py exports are identical");

        // The Python side did change: its entry points are live, its package is covered.
        for f in [
            "app/__main__.py",
            "app/main.py",
            "app/cli.py",
            "app/composition_root.py",
        ] {
            assert!(
                !dead_mixed_all.iter().any(|d| d.file == f),
                "export of {f} counted as dead"
            );
        }
        assert!(
            dead_mixed_all.iter().any(|d| d.file == "app/__init__.py")
                || classify_layer("app/__init__.py") != HexLayer::EntryPoint,
            "__init__.py is not an entry point"
        );
        assert_ne!(classify_layer("app/__init__.py"), HexLayer::EntryPoint);
        assert_eq!(
            with_py.classify("app/store/__init__.py"),
            HexLayer::AdaptersSecondary
        );
    }
}
