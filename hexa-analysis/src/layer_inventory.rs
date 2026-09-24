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
use tree_sitter::Node;

use crate::analyzer::collect_source_files;
use crate::domain::{HexLayer, Language};
use crate::layer_classifier::classify_layer;
use crate::ports::AnalysisError;
use crate::treesitter_adapter::parse_source;

/// What one file, or one (language, layer) cell, declares.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ItemCounts {
    pub interfaces: usize,
    pub types: usize,
    /// `None` where the language has no syntax for it (Go).
    pub implementations: Option<usize>,
    pub functions: usize,
}

impl ItemCounts {
    fn add(&mut self, o: &ItemCounts) {
        self.interfaces += o.interfaces;
        self.types += o.types;
        self.implementations = match (self.implementations, o.implementations) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
        };
        self.functions += o.functions;
    }
}

/// One row: a language's files in one layer, and what they declare.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InventoryRow {
    pub language: Language,
    pub layer: String,
    pub files: usize,
    #[serde(flatten)]
    pub counts: ItemCounts,
}

/// Count the items `source` declares. `Language::Unknown` declares nothing.
fn count_items(source: &str, lang: Language) -> Result<ItemCounts, AnalysisError> {
    if lang == Language::Unknown {
        return Ok(ItemCounts::default());
    }
    let tree = parse_source(source, lang)?;
    let src = source.as_bytes();
    let mut c = ItemCounts {
        implementations: if lang == Language::Go { None } else { Some(0) },
        ..ItemCounts::default()
    };
    match lang {
        Language::Rust => rust(tree.root_node(), src, &mut c),
        Language::Go => go(tree.root_node(), &mut c),
        Language::TypeScript => typescript(tree.root_node(), &mut c),
        Language::Unknown => {}
    }
    Ok(c)
}

fn children(n: Node<'_>) -> Vec<Node<'_>> {
    let mut cur = n.walk();
    n.named_children(&mut cur).collect()
}

/// Rust: walk item containers (the file, inline modules), skipping anything
/// under `#[cfg(test)]`. Trait and impl bodies are not descended into, so a
/// method is never a free function.
fn rust(n: Node<'_>, src: &[u8], c: &mut ItemCounts) {
    let mut cfg_test = false;
    for ch in children(n) {
        if ch.kind() == "attribute_item" {
            let t = ch.utf8_text(src).unwrap_or("");
            cfg_test |= t.replace(' ', "").starts_with("#[cfg(test)");
            continue;
        }
        if std::mem::take(&mut cfg_test) {
            continue;
        }
        match ch.kind() {
            "trait_item" => c.interfaces += 1,
            "struct_item" | "enum_item" | "union_item" => c.types += 1,
            "impl_item" => {
                if ch.child_by_field_name("trait").is_some() {
                    *c.implementations.get_or_insert(0) += 1;
                }
            }
            "function_item" => c.functions += 1,
            "mod_item" => {
                if let Some(body) = ch.child_by_field_name("body") {
                    rust(body, src, c);
                }
            }
            _ => {}
        }
    }
}

/// Go: top-level declarations only; Go has no nested ones worth counting.
fn go(root: Node<'_>, c: &mut ItemCounts) {
    for ch in children(root) {
        match ch.kind() {
            "function_declaration" => c.functions += 1,
            "type_declaration" => {
                for spec in children(ch) {
                    if !matches!(spec.kind(), "type_spec" | "type_alias") {
                        continue;
                    }
                    match spec.child_by_field_name("type").map(|t| t.kind()) {
                        Some("interface_type") => c.interfaces += 1,
                        _ => c.types += 1,
                    }
                }
            }
            _ => {}
        }
    }
}

/// TypeScript: top-level declarations, exported or not. A function declared
/// inside another is part of it, not one of the module's.
fn typescript(root: Node<'_>, c: &mut ItemCounts) {
    for ch in children(root) {
        let decl = if ch.kind() == "export_statement" {
            match ch.child_by_field_name("declaration") {
                Some(d) => d,
                None => continue,
            }
        } else {
            ch
        };
        ts_declaration(decl, c);
    }
}

fn ts_declaration(d: Node<'_>, c: &mut ItemCounts) {
    match d.kind() {
        "interface_declaration" => c.interfaces += 1,
        "class_declaration" | "abstract_class_declaration" => {
            c.types += 1;
            let implemented = children(d)
                .into_iter()
                .filter(|h| h.kind() == "class_heritage")
                .flat_map(children)
                .filter(|h| h.kind() == "implements_clause")
                .map(|h| children(h).len())
                .sum::<usize>();
            *c.implementations.get_or_insert(0) += implemented;
        }
        "enum_declaration" | "type_alias_declaration" => c.types += 1,
        "function_declaration" | "generator_function_declaration" => c.functions += 1,
        "lexical_declaration" | "variable_declaration" => {
            for v in children(d).into_iter().filter(|v| v.kind() == "variable_declarator") {
                if let Some(val) = v.child_by_field_name("value") {
                    if matches!(val.kind(), "arrow_function" | "function_expression" | "function") {
                        c.functions += 1;
                    }
                }
            }
        }
        _ => {}
    }
}

/// Inventory the project at `root`: one row per (language, layer) that has
/// at least one file, in layer order then language order.
pub async fn inventory(root: &Path) -> Result<Vec<InventoryRow>, AnalysisError> {
    let mut cells: BTreeMap<(usize, usize), (Language, usize, ItemCounts)> = BTreeMap::new();
    for rel in collect_source_files(root).await? {
        let lang = Language::from_path(&rel);
        if lang == Language::Unknown {
            continue;
        }
        let Ok(source) = tokio::fs::read_to_string(root.join(&rel)).await else { continue };
        let counts = count_items(&source, lang)?;
        let cell = cells
            .entry((layer_rank(classify_layer(&rel)), lang as usize))
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
