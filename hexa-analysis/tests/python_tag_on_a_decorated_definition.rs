//! A trailing `# @hexa:public` on a definition's own `def`/`class` line tags
//! it, whether or not the definition is decorated (ADR-2611221644 §1). The
//! comment run above the first decorator still counts, and the tag of one
//! line never reaches another definition.

use std::path::Path;

use hexa_analysis::domain::{ExportDeclaration, Language};
use hexa_analysis::ports::AstPort;
use hexa_analysis::treesitter_adapter::TreeSitterAdapter;

fn public(source: &str, name: &str) -> bool {
    let all: Vec<ExportDeclaration> = TreeSitterAdapter::new()
        .extract_exports(Path::new("lib.py"), source, Language::Python)
        .unwrap();
    let found: Vec<&ExportDeclaration> = all.iter().filter(|e| e.name == name).collect();
    assert_eq!(found.len(), 1, "one export named {name} in:\n{source}");
    found[0].hexa_public
}

#[test]
fn a_trailing_tag_on_a_decorated_def_line_tags_it() {
    let src = "@cache\ndef connect():  # @hexa:public\n    pass\n";
    assert!(public(src, "connect"), "{src}");
}

#[test]
fn a_trailing_tag_on_a_decorated_class_line_tags_it() {
    let src = "@dataclass\n@frozen\nclass Client:  # @hexa:public\n    pass\n";
    assert!(public(src, "Client"), "{src}");
}

#[test]
fn a_trailing_tag_on_an_undecorated_def_line_tags_it() {
    let src = "def connect():  # @hexa:public\n    pass\n";
    assert!(public(src, "connect"), "{src}");
}

#[test]
fn the_run_above_the_first_decorator_still_tags_it() {
    let src = "# @hexa:public\n@cache\ndef connect():\n    pass\n";
    assert!(public(src, "connect"), "{src}");
}

#[test]
fn an_untagged_decorated_def_is_not_public() {
    let src = "@cache\ndef connect():  # cached\n    pass\n\n@cache\ndef other():\n    pass\n";
    assert!(!public(src, "connect"), "{src}");
    assert!(!public(src, "other"), "{src}");
}

#[test]
fn a_tag_in_a_string_on_the_def_line_is_not_a_tag() {
    let src = "@cache\ndef connect(x=\"# @hexa:public\"):\n    pass\n";
    assert!(!public(src, "connect"), "{src}");
}
