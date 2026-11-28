#![allow(unused, clippy::all)]
mod part_1 {
    use hexa_analysis::domain::{ExportDeclaration, Language};
    use hexa_analysis::ports::AstPort;
    use hexa_analysis::treesitter_adapter::TreeSitterAdapter;
    use std::collections::BTreeSet;
    use std::path::Path;

    fn exports(source: &str) -> Vec<ExportDeclaration> {
        TreeSitterAdapter::new()
            .extract_exports(Path::new("lib.py"), source, Language::Python)
            .expect("extract_exports should succeed on valid Python")
    }

    fn public_of(all: &[ExportDeclaration], name: &str, source: &str) -> bool {
        let found: Vec<&ExportDeclaration> = all.iter().filter(|e| e.name == name).collect();
        assert_eq!(
            found.len(),
            1,
            "expected exactly one export named `{name}` in:\n{source}\ngot: {:?}",
            all.iter().map(|e| e.name.as_str()).collect::<Vec<_>>()
        );
        found[0].hexa_public
    }

    /// S01: A tagged top-level def is public
    #[test]
    fn s01_tagged_top_level_def_is_public() {
        let src = "# @hexa:public\ndef connect():\n    pass\n";
        let all = exports(src);
        assert!(
            public_of(&all, "connect", src),
            "`connect` should be hexa_public"
        );
    }

    /// S02: The tag may be on any line of a multi-line comment run
    #[test]
    fn s02_tag_on_any_line_of_comment_run() {
        let src = "# Opens a session.\n# @hexa:public\n# Thread-safe.\nclass Client:\n    pass\n";
        let all = exports(src);
        assert!(
            public_of(&all, "Client", src),
            "`Client` should be hexa_public"
        );
    }

    /// S03: A tagged top-level assignment is public
    #[test]
    fn s03_tagged_assignment_is_public() {
        let src = "# @hexa:public\nDEFAULT_TIMEOUT = 30\n";
        let all = exports(src);
        assert!(
            public_of(&all, "DEFAULT_TIMEOUT", src),
            "`DEFAULT_TIMEOUT` should be hexa_public"
        );
    }

    /// S04: For a decorated def, the comment run above the first decorator counts
    #[test]
    fn s04_decorated_def_comment_above_first_decorator() {
        let src = "# @hexa:public\n@cache\n@trace\ndef lookup(key):\n    pass\n";
        let all = exports(src);
        assert!(
            public_of(&all, "lookup", src),
            "`lookup` should be hexa_public"
        );
    }

    /// S05: For a decorated class, the comment run above the first decorator counts
    #[test]
    fn s05_decorated_class_comment_above_first_decorator() {
        let src = "# @hexa:public\n@dataclass\nclass Point:\n    x: int\n";
        let all = exports(src);
        assert!(
            public_of(&all, "Point", src),
            "`Point` should be hexa_public"
        );
    }

    /// S08: A trailing comment belongs to the code line it ends, not to the next definition
    #[test]
    fn s08_trailing_comment_belongs_to_its_line() {
        let src = "VERSION = \"1.0\"  # @hexa:public\ndef connect():\n    pass\n";
        let all = exports(src);
        assert!(
            public_of(&all, "VERSION", src),
            "`VERSION` should be hexa_public"
        );
        assert!(
            !public_of(&all, "connect", src),
            "`connect` must not take the trailing tag of the line above"
        );
    }

    /// S10: A tag above one definition does not carry over to the next one
    #[test]
    fn s10_tag_does_not_carry_over() {
        let src = "# @hexa:public\ndef a():\n    pass\n\ndef b():\n    pass\n";
        let all = exports(src);
        assert!(public_of(&all, "a", src), "`a` should be hexa_public");
        assert!(!public_of(&all, "b", src), "`b` must not be hexa_public");
    }

    /// S12: The tag must be on a comment line of the run, not on a line further up
    #[test]
    fn s12_tag_must_be_in_directly_above_run() {
        let src = "# @hexa:public\nx = 1\n# helper\ndef connect():\n    pass\n";
        let all = exports(src);
        assert!(public_of(&all, "x", src), "`x` should be hexa_public");
        assert!(
            !public_of(&all, "connect", src),
            "`connect` must not be hexa_public"
        );
    }

