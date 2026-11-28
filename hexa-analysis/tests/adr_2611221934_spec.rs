#![allow(unused, clippy::all)]
mod part_1 {
    #![allow(dead_code)]

    use hexa_analysis::ports::{AstPort, ExportDeclaration, Language};
    use hexa_analysis::treesitter_adapter::TreeSitterAdapter;
    use std::path::Path;

    fn exports(source: &str) -> Vec<ExportDeclaration> {
        TreeSitterAdapter::new()
            .extract_exports(Path::new("pkg/file.go"), source, Language::Go)
            .expect("extract_exports should succeed on valid Go")
    }

    fn public_of(source: &str, name: &str) -> bool {
        let all = exports(source);
        let found: Vec<&ExportDeclaration> = all.iter().filter(|e| e.name == name).collect();
        assert!(
            !found.is_empty(),
            "no export named {name} in {:?}\nsource:\n{source}",
            all.iter().map(|e| e.name.clone()).collect::<Vec<_>>()
        );
        found[0].hexa_public
    }

    /// S01: A tag in the second of two comments after code does not tag the next spec
    #[test]
    fn s01_tag_in_second_of_two_trailing_comments_does_not_tag_next_spec() {
        let src = "package p\n\nconst (\n\tA = 1 /* x */ // @hexa:public\n\tB = 2\n)\n";
        assert!(
            !public_of(src, "B"),
            "B must not be tagged by A's line comments"
        );
    }

    /// S02: Three comments after code all belong to that code's line
    #[test]
    fn s02_three_trailing_comments_belong_to_code_line() {
        let src = "package p\n\nvar A = 1 /* x */ /* y */ // @hexa:public\nfunc F() {}\n";
        assert!(
            !public_of(src, "F"),
            "F must not be tagged by A's line comments"
        );
    }

    /// S06: Comments after code on a line never tag the declaration on the next line, whatever their number or kind
    #[test]
    fn s06_trailing_comments_never_tag_next_declaration() {
        let mut cases: Vec<String> = Vec::new();
        for n in 1..=4usize {
            for final_line in [false, true] {
                for tag_at in 0..n {
                    let mut comments = Vec::new();
                    for i in 0..n {
                        let text = if i == tag_at { "@hexa:public" } else { "note" };
                        if final_line && i == n - 1 {
                            comments.push(format!("// {text}"));
                        } else {
                            comments.push(format!("/* {text} */"));
                        }
                    }
                    cases.push(comments.join(" "));
                }
            }
        }
        for comments in &cases {
            // Top-level declaration after a var line.
            let src = format!("package p\n\nvar A = 1 {comments}\nfunc D() {{}}\n");
            assert!(
                !public_of(&src, "D"),
                "D tagged by trailing comments in:\n{src}"
            );
            // Spec inside a const group.
            let src = format!("package p\n\nconst (\n\tA = 1 {comments}\n\tD = 2\n)\n");
            assert!(
                !public_of(&src, "D"),
                "D tagged by trailing comments in:\n{src}"
            );
            // Type declaration after a var line.
            let src = format!("package p\n\nvar A = 1 {comments}\ntype D struct{{}}\n");
            assert!(
                !public_of(&src, "D"),
                "D tagged by trailing comments in:\n{src}"
            );
        }
    }

    /// S07: A comment-only line of two comments, tag in the first, tags the function below
    #[test]
    fn s07_comment_only_line_tag_in_first_tags_function() {
        let src = "package p\n\n/* @hexa:public */ // note\nfunc F() {}\n";
        assert!(public_of(src, "F"), "F's doc line must tag it");
    }

    /// S09: A line of two comments counts as one line, so the run continues to the line above it
    #[test]
    fn s09_two_comment_line_counts_once_run_continues() {
        let src = "package p\n// @hexa:public\n/* a */ /* b */\nfunc F() {}\n";
        assert!(
            public_of(src, "F"),
            "the run must continue past the two-comment line"
        );
    }
}

mod part_2 {
    #![allow(dead_code)]

    use hexa_analysis::domain::{ExportDeclaration, Language};
    use hexa_analysis::ports::AstPort;
    use hexa_analysis::treesitter_adapter::TreeSitterAdapter;
    use std::path::Path;

    fn exports(source: &str, lang: Language, file: &str) -> Vec<ExportDeclaration> {
        TreeSitterAdapter::new()
            .extract_exports(Path::new(file), source, lang)
            .expect("extract_exports should succeed")
    }

    fn go_exports(source: &str) -> Vec<ExportDeclaration> {
        exports(source, Language::Go, "pkg/file.go")
    }

    fn find<'a>(
        exports: &'a [ExportDeclaration],
        name: &str,
        source: &str,
    ) -> &'a ExportDeclaration {
        exports.iter().find(|e| e.name == name).unwrap_or_else(|| {
            panic!("no export named `{name}` in:\n{source}\nexports: {exports:?}")
        })
    }

    fn go_public(source: &str, name: &str) -> bool {
        let ex = go_exports(source);
        find(&ex, name, source).hexa_public
    }

    /// S13: On a comment-only line directly above a declaration, the tag is read whatever the position and number of the comments
    #[test]
    fn s13_tag_read_from_any_comment_on_comment_only_line() {
        let mut checked = 0;
        for k in 1..=4usize {
            // Whether the last comment is a `//` comment.
            for final_line in [false, true] {
                // Bitmask of which comments carry the tag.
                for mask in 0..(1u32 << k) {
                    let mut parts = Vec::new();
                    for i in 0..k {
                        let tagged = mask & (1 << i) != 0;
                        let is_line = final_line && i == k - 1;
                        let body = if tagged {
                            "@hexa:public".to_string()
                        } else {
                            format!("note{i}")
                        };
                        if is_line {
                            parts.push(format!("// {body}"));
                        } else {
                            parts.push(format!("/* {body} */"));
                        }
                    }
                    let line = parts.join(" ");
                    let src = format!("package p\n\n{line}\nfunc F() {{}}\n");
                    let expected = mask != 0;
                    assert_eq!(
                        go_public(&src, "F"),
                        expected,
                        "F hexa_public should be {expected} for:\n{src}"
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked > 0);
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Item {
        Blank,
        Tagged,
        Plain,
        Func,
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
    }
}
