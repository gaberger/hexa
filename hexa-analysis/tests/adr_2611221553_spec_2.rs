#![allow(unused, clippy::all)]
mod part_1 {
    #![allow(dead_code, clippy::needless_range_loop)]

    use std::path::Path;

    use hexa_analysis::dead_export_finder::{find_dead_exports, FileData};
    use hexa_analysis::domain::{DeadExport, ExportDeclaration, ExportKind, Language};
    use hexa_analysis::ports::AstPort;
    use hexa_analysis::treesitter_adapter::TreeSitterAdapter;

    fn go_exports(src: &str) -> Vec<ExportDeclaration> {
        let adapter = TreeSitterAdapter::new();
        adapter
            .extract_exports(Path::new("pkg/domain/lib.go"), src, Language::Go)
            .expect("extract_exports on Go source should succeed")
    }

    fn find<'a>(exports: &'a [ExportDeclaration], name: &str) -> &'a ExportDeclaration {
        let matches: Vec<&ExportDeclaration> = exports.iter().filter(|e| e.name == name).collect();
        assert_eq!(
            matches.len(),
            1,
            "expected exactly one export named `{name}`, got {:?}",
            exports
                .iter()
                .map(|e| (e.name.clone(), e.kind, e.hexa_public))
                .collect::<Vec<_>>()
        );
        matches[0]
    }

    fn file_data(path: &str, src: &str) -> FileData {
        let adapter = TreeSitterAdapter::new();
        let p = Path::new(path);
        FileData {
            path: path.to_string(),
            imports: adapter
                .extract_imports(p, src, Language::Go)
                .expect("extract_imports"),
            exports: adapter
                .extract_exports(p, src, Language::Go)
                .expect("extract_exports"),
            references: adapter
                .extract_references(p, src, Language::Go)
                .expect("extract_references"),
            members: adapter
                .extract_members(p, src, Language::Go)
                .expect("extract_members"),
        }
    }

    fn dead_names(dead: &[DeadExport]) -> Vec<String> {
        dead.iter().map(|d| d.export_name.clone()).collect()
    }

    /// S01: A tagged exported Go function is public
    #[test]
    fn s01_tagged_exported_go_function_is_public() {
        let src = "package domain\n\
\n\
type Client struct{}\n\
\n\
// Build makes a client. @hexa:public\n\
func Build() *Client { return nil }\n";
        let exports = go_exports(src);
        let build = find(&exports, "Build");
        assert_eq!(build.kind, ExportKind::Function);
        assert!(
            build.hexa_public,
            "`Build` carries @hexa:public in its doc comment"
        );
    }

    /// S02: The tag may be on any line of a multi-line doc comment
    #[test]
    fn s02_tag_on_any_line_of_multi_line_doc_comment() {
        let src = "package domain\n\
\n\
// New builds the thing.\n\
// @hexa:public\n\
// It is safe to call more than once.\n\
func New() {}\n";
        let exports = go_exports(src);
        let new = find(&exports, "New");
        assert!(
            new.hexa_public,
            "the tag on the middle line marks `New` public"
        );
    }

    /// S03: A tagged method is public
    #[test]
    fn s03_tagged_method_is_public() {
        let src = "package domain\n\
\n\
type Client struct{}\n\
\n\
// @hexa:public\n\
func (c *Client) Send() error { return nil }\n";
        let exports = go_exports(src);
        let send = find(&exports, "Send");
        assert_eq!(send.kind, ExportKind::Method);
        assert!(send.hexa_public, "`Send` carries @hexa:public");
    }

    /// S04: Tagged standalone type, const and var declarations are public
    #[test]
    fn s04_tagged_type_const_var_are_public() {
        let src = "package domain\n\
\n\
// @hexa:public\n\
type Option struct{}\n\
\n\
// @hexa:public\n\
const MaxRetries = 3\n\
\n\
// @hexa:public\n\
var DefaultTimeout = 5\n";
        let exports = go_exports(src);
        for name in ["Option", "MaxRetries", "DefaultTimeout"] {
            let e = find(&exports, name);
            assert!(
                e.hexa_public,
                "`{name}` carries @hexa:public and must be public"
            );
        }
    }

    /// S05: A tagged Go export with no caller is not reported dead
    #[test]
    fn s05_tagged_go_export_without_caller_not_dead() {
        let lib = "package domain\n\
\n\
// @hexa:public\n\
func Build() {}\n";
        let other = "package domain\n\
\n\
func helper() {}\n";
        let files = vec![
            file_data("pkg/domain/build.go", lib),
            file_data("pkg/domain/other.go", other),
        ];
        assert!(
            find(&files[0].exports, "Build").hexa_public,
            "precondition: `Build` is read as public"
        );
        let dead = find_dead_exports(&files, &[]);
        assert!(
            !dead.iter().any(|d| d.export_name == "Build"),
            "`Build` is tagged @hexa:public and must not be reported dead; dead = {:?}",
            dead_names(&dead)
        );
    }

    /// S07: The run ends at a line that is not a comment
    #[test]
    fn s07_run_ends_at_non_comment_line() {
        let src = "package domain\n\
\n\
// @hexa:public\n\
func Helper() {}\n\
func Build() {}\n";
        let exports = go_exports(src);
        assert!(find(&exports, "Helper").hexa_public, "`Helper` is tagged");
        assert!(
            !find(&exports, "Build").hexa_public,
            "`Build` has no doc comment of its own"
        );
    }

    /// S10: A tagged export with no caller elsewhere is still not reported dead even though unused
    #[test]
    fn s10_tagged_unused_const_not_reported_dead() {
        let lib = "package domain\n\
\n\
// @hexa:public\n\
const Unused = 1\n";
        let files = vec![file_data("pkg/domain/consts.go", lib)];
        assert!(
            find(&files[0].exports, "Unused").hexa_public,
            "precondition: `Unused` is read as public"
        );
        let dead = find_dead_exports(&files, &[]);
        let counted = dead.iter().filter(|d| d.export_name == "Unused").count();
        assert_eq!(
            counted,
            0,
            "`Unused` is tagged and must not be in dead_exports; dead = {:?}",
            dead_names(&dead)
        );
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

    fn prose_line(rng: &mut Lcg) -> &'static str {
        match rng.below(3) {
            0 => "// Some prose about it.",
            1 => "// It does a thing, carefully.",
            _ => "// See the package docs.",
        }
    }

    fn tag_line(rng: &mut Lcg) -> &'static str {
        match rng.below(3) {
            0 => "// @hexa:public",
            1 => "// This is part of the API. @hexa:public",
            _ => "// @hexa:public stable since v1",
        }
    }

    /// S11: For every exported Go declaration, hexa_public is exactly whether the contiguous comment run above it contains the tag
    #[test]
    fn s11_hexa_public_iff_contiguous_run_contains_tag() {
        for seed in 1..=30u64 {
            let mut rng = Lcg(seed.wrapping_mul(0x9E3779B97F4A7C15));
            let mut src = String::from("package domain\n\ntype Recv struct{}\n");
            let mut expected: Vec<(String, bool)> = Vec::new();

            for i in 0..12 {
                // Always separate from the previous declaration by a blank line.
                src.push('\n');

                // Optional earlier block, separated by blank lines from what follows.
                if rng.below(2) == 0 {
                    let len = 1 + rng.below(3);
                    let tag_at = if rng.below(2) == 0 {
                        Some(rng.below(len))
                    } else {
                        None
                    };
                    for j in 0..len {
                        if Some(j) == tag_at {
                            src.push_str(tag_line(&mut rng));
                        } else {
                            src.push_str(prose_line(&mut rng));
                        }
                        src.push('\n');
                    }
                    for _ in 0..(1 + rng.below(2)) {
                        src.push('\n');
                    }
                }

                // Final run of comment lines.
                let run_len = rng.below(4);
                let tag_in_run = run_len > 0 && rng.below(2) == 0;
                let tag_at = if tag_in_run {
                    Some(rng.below(run_len))
                } else {
                    None
                };
                for j in 0..run_len {
                    if Some(j) == tag_at {
                        src.push_str(tag_line(&mut rng));
                    } else {
                        src.push_str(prose_line(&mut rng));
                    }
                    src.push('\n');
                }

                // Blank lines between the run and the declaration.
                let blanks_after = if rng.below(3) == 0 {
                    1 + rng.below(2)
                } else {
                    0
                };
                for _ in 0..blanks_after {
                    src.push('\n');
                }

                let (name, decl) = match rng.below(5) {
                    0 => {
                        let n = format!("Func{i}");
                        let d = format!("func {n}() {{}}");
                        (n, d)
                    }
                    1 => {
                        let n = format!("Method{i}");
                        let d = format!("func (r *Recv) {n}() {{}}");
                        (n, d)
                    }
                    2 => {
                        let n = format!("Type{i}");
                        let d = format!("type {n} struct{{}}");
                        (n, d)
                    }
                    3 => {
                        let n = format!("Const{i}");
                        let d = format!("const {n} = 1");
                        (n, d)
                    }
                    _ => {
                        let n = format!("Var{i}");
                        let d = format!("var {n} = 2");
                        (n, d)
                    }
                };
                src.push_str(&decl);
                src.push('\n');

                let is_public = blanks_after == 0 && tag_in_run;
                expected.push((name, is_public));
            }

            let exports = go_exports(&src);
            for (name, want) in &expected {
                let e = find(&exports, name);
                assert_eq!(
                    e.hexa_public, *want,
                    "seed {seed}: `{name}` hexa_public should be {want}\n--- source ---\n{src}"
                );
            }
            assert!(
                !find(&exports, "Recv").hexa_public,
                "seed {seed}: untagged `Recv` must not be public"
            );
        }
    }

    /// S12: A tag on a const group marks every spec in it
    #[test]
    fn s12_tag_on_const_group_marks_every_spec() {
        let src = "package domain\n\
\n\
// @hexa:public\n\
const (\n\
\tA = 1\n\
\tB = 2\n\
\tC = 3\n\
)\n";
        let exports = go_exports(src);
        for name in ["A", "B", "C"] {
            assert!(
                find(&exports, name).hexa_public,
                "`{name}` is in a group tagged @hexa:public"
            );
        }
    }

    /// S13: A tag on one spec inside a group marks only that spec
    #[test]
    fn s13_tag_on_one_spec_in_group_marks_only_that_spec() {
        let src = "package domain\n\
\n\
type (\n\
\tAlpha struct{}\n\
\t// @hexa:public\n\
\tBeta struct{}\n\
\tGamma struct{}\n\
)\n";
        let exports = go_exports(src);
        assert!(find(&exports, "Beta").hexa_public, "`Beta` is tagged");
        assert!(
            !find(&exports, "Alpha").hexa_public,
            "`Alpha` is not tagged"
        );
        assert!(
            !find(&exports, "Gamma").hexa_public,
            "`Gamma` is not tagged"
        );
    }
}