    #[derive(Clone, Copy, Debug)]
    enum Pre {
        TagComment,
        PlainComment,
        Blank,
        Filler { tagged: bool },
    }

    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 33
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    /// S13: An export is public exactly when its directly-above comment run carries the tag
    #[test]
    fn s13_public_iff_directly_above_run_carries_tag() {
        let mut rng = Lcg(0x5eed_2611_2216_44);
        let mut any_public = false;
        let mut any_private = false;
        for case in 0..150 {
            let mut src = String::new();
            let mut expected: Vec<(String, bool)> = Vec::new();
            let mut filler = 0usize;
            let items = 1 + rng.below(6) as usize;
            for i in 0..items {
                let pre_len = rng.below(5) as usize;
                let mut pre = Vec::new();
                for _ in 0..pre_len {
                    pre.push(match rng.below(5) {
                        0 => Pre::TagComment,
                        1 => Pre::PlainComment,
                        2 => Pre::Blank,
                        3 => Pre::Filler { tagged: true },
                        _ => Pre::Filler { tagged: false },
                    });
                }
                for p in &pre {
                    match p {
                        Pre::TagComment => src.push_str("# note @hexa:public\n"),
                        Pre::PlainComment => src.push_str("# just a comment\n"),
                        Pre::Blank => src.push('\n'),
                        Pre::Filler { tagged } => {
                            filler += 1;
                            if *tagged {
                                src.push_str(&format!("_p{filler} = 0  # @hexa:public\n"));
                            } else {
                                src.push_str(&format!("_p{filler} = 0  # trailing\n"));
                            }
                        }
                    }
                }
                // A run of whole-line comments directly above; stops at a blank or code line.
                let mut public = false;
                for p in pre.iter().rev() {
                    match p {
                        Pre::TagComment => public = true,
                        Pre::PlainComment => {}
                        _ => break,
                    }
                }
                let kind = rng.below(3);
                let name = match kind {
                    0 => format!("f{i}"),
                    1 => format!("C{i}"),
                    _ => format!("V{i}"),
                };
                if kind != 2 {
                    let decos = rng.below(3);
                    for d in 0..decos {
                        src.push_str(&format!("@deco{d}\n"));
                    }
                }
                match kind {
                    0 => src.push_str(&format!("def {name}():\n    pass\n")),
                    1 => src.push_str(&format!("class {name}:\n    pass\n")),
                    _ => src.push_str(&format!("{name} = {i}\n")),
                }
                if public {
                    any_public = true;
                } else {
                    any_private = true;
                }
                expected.push((name, public));
            }
            let all = exports(&src);
            for (name, public) in &expected {
                assert_eq!(
                    public_of(&all, name, &src),
                    *public,
                    "case {case}: `{name}` hexa_public should be {public} in:\n{src}"
                );
            }
            for e in &all {
                assert!(
                    expected.iter().any(|(n, _)| *n == e.name),
                    "case {case}: unexpected export `{}` in:\n{src}",
                    e.name
                );
            }
        }
        assert!(
            any_public && any_private,
            "generator should cover both outcomes"
        );
    }

    /// S14: A tag above __all__ marks every listed name
    #[test]
    fn s14_tag_above_all_marks_every_listed_name() {
        let src = "# @hexa:public\n__all__ = [\"connect\", \"Client\"]\n\ndef connect():\n    pass\n\nclass Client:\n    pass\n\ndef extra():\n    pass\n";
        let all = exports(src);
        let names: BTreeSet<&str> = all.iter().map(|e| e.name.as_str()).collect();
        let want: BTreeSet<&str> = ["connect", "Client"].into_iter().collect();
        assert_eq!(
            names, want,
            "exports should be exactly those listed in __all__"
        );
        assert_eq!(all.len(), 2, "each listed name exported once: {all:?}");
        assert!(
            public_of(&all, "connect", src),
            "`connect` should be hexa_public"
        );
        assert!(
            public_of(&all, "Client", src),
            "`Client` should be hexa_public"
        );
    }
}

mod part_2 {
    #![allow(clippy::all)]

    use std::path::Path;

    use hexa_analysis::dead_export_finder::{find_dead_exports, FileData};
    use hexa_analysis::domain::{ExportDeclaration, Language};
    use hexa_analysis::ports::AstPort;
    use hexa_analysis::treesitter_adapter::TreeSitterAdapter;

    fn python_exports(path: &str, source: &str) -> Vec<ExportDeclaration> {
        let adapter = TreeSitterAdapter::new();
        adapter
            .extract_exports(Path::new(path), source, Language::Python)
            .expect("extract_exports should succeed on valid Python")
    }

