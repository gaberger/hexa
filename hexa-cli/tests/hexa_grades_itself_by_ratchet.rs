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
    // Surfaced when `super::` resolved to a real module, inline paths and
    // nested/`pub use`/aliased imports became edges. Grouped by the rule.
    // domain must not import from outside domain.
    ("hexa-analysis/src/domain.rs", "super::frontend_checker::FrontendCheckResult"),
    // adapters must not import from domain directly.
    ("hexa-cli/src/commands/analyze.rs", "hexa_analysis::domain::ArchAnalysisResult::compute_health_score"),
    ("hexa-cli/src/commands/analyze.rs", "hexa_analysis::domain::Language"),
    ("hexa-cli/src/commands/analyze.rs", "hexa_analysis::domain::Language::Rust"),
    ("hexa-cli/src/commands/analyze.rs", "hexa_analysis::domain::Language::from_path"),
    ("hexa-exec/src/tools/code_patch.rs", "hexa_core::domain::validation::is_critical_path"),
    // adapters must not import from other adapters.
    ("hexa-cli/src/commands/analyze.rs", "hexa_analysis::treesitter_adapter::ReferenceKind"),
    ("hexa-cli/src/commands/analyze.rs", "hexa_analysis::treesitter_adapter::extract_module_references"),
    ("hexa-cli/src/commands/bootstrap/models.rs", "hexa_infer::local_provider"),
    ("hexa-cli/src/commands/bootstrap/prereq.rs", "hexa_infer::local_provider"),
    ("hexa-cli/src/commands/bootstrap/services.rs", "hexa_infer::local_provider"),
    ("hexa-cli/src/commands/bootstrap/validate.rs", "hexa_infer::discover"),
    ("hexa-cli/src/commands/bootstrap/validate.rs", "hexa_infer::local_provider"),
    ("hexa-cli/src/commands/build.rs", "hexa_exec::provenance::Facts"),
    ("hexa-cli/src/commands/build.rs", "hexa_exec::provenance::write"),
    ("hexa-cli/src/commands/build.rs", "hexa_infer::discover::any_path"),
    ("hexa-cli/src/commands/build.rs", "hexa_infer::discover::discover"),
    ("hexa-cli/src/commands/doctor/composition.rs", "hexa_infer::discover"),
    ("hexa-cli/src/commands/doctor/composition.rs", "hexa_infer::discover::any_path"),
    ("hexa-cli/src/commands/doctor/composition.rs", "hexa_infer::discover::path_words"),
    ("hexa-cli/src/commands/hey.rs", "hexa_infer::local_provider"),
    ("hexa-cli/src/commands/hook/mod.rs", "hexa_exec::local_store::memory_delete"),
    ("hexa-cli/src/commands/hook/mod.rs", "hexa_exec::local_store::memory_get"),
    ("hexa-cli/src/commands/hook/mod.rs", "hexa_exec::local_store::memory_put"),
    ("hexa-cli/src/commands/hook/mod.rs", "hexa_exec::local_store::persist_run"),
    ("hexa-cli/src/commands/inference.rs", "hexa_infer::registry::load"),
    ("hexa-cli/src/commands/inference.rs", "hexa_infer::registry::remove"),
    ("hexa-cli/src/commands/inference.rs", "hexa_infer::registry::save"),
    ("hexa-cli/src/commands/inference.rs", "hexa_infer::registry::upsert"),
    ("hexa-cli/src/commands/test.rs", "hexa_infer::registry::load"),
    // usecases may only import from domain and ports.
    ("hexa-exec/src/direct_exec.rs", "crate::local_store::memory_entries"),
    ("hexa-exec/src/direct_exec.rs", "crate::local_store::persist_run"),
    ("hexa-exec/src/direct_exec.rs", "crate::local_store::recent_runs"),
    ("hexa-exec/src/frontier.rs", "hexa_infer::spend::model_from_frontier_json"),
    ("hexa-exec/src/frontier.rs", "hexa_infer::spend::record_with"),
    ("hexa-exec/src/resource_governor.rs", "hexa_infer::local_provider"),
    // adapters must not import from usecases.
    ("hexa-infer/src/local_provider.rs", "crate::tiers::project_root"),
    ("hexa-infer/src/local_provider.rs", "crate::tiers::tier_model_in"),
    ("hexa-infer/src/spend.rs", "crate::tiers::project_root"),
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
