#![allow(unused, clippy::all)]
mod part_1 {
    #![allow(clippy::all)]

    use hexa_analysis::domain::{ExportDeclaration, ExportKind, Language, TypeBody};
    use hexa_analysis::ports::AstPort;
    use hexa_analysis::treesitter_adapter::TreeSitterAdapter;
    use std::collections::BTreeSet;
    use std::path::Path;

    fn go_exports(path: &str, source: &str) -> Vec<ExportDeclaration> {
        TreeSitterAdapter::new()
            .extract_exports(Path::new(path), source, Language::Go)
            .expect("extract_exports must succeed on valid Go")
    }

    fn names_of(exports: &[ExportDeclaration]) -> BTreeSet<String> {
        exports.iter().map(|e| e.name.clone()).collect()
    }

    fn find<'a>(exports: &'a [ExportDeclaration], name: &str) -> &'a ExportDeclaration {
        exports
            .iter()
            .find(|e| e.name == name)
            .unwrap_or_else(|| panic!("no export named {name:?} in {exports:?}"))
    }

    fn go_exported(name: &str) -> bool {
        name.chars().next().map_or(false, |c| c.is_uppercase())
    }

    /// S01: A multi-name const spec exports every name it declares, at the spec's line, as a value
    #[test]
    fn s01_multi_name_const_spec_exports_every_name() {
        let src = "package p\n\nconst A, B = 1, 2\n";
        let exports = go_exports("p/consts.go", src);
        assert_eq!(
            exports.len(),
            2,
            "expected exactly A and B, got {exports:?}"
        );
        for name in ["A", "B"] {
            let e = find(&exports, name);
            assert_eq!(e.line, 3, "{name} must be at the spec's line");
            assert_eq!(e.kind, ExportKind::Value, "{name} must be a Value");
        }
    }

    /// S02: A grouped const spec with several names exports every name
    #[test]
    fn s02_grouped_const_spec_exports_every_name() {
        let src = "const (\n\tX, Y = 1, 2\n\tZ = 3\n)\n";
        let exports = go_exports("p/consts.go", src);
        let expected: BTreeSet<String> = ["X", "Y", "Z"].iter().map(|s| s.to_string()).collect();
        assert_eq!(names_of(&exports), expected, "got {exports:?}");
        assert_eq!(exports.len(), 3, "one export per name, got {exports:?}");
        assert_eq!(find(&exports, "X").line, 2);
        assert_eq!(find(&exports, "Y").line, 2);
        assert_eq!(find(&exports, "Z").line, 3);
        for e in &exports {
            assert_eq!(e.kind, ExportKind::Value, "{e:?}");
        }
    }

    /// S03: The commas between names are not names
    #[test]
    fn s03_commas_are_not_names() {
        let src = "package p\n\nconst A, B, C = 1, 2, 3\n";
        let exports = go_exports("p/consts.go", src);
        let names: Vec<String> = exports.iter().map(|e| e.name.clone()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(
            sorted,
            vec!["A".to_string(), "B".to_string(), "C".to_string()],
            "got {names:?}"
        );
        for n in &names {
            assert!(!n.is_empty(), "empty export name");
            assert!(!n.contains(','), "comma in export name {n:?}");
        }
    }

    /// S05: An unused second const name is a dead export, like the same name written alone
    #[test]
    fn s05_unused_second_const_name_is_dead() {
        let adapter = TreeSitterAdapter::new();
        let decl = "package p\n\nconst A, B = 1, 2\n";
        let user = "package p\n\nfunc Use() int { return A }\n";

        let exports = adapter
            .extract_exports(Path::new("p/consts.go"), decl, Language::Go)
            .expect("exports");
        let user_refs = adapter
            .extract_references(Path::new("p/use.go"), user, Language::Go)
            .expect("references");

        // The dead-export rule: an export no other file names is dead.
        let dead: Vec<&ExportDeclaration> = exports
            .iter()
            .filter(|e| !e.hexa_public && user_refs.get(&e.name).copied().unwrap_or(0) == 0)
            .collect();

        let b = dead
            .iter()
            .find(|e| e.name == "B")
            .unwrap_or_else(|| panic!("B must be a dead export; exports {exports:?}"));
        assert_eq!(b.line, 3, "B must be at the spec's line");
        assert!(exports.iter().any(|e| e.name == "A"), "A must be an export");
        assert!(
            !dead.iter().any(|e| e.name == "A"),
            "A is used and must not be dead"
        );
    }

    /// S06: Every exported name of every const spec is an export at the spec's line
    #[test]
    fn s06_every_exported_const_name_is_an_export() {
        // Each case: list of specs (names on one line), grouped or not.
        let cases: Vec<(bool, Vec<Vec<&str>>)> = vec![
            (false, vec![vec!["A"]]),
            (false, vec![vec!["A", "B"]]),
            (false, vec![vec!["a", "B", "c", "D"]]),
            (false, vec![vec!["Ärger", "ärger"], vec!["Q"]]),
            (
                true,
                vec![vec!["X", "Y"], vec!["z"], vec!["Δelta", "W", "v"]],
            ),
            (true, vec![vec!["One"]]),
            (true, vec![vec!["p", "q"], vec!["R", "S", "T"]]),
        ];

        for (grouped, specs) in &cases {
            let render = |kw: &str| -> (String, Vec<(String, usize)>) {
                let mut src = String::from("package p\n");
                let mut line = 1usize;
                let mut expected = Vec::new();
                if *grouped {
                    src.push_str(&format!("{kw} (\n"));
                    line += 1;
                }
                for spec in specs {
                    let values: Vec<String> = (1..=spec.len()).map(|i| i.to_string()).collect();
                    line += 1;
                    if *grouped {
                        src.push_str(&format!("\t{} = {}\n", spec.join(", "), values.join(", ")));
                    } else {
                        src.push_str(&format!(
                            "{kw} {} = {}\n",
                            spec.join(", "),
                            values.join(", ")
                        ));
                    }
                    for n in spec {
                        if go_exported(n) {
                            expected.push((n.to_string(), line));
                        }
                    }
                }
                if *grouped {
                    src.push_str(")\n");
                }
                (src, expected)
            };

            let (const_src, expected) = render("const");
            let exports = go_exports("p/c.go", &const_src);
            assert_eq!(
                exports.len(),
                expected.len(),
                "one export per exported identifier in\n{const_src}\ngot {exports:?}"
            );
            for (name, line) in &expected {
                let e = find(&exports, name);
                assert_eq!(e.line, *line, "{name} line in\n{const_src}");
                assert_eq!(e.kind, ExportKind::Value, "{name} kind in\n{const_src}");
            }
            let expected_names: BTreeSet<String> =
                expected.iter().map(|(n, _)| n.clone()).collect();
            assert_eq!(names_of(&exports), expected_names, "in\n{const_src}");

            let (var_src, _) = render("var");
            let var_exports = go_exports("p/v.go", &var_src);
            assert_eq!(
            names_of(&var_exports),
            names_of(&exports),
            "const and var specs with the same names must export the same names\n{const_src}\n{var_src}"
        );
        }
    }

    fn struct_fields(source: &str, ty: &str) -> Vec<String> {
        let facts = TreeSitterAdapter::new()
            .extract_api(Path::new("p/report.go"), source, Language::Go)
            .expect("extract_api");
        let decl = facts
            .types
            .iter()
            .find(|t| t.name == ty)
            .unwrap_or_else(|| panic!("no TypeDecl {ty} in {:?}", facts.types));
        match &decl.body {
            TypeBody::Struct(fields) => fields.iter().map(|f| f.wire_name.clone()).collect(),
            other => panic!("{ty} must have a Struct body, got {other:?}"),
        }
    }

    /// S07: A struct field whose first letter is a non-ASCII upper-case letter is in the API contract
    #[test]
    fn s07_non_ascii_upper_field_is_in_contract() {
        let src = "package p\n\ntype Report struct {\n\tÄrger string\n\tCount int\n}\n";
        let fields = struct_fields(src, "Report");
        assert!(
            fields.iter().any(|f| f == "Ärger"),
            "Ärger missing: {fields:?}"
        );
        assert!(
            fields.iter().any(|f| f == "Count"),
            "Count missing: {fields:?}"
        );
    }

    /// S09: The contract reader and the analyzer agree on which names are exported
    #[test]
    fn s09_contract_reader_and_analyzer_agree() {
        let names = [
            "Ärger", "ärger", "Count", "count", "Δelta", "δelta", "Éclair", "éclair", "日本", "Zz",
            "_X",
        ];
        for name in names {
            let src =
                format!("package p\n\ntype Report struct {{\n\t{name} string\n\tKeep int\n}}\n");
            let fields = struct_fields(&src, "Report");
            let kept = fields.iter().any(|f| f == name);
            let rule = go_exported(name);
            assert_eq!(
                kept, rule,
                "field {name:?}: kept={kept}, Go rule={rule}, fields {fields:?}"
            );

            let var_src = format!("package p\n\nvar {name} = 1\n");
            let analyzer = go_exports("p/v.go", &var_src)
                .iter()
                .any(|e| e.name == name);
            assert_eq!(
                kept, analyzer,
                "contract reader and analyzer disagree on {name:?}"
            );
        }
    }

    fn graph_entities(source: &str) -> Vec<(String, String, usize)> {
        use hexa_graph::extract::code::{extract_file, Language as GLang};
        let fx = extract_file(source, GLang::Go);
        fx.entities
            .iter()
            .map(|e| (e.name.to_string(), format!("{:?}", e.kind), e.line as usize))
            .collect()
    }

    fn assert_entity(ents: &[(String, String, usize)], name: &str, line: usize) {
        let e = ents
            .iter()
            .find(|(n, _, _)| n == name)
            .unwrap_or_else(|| panic!("no entity {name} in {ents:?}"));
        assert_eq!(e.1, "Const", "{name} must be of kind Const: {ents:?}");
        assert_eq!(e.2, line, "{name} line: {ents:?}");
    }

    /// S10: Grouped vars are code-graph entities
    #[test]
    fn s10_grouped_vars_are_graph_entities() {
        let src = "var (\n\tAlpha = 1\n\tBeta = 2\n)\n";
        let ents = graph_entities(src);
        assert_entity(&ents, "Alpha", 2);
        assert_entity(&ents, "Beta", 3);
    }

    /// S11: Every name of a multi-name spec is a code-graph entity
    #[test]
    fn s11_every_name_of_multi_name_spec_is_graph_entity() {
        let src = "package p\n\n// one\n// two\nconst P, Q = 1, 2\nvar M, N = 3, 4\n";
        let ents = graph_entities(src);
        assert_entity(&ents, "P", 5);
        assert_entity(&ents, "Q", 5);
        assert_entity(&ents, "M", 6);
        assert_entity(&ents, "N", 6);
    }

    /// S12: Graph consumers report the users of a grouped var
    #[test]
    fn s12_graph_consumers_report_grouped_var_users() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        std::fs::write(root.join("go.mod"), "module example.com/p\n\ngo 1.21\n").unwrap();
        std::fs::write(
            root.join("vars.go"),
            "package p\n\nvar (\n\tAlpha = 1\n\tBeta = 2\n)\n",
        )
        .unwrap();
        std::fs::write(
            root.join("user.go"),
            "package p\n\nfunc UseBeta() int { return Beta }\n",
        )
        .unwrap();

        let bin = env!("CARGO_BIN_EXE_hexa");
        let _ = std::process::Command::new(bin)
            .args(["graph", "build"])
            .current_dir(root)
            .env("NO_COLOR", "1")
            .output();
        let out = std::process::Command::new(bin)
            .args(["graph", "consumers", "Beta"])
            .current_dir(root)
            .env("NO_COLOR", "1")
            .output()
            .expect("run hexa graph consumers");
        let text = format!(
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            text.contains("user.go"),
            "user.go must be a consumer of Beta:\n{text}"
        );
        assert!(
            !text.to_lowercase().contains("safe to remove"),
            "Beta must not be reported safe to remove:\n{text}"
        );
    }
}

