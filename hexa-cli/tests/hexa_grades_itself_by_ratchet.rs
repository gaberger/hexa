//! hexa's own boundary violations can only shrink.
//!
//! hexa graded itself A+ while its grade read about a fifth of its code:
//! 118 of 144 Rust files had no layer, and no import between its crates was
//! an edge. Once both were fixed, the same tree graded C with three real
//! violations. Asserting A+ would now be false; asserting C would bless the
//! three. This asserts the exact set instead:
//!
//! - a violation not in `KNOWN` fails — nothing new gets in;
//! - a violation in `KNOWN` that is gone also fails — delete its line, so a
//!   fix is locked in and the list only ever gets shorter.
//!
//! A tool that holds other projects to these rules holds itself to them
//! first, in CI, with no way to relax the list without a visible diff to it.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

/// (importing file, import) — each with the reason it is still here.
const KNOWN: &[(&str, &str)] = &[
    // Surfaced together when the "own submodule" exemption stopped exempting
    // every import a crate-root file made, and a file's declared layer began
    // to match its module path. They were always there; the grade could not
    // see them, and printed A+.
    // use case → adapter, no port between them.
    ("hexa-analysis/src/analyzers/dead_layer.rs", "crate::treesitter_adapter::TreeSitterAdapter"),
    ("hexa-analysis/src/layer_inventory.rs", "crate::treesitter_adapter::parse_source"),
    ("hexa-exec/src/direct_react.rs", "crate::tools::ToolRegistry"),
    ("hexa-exec/src/direct_workspace.rs", "hexa_git::worktree"),
    ("hexa-exec/src/simple_agent.rs", "crate::tools::ToolRegistry"),
    // CLI command → adapter or infrastructure directly — the command is doing the composition root's wiring.
    ("hexa-cli/src/commands/assets_cmd.rs", "crate::assets::Assets"),
    ("hexa-cli/src/commands/doctor/mod.rs", "crate::assets::Assets"),
    ("hexa-cli/src/commands/inference.rs", "crate::assets::Assets"),
    ("hexa-cli/src/commands/memory/mod.rs", "hexa_exec::local_store::MemoryScope"),
    ("hexa-cli/src/commands/memory/mod.rs", "hexa_exec::local_store::self"),
    ("hexa-cli/src/commands/skill.rs", "crate::assets::Assets"),
    ("hexa-cli/src/commands/spend_cmd.rs", "hexa_infer::spend::Totals"),
    ("hexa-cli/src/commands/spend_cmd.rs", "hexa_infer::spend::self"),
    // adapter → domain type the port does not re-export.
    ("hexa-cli/src/commands/graph.rs", "hexa_graph::model::KnowledgeGraph"),
    ("hexa-graph/src/extract/code.rs", "crate::model::NodeKind"),
];

#[test]
fn hexas_boundary_violations_are_exactly_the_known_ones() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("workspace root");
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(["analyze", ".", "--json"])
        .current_dir(root)
        .env("HOME", home.path())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}:\n{text}"));
    let found: BTreeSet<(String, String)> = v["boundary_violations"]
        .as_array()
        .unwrap_or_else(|| panic!("no boundary_violations:\n{text}"))
        .iter()
        .map(|x| {
            (
                x["from_file"].as_str().unwrap_or("").to_string(),
                x["import_path"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect();
    let known: BTreeSet<(String, String)> =
        KNOWN.iter().map(|(f, i)| (f.to_string(), i.to_string())).collect();

    let new: Vec<_> = found.difference(&known).collect();
    let fixed: Vec<_> = known.difference(&found).collect();
    assert!(new.is_empty(), "new boundary violations in hexa itself — fix them, do not list them:\n{new:#?}");
    assert!(fixed.is_empty(), "fixed — delete these lines from KNOWN so they stay fixed:\n{fixed:#?}");
}
