//! hexa's inference adapters are graded as what they are: driven, secondary.
//!
//! They lived in a flat `hexa-infer/src/adapters/`, which the classifier maps
//! to *primary* — the permissive side. So the one rule that is only about
//! secondary adapters (they may reach ports, never use cases) was checked
//! against none of hexa's own code, and the layer inventory showed zero
//! secondary adapters in a tool whose whole job is calling out to providers.

use hexa_analysis::domain::HexLayer;
use hexa_analysis::layer_classifier::classify_layer;
use std::path::Path;

fn rust_files(dir: &Path, root: &Path, out: &mut Vec<String>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            rust_files(&p, root, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"));
        }
    }
}

#[test]
fn every_inference_adapter_classifies_as_secondary() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("workspace root");
    let mut files = Vec::new();
    rust_files(&root.join("hexa-infer/src/adapters"), root, &mut files);
    // `adapters/mod.rs` only declares the `secondary` module: structure, not an adapter.
    files.retain(|f| f != "hexa-infer/src/adapters/mod.rs");
    assert!(files.len() >= 6, "the provider adapters were not found: {files:?}");
    let wrong: Vec<_> = files
        .iter()
        .filter(|f| classify_layer(f) != HexLayer::AdaptersSecondary)
        .map(|f| format!("{f} → {:?}", classify_layer(f)))
        .collect();
    assert!(wrong.is_empty(), "graded as some other layer:\n{}", wrong.join("\n"));
}

#[test]
fn the_adapters_module_root_holds_no_adapter() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("workspace root");
    let text = std::fs::read_to_string(root.join("hexa-infer/src/adapters/mod.rs")).unwrap();
    let code: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("//"))
        .collect();
    assert_eq!(code, ["pub mod secondary;"], "adapters/mod.rs must only nest the layer");
}