mod part_2 {
    #![allow(clippy::all)]

    use std::collections::BTreeSet;
    use std::path::Path;

    use hexa_analysis::ports::AstPort;
    use hexa_analysis::treesitter_adapter::TreeSitterAdapter;
    use hexa_graph::extract::code::{extract_file, Language as GraphLanguage};

    /// The graph's const/var entities of a Go source, as (name, line).
    fn graph_consts(source: &str) -> BTreeSet<(String, usize)> {
        let extract = extract_file(source, GraphLanguage::Go);
        extract
            .entities
            .iter()
            .filter(|e| format!("{:?}", e.kind) == "Const")
            .map(|e| (e.name.to_string(), e.line as usize))
            .collect()
    }

    /// Every entity name the graph extracts from a Go source, of any kind.
    fn graph_all_names(source: &str) -> Vec<String> {
        let extract = extract_file(source, GraphLanguage::Go);
        extract
            .entities
            .iter()
            .map(|e| e.name.to_string())
            .collect()
    }

    /// The analyzer's Value exports of a Go source, as (name, line).
    fn analyzer_values(source: &str) -> BTreeSet<(String, usize)> {
        let adapter = TreeSitterAdapter::new();
        let exports = adapter
            .extract_exports(
                Path::new("pkg/values.go"),
                source,
                hexa_analysis::domain::Language::Go,
            )
            .expect("the analyzer reads the Go file");
        exports
            .iter()
            .filter(|e| e.kind == hexa_analysis::domain::ExportKind::Value)
            .map(|e| (e.name.clone(), e.line))
            .collect()
    }

