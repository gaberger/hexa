#![allow(unused, clippy::all)]
mod part_1 {
    #![allow(dead_code, clippy::all)]

    use hexa_analysis::domain::ArchAnalysisResult;
    use hexa_analysis::ports::ArchAnalysisPort;
    use std::path::Path;

    // ── Fixture helpers ──────────────────────────────────────

    /// The analyzer under test, composed from the crate's adapters. This is the
    /// one place that names the concrete `ArchAnalysisPort` implementation.
    fn analyzer() -> impl ArchAnalysisPort {
        hexa_analysis::analyzer::ArchAnalyzer::new(std::sync::Arc::new(
            hexa_analysis::treesitter_adapter::TreeSitterAdapter::new(),
        ))
    }

    fn analyze(root: &Path) -> ArchAnalysisResult {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        let a = analyzer();
        rt.block_on(a.analyze(root)).expect("analysis runs")
    }

    fn source_for(path: &str, n: usize) -> String {
        if path.ends_with(".rs") {
            format!("pub fn item_{n}() -> u32 {{ {n} }}\n")
        } else if path.ends_with(".go") {
            format!("package p\n\nfunc Item{n}() int {{ return {n} }}\n")
        } else if path.ends_with(".py") {
            format!("def item_{n}():\n    return {n}\n")
        } else {
            format!("export const item{n} = {n};\n")
        }
    }

