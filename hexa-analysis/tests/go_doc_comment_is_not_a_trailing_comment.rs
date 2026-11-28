//! A Go doc comment is the run of comment *lines* above a declaration
//! (ADR-2611221553 §1). A comment that ends another line — after a spec, after
//! `const (`, after `)` — is that line's comment, not the next declaration's
//! doc, so a tag there marks nothing below it.

use std::path::Path;

use hexa_analysis::domain::{ExportDeclaration, Language};
use hexa_analysis::ports::AstPort;
use hexa_analysis::treesitter_adapter::TreeSitterAdapter;

fn exports(src: &str) -> Vec<ExportDeclaration> {
    TreeSitterAdapter::new().extract_exports(Path::new("pkg/lib.go"), src, Language::Go).unwrap()
}

fn public(src: &str, name: &str) -> bool {
    exports(src).iter().find(|e| e.name == name).unwrap_or_else(|| panic!("no export {name} in {src}")).hexa_public
}

#[test]
fn a_tag_at_the_end_of_a_spec_does_not_mark_the_next_one() {
    let src = "package pkg\n\nconst (\n\tMaxRetries = 3 // @hexa:public\n\tOther = 4\n)\n";
    assert!(!public(src, "Other"), "the trailing tag is MaxRetries' line comment, not Other's doc");
}

#[test]
fn a_tag_after_the_opening_paren_does_not_mark_the_first_spec() {
    let src = "package pkg\n\nconst ( // @hexa:public\n\tFirst = 1\n)\n";
    assert!(!public(src, "First"), "a comment on the `const (` line is not First's doc");
}

#[test]
fn a_tag_after_a_closing_paren_does_not_mark_the_next_declaration() {
    let src = "package pkg\n\nconst (\n\tA = 1\n) // @hexa:public\nfunc Build() {}\n";
    assert!(!public(src, "Build"), "a comment on the `)` line is not Build's doc");
}

#[test]
fn a_tag_on_its_own_line_still_marks_the_next_declaration() {
    let src = "package pkg\n\nconst (\n\tMaxRetries = 3 // retries\n\t// @hexa:public\n\tOther = 4\n)\n";
    assert!(public(src, "Other"), "a doc comment on its own line is the next spec's doc");
}

/// ADR-2611221609 S04 through hexa's own finder, not a restatement of its rule:
/// a grouped exported var nothing names is a dead export, and one another file
/// names is not.
#[test]
fn a_grouped_var_reaches_the_dead_export_finder() {
    use hexa_analysis::dead_export_finder::{find_dead_exports, FileData};
    use std::collections::HashMap;
    let a = TreeSitterAdapter::new();
    let file = |path: &str, src: &str| FileData {
        path: path.to_string(),
        imports: a.extract_imports(Path::new(path), src, Language::Go).unwrap(),
        exports: a.extract_exports(Path::new(path), src, Language::Go).unwrap(),
        references: a.extract_references(Path::new(path), src, Language::Go).unwrap_or_default(),
        members: HashMap::new(),
    };
    let vars = file("internal/app/vars.go", "package app\n\nvar (\n\tUnusedFlag = true\n\tShared = 1\n)\n");
    let user = file("internal/app/use.go", "package app\n\nfunc f() int { return Shared }\n");
    let dead: Vec<String> = find_dead_exports(&[vars, user], &[]).into_iter().map(|d| d.export_name).collect();
    assert!(dead.contains(&"UnusedFlag".to_string()), "an unused grouped var is dead: {dead:?}");
    assert!(!dead.contains(&"Shared".to_string()), "a grouped var another file names is not: {dead:?}");
}

/// A trailing comment ends the run even when it is not the line directly
/// above: here a plain doc line sits between it and `B`.
#[test]
fn a_trailing_tag_one_line_further_up_does_not_mark_the_next_spec() {
    let src = "package pkg\n\nconst (\n\tA = 1 // @hexa:public\n\t// doc\n\tB = 2\n)\n";
    assert!(!public(src, "B"), "the tag is A's line comment; B's doc is only `// doc`");
}

/// The same rule in a var group, whose specs sit one level down, and for a
/// top-level var followed by a function.
#[test]
fn a_trailing_tag_in_a_var_group_or_after_a_top_level_var_marks_nothing_below() {
    let grouped = "package pkg\n\nvar (\n\tA = 1 // @hexa:public\n\tB = 2\n)\n";
    assert!(!public(grouped, "B"), "in a var group: {grouped}");
    let top = "package pkg\n\nvar X = 1 // @hexa:public\nfunc F() {}\n";
    assert!(!public(top, "F"), "after a top-level var: {top}");
}
