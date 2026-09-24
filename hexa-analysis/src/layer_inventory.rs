//! Layer inventory — what each hexagonal layer holds, per language.
//!
//! The grade says whether the layers are kept apart. This says what is in
//! them: interfaces, types, implementations and functions, counted from the
//! syntax tree, over the same files the grade reads, with each file's
//! language taken from `Language::from_path` and its layer from
//! `classify_layer` — the detector and classifier the analysis itself uses,
//! so the two can never describe different code.
//!
//! What counts, per language:
//!
//! | | Rust | Go | TypeScript |
//! |---|---|---|---|
//! | interfaces | `trait` | `type X interface` | `interface` |
//! | types | struct, enum, union | any other `type` spec | class, enum, type alias |
//! | implementations | `impl Trait for T` | — (implicit, never declared) | each name in `implements` |
//! | functions | free `fn` | `func` (not methods) | `function`, top-level arrow/function consts |
//!
//! Go's implementations are `None`, not `0`: satisfying an interface is never
//! written down in Go, so a syntax count of zero would be a false statement.
//! Rust items under `#[cfg(test)]` are test code and are not counted.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;
use crate::analyzer::collect_source_files;
use crate::domain::{HexLayer, ItemCounts, Language};
use crate::layer_classifier::LayerMap;
use crate::ports::{AnalysisError, AstPort};

/// One row: a language's files in one layer, and what they declare.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InventoryRow {
    pub language: Language,
    pub layer: String,
    pub files: usize,
    #[serde(flatten)]
    pub counts: ItemCounts,
}

/// Inventory the project at `root`: one row per (language, layer) that has
/// at least one file, in layer order then language order.
pub async fn inventory(root: &Path, ast: &dyn AstPort) -> Result<Vec<InventoryRow>, AnalysisError> {
    let layers = LayerMap::from_project(root).map_err(AnalysisError::Other)?;
    let mut cells: BTreeMap<(usize, usize), (Language, usize, ItemCounts)> = BTreeMap::new();
    for rel in collect_source_files(root).await? {
        let lang = Language::from_path(&rel);
        if lang == Language::Unknown {
            continue;
        }
        let Ok(source) = tokio::fs::read_to_string(root.join(&rel)).await else { continue };
        let counts = ast.count_items(&source, lang)?;
        let cell = cells
            .entry((layer_rank(layers.classify(&rel)), lang as usize))
            .or_insert((lang, 0, ItemCounts { implementations: counts.implementations.map(|_| 0), ..Default::default() }));
        cell.1 += 1;
        cell.2.add(&counts);
    }
    Ok(cells
        .into_iter()
        .map(|((rank, _), (language, files, counts))| InventoryRow {
            language,
            layer: LAYER_ORDER[rank].to_string(),
            files,
            counts,
        })
        .collect())
}

/// Inside-out: the order a reader walks a hexagon.
const LAYER_ORDER: [HexLayer; 9] = [
    HexLayer::Domain,
    HexLayer::Ports,
    HexLayer::Usecases,
    HexLayer::AdaptersPrimary,
    HexLayer::AdaptersSecondary,
    HexLayer::Infrastructure,
    HexLayer::CompositionRoot,
    HexLayer::EntryPoint,
    HexLayer::Unknown,
];

fn layer_rank(l: HexLayer) -> usize {
    LAYER_ORDER.iter().position(|x| *x == l).unwrap_or(LAYER_ORDER.len() - 1)
}