    fn project(files: &[&str], layers: &[(&str, &str)], exclude: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        for (i, f) in files.iter().enumerate() {
            let p = dir.path().join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, source_for(f, i)).unwrap();
        }
        let mut layer_obj = serde_json::Map::new();
        for (k, v) in layers {
            layer_obj.insert(
                (*k).to_string(),
                serde_json::Value::String((*v).to_string()),
            );
        }
        let mut analyze_obj = serde_json::Map::new();
        if !layers.is_empty() {
            analyze_obj.insert("layers".into(), serde_json::Value::Object(layer_obj));
        }
        if !exclude.is_empty() {
            analyze_obj.insert(
                "exclude".into(),
                serde_json::Value::Array(
                    exclude
                        .iter()
                        .map(|e| serde_json::Value::String((*e).to_string()))
                        .collect(),
                ),
            );
        }
        let config = serde_json::json!({ "analyze": serde_json::Value::Object(analyze_obj) });
        std::fs::create_dir_all(dir.path().join(".hexa")).unwrap();
        std::fs::write(
            dir.path().join(".hexa/project.json"),
            serde_json::to_string_pretty(&config).unwrap(),
        )
        .unwrap();
        dir
    }

    const UNMATCHED: &str = "matches no file the grade reads";

    fn layer_key_errors(r: &ArchAnalysisResult) -> Vec<&String> {
        r.config_errors
            .iter()
            .filter(|e| e.contains("analyze.layers[") && e.contains(UNMATCHED))
            .collect()
    }

    fn names_key(entry: &str, key: &str) -> bool {
        entry.contains(&format!("analyze.layers[\"{key}\"]"))
    }

    fn entry_for<'a>(r: &'a ArchAnalysisResult, key: &str) -> Option<&'a String> {
        layer_key_errors(r).into_iter().find(|e| names_key(e, key))
    }

    fn mentions_quoted(entry: &str, key: &str) -> bool {
        entry.contains(&format!("\"{key}\""))
    }

    // ── Scenarios ────────────────────────────────────────────

    /// S01: A glob key that matches no graded file is reported by name
    #[test]
    fn s01_glob_key_matching_no_file_is_reported_by_name() {
        let dir = project(
            &["src/domain/order.ts", "src/ui/view.ts"],
            &[("src/domain", "domain"), ("src/ui/**", "adapters/primary")],
            &[],
        );
        let r = analyze(dir.path());
        let errs = layer_key_errors(&r);
        assert_eq!(
            errs.len(),
            1,
            "exactly one layer-key entry, got {:?}",
            r.config_errors
        );
        assert!(
            errs[0].contains("analyze.layers[\"src/ui/**\"] matches no file the grade reads"),
            "entry was {:?}",
            errs[0]
        );
        assert!(
            !r.config_errors
                .iter()
                .any(|e| mentions_quoted(e, "src/domain")),
            "no entry names src/domain: {:?}",
            r.config_errors
        );
    }

    /// S02: Each unmatched key costs what one rule error costs
    #[test]
    fn s02_each_unmatched_key_costs_one_rule_error() {
        let files = ["src/domain/a.rs", "src/ports/b.rs"];
        let a = project(
            &files,
            &[("src/domain", "domain"), ("src/ports", "ports")],
            &[],
        );
        let b = project(
            &files,
            &[
                ("src/domain", "domain"),
                ("src/ports", "ports"),
                ("src/adpaters", "adapters/secondary"),
            ],
            &[],
        );
        let ra = analyze(a.path());
        let rb = analyze(b.path());
        assert_eq!(
            rb.config_errors.len(),
            ra.config_errors.len() + 1,
            "A: {:?}\nB: {:?}",
            ra.config_errors,
            rb.config_errors
        );
        let extra: Vec<&String> = rb
            .config_errors
            .iter()
            .filter(|e| !ra.config_errors.contains(e))
            .collect();
        assert_eq!(extra.len(), 1, "B's extra entries: {extra:?}");
        assert!(
            names_key(extra[0], "src/adpaters"),
            "extra entry: {:?}",
            extra[0]
        );
        assert!(extra[0].contains(UNMATCHED), "extra entry: {:?}", extra[0]);
        assert_eq!(
            i32::from(ra.health_score) - i32::from(rb.health_score),
            10,
            "A scored {}, B scored {}",
            ra.health_score,
            rb.health_score
        );
    }

    /// S03: A key whose only files are removed by analyze.exclude is reported
    #[test]
    fn s03_key_whose_files_are_all_excluded_is_reported() {
        let dir = project(
            &["src/domain/a.ts", "src/generated/client.ts"],
            &[
                ("src/domain", "domain"),
                ("src/generated", "adapters/secondary"),
            ],
            &["src/generated"],
        );
        let r = analyze(dir.path());
        assert!(
            entry_for(&r, "src/generated").is_some(),
            "expected an entry for src/generated: {:?}",
            r.config_errors
        );
        assert!(
            !r.config_errors
                .iter()
                .any(|e| mentions_quoted(e, "src/domain")),
            "no entry names src/domain: {:?}",
            r.config_errors
        );
    }

    /// S04: Two unmatched keys produce two entries
    #[test]
    fn s04_two_unmatched_keys_produce_two_entries() {
        let files = ["src/domain/a.ts"];
        let base = project(&files, &[("src/domain", "domain")], &[]);
        let with = project(
            &files,
            &[
                ("src/domain", "domain"),
                ("src/old_folder", "ports"),
                ("lib/ui", "adapters/primary"),
            ],
            &[],
        );
        let rb = analyze(base.path());
        let rw = analyze(with.path());
        let errs = layer_key_errors(&rw);
        assert_eq!(
            errs.len(),
            2,
            "two layer-key entries: {:?}",
            rw.config_errors
        );
        assert_eq!(
            errs.iter()
                .filter(|e| names_key(e, "src/old_folder"))
                .count(),
            1
        );
        assert_eq!(errs.iter().filter(|e| names_key(e, "lib/ui")).count(), 1);
        assert_eq!(
            i32::from(rb.health_score) - i32::from(rw.health_score),
            20,
            "without keys {}, with keys {}",
            rb.health_score,
            rw.health_score
        );
    }

    /// S05: A string prefix that does not fall on a segment boundary is not a match
    #[test]
    fn s05_prefix_off_a_segment_boundary_is_not_a_match() {
        let dir = project(&["src/domainx/a.ts"], &[("src/domain", "domain")], &[]);
        let r = analyze(dir.path());
        assert!(
            entry_for(&r, "src/domain").is_some(),
            "src/domain must not count as matching src/domainx/a.ts: {:?}",
            r.config_errors
        );
    }

    /// S08: Layer-key errors correspond one to one with unmatched keys
    #[test]
    fn s08_layer_key_errors_correspond_one_to_one_with_unmatched_keys() {
        let files = ["src/domain/a.rs", "src/ports/b.rs", "src/gen/c.rs"];
        let exclude = ["src/gen"];
        let matched: [(&str, &str); 2] = [("src/domain", "domain"), ("src/ports", "ports")];
        let unmatched: [&str; 12] = [
            "src/gen",
            "src/dom",
            "src/domainx",
            "lib/x",
            "src/ui/**",
            "src/a?c",
            "src/old",
            "pkg/one",
            "pkg/two",
            "src/ports/b.rs/zz",
            "src/missing.rs",
            "other",
        ];

        let base = project(&files, &matched, &exclude);
        let rb = analyze(base.path());
        assert!(
            layer_key_errors(&rb).is_empty(),
            "baseline: {:?}",
            rb.config_errors
        );

        for n in [1usize, 2, 3, 5, 12] {
            let mut layers: Vec<(&str, &str)> = matched.to_vec();
            for k in &unmatched[..n] {
                layers.push((k, "adapters/secondary"));
            }
            let dir = project(&files, &layers, &exclude);
            let r = analyze(dir.path());
            let errs = layer_key_errors(&r);
            assert_eq!(errs.len(), n, "n = {n}: {:?}", r.config_errors);
            for k in &unmatched[..n] {
                assert_eq!(
                    errs.iter().filter(|e| names_key(e, k)).count(),
                    1,
                    "key {k} must be named in exactly one entry: {:?}",
                    r.config_errors
                );
            }
            for (k, _) in &matched {
                assert!(
                    !errs.iter().any(|e| names_key(e, k)),
                    "matched key {k} reported: {:?}",
                    r.config_errors
                );
            }
            let expected = i32::from(rb.health_score) - 10 * n as i32;
            assert_eq!(
                i32::from(r.health_score),
                expected.max(0),
                "n = {n}: baseline {}, got {}",
                rb.health_score,
                r.health_score
            );
        }
    }

    /// S09: A ** key is told that keys are prefixes and is offered the prefix
    #[test]
    fn s09_double_star_key_is_told_keys_are_prefixes_and_offered_prefix() {
        let dir = project(
            &["src/ui/view.ts"],
            &[("src/ui/**", "adapters/primary")],
            &[],
        );
        let r = analyze(dir.path());
        let entry = entry_for(&r, "src/ui/**")
            .unwrap_or_else(|| panic!("no entry for src/ui/**: {:?}", r.config_errors));
        let lower = entry.to_lowercase();
        assert!(
            lower.contains("prefix"),
            "must say keys are path prefixes: {entry:?}"
        );
        assert!(
            lower.contains("not glob"),
            "must say keys are not globs: {entry:?}"
        );
        let rest = entry.replace("src/ui/**", "");
        assert!(
            rest.contains("src/ui"),
            "must name the prefix src/ui: {entry:?}"
        );
        assert!(
            !rest.contains("src/ui/"),
            "prefix must have its trailing slash removed: {entry:?}"
        );
    }

    /// S10: The ? and [ characters count as glob characters, and the prefix stops before the first of them
    #[test]
    fn s10_question_mark_and_bracket_are_glob_characters() {
        let dir = project(
            &["src/domain/a.ts"],
            &[
                ("src/domain", "domain"),
                ("src/a?c/x", "ports"),
                ("lib/[ab]/y", "adapters/primary"),
            ],
            &[],
        );
        let r = analyze(dir.path());

        let q = entry_for(&r, "src/a?c/x")
            .unwrap_or_else(|| panic!("no entry for src/a?c/x: {:?}", r.config_errors));
        let ql = q.to_lowercase();
        assert!(ql.contains("prefix") && ql.contains("not glob"), "{q:?}");
        let q_rest = q.replace("src/a?c/x", "");
        assert!(
            q_rest.contains("src/a"),
            "must name the prefix src/a: {q:?}"
        );
        assert!(!q_rest.contains("src/a?"), "prefix stops before '?': {q:?}");

        let b = entry_for(&r, "lib/[ab]/y")
            .unwrap_or_else(|| panic!("no entry for lib/[ab]/y: {:?}", r.config_errors));
        let bl = b.to_lowercase();
        assert!(bl.contains("prefix") && bl.contains("not glob"), "{b:?}");
        let b_rest = b.replace("lib/[ab]/y", "");
        assert!(b_rest.contains("lib"), "must name the prefix lib: {b:?}");
        assert!(
            !b_rest.contains("lib/"),
            "prefix must have its trailing slash removed: {b:?}"
        );
    }
}