    fn file_data(path: &str, source: &str) -> FileData {
        let adapter = TreeSitterAdapter::new();
        let p = Path::new(path);
        FileData {
            path: path.to_string(),
            imports: adapter
                .extract_imports(p, source, Language::Python)
                .expect("extract_imports"),
            exports: adapter
                .extract_exports(p, source, Language::Python)
                .expect("extract_exports"),
            references: adapter
                .extract_references(p, source, Language::Python)
                .expect("extract_references"),
            members: adapter
                .extract_members(p, source, Language::Python)
                .expect("extract_members"),
        }
    }

    fn find<'a>(exports: &'a [ExportDeclaration], name: &str) -> &'a ExportDeclaration {
        exports
            .iter()
            .find(|e| e.name == name)
            .unwrap_or_else(|| panic!("no export named `{name}` in {exports:?}"))
    }

    /// S15: Tagged __all__ names are never reported as dead exports
    #[test]
    fn s15_tagged_all_names_are_never_reported_as_dead_exports() {
        let source = "# @hexa:public\n__all__ = [\"connect\"]\n\n\ndef connect():\n    return 1\n";
        let exports = python_exports("pkg/client.py", source);
        let connect = find(&exports, "connect");
        assert!(
            connect.hexa_public,
            "`connect` is listed by a tagged `__all__` and must be hexa_public: {exports:?}"
        );

        let files = vec![file_data("pkg/client.py", source)];
        let dead = find_dead_exports(&files, &[]);
        assert!(
            dead.iter().all(|d| d.export_name != "connect"),
            "`connect` is public via a tagged `__all__` and must not be dead: {dead:?}"
        );
    }

    /// S18: A tag on one listed definition does not mark the other listed names
    #[test]
    fn s18_tag_on_one_listed_definition_does_not_mark_others() {
        let source = "__all__ = [\"a\", \"b\"]\n\n# @hexa:public\ndef a():\n    pass\n\ndef b():\n    pass\n";
        let exports = python_exports("pkg/mod.py", source);
        assert!(
            find(&exports, "a").hexa_public,
            "`a` is tagged: {exports:?}"
        );
        assert!(
            !find(&exports, "b").hexa_public,
            "`b` is not tagged and `__all__` is not tagged: {exports:?}"
        );
    }

    /// S19: With a tagged __all__, every export is public
    #[test]
    fn s19_with_tagged_all_every_export_is_public() {
        let cases: Vec<(&str, Vec<&str>)> = vec![
        (
            "# @hexa:public\n__all__ = [\"connect\", \"Client\"]\n\ndef connect():\n    pass\n\nclass Client:\n    pass\n\ndef helper():\n    pass\n",
            vec!["connect", "Client"],
        ),
        (
            "\"\"\"Module doc.\"\"\"\n\n# Public API of this library.\n# @hexa:public\n# Keep this list sorted.\n__all__ = [\"VERSION\", \"run\"]\n\nVERSION = \"1.0\"\n\ndef run():\n    pass\n\nOTHER = 2\n",
            vec!["VERSION", "run"],
        ),
        (
            "import functools\n\n# @hexa:public\n__all__ = [\"cached\"]\n\n@functools.lru_cache\ndef cached():\n    pass\n\nclass Hidden:\n    pass\n",
            vec!["cached"],
        ),
        (
            "# something @hexa:public here\n__all__ = [\"x\", \"y\", \"z\"]\nx = 1\ny = 2\n\ndef z():\n    pass\n",
            vec!["x", "y", "z"],
        ),
    ];

        for (source, listed) in cases {
            let exports = python_exports("pkg/lib.py", source);
            assert!(
                !exports.is_empty(),
                "expected exports for source:\n{source}"
            );
            for e in &exports {
                assert!(
                    listed.contains(&e.name.as_str()),
                    "export `{}` is not listed in `__all__` {listed:?}; source:\n{source}",
                    e.name
                );
                assert!(
                e.hexa_public,
                "export `{}` must be hexa_public under a tagged `__all__`; source:\n{source}\nexports: {exports:?}",
                e.name
            );
            }
        }
    }

    /// S22: The tag does not change which names are Python exports
    #[test]
    fn s22_tag_does_not_change_which_names_are_exports() {
        let source = "# @hexa:public\n__all__ = [\"a\"]\n\ndef a():\n    pass\n\ndef b():\n    pass\n\ndef _c():\n    pass\n";
        let exports = python_exports("pkg/mod.py", source);
        let names: Vec<&str> = exports.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["a"], "only `a` is an export: {exports:?}");
        assert!(
            find(&exports, "a").hexa_public,
            "`a` is listed by a tagged `__all__`: {exports:?}"
        );
    }
}