mod part_2 {
    #![allow(dead_code)]

    use hexa_analysis::domain::{ExportDeclaration, ExportKind, Language};
    use hexa_analysis::ports::AstPort;
    use hexa_analysis::treesitter_adapter::TreeSitterAdapter;
    use std::path::Path;

    fn go_exports(source: &str) -> Vec<ExportDeclaration> {
        TreeSitterAdapter::new()
            .extract_exports(Path::new("pkg/lib.go"), source, Language::Go)
            .expect("Go source parses")
    }

    fn find<'a>(
        exports: &'a [ExportDeclaration],
        name: &str,
        source: &str,
    ) -> &'a ExportDeclaration {
        exports.iter().find(|e| e.name == name).unwrap_or_else(|| {
            panic!("export `{name}` not found in:\n{source}\nexports: {exports:?}")
        })
    }

    /// S14: A tag on a var group covers its specs
    #[test]
    fn s14_a_tag_on_a_var_group_covers_its_specs() {
        let src =
            "package p\n\n// Values for things.\n// @hexa:public\nvar (\n\tX = 1\n\tY = 2\n)\n";
        let exports = go_exports(src);
        assert!(find(&exports, "X", src).hexa_public, "X should be public");
        assert!(find(&exports, "Y", src).hexa_public, "Y should be public");
    }

    /// S16: A spec's own tag does not spread to its siblings
    #[test]
    fn s16_a_specs_own_tag_does_not_spread_to_its_siblings() {
        let src = "package p\n\nconst (\n\t// @hexa:public\n\tA = 1\n\tB = 2\n)\n";
        let exports = go_exports(src);
        assert!(find(&exports, "A", src).hexa_public, "A should be public");
        assert!(
            !find(&exports, "B", src).hexa_public,
            "B should not be public"
        );
    }

    /// S17: A group's tag does not cover a declaration after the group
    #[test]
    fn s17_a_groups_tag_does_not_cover_a_declaration_after_the_group() {
        let src = "package p\n\n// @hexa:public\nconst (\n\tA = 1\n)\nconst B = 2\n";
        let exports = go_exports(src);
        assert!(find(&exports, "A", src).hexa_public, "A should be public");
        assert!(
            !find(&exports, "B", src).hexa_public,
            "B should not be public"
        );
    }

    /// A preceding comment configuration: blocks of comment lines (true = tagged
    /// line), separated from each other by blank lines, and an optional blank line
    /// between the last block and the declaration.
    #[derive(Clone, Debug)]
    struct Prefix {
        blocks: Vec<Vec<bool>>,
        gap_before_decl: bool,
    }

    impl Prefix {
        fn expected_tag(&self) -> bool {
            if self.gap_before_decl {
                return false;
            }
            self.blocks.last().map_or(false, |b| b.iter().any(|t| *t))
        }

        fn render(&self, indent: &str, out: &mut String) {
            for (i, block) in self.blocks.iter().enumerate() {
                if i > 0 {
                    out.push('\n');
                }
                for tagged in block {
                    out.push_str(indent);
                    if *tagged {
                        out.push_str("// @hexa:public\n");
                    } else {
                        out.push_str("// Documentation line.\n");
                    }
                }
            }
            if self.gap_before_decl && !self.blocks.is_empty() {
                out.push('\n');
            }
        }
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
        fn chance(&mut self, num: u64, den: u64) -> bool {
            self.below(den) < num
        }
    }

    fn random_prefix(rng: &mut Lcg) -> Prefix {
        let nblocks = rng.below(3) as usize;
        let mut blocks = Vec::new();
        for _ in 0..nblocks {
            let nlines = 1 + rng.below(3) as usize;
            let block: Vec<bool> = (0..nlines).map(|_| rng.chance(1, 3)).collect();
            blocks.push(block);
        }
        let gap_before_decl = !blocks.is_empty() && rng.chance(1, 4);
        Prefix {
            blocks,
            gap_before_decl,
        }
    }

    /// S18: A grouped spec is public exactly when its own run or the group's run carries the tag
    #[test]
    fn s18_a_grouped_spec_is_public_exactly_when_its_run_or_the_groups_run_carries_the_tag() {
        let mut rng = Lcg(0x2611_2215_53);
        for case in 0..200 {
            let mut src = String::from("package p\n\n");
            let mut expected: Vec<(String, bool)> = Vec::new();
            let ngroups = 1 + rng.below(4) as usize;
            for g in 0..ngroups {
                let kind = rng.below(3);
                let group_prefix = random_prefix(&mut rng);
                group_prefix.render("", &mut src);
                let keyword = match kind {
                    0 => "type",
                    1 => "const",
                    _ => "var",
                };
                src.push_str(keyword);
                src.push_str(" (\n");
                let nspecs = 1 + rng.below(4) as usize;
                for s in 0..nspecs {
                    let spec_prefix = random_prefix(&mut rng);
                    spec_prefix.render("\t", &mut src);
                    let name = match kind {
                        0 => format!("TypeG{g}S{s}"),
                        1 => format!("ConstG{g}S{s}"),
                        _ => format!("VarG{g}S{s}"),
                    };
                    match kind {
                        0 => src.push_str(&format!("\t{name} struct{{}}\n")),
                        _ => src.push_str(&format!("\t{name} = {s}\n")),
                    }
                    expected.push((
                        name,
                        spec_prefix.expected_tag() || group_prefix.expected_tag(),
                    ));
                }
                src.push_str(")\n\n");
            }
            let exports = go_exports(&src);
            for (name, want) in &expected {
                let e = find(&exports, name, &src);
                assert_eq!(
                    e.hexa_public, *want,
                    "case {case}: `{name}` hexa_public should be {want} in:\n{src}"
                );
            }
        }
    }

    /// S22: The tag does not change any other field of a Go export
    #[test]
    fn s22_the_tag_does_not_change_any_other_field_of_a_go_export() {
        let tagged_src = "// @hexa:public\nfunc Build() {}\n";
        let untagged_src = "// Build builds.\nfunc Build() {}\n";
        let tagged = go_exports(tagged_src);
        let untagged = go_exports(untagged_src);

        let t = find(&tagged, "Build", tagged_src);
        let u = find(&untagged, "Build", untagged_src);

        assert_eq!(t.name, "Build");
        assert_eq!(t.line, 2);
        assert_eq!(t.kind, ExportKind::Function);
        assert!(t.hexa_public, "tagged Build should be public");

        assert!(!u.hexa_public, "untagged Build should not be public");
        assert_eq!(t.file, u.file);
        assert_eq!(t.name, u.name);
        assert_eq!(t.line, u.line);
        assert_eq!(t.kind, u.kind);
        assert_eq!(tagged.len(), untagged.len(), "same number of exports");
    }
}