mod part_2 {
    #![allow(dead_code, unused_imports, clippy::all)]

    use hexa_analysis::domain::{ArchAnalysisResult, HexLayer};
    use hexa_analysis::layer_classifier::{classify_layer, LayerMap};
    use hexa_analysis::ports::{
        AnalysisError, ArchAnalysisPort, AstPort, ExportDeclaration, ImportStatement, ItemCounts,
        Language, ModuleReference,
    };
    use std::future::Future;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    // ── helpers ──────────────────────────────────────────────

    fn block_on<F: Future>(fut: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime")
            .block_on(fut)
    }

    /// A small, deterministic AstPort for the fixtures below: it reads
    /// `import ... from "<spec>"` (TypeScript) and `use crate::a::b;` (Rust),
    /// resolving each to a project-relative file path.
    struct FixtureAst {
        root: PathBuf,
    }

    impl FixtureAst {
        fn rel(&self, path: &Path) -> String {
            let p = path.strip_prefix(&self.root).unwrap_or(path);
            p.to_string_lossy()
                .replace('\\', "/")
                .trim_start_matches("./")
                .to_string()
        }
    }

    fn normalize(parts: Vec<&str>) -> String {
        let mut out: Vec<&str> = Vec::new();
        for part in parts {
            match part {
                "" | "." => {}
                ".." => {
                    out.pop();
                }
                p => out.push(p),
            }
        }
        out.join("/")
    }