    fn is_exported(name: &str) -> bool {
        name.chars().next().map_or(false, |c| c.is_uppercase())
    }

    /// S13: The graph does not invent entities from separators or skip any declared name
    #[test]
    fn s13_graph_reads_every_name_of_a_grouped_var_spec_and_nothing_else() {
        let source = "package p\n\nvar (\n\tA, B = 1, 2\n)\n";

        let consts: BTreeSet<String> = graph_consts(source).into_iter().map(|(n, _)| n).collect();
        let expected: BTreeSet<String> = ["A", "B"].iter().map(|s| s.to_string()).collect();
        assert_eq!(
            consts, expected,
            "the const/var entities must be exactly A and B"
        );

        for name in graph_all_names(source) {
            assert!(
                !name.trim().is_empty(),
                "the graph made an entity with an empty name"
            );
            assert!(
                !name.contains(','),
                "the graph made an entity from a separator: {name:?}"
            );
            assert_ne!(
                name, "var_spec_list",
                "the graph made an entity from a node kind"
            );
            assert_ne!(
                name, "var_spec",
                "the graph made an entity from a node kind"
            );
        }

        let lines: BTreeSet<usize> = graph_consts(source).into_iter().map(|(_, l)| l).collect();
        assert_eq!(
            lines,
            [4usize].into_iter().collect(),
            "both names sit at the spec's line"
        );
    }

