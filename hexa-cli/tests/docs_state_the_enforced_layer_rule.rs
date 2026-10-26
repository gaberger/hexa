//! Rule 4 as the docs state it must be the rule `hexa analyze` enforces
//! (ADR-2610261356, issue #19).
//!
//! The analyzer has always let a primary adapter import `usecases/`
//! (`layer_classifier::allowed_targets`), while the README, ARCHITECTURE.md,
//! CLAUDE.md and the CLAUDE.md template every scaffold ships all said
//! "ports only". A user who read the rule and trusted the grade got two
//! different answers. Prose cannot fail; this test makes the prose fail when it
//! drifts from the table again — in either direction.

use hexa_analysis::domain::HexLayer;
use hexa_analysis::layer_classifier::get_violation_rule;
use std::path::Path;

fn read(rel: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("workspace root");
    std::fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// The one line in `doc` that states rule 4 for primary adapters.
fn rule_four<'a>(name: &str, doc: &'a str, starts: &str) -> &'a str {
    let hits: Vec<&str> = doc
        .lines()
        .filter(|l| l.trim_start().starts_with(starts) && l.contains("adapter"))
        .collect();
    assert_eq!(hits.len(), 1, "{name}: expected exactly one rule-4 line starting {starts:?}, found {hits:?}");
    hits[0]
}

#[test]
fn every_shipped_statement_of_rule_four_matches_the_analyzer() {
    let primary_may_drive_usecases = get_violation_rule(HexLayer::AdaptersPrimary, HexLayer::Usecases).is_none();
    let secondary_may_not = get_violation_rule(HexLayer::AdaptersSecondary, HexLayer::Usecases).is_some();
    assert!(secondary_may_not, "a driven adapter calling back into usecases must stay a violation");

    let template = include_str!("../assets/templates/claude-md-hexa-section.md");
    let docs = [
        ("assets/templates/claude-md-hexa-section.md", template.to_string(), "4. "),
        ("CLAUDE.md", read("CLAUDE.md"), "4. "),
        ("README.md", read("README.md"), "| 4 |"),
        ("ARCHITECTURE.md", read("ARCHITECTURE.md"), "| 4 |"),
    ];
    for (name, text, starts) in &docs {
        let line = rule_four(name, text, starts);
        assert_eq!(
            line.contains("usecases"),
            primary_may_drive_usecases,
            "{name} states rule 4 as {line:?}, but the analyzer {} a primary adapter importing usecases",
            if primary_may_drive_usecases { "allows" } else { "forbids" }
        );
    }
}