    impl AstPort for FixtureAst {
        fn extract_imports(
            &self,
            path: &Path,
            source: &str,
            lang: Language,
        ) -> Result<Vec<ImportStatement>, AnalysisError> {
            let from = self.rel(path);
            let mut out = Vec::new();
            for (i, line) in source.lines().enumerate() {
                let line = line.trim();
                match lang {
                    Language::TypeScript => {
                        if !line.starts_with("import") {
                            continue;
                        }
                        let Some(idx) = line.find("from") else {
                            continue;
                        };
                        let rest = &line[idx + 4..];
                        let spec: String = rest
                            .trim()
                            .trim_end_matches(';')
                            .trim_matches(|c| c == '"' || c == '\'')
                            .to_string();
                        if !spec.starts_with('.') {
                            continue;
                        }
                        let dir: Vec<&str> = from
                            .rsplit_once('/')
                            .map(|(d, _)| d)
                            .unwrap_or("")
                            .split('/')
                            .collect();
                        let mut parts = dir;
                        parts.extend(spec.split('/'));
                        let mut resolved = normalize(parts);
                        if !resolved.ends_with(".ts") {
                            resolved.push_str(".ts");
                        }
                        out.push(ImportStatement {
                            from_file: from.clone(),
                            raw_path: spec.clone(),
                            resolved_path: resolved,
                            names: Vec::new(),
                            line: i + 1,
                        });
                    }
                    Language::Rust => {
                        let Some(rest) = line.strip_prefix("use crate::") else {
                            continue;
                        };
                        let raw = rest.trim_end_matches(';').to_string();
                        let segs: Vec<&str> = raw.split("::").collect();
                        let resolved = format!("src/{}.rs", segs.join("/"));
                        out.push(ImportStatement {
                            from_file: from.clone(),
                            raw_path: format!("crate::{raw}"),
                            resolved_path: resolved,
                            names: Vec::new(),
                            line: i + 1,
                        });
                    }
                    _ => {}
                }
            }
            Ok(out)
        }

        fn count_items(&self, _source: &str, _lang: Language) -> Result<ItemCounts, AnalysisError> {
            Ok(ItemCounts::default())
        }

        fn module_references(
            &self,
            _source: &str,
            _lang: Language,
        ) -> Result<Vec<ModuleReference>, AnalysisError> {
            Ok(Vec::new())
        }

        fn extract_exports(
            &self,
            _path: &Path,
            _source: &str,
            _lang: Language,
        ) -> Result<Vec<ExportDeclaration>, AnalysisError> {
            Ok(Vec::new())
        }
    }

