#![allow(unused, clippy::all)]
mod part_1 {
    #[allow(unused_imports)]
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    /// Write `contents` to `root/rel`, creating parent directories.
    fn write(root: &Path, rel: &str, contents: &str) {
        let p = root.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, contents).unwrap();
    }

    /// A `.hexa/project.json` with the given `analyze.layers` map (or none).
    fn project_json(root: &Path, layers: Option<&[(&str, &str)]>) {
        let mut analyze = serde_json::Map::new();
        if let Some(layers) = layers {
            let mut m = serde_json::Map::new();
            for (k, v) in layers {
                m.insert(
                    (*k).to_string(),
                    serde_json::Value::String((*v).to_string()),
                );
            }
            analyze.insert("layers".into(), serde_json::Value::Object(m));
        }
        let doc = serde_json::json!({ "analyze": serde_json::Value::Object(analyze) });
        write(
            root,
            ".hexa/project.json",
            &serde_json::to_string_pretty(&doc).unwrap(),
        );
    }

    /// A rules file with one `[[import_policy]]` per (layer, deny list).
    fn rules(root: &Path, policies: &[(&str, &[&str])]) {
        let mut s = String::new();
        for (layer, deny) in policies {
            s.push_str("[[import_policy]]\n");
            // The fields a policy requires besides its layer and lists.
            s.push_str("adr = \"ADR-2609211430\"\nid = \"fixture-policy\"\nmessage = \"fixture policy\"\n");
            s.push_str(&format!("layer = {:?}\n", layer));
            let list: Vec<String> = deny.iter().map(|d| format!("{:?}", d)).collect();
            s.push_str(&format!("deny = [{}]\n", list.join(", ")));
            s.push_str("severity = \"error\"\n\n");
        }
        write(root, ".hexa/ADR-rules.toml", &s);
    }

    fn go_mod(root: &Path) {
        write(root, "go.mod", "module example.com/proj\n\ngo 1.21\n");
    }

    fn py_project(root: &Path) {
        write(
            root,
            "pyproject.toml",
            "[project]\nname = \"proj\"\nversion = \"0.1.0\"\n",
        );
    }

    struct Output {
        json: Option<serde_json::Value>,
        text: String,
    }

    fn analyze(root: &Path) -> Output {
        let bin = env!("CARGO_BIN_EXE_hexa");
        let out = Command::new(bin)
            .args(["analyze", ".", "--json"])
            .current_dir(root)
            .output()
            .expect("run hexa");
        let mut stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let mut stderr = String::from_utf8_lossy(&out.stderr).to_string();
        if out.status.code() == Some(2) && stderr.contains("unexpected argument") {
            let out = Command::new(bin)
                .args(["analyze", "."])
                .current_dir(root)
                .output()
                .expect("run hexa");
            stdout = String::from_utf8_lossy(&out.stdout).to_string();
            stderr = String::from_utf8_lossy(&out.stderr).to_string();
        }
        let json = serde_json::from_str::<serde_json::Value>(stdout.trim())
            .ok()
            .or_else(|| {
                let start = stdout.find('{')?;
                let end = stdout.rfind('}')?;
                serde_json::from_str(&stdout[start..=end]).ok()
            });
        Output {
            json,
            text: format!("{stdout}\n{stderr}"),
        }
    }

    fn scalar(v: &serde_json::Value) -> Option<String> {
        match v {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    }

    /// Flat records: each JSON object's direct scalar fields, or each text line.
    fn records(out: &Output) -> Vec<(Vec<String>, Vec<u64>)> {
        let mut recs = Vec::new();
        fn walk(v: &serde_json::Value, recs: &mut Vec<(Vec<String>, Vec<u64>)>) {
            match v {
                serde_json::Value::Object(m) => {
                    let strs: Vec<String> = m.values().filter_map(scalar).collect();
                    let nums: Vec<u64> = m.values().filter_map(|x| x.as_u64()).collect();
                    recs.push((strs, nums));
                    for x in m.values() {
                        walk(x, recs);
                    }
                }
                serde_json::Value::Array(a) => {
                    for x in a {
                        walk(x, recs);
                    }
                }
                serde_json::Value::String(s) => recs.push((vec![s.clone()], vec![])),
                _ => {}
            }
        }
        if let Some(j) = &out.json {
            walk(j, &mut recs);
        } else {
            for line in out.text.lines() {
                recs.push((vec![line.to_string()], vec![]));
            }
        }
        recs
    }

    fn names_module(s: &str, module: &str) -> bool {
        let re = regex::Regex::new(&format!(
            r"(^|[^A-Za-z0-9_/:.]){}($|[^A-Za-z0-9_/])",
            regex::escape(module)
        ))
        .unwrap();
        re.is_match(s)
    }

    fn mentions_file(s: &str, file: &str) -> bool {
        let re = regex::Regex::new(&format!(
            r"(^|[^A-Za-z0-9_./-]){}($|[^A-Za-z0-9_/-])",
            regex::escape(file)
        ))
        .unwrap();
        re.is_match(s)
    }

    /// Findings that name `file` and `module`.
    fn findings(out: &Output, file: &str, module: &str) -> Vec<(Vec<String>, Vec<u64>)> {
        records(out)
            .into_iter()
            .filter(|(strs, _)| {
                strs.iter().any(|s| mentions_file(s, file))
                    && strs.iter().any(|s| names_module(s, module))
            })
            .collect()
    }

    fn has_line(rec: &(Vec<String>, Vec<u64>), file: &str, line: u64) -> bool {
        rec.1.contains(&line)
            || rec.0.iter().any(|s| s.contains(&format!("{file}:{line}")))
            || rec.0.iter().any(|s| s.contains(&format!("line {line}")))
    }

    fn score(out: &Output) -> Option<u64> {
        fn find(v: &serde_json::Value, key: &str) -> Option<u64> {
            match v {
                serde_json::Value::Object(m) => {
                    if let Some(n) = m.get(key).and_then(|x| x.as_u64()) {
                        return Some(n);
                    }
                    m.values().find_map(|x| find(x, key))
                }
                serde_json::Value::Array(a) => a.iter().find_map(|x| find(x, key)),
                _ => None,
            }
        }
        if let Some(j) = &out.json {
            for key in ["health_score", "score", "architecture_score"] {
                if let Some(n) = find(j, key) {
                    return Some(n);
                }
            }
        }
        let re = regex::Regex::new(r"(\d{1,3})\s*/\s*100").unwrap();
        re.captures(&out.text).and_then(|c| c[1].parse().ok())
    }

    /// S01: A Go file declared domain by analyze.layers is held to the domain policy
    #[test]
    fn s01_go_file_declared_domain_is_held_to_domain_policy() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        go_mod(root);
        project_json(root, Some(&[("perfpayload", "domain")]));
        rules(root, &[("/domain/", &["os/exec"])]);
        write(
            root,
            "domain/access_control.go",
            "package domain\n\nimport _ \"os/exec\"\n",
        );
        write(
            root,
            "perfpayload/encode.go",
            "package perfpayload\n\nimport _ \"os/exec\"\n",
        );

        let out = analyze(root);
        let a = findings(&out, "domain/access_control.go", "os/exec");
        let b = findings(&out, "perfpayload/encode.go", "os/exec");
        assert!(
            !a.is_empty(),
            "no finding for domain/access_control.go:\n{}",
            out.text
        );
        assert!(
            !b.is_empty(),
            "no finding for perfpayload/encode.go:\n{}",
            out.text
        );
        assert!(
            a.iter().any(|r| has_line(r, "domain/access_control.go", 3)),
            "line 3 not named:\n{}",
            out.text
        );
        assert!(
            b.iter().any(|r| has_line(r, "perfpayload/encode.go", 3)),
            "line 3 not named:\n{}",
            out.text
        );
    }

    fn s02_tree(with_imports: bool) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        py_project(root);
        project_json(root, Some(&[("sdk/core", "domain")]));
        rules(root, &[("/domain/", &["os", "ssl", "subprocess"])]);
        let body = if with_imports {
            "import os\nimport ssl\nimport subprocess\n\n\ndef run():\n    return 1\n"
        } else {
            "\n\n\n\n\ndef run():\n    return 1\n"
        };
        write(root, "sdk/core/client.py", body);
        dir
    }

    /// S02: A Python domain declared only by map is checked by the domain policy
    #[test]
    fn s02_python_domain_declared_by_map_is_checked() {
        let with = s02_tree(true);
        let without = s02_tree(false);
        let out = analyze(with.path());
        for m in ["os", "ssl", "subprocess"] {
            assert!(
                !findings(&out, "sdk/core/client.py", m).is_empty(),
                "no finding for {m} in sdk/core/client.py:\n{}",
                out.text
            );
        }
        let clean = analyze(without.path());
        let s_with = score(&out).expect("score with imports");
        let s_without = score(&clean).expect("score without imports");
        assert_eq!(
            s_without,
            s_with + 30,
            "with: {s_with}, without: {s_without}\n{}",
            out.text
        );
    }

    /// S03: A layer name with a slash in it, written with or without surrounding slashes, picks up its declared files
    #[test]
    fn s03_slashed_layer_name_picks_up_declared_files() {
        for layer in [
            "adapters/secondary/",
            "/adapters/secondary",
            "adapters/secondary",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path();
            go_mod(root);
            project_json(root, Some(&[("src/db", "adapters/secondary")]));
            rules(root, &[(layer, &["net/http"])]);
            write(
                root,
                "src/db/store.go",
                "package db\n\nimport _ \"net/http\"\n",
            );
            let out = analyze(root);
            assert!(
                !findings(&out, "src/db/store.go", "net/http").is_empty(),
                "layer {layer:?}: no finding for src/db/store.go:\n{}",
                out.text
            );
        }
    }

    /// S04: A key naming a source file also covers its module
    #[test]
    fn s04_source_file_key_covers_its_module() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        py_project(root);
        project_json(root, Some(&[("src/model.py", "domain")]));
        rules(root, &[("/domain/", &["subprocess"])]);
        write(root, "src/model.py", "def f():\n    return 1\n");
        write(
            root,
            "src/model/history.py",
            "import subprocess\n\n\ndef g():\n    return 2\n",
        );
        let out = analyze(root);
        assert!(
            !findings(&out, "src/model/history.py", "subprocess").is_empty(),
            "no finding for src/model/history.py:\n{}",
            out.text
        );
    }

    /// S05: A file declared in a different layer is not added to the policy
    #[test]
    fn s05_file_declared_in_other_layer_is_not_added() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        py_project(root);
        project_json(root, Some(&[("src/http", "adapters/primary")]));
        rules(root, &[("/domain/", &["os"])]);
        write(
            root,
            "src/http/server.py",
            "import os\n\n\ndef serve():\n    return 1\n",
        );
        // Control: the policy does run on a path-matched domain file.
        write(
            root,
            "src/domain/rule.py",
            "import os\n\n\ndef rule():\n    return 1\n",
        );
        let out = analyze(root);
        assert!(
            !findings(&out, "src/domain/rule.py", "os").is_empty(),
            "control: domain policy did not run:\n{}",
            out.text
        );
        assert!(
            findings(&out, "src/http/server.py", "os").is_empty(),
            "domain policy judged src/http/server.py:\n{}",
            out.text
        );
    }

    /// S06: A policy layer that only contains a layer name, without being exactly one, adds no declared files
    #[test]
    fn s06_policy_layer_containing_layer_name_adds_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        go_mod(root);
        project_json(root, Some(&[("perfpayload", "domain")]));
        rules(root, &[("/domain/models/", &["os/exec"])]);
        write(
            root,
            "perfpayload/encode.go",
            "package perfpayload\n\nimport _ \"os/exec\"\n",
        );
        // Control: the policy does run on files its path matches.
        write(
            root,
            "domain/models/user.go",
            "package models\n\nimport _ \"os/exec\"\n",
        );
        let out = analyze(root);
        assert!(
            !findings(&out, "domain/models/user.go", "os/exec").is_empty(),
            "control: policy did not run:\n{}",
            out.text
        );
        assert!(
            findings(&out, "perfpayload/encode.go", "os/exec").is_empty(),
            "policy judged perfpayload/encode.go:\n{}",
            out.text
        );
    }

    /// S07: Every declared file is in scope of the policy for its declared layer
    #[test]
    fn s07_declared_files_are_in_scope_of_their_layer_policy() {
        let layers = ["domain", "ports", "usecases", "adapters/secondary"];
        for (i, layer) in layers.iter().enumerate() {
            let other = layers[(i + 1) % layers.len()];
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path();
            go_mod(root);
            project_json(
                root,
                Some(&[
                    ("lib/thing", layer),
                    ("lib/else", other),
                    ("tests/fixtures", layer),
                ]),
            );
            rules(root, &[(&format!("/{layer}/"), &["os/exec"])]);
            let src = "package p\n\nimport _ \"os/exec\"\n";
            let path_matched = format!("{layer}/p.go");
            let candidates = [
                path_matched.clone(),
                "lib/thing/f.go".to_string(),
                "lib/thing/deep/g.go".to_string(),
                "lib/else/h.go".to_string(),
                "lib/other/k.go".to_string(),
                "tests/fixtures/t.go".to_string(),
                format!("tests/{layer}/u.go"),
            ];
            for c in &candidates {
                write(root, c, src);
            }
            let out = analyze(root);
            let judged: BTreeSet<String> = candidates
                .iter()
                .filter(|c| !findings(&out, c, "os/exec").is_empty())
                .cloned()
                .collect();
            let expected: BTreeSet<String> = [
                path_matched,
                "lib/thing/f.go".to_string(),
                "lib/thing/deep/g.go".to_string(),
            ]
            .into_iter()
            .collect();
            assert_eq!(judged, expected, "layer {layer}:\n{}", out.text);
        }
    }

    /// S08: Without analyze.layers, a policy judges exactly the files its path matches
    #[test]
    fn s08_without_layers_policy_is_path_only() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            "Cargo.toml",
            "[package]\nname = \"proj\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        project_json(root, None);
        rules(root, &[("/domain/", &["std::fs"])]);
        write(root, "src/lib.rs", "pub mod domain;\npub mod core;\n");
        write(root, "src/domain/mod.rs", "pub mod a;\n");
        write(root, "src/core/mod.rs", "pub mod b;\n");
        write(
            root,
            "src/domain/a.rs",
            "use std::fs;\n\npub fn a() { let _ = fs::read(\"x\"); }\n",
        );
        write(
            root,
            "src/core/b.rs",
            "use std::fs;\n\npub fn b() { let _ = fs::read(\"x\"); }\n",
        );
        let out = analyze(root);
        let a = findings(&out, "src/domain/a.rs", "std::fs");
        assert!(
            !a.is_empty(),
            "no finding for src/domain/a.rs:\n{}",
            out.text
        );
        assert!(
            findings(&out, "src/core/b.rs", "std::fs").is_empty(),
            "src/core/b.rs judged:\n{}",
            out.text
        );
    }

    /// S09: A crate named for its layer adds nothing to the policy
    #[test]
    fn s09_crate_named_for_layer_adds_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            "Cargo.toml",
            "[workspace]\nmembers = [\"app-domain\", \"app\"]\nresolver = \"2\"\n",
        );
        write(
            root,
            "app-domain/Cargo.toml",
            "[package]\nname = \"app-domain\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        write(
            root,
            "app-domain/src/lib.rs",
            "use std::fs;\n\npub fn a() { let _ = fs::read(\"x\"); }\n",
        );
        write(
            root,
            "app/Cargo.toml",
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        write(root, "app/src/lib.rs", "pub mod domain;\n");
        write(
            root,
            "app/src/domain/mod.rs",
            "use std::fs;\n\npub fn c() { let _ = fs::read(\"x\"); }\n",
        );
        project_json(root, None);
        rules(root, &[("/domain/", &["std::fs"])]);
        let out = analyze(root);
        assert!(
            !findings(&out, "app/src/domain/mod.rs", "std::fs").is_empty(),
            "control: domain policy did not run:\n{}",
            out.text
        );
        assert!(
            findings(&out, "app-domain/src/lib.rs", "std::fs").is_empty(),
            "app-domain/src/lib.rs judged:\n{}",
            out.text
        );
    }

    /// S10: A key covers whole path segments only
    #[test]
    fn s10_key_covers_whole_segments_only() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        py_project(root);
        project_json(root, Some(&[("src/core", "domain")]));
        rules(root, &[("/domain/", &["os"])]);
        write(
            root,
            "src/core/model.py",
            "import os\n\n\ndef m():\n    return 1\n",
        );
        write(
            root,
            "src/core_utils/io.py",
            "import os\n\n\ndef io():\n    return 1\n",
        );
        let out = analyze(root);
        assert!(
            !findings(&out, "src/core/model.py", "os").is_empty(),
            "control: declared file src/core/model.py not judged:\n{}",
            out.text
        );
        assert!(
            findings(&out, "src/core_utils/io.py", "os").is_empty(),
            "src/core_utils/io.py judged:\n{}",
            out.text
        );
    }
}

