#![allow(unused, clippy::all)]
mod part_1 {
    #[allow(unused_imports)]
    use hexa_analysis::domain::{ExportDeclaration, ExportKind, Language};
    #[allow(unused_imports)]
    use hexa_analysis::ports::AstPort;
    #[allow(unused_imports)]
    use hexa_analysis::treesitter_adapter::TreeSitterAdapter;
    use std::path::Path;

    fn exports_of(file: &str, src: &str) -> Vec<ExportDeclaration> {
        TreeSitterAdapter::new()
            .extract_exports(Path::new(file), src, Language::Go)
            .expect("extract_exports should succeed on valid Go")
    }

    fn named<'a>(exports: &'a [ExportDeclaration], name: &str) -> Vec<&'a ExportDeclaration> {
        exports.iter().filter(|e| e.name == name).collect()
    }

    fn describe(exports: &[ExportDeclaration]) -> String {
        exports
            .iter()
            .map(|e| format!("{}@{}:{:?}", e.name, e.line, e.kind))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// S01: A function whose name starts with a non-ASCII capital is an export
    #[test]
    fn s01_function_with_non_ascii_capital_is_export() {
        let src = "package p\n\nfunc Äpfel() {}\n";
        let ex = exports_of("p.go", src);
        assert!(
            ex.iter().any(|e| e.file == "p.go"
                && e.name == "Äpfel"
                && e.line == 3
                && e.kind == ExportKind::Function),
            "expected Äpfel as Function at p.go:3, got [{}]",
            describe(&ex)
        );
    }

    /// S02: The Unicode rule covers types, methods, consts and vars
    #[test]
    fn s02_unicode_rule_covers_types_methods_consts_vars() {
        let src = "package p\n\ntype Ωmega struct{}\nfunc (o Ωmega) Ärger() {}\nconst Ñame = 1\nvar Øre = 2\n";
        let ex = exports_of("p.go", src);
        let expect = [
            ("Ωmega", ExportKind::Type, 3usize),
            ("Ärger", ExportKind::Method, 4),
            ("Ñame", ExportKind::Value, 5),
            ("Øre", ExportKind::Value, 6),
        ];
        for (name, kind, line) in expect {
            assert!(
                ex.iter()
                    .any(|e| e.name == name && e.kind == kind && e.line == line),
                "expected {name} as {kind:?} at line {line}, got [{}]",
                describe(&ex)
            );
        }
    }

    /// S06: Exported exactly when the first character is a Unicode upper-case letter
    #[test]
    fn s06_exported_iff_first_char_unicode_uppercase() {
        let firsts = [
            'A', 'Z', 'a', 'z', 'Ä', 'ä', 'Ω', 'ω', 'Ñ', 'ñ', 'Ø', 'ø', 'Σ', 'σ', 'Ж', 'ж', 'É',
            'é', 'Ł', 'ł', 'Ç', 'ç', 'Δ', 'δ', '_', 'Ü', 'ü', 'Я', 'я',
        ];
        let suffixes = ["", "pfel", "x1", "_Y", "Ärger"];
        for first in firsts {
            for suffix in suffixes {
                let name = format!("{first}{suffix}");
                if name == "_" {
                    continue;
                }
                let src = format!("package p\n\nfunc {name}() {{}}\n");
                let ex = exports_of("p.go", &src);
                let present = ex.iter().any(|e| e.name == name);
                let should = first.is_uppercase();
                assert_eq!(
                    present,
                    should,
                    "name {name:?}: exported should be {should}, got [{}]",
                    describe(&ex)
                );
            }
        }
    }

    /// S07: An ungrouped var spec with two names exports both
    #[test]
    fn s07_ungrouped_var_spec_exports_both_names() {
        let src = "package p\n\nvar A, B = 1, 2\n";
        let ex = exports_of("p.go", src);
        for name in ["A", "B"] {
            assert!(
                ex.iter().any(|e| e.name == name
                    && e.file == "p.go"
                    && e.line == 3
                    && e.kind == ExportKind::Value),
                "expected {name} as Value at p.go:3, got [{}]",
                describe(&ex)
            );
        }
    }

    /// S08: A var spec inside a group with several names exports each at the spec's line
    #[test]
    fn s08_grouped_multi_name_var_spec_exports_each() {
        let src = "package p\n\nvar (\nC, D = 2, 3\n)\n";
        let ex = exports_of("p.go", src);
        for name in ["C", "D"] {
            assert!(
                ex.iter()
                    .any(|e| e.name == name && e.line == 4 && e.kind == ExportKind::Value),
                "expected {name} as Value at line 4, got [{}]",
                describe(&ex)
            );
        }
    }

    /// S09: A typed multi-name var spec with no initialiser exports every name
    #[test]
    fn s09_typed_multi_name_var_without_initialiser() {
        let src = "package p\n\nvar X, Y, Z int\n";
        let ex = exports_of("p.go", src);
        for name in ["X", "Y", "Z"] {
            assert!(
                ex.iter()
                    .any(|e| e.name == name && e.line == 3 && e.kind == ExportKind::Value),
                "expected {name} as Value at line 3, got [{}]",
                describe(&ex)
            );
        }
    }

    /// S10: A second name of a var spec can be a dead export
    #[test]
    fn s10_second_name_of_var_spec_can_be_dead() {
        let adapter = TreeSitterAdapter::new();
        let decl_src = "package p\n\nvar A, B = 1, 2\n";
        let user_src = "package p\n\nfunc use() int { return A }\n";

        let ex = adapter
            .extract_exports(Path::new("a.go"), decl_src, Language::Go)
            .expect("exports");
        let refs = adapter
            .extract_references(Path::new("b.go"), user_src, Language::Go)
            .expect("references");

        // B must be an export at the spec's line, so the dead-export finder can see it.
        let b = named(&ex, "B");
        assert_eq!(
            b.len(),
            1,
            "expected B exported once, got [{}]",
            describe(&ex)
        );
        assert_eq!(b[0].line, 3);
        let a = named(&ex, "A");
        assert_eq!(
            a.len(),
            1,
            "expected A exported once, got [{}]",
            describe(&ex)
        );

        // Dead-export rule: an export no other file names is dead.
        let names_elsewhere = |n: &str| refs.get(n).copied().unwrap_or(0) > 0;
        let dead: Vec<(&str, usize)> = ex
            .iter()
            .filter(|e| !names_elsewhere(&e.name))
            .map(|e| (e.name.as_str(), e.line))
            .collect();
        assert!(
            dead.contains(&("B", 3)),
            "B should be dead at line 3: {dead:?}"
        );
        assert!(
            !dead.iter().any(|(n, _)| *n == "A"),
            "A should not be dead: {dead:?}"
        );
    }

    /// S11: A var spec is not reduced to its first name
    #[test]
    fn s11_var_spec_not_reduced_to_first_name() {
        let src = "package p\n\nvar A, B = 1, 2\n";
        let ex = exports_of("p.go", src);
        assert_eq!(named(&ex, "B").len(), 1, "got [{}]", describe(&ex));
        assert_eq!(named(&ex, "A").len(), 1, "got [{}]", describe(&ex));
    }

    /// S13: Names of a var spec are not given another kind or another line
    #[test]
    fn s13_var_spec_names_have_value_kind_and_spec_line() {
        let src = "package p\n\nvar A, B = 1, 2\n";
        let ex = exports_of("p.go", src);
        let relevant: Vec<_> = ex
            .iter()
            .filter(|e| e.name == "A" || e.name == "B")
            .collect();
        assert!(
            relevant.iter().any(|e| e.name == "A") && relevant.iter().any(|e| e.name == "B"),
            "both A and B must be present, got [{}]",
            describe(&ex)
        );
        for e in relevant {
            assert_eq!(
                e.kind,
                ExportKind::Value,
                "{} has kind {:?}",
                e.name,
                e.kind
            );
            assert_eq!(e.line, 3, "{} has line {}", e.name, e.line);
        }
    }

    /// S14: Every exported name of any var spec is its own Value export at the spec's line
    #[test]
    fn s14_every_exported_var_spec_name_is_own_value_export() {
        let name_sets: Vec<Vec<&str>> = vec![
            vec!["A"],
            vec!["a"],
            vec!["A", "B"],
            vec!["a", "B"],
            vec!["Äpfel", "b", "Ωmega"],
            vec!["x", "y"],
            vec!["P", "Q", "R", "S"],
            vec!["ñu", "Ñu", "_Z"],
            vec!["Øre", "øre"],
        ];
        for names in &name_sets {
            let k = names.len();
            let list = names.join(", ");
            let values = vec!["1"; k].join(", ");
            let forms = [
                format!("{list} = {values}"),
                format!("{list} int"),
                format!("{list} int = {values}"),
            ];
            for form in &forms {
                for grouped in [false, true] {
                    let (src, line) = if grouped {
                        (format!("package p\n\nvar (\n\t{form}\n)\n"), 4usize)
                    } else {
                        (format!("package p\n\nvar {form}\n"), 3usize)
                    };
                    let ex = exports_of("p.go", &src);
                    for n in names {
                        let found = named(&ex, n);
                        let exported = n.chars().next().unwrap().is_uppercase();
                        if exported {
                            assert_eq!(
                                found.len(),
                                1,
                                "{n} should be exported once in {src:?}, got [{}]",
                                describe(&ex)
                            );
                            assert_eq!(found[0].line, line, "{n} line in {src:?}");
                            assert_eq!(found[0].kind, ExportKind::Value, "{n} kind in {src:?}");
                        } else {
                            assert!(
                                found.is_empty(),
                                "{n} should not be exported in {src:?}, got [{}]",
                                describe(&ex)
                            );
                        }
                    }
                }
            }
        }
    }
}

mod part_2 {
    use hexa_analysis::domain::{ExportDeclaration, Language};
    use hexa_analysis::ports::AstPort;
    use hexa_analysis::treesitter_adapter::TreeSitterAdapter;
    use std::path::Path;

    fn go_exports(source: &str) -> Vec<ExportDeclaration> {
        TreeSitterAdapter::new()
            .extract_exports(Path::new("pkg/values.go"), source, Language::Go)
            .expect("extract_exports on a valid Go file")
    }

    fn names(exports: &[ExportDeclaration]) -> Vec<String> {
        exports.iter().map(|e| e.name.clone()).collect()
    }
}