    fn analyze(root: &Path) -> ArchAnalysisResult {
        let ast: Arc<dyn AstPort> = Arc::new(FixtureAst {
            root: root.to_path_buf(),
        });
        let analyzer = hexa_analysis::analyzer::ArchAnalyzer::new(ast);
        block_on(analyzer.analyze(root)).expect("analysis runs")
    }

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    /// A project with the given files and, when `layers` is `Some`, those
    /// `analyze.layers` entries.
    fn project(files: &[(&str, &str)], layers: Option<&[(&str, &str)]>) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (rel, body) in files {
            write(dir.path(), rel, body);
        }
        let config = match layers {
            None => serde_json::json!({ "analyze": {} }),
            Some(entries) => {
                let mut map = serde_json::Map::new();
                for (k, v) in entries {
                    map.insert(k.to_string(), serde_json::Value::String(v.to_string()));
                }
                serde_json::json!({ "analyze": { "layers": map } })
            }
        };
        write(
            dir.path(),
            ".hexa/project.json",
            &serde_json::to_string_pretty(&config).unwrap(),
        );
        dir
    }

    fn layer_key_errors(r: &ArchAnalysisResult) -> Vec<String> {
        r.config_errors
            .iter()
            .filter(|e| {
                e.contains("analyze.layers[") && e.contains("matches no file the grade reads")
            })
            .cloned()
            .collect()
    }

    fn entry_for(r: &ArchAnalysisResult, key: &str) -> Option<String> {
        let needle = format!("analyze.layers[\"{key}\"]");
        r.config_errors
            .iter()
            .find(|e| e.contains(&needle))
            .cloned()
    }

    fn has_glob_hint(entry: &str, key: &str, prefix: &str) -> bool {
        let lower = entry.to_lowercase();
        let without_key = entry.replace(key, "");
        lower.contains("glob") && lower.contains("prefix") && without_key.contains(prefix)
    }

    fn has_no_glob_hint(entry: &str) -> bool {
        let lower = entry.to_lowercase();
        !lower.contains("glob") && !lower.contains("prefix")
    }

    fn expected_prefix(key: &str) -> String {
        let cut = key
            .find(|c| c == '*' || c == '?' || c == '[')
            .unwrap_or(key.len());
        key[..cut].trim_end_matches('/').to_string()
    }

    fn v<T: serde::Serialize>(x: &T) -> serde_json::Value {
        serde_json::to_value(x).unwrap()
    }

    const S16_FILES: &[(&str, &str)] = &[
        (
            "src/domain/a.rs",
            "use crate::adapters::primary::cli;\npub fn a() {}\n",
        ),
        ("src/adapters/primary/cli.rs", "pub fn run() {}\n"),
    ];
    const S16_LAYERS: &[(&str, &str)] = &[
        ("src/domain", "domain"),
        ("src/adapters/primary", "adapters/primary"),
    ];

    fn assert_same_findings(p: &ArchAnalysisResult, q: &ArchAnalysisResult) {
        assert_eq!(v(&p.violations), v(&q.violations), "violations");
        assert_eq!(p.coverage, q.coverage, "coverage");
        assert_eq!(v(&p.dead_exports), v(&q.dead_exports), "dead_exports");
        assert_eq!(p.circular_deps, q.circular_deps, "circular_deps");
        assert_eq!(p.orphan_files, q.orphan_files, "orphan_files");
        assert_eq!(p.unused_ports, q.unused_ports, "unused_ports");
        assert_eq!(p.file_count, q.file_count, "file_count");
        assert_eq!(p.edge_count, q.edge_count, "edge_count");
    }

    // ── scenarios ────────────────────────────────────────────

    /// S11: An unmatched key with no glob character gets no glob hint
    #[test]
    fn s11_unmatched_key_without_glob_gets_no_glob_hint() {
        let dir = project(
            &[
                ("src/domain/a.rs", "pub fn a() {}\n"),
                ("src/adapters/primary/cli.rs", "pub fn run() {}\n"),
            ],
            Some(&[
                ("src/domain", "domain"),
                ("src/adpaters", "adapters/primary"),
            ]),
        );
        let r = analyze(dir.path());
        let entry = entry_for(&r, "src/adpaters")
            .unwrap_or_else(|| panic!("no entry for src/adpaters in {:?}", r.config_errors));
        assert!(entry.contains("matches no file the grade reads"), "{entry}");
        assert!(has_no_glob_hint(&entry), "unexpected glob hint: {entry}");
    }

    /// S12: A glob key is not read as a glob even when files would match the pattern
    #[test]
    fn s12_glob_key_is_not_read_as_glob() {
        let dir = project(
            &[
                ("src/ui/view.ts", "export const view = 1;\n"),
                ("src/ui/deep/panel.ts", "export const panel = 1;\n"),
            ],
            Some(&[("src/ui/**", "adapters/primary")]),
        );
        let map = LayerMap::from_project(dir.path()).expect("layers read");
        for f in ["src/ui/view.ts", "src/ui/deep/panel.ts"] {
            assert_ne!(
                map.classify(f),
                HexLayer::AdaptersPrimary,
                "{f} classified by a glob"
            );
        }
        let r = analyze(dir.path());
        for f in ["src/ui/view.ts", "src/ui/deep/panel.ts"] {
            assert!(
                r.coverage.unclassified.iter().any(|u| u == f),
                "{f} not unclassified: {:?}",
                r.coverage.unclassified
            );
        }
        let entry = entry_for(&r, "src/ui/**")
            .unwrap_or_else(|| panic!("no entry for src/ui/** in {:?}", r.config_errors));
        assert!(has_glob_hint(&entry, "src/ui/**", "src/ui"), "{entry}");
    }

    /// S13: The proposed prefix is the key cut at its first glob character
    #[test]
    fn s13_proposed_prefix_is_key_cut_at_first_glob_char() {
        let glob_keys = [
            "src/ui/**",
            "lib/x?y",
            "src/[ab]/c",
            "pkg/*.ts",
            "a/b/c/**/d",
        ];
        let plain_keys = ["src/adpaters", "lib/gone"];
        let mut layers: Vec<(&str, &str)> = vec![("src/domain", "domain")];
        for k in glob_keys.iter().chain(plain_keys.iter()) {
            layers.push((k, "adapters/primary"));
        }
        let dir = project(&[("src/domain/a.rs", "pub fn a() {}\n")], Some(&layers));
        let r = analyze(dir.path());
        for k in glob_keys {
            let entry = entry_for(&r, k)
                .unwrap_or_else(|| panic!("no entry for {k}: {:?}", r.config_errors));
            let p = expected_prefix(k);
            assert!(has_glob_hint(&entry, k, &p), "key {k}, prefix {p}: {entry}");
        }
        for k in plain_keys {
            let entry = entry_for(&r, k)
                .unwrap_or_else(|| panic!("no entry for {k}: {:?}", r.config_errors));
            assert!(has_no_glob_hint(&entry), "{k}: {entry}");
        }
    }

    /// S14: A project without analyze.layers grades as before
    #[test]
    fn s14_project_without_layers_grades_as_before() {
        let dir = project(
            &[
                (
                    "src/domain/a.rs",
                    "use crate::adapters::primary::cli;\npub fn a() {}\n",
                ),
                ("src/adapters/primary/cli.rs", "pub fn run() {}\n"),
            ],
            None,
        );
        let r = analyze(dir.path());
        assert!(layer_key_errors(&r).is_empty(), "{:?}", r.config_errors);
        assert!(
            !r.config_errors.iter().any(|e| e.contains("analyze.layers")),
            "{:?}",
            r.config_errors
        );
        let map = LayerMap::from_project(dir.path()).expect("no layers is an empty map");
        for f in ["src/domain/a.rs", "src/adapters/primary/cli.rs"] {
            assert_eq!(map.classify(f), classify_layer(f), "{f}");
        }
        // Identical to a run of the same project: nothing about layers is added.
        let again = analyze(dir.path());
        assert_same_findings(&r, &again);
        assert_eq!(r.health_score, again.health_score);
    }

    /// S15: The S26 fixture: the keys are still read as prefixes, and the gap is now reported
    #[test]
    fn s15_s26_fixture_reports_both_glob_keys() {
        let files: &[(&str, &str)] = &[
            ("src/domain/order.ts", "export const order = 1;\n"),
            (
                "src/ui/view.ts",
                "import { order } from \"../domain/order\";\nexport const view = order;\n",
            ),
        ];
        let with = project(
            files,
            Some(&[
                ("src/domain/**", "domain"),
                ("src/ui/**", "adapters/primary"),
            ]),
        );
        let without = project(files, None);
        let r = analyze(with.path());
        let base = analyze(without.path());

        assert!(
            r.coverage.unclassified.iter().any(|u| u == "src/ui/view.ts"),
            "src/ui/view.ts stays unclassified: {:?}",
            r.coverage
        );
        assert_eq!(r.coverage, base.coverage, "coverage unchanged");
        assert_eq!(layer_key_errors(&r).len(), 2, "{:?}", r.config_errors);
        let d = entry_for(&r, "src/domain/**").expect("entry for src/domain/**");
        assert!(has_glob_hint(&d, "src/domain/**", "src/domain"), "{d}");
        let u = entry_for(&r, "src/ui/**").expect("entry for src/ui/**");
        assert!(has_glob_hint(&u, "src/ui/**", "src/ui"), "{u}");
        // One file in two has a layer: the coverage ceiling (50) caps both
        // scores, so the errors' cost is hidden here (S02 shows it).
        assert!(r.health_score <= base.health_score, "{} > {}", r.health_score, base.health_score);
        assert_eq!(r.coverage.ceiling(), 50);
    }

    /// S16: An unmatched key does not change how other files are classified
    #[test]
    fn s16_unmatched_key_does_not_change_classification() {
        let p = project(S16_FILES, Some(S16_LAYERS));
        let mut extra = S16_LAYERS.to_vec();
        extra.push(("src/nothing", "infrastructure"));
        let q = project(S16_FILES, Some(&extra));

        let rp = analyze(p.path());
        let rq = analyze(q.path());
        assert_eq!(
            rp.violations.len(),
            1,
            "one boundary violation: {:?}",
            v(&rp.violations)
        );
        assert_same_findings(&rp, &rq);

        let mp = LayerMap::from_project(p.path()).unwrap();
        let mq = LayerMap::from_project(q.path()).unwrap();
        assert_eq!(mp.classify("src/domain/a.rs"), HexLayer::Domain);
        assert_eq!(
            mp.classify("src/adapters/primary/cli.rs"),
            HexLayer::AdaptersPrimary
        );
        for (f, _) in S16_FILES {
            assert_eq!(mp.classify(f), mq.classify(f), "{f}");
        }

        let added: Vec<&String> = rq
            .config_errors
            .iter()
            .filter(|e| !rp.config_errors.contains(e))
            .collect();
        assert_eq!(added.len(), 1, "{:?}", rq.config_errors);
        assert!(
            added[0].contains("analyze.layers[\"src/nothing\"]"),
            "{}",
            added[0]
        );
        assert_eq!(rq.config_errors.len(), rp.config_errors.len() + 1);
        assert_eq!(rq.health_score, rp.health_score.saturating_sub(10));
    }

    /// S17: A project whose keys all match gains no layer-key config errors
    #[test]
    fn s17_all_matching_keys_add_no_errors() {
        let p = project(S16_FILES, Some(S16_LAYERS));
        let r = analyze(p.path());
        assert!(layer_key_errors(&r).is_empty(), "{:?}", r.config_errors);

        // Another key that also matches a graded file changes nothing.
        let mut more = S16_LAYERS.to_vec();
        more.push(("src/domain/a.rs", "domain"));
        let q = project(S16_FILES, Some(&more));
        let rq = analyze(q.path());
        assert!(layer_key_errors(&rq).is_empty(), "{:?}", rq.config_errors);
        assert_eq!(rq.config_errors, r.config_errors);
        assert_eq!(rq.health_score, r.health_score);
    }

    /// S18: Unmatched keys change only config_errors and the score
    #[test]
    fn s18_unmatched_keys_change_only_config_errors_and_score() {
        let base_dir = project(S16_FILES, Some(S16_LAYERS));
        let base = analyze(base_dir.path());

        let small: Vec<&str> = vec!["src/nothing", "lib/ghost", "src/ui/**"];
        let many: Vec<String> = (0..12).map(|i| format!("gone/k{i}")).collect();
        let many_refs: Vec<&str> = many.iter().map(|s| s.as_str()).collect();

        for added in [vec!["src/nothing"], small, many_refs] {
            let mut layers = S16_LAYERS.to_vec();
            for k in &added {
                layers.push((k, "infrastructure"));
            }
            let dir = project(S16_FILES, Some(&layers));
            let r = analyze(dir.path());
            assert_same_findings(&base, &r);

            let mut expected_len = base.config_errors.len();
            for e in &base.config_errors {
                assert!(r.config_errors.contains(e), "lost {e}");
            }
            for k in &added {
                assert!(
                    entry_for(&r, k).is_some(),
                    "no entry for {k}: {:?}",
                    r.config_errors
                );
                expected_len += 1;
            }
            assert_eq!(r.config_errors.len(), expected_len, "{:?}", r.config_errors);
            let penalty = u8::try_from((added.len() * 10).min(255)).unwrap();
            assert_eq!(
                r.health_score,
                base.health_score.saturating_sub(penalty),
                "added {added:?}"
            );
        }
    }
}