mod part_2 {
    #[allow(unused_imports)]
    use std::path::{Path, PathBuf};

    /// Write `contents` to `root/rel`, creating parent directories.
    fn write(root: &Path, rel: &str, contents: &str) {
        let p = root.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).expect("create dirs");
        }
        std::fs::write(p, contents).expect("write file");
    }

    /// A Python project skeleton.
    fn python_project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            dir.path(),
            "pyproject.toml",
            "[project]\nname = \"sample\"\nversion = \"0.1.0\"\n",
        );
        dir
    }

    /// A Go project skeleton.
    fn go_project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            dir.path(),
            "go.mod",
            "module example.com/sample\n\ngo 1.21\n",
        );
        dir
    }

    fn set_layers(root: &Path, layers: &[(&str, &str)]) {
        let mut map = serde_json::Map::new();
        for (k, v) in layers {
            map.insert(
                (*k).to_string(),
                serde_json::Value::String((*v).to_string()),
            );
        }
        let doc = serde_json::json!({ "analyze": { "layers": serde_json::Value::Object(map) } });
        write(
            root,
            ".hexa/project.json",
            &serde_json::to_string_pretty(&doc).expect("serialize project.json"),
        );
    }

    struct Policy<'a> {
        layer: &'a str,
        allow: Option<&'a [&'a str]>,
        deny: Option<&'a [&'a str]>,
        severity: &'a str,
    }

    fn toml_list(items: &[&str]) -> String {
        let inner: Vec<String> = items.iter().map(|s| format!("\"{s}\"")).collect();
        format!("[{}]", inner.join(", "))
    }

    fn set_policies(root: &Path, policies: &[Policy<'_>]) {
        let mut out = String::new();
        for p in policies {
            out.push_str("[[import_policy]]\n");
            // The fields a policy requires besides its layer and lists.
            out.push_str("adr = \"ADR-2609211430\"\nid = \"fixture-policy\"\nmessage = \"fixture policy\"\n");
            out.push_str(&format!("layer = \"{}\"\n", p.layer));
            if let Some(a) = p.allow {
                out.push_str(&format!("allow = {}\n", toml_list(a)));
            }
            if let Some(d) = p.deny {
                out.push_str(&format!("deny = {}\n", toml_list(d)));
            }
            out.push_str(&format!("severity = \"{}\"\n\n", p.severity));
        }
        write(root, ".hexa/ADR-rules.toml", &out);
    }

    /// Run `hexa analyze <root> --json` and parse its output.
    fn analyze(root: &Path) -> serde_json::Value {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_hexa"))
            .arg("analyze")
            .arg(root)
            .arg("--json")
            .current_dir(root)
            .output()
            .expect("run hexa");
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        let start = stdout.find('{');
        let end = stdout.rfind('}');
        assert!(
            start.is_some() && end.is_some() && end >= start,
            "no JSON in hexa analyze output.\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
        let (start, end) = (start.unwrap_or(0), end.unwrap_or(0));
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(&stdout[start..=end]);
        assert!(
            parsed.is_ok(),
            "hexa analyze output is not JSON:\n{stdout}\nstderr:\n{stderr}"
        );
        parsed.unwrap_or(serde_json::Value::Null)
    }

    fn word_re(module: &str) -> regex::Regex {
        regex::Regex::new(&format!(
            r"(^|[^A-Za-z0-9_]){}([^A-Za-z0-9_]|$)",
            regex::escape(module)
        ))
        .expect("valid regex")
    }

    fn matches(text: &str, file: &str, re: &regex::Regex, exclude: Option<&regex::Regex>) -> bool {
        text.contains(file) && re.is_match(text) && exclude.map_or(true, |x| !x.is_match(text))
    }

    fn count_rec(
        v: &serde_json::Value,
        file: &str,
        re: &regex::Regex,
        exclude: Option<&regex::Regex>,
    ) -> usize {
        match v {
            serde_json::Value::Object(map) => {
                let own: Vec<String> = map
                    .values()
                    .filter_map(|x| match x {
                        serde_json::Value::String(s) => Some(s.clone()),
                        serde_json::Value::Number(n) => Some(n.to_string()),
                        _ => None,
                    })
                    .collect();
                let joined = own.join(" \u{1} ");
                let mut n = usize::from(matches(&joined, file, re, exclude));
                for x in map.values() {
                    n += count_rec(x, file, re, exclude);
                }
                n
            }
            serde_json::Value::Array(items) => items
                .iter()
                .map(|x| match x {
                    serde_json::Value::String(s) => usize::from(matches(s, file, re, exclude)),
                    other => count_rec(other, file, re, exclude),
                })
                .sum(),
            _ => 0,
        }
    }

    /// How many findings in the report name `file` and `module`.
    fn findings(report: &serde_json::Value, file: &str, module: &str) -> usize {
        count_rec(report, file, &word_re(module), None)
    }

    /// Like `findings`, but ignoring entries that also name `except`.
    fn findings_except(
        report: &serde_json::Value,
        file: &str,
        module: &str,
        except: &str,
    ) -> usize {
        let ex = word_re(except);
        count_rec(report, file, &word_re(module), Some(&ex))
    }

    fn find_number(v: &serde_json::Value, key: &str) -> Option<f64> {
        match v {
            serde_json::Value::Object(map) => {
                if let Some(n) = map.get(key).and_then(|x| x.as_f64()) {
                    return Some(n);
                }
                map.values().find_map(|x| find_number(x, key))
            }
            serde_json::Value::Array(items) => items.iter().find_map(|x| find_number(x, key)),
            _ => None,
        }
    }

    fn score(report: &serde_json::Value) -> f64 {
        let s = ["health_score", "score", "architecture_score"]
            .iter()
            .find_map(|k| find_number(report, k));
        assert!(s.is_some(), "no score in report: {report}");
        s.unwrap_or(-1.0)
    }

    fn find_key<'a>(v: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
        match v {
            serde_json::Value::Object(map) => {
                if let Some(x) = map.get(key) {
                    return Some(x);
                }
                map.values().find_map(|x| find_key(x, key))
            }
            serde_json::Value::Array(items) => items.iter().find_map(|x| find_key(x, key)),
            _ => None,
        }
    }

    /// S11: No declaration, no change
    #[test]
    fn s11_no_declaration_no_change() {
        let dir = python_project();
        let root = dir.path();
        write(root, "src/domain/x.py", "import os\n");
        write(root, "lib/core/y.py", "import os\n");
        set_policies(
            root,
            &[Policy {
                layer: "/domain/",
                allow: None,
                deny: Some(&["os"]),
                severity: "error",
            }],
        );
        let with_rules = analyze(root);
        assert_eq!(
            findings(&with_rules, "src/domain/x.py", "os"),
            1,
            "the path-matched file is reported exactly once: {with_rules}"
        );
        assert_eq!(
            findings(&with_rules, "lib/core/y.py", "os"),
            0,
            "a file outside the path, with no analyze.layers, is not judged: {with_rules}"
        );

        // An empty project.json (no analyze.layers) changes nothing.
        write(root, ".hexa/project.json", "{}\n");
        let with_empty_config = analyze(root);
        assert_eq!(findings(&with_empty_config, "src/domain/x.py", "os"), 1);
        assert_eq!(findings(&with_empty_config, "lib/core/y.py", "os"), 0);
        assert_eq!(score(&with_rules), score(&with_empty_config));
    }

    /// S12: A file matched by path and also declared is judged once
    #[test]
    fn s12_path_matched_and_declared_is_judged_once() {
        let policy = [Policy {
            layer: "/domain/",
            allow: None,
            deny: Some(&["os"]),
            severity: "error",
        }];

        let dir = python_project();
        let root = dir.path();
        set_layers(root, &[("src/domain", "domain")]);
        set_policies(root, &policy);
        write(root, "src/domain/x.py", "import os\n\nX = 1\n");
        let report = analyze(root);

        let base_dir = python_project();
        let base = base_dir.path();
        set_layers(base, &[("src/domain", "domain")]);
        set_policies(base, &policy);
        write(base, "src/domain/x.py", "X = 1\n");
        let baseline = analyze(base);

        assert_eq!(
            findings(&report, "src/domain/x.py", "os"),
            1,
            "exactly one finding for the import: {report}"
        );
        assert_eq!(
            score(&baseline) - score(&report),
            10.0,
            "the score falls by 10, not 20: baseline {baseline}, report {report}"
        );
    }

    /// S13: Allow and severity apply to declared files as written
    #[test]
    fn s13_allow_and_severity_apply_to_declared_files() {
        let policy = [Policy {
            layer: "/domain/",
            allow: Some(&["fmt"]),
            deny: None,
            severity: "warning",
        }];

        let dir = go_project();
        let root = dir.path();
        set_layers(root, &[("pkg/model", "domain")]);
        set_policies(root, &policy);
        write(
            root,
            "pkg/model/a.go",
            "package model\n\nimport (\n\t_ \"fmt\"\n\t_ \"example.com/ext\"\n)\n",
        );
        let report = analyze(root);

        let base_dir = go_project();
        let base = base_dir.path();
        set_layers(base, &[("pkg/model", "domain")]);
        set_policies(base, &policy);
        write(base, "pkg/model/a.go", "package model\n");
        let baseline = analyze(base);

        assert_eq!(
            findings_except(&report, "pkg/model/a.go", "fmt", "example.com/ext"),
            0,
            "fmt is allowed: {report}"
        );
        assert_eq!(
            findings(&report, "pkg/model/a.go", "example.com/ext"),
            1,
            "one warning for example.com/ext (an external import the allow list omits; the standard library is permitted unless denied): {report}"
        );
        assert_eq!(
            score(&report),
            score(&baseline),
            "a warning does not change the score"
        );
    }

    /// S14: A declared file under top-level tests/ stays out of scope
    #[test]
    fn s14_declared_file_under_top_level_tests_stays_out_of_scope() {
        let dir = python_project();
        let root = dir.path();
        set_layers(
            root,
            &[
                ("tests/fixtures", "domain"),
                ("benches/fixtures", "domain"),
                ("examples/fixtures", "domain"),
                ("app/core", "domain"),
            ],
        );
        set_policies(
            root,
            &[Policy {
                layer: "/domain/",
                allow: None,
                deny: Some(&["subprocess"]),
                severity: "error",
            }],
        );
        write(root, "tests/fixtures/run.py", "import subprocess\n");
        write(root, "benches/fixtures/run.py", "import subprocess\n");
        write(root, "examples/fixtures/run.py", "import subprocess\n");
        write(root, "app/core/m.py", "import subprocess\n");
        let report = analyze(root);

        for f in [
            "tests/fixtures/run.py",
            "benches/fixtures/run.py",
            "examples/fixtures/run.py",
        ] {
            assert_eq!(
                findings(&report, f, "subprocess"),
                0,
                "{f} is out of scope: {report}"
            );
        }
        assert_eq!(
            findings(&report, "app/core/m.py", "subprocess"),
            1,
            "a declared domain file elsewhere is judged: {report}"
        );
    }

    /// S15: Each import is judged at most once per policy
    #[test]
    fn s15_each_import_judged_at_most_once_per_policy() {
        let dir = python_project();
        let root = dir.path();
        set_layers(
            root,
            &[
                ("src", "domain"),
                ("src/domain", "domain"),
                ("tests/domain", "domain"),
            ],
        );
        set_policies(
            root,
            &[Policy {
                layer: "/domain/",
                allow: None,
                deny: Some(&["socket"]),
                severity: "error",
            }],
        );
        write(root, "src/domain/x.py", "import socket\n");
        write(root, "src/other/y.py", "import socket\n");
        write(root, "tests/domain/z.py", "import socket\n");
        let report = analyze(root);

        assert_eq!(
            findings(&report, "src/domain/x.py", "socket"),
            1,
            "path-matched and declared: exactly one finding: {report}"
        );
        assert_eq!(
            findings(&report, "src/other/y.py", "socket"),
            1,
            "declared only: exactly one finding: {report}"
        );
        assert_eq!(
            findings(&report, "tests/domain/z.py", "socket"),
            0,
            "top-level tests/ yields nothing: {report}"
        );
    }

    /// S16: A path policy that names no layer stays path-only
    #[test]
    fn s16_path_policy_naming_no_layer_stays_path_only() {
        let dir = python_project();
        let root = dir.path();
        set_layers(
            root,
            &[
                ("netbox_plugin/utilities", "domain"),
                ("netbox_plugin/core", "domain"),
            ],
        );
        set_policies(
            root,
            &[Policy {
                layer: "netbox_plugin/utilities",
                allow: None,
                deny: Some(&["requests"]),
                severity: "error",
            }],
        );
        write(root, "netbox_plugin/__init__.py", "");
        write(root, "netbox_plugin/utilities/a.py", "import requests\n");
        write(root, "netbox_plugin/core/b.py", "import requests\n");
        let report = analyze(root);

        assert_eq!(
            findings(&report, "netbox_plugin/utilities/a.py", "requests"),
            1,
            "{report}"
        );
        assert_eq!(
            findings(&report, "netbox_plugin/core/b.py", "requests"),
            0,
            "{report}"
        );
    }

    /// S18: Non-layer policies are unaffected by any analyze.layers map
    #[test]
    fn s18_non_layer_policies_unaffected_by_layers_map() {
        let build = |with_policy: bool| {
            let dir = python_project();
            let root = dir.path();
            set_layers(root, &[("lib/x", "domain"), ("other", "domain")]);
            if with_policy {
                set_policies(
                    root,
                    &[Policy {
                        layer: "lib",
                        allow: None,
                        deny: Some(&["json"]),
                        severity: "warning",
                    }],
                );
            }
            write(root, "lib/x/a.py", "import json\n");
            write(root, "other/b.py", "import json\n");
            let report = analyze(root);
            (dir, report)
        };
        let (_a, with_policy) = build(true);
        let (_b, without_policy) = build(false);

        assert_eq!(
            findings(&with_policy, "lib/x/a.py", "json"),
            1,
            "{with_policy}"
        );
        assert_eq!(
        findings(&with_policy, "other/b.py", "json"),
        0,
        "a declared domain file outside the path is not judged by a non-layer policy: {with_policy}"
    );
        assert_eq!(score(&with_policy), score(&without_policy));
        assert_eq!(
            find_key(&with_policy, "violations"),
            find_key(&without_policy, "violations")
        );
    }
}