    /// Builds one Go file of const and var declarations from a seed.
    fn generate_go(seed: u64) -> String {
        let mut state = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let mut next = |m: u64| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) % m
        };
        let pool = [
            "Alpha", "Beta", "gamma", "Delta", "ärger", "Ärger", "Zeta", "eta", "Theta", "Iota",
            "kappa", "Lambda", "Mu", "nu", "Xi", "Omikron", "Pi", "rho", "Sigma", "Tau",
        ];
        let mut counter = 0usize;
        let mut fresh = |pick: u64| {
            counter += 1;
            format!("{}{}", pool[pick as usize], counter)
        };

        let mut out = String::from("package p\n\n");
        let decls = 2 + next(4);
        for _ in 0..decls {
            let keyword = if next(2) == 0 { "const" } else { "var" };
            let grouped = next(2) == 0;
            let specs = if grouped { 1 + next(3) } else { 1 };
            if grouped {
                out.push_str(&format!("{keyword} (\n"));
            }
            for _ in 0..specs {
                let count = 1 + next(3) as usize;
                let names: Vec<String> =
                    (0..count).map(|_| fresh(next(pool.len() as u64))).collect();
                let values: Vec<String> = (0..count).map(|i| format!("{}", i + 1)).collect();
                let spec = format!("{} = {}", names.join(", "), values.join(", "));
                if grouped {
                    out.push_str(&format!("\t{spec}\n"));
                } else {
                    out.push_str(&format!("{keyword} {spec}\n"));
                }
            }
            if grouped {
                out.push_str(")\n");
            }
            out.push('\n');
        }
        out
    }

    /// S14: The code graph and the analyzer agree on what a Go const or var export is
    #[test]
    fn s14_graph_and_analyzer_agree_on_go_const_and_var_exports() {
        let mut sources: Vec<String> = vec![
            "package p\n\nconst A, B = 1, 2\n".to_string(),
            "package p\n\nvar C, D = 1, 2\n".to_string(),
            "package p\n\nconst (\n\tE, F = 1, 2\n\tg, H = 3, 4\n)\n".to_string(),
            "package p\n\nvar (\n\tI, j = 1, 2\n\tK = 3\n)\n".to_string(),
            "package p\n\nconst (\n\tL = iota\n\tM\n)\n\nvar Ärger, N int\n".to_string(),
        ];
        for seed in 0..40u64 {
            sources.push(generate_go(seed));
        }

        let mut multi_name_seen = false;
        for source in &sources {
            let analyzer: BTreeSet<(String, usize)> = analyzer_values(source)
                .into_iter()
                .filter(|(n, _)| is_exported(n))
                .collect();
            let graph: BTreeSet<(String, usize)> = graph_consts(source)
                .into_iter()
                .filter(|(n, _)| is_exported(n))
                .collect();
            assert_eq!(
                analyzer, graph,
                "the analyzer's Value exports and the graph's Const entities differ for:\n{source}"
            );
            let mut by_line = std::collections::HashMap::new();
            for (_, l) in &analyzer {
                *by_line.entry(*l).or_insert(0usize) += 1;
            }
            if by_line.values().any(|c| *c > 1) {
                multi_name_seen = true;
            }
        }
        assert!(
            multi_name_seen,
            "at least one spec must export several names on one line in both readers"
        );
    }
}
