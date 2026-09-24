//! Native tree-sitter adapter for import/export extraction.
//!
//! Implements `AstPort` using tree-sitter's Rust bindings with native grammar
//! libraries (no WASM). Supports TypeScript, Go, and Rust.
//!
//! ADR-034 Phase 2.

use std::path::Path;
use tree_sitter::{Language as TsLanguage, Node as TsNode, Parser, Tree};

use super::ports::{
    AnalysisError, AstPort, ExportDeclaration, ExportKind, ImportStatement, ItemCounts, Language,
};

// ── Grammar Loading ──────────────────────────────────────

fn get_language(lang: Language) -> Result<TsLanguage, AnalysisError> {
    match lang {
        Language::TypeScript => Ok(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        Language::Go => Ok(tree_sitter_go::LANGUAGE.into()),
        Language::Rust => Ok(tree_sitter_rust::LANGUAGE.into()),
        Language::Unknown => Err(AnalysisError::Other(
            "cannot parse files with unknown language".to_string(),
        )),
    }
}

pub(crate) fn parse_source(source: &str, lang: Language) -> Result<Tree, AnalysisError> {
    let ts_lang = get_language(lang)?;
    let mut parser = Parser::new();
    parser.set_language(&ts_lang).map_err(|e| AnalysisError::Other(e.to_string()))?;
    parser
        .parse(source, None)
        .ok_or_else(|| AnalysisError::Other("tree-sitter parse returned None".to_string()))
}

// ── Item counting (the layer inventory) ──────────────────
//
// What counts, per language:
//
// | | Rust | Go | TypeScript |
// |---|---|---|---|
// | interfaces | `trait` | `type X interface` | `interface` |
// | types | struct, enum, union | any other `type` spec | class, enum, type alias |
// | implementations | `impl Trait for T` | — (implicit, never declared) | each name in `implements` |
// | functions | free `fn` | `func` (not methods) | `function`, top-level arrow/function consts |
//
// Go's implementations are `None`, not `0`: satisfying an interface is never
// written down in Go. Rust items under `#[cfg(test)]` are test code.

/// Count the items `source` declares. `Language::Unknown` declares nothing.
fn count_items_in(source: &str, lang: Language) -> Result<ItemCounts, AnalysisError> {
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

fn named_children_of(n: TsNode<'_>) -> Vec<TsNode<'_>> {
    let mut cur = n.walk();
    n.named_children(&mut cur).collect()
}

/// Rust: walk item containers (the file, inline modules), skipping anything
/// under `#[cfg(test)]`. Trait and impl bodies are not descended into, so a
/// method is never a free function.
fn rust(n: TsNode<'_>, src: &[u8], c: &mut ItemCounts) {
    let mut cfg_test = false;
    for ch in named_children_of(n) {
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
fn go(root: TsNode<'_>, c: &mut ItemCounts) {
    for ch in named_children_of(root) {
        match ch.kind() {
            "function_declaration" => c.functions += 1,
            "type_declaration" => {
                for spec in named_children_of(ch) {
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
fn typescript(root: TsNode<'_>, c: &mut ItemCounts) {
    for ch in named_children_of(root) {
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

fn ts_declaration(d: TsNode<'_>, c: &mut ItemCounts) {
    match d.kind() {
        "interface_declaration" => c.interfaces += 1,
        "class_declaration" | "abstract_class_declaration" => {
            c.types += 1;
            let implemented = named_children_of(d)
                .into_iter()
                .filter(|h| h.kind() == "class_heritage")
                .flat_map(named_children_of)
                .filter(|h| h.kind() == "implements_clause")
                .map(|h| named_children_of(h).len())
                .sum::<usize>();
            *c.implementations.get_or_insert(0) += implemented;
        }
        "enum_declaration" | "type_alias_declaration" => c.types += 1,
        "function_declaration" | "generator_function_declaration" => c.functions += 1,
        "lexical_declaration" | "variable_declaration" => {
            for v in named_children_of(d).into_iter().filter(|v| v.kind() == "variable_declarator") {
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

// ── Adapter ──────────────────────────────────────────────

/// Native tree-sitter implementation of `AstPort`.
pub struct TreeSitterAdapter;

impl Default for TreeSitterAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl TreeSitterAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl AstPort for TreeSitterAdapter {
    fn count_items(&self, source: &str, lang: Language) -> Result<ItemCounts, AnalysisError> {
        count_items_in(source, lang)
    }

    fn extract_imports(
        &self,
        path: &Path,
        source: &str,
        lang: Language,
    ) -> Result<Vec<ImportStatement>, AnalysisError> {
        let tree = parse_source(source, lang)?;
        let root = tree.root_node();
        let from_file = path.to_string_lossy().to_string();

        match lang {
            Language::TypeScript => extract_ts_imports(&root, source, &from_file),
            Language::Go => extract_go_imports(&root, source, &from_file),
            Language::Rust => extract_rust_imports(&root, source, &from_file),
            Language::Unknown => Ok(vec![]),
        }
    }

    fn extract_exports(
        &self,
        path: &Path,
        source: &str,
        lang: Language,
    ) -> Result<Vec<ExportDeclaration>, AnalysisError> {
        let tree = parse_source(source, lang)?;
        let root = tree.root_node();
        let file = path.to_string_lossy().to_string();

        match lang {
            Language::TypeScript => extract_ts_exports(&root, source, &file),
            Language::Go => extract_go_exports(&root, source, &file),
            Language::Rust => extract_rust_exports(&root, source, &file),
            Language::Unknown => Ok(vec![]),
        }
    }

    fn extract_references(
        &self,
        _path: &Path,
        source: &str,
        lang: Language,
    ) -> Result<std::collections::HashMap<String, usize>, AnalysisError> {
        if lang == Language::Unknown {
            return Ok(std::collections::HashMap::new());
        }
        let tree = parse_source(source, lang)?;
        let mut out = std::collections::HashMap::new();
        collect_references(&tree.root_node(), source, &mut out);
        Ok(out)
    }

    fn extract_members(
        &self,
        _path: &Path,
        source: &str,
        lang: Language,
    ) -> Result<std::collections::HashMap<String, Vec<String>>, AnalysisError> {
        if lang == Language::Unknown {
            return Ok(std::collections::HashMap::new());
        }
        let tree = parse_source(source, lang)?;
        let mut out = std::collections::HashMap::new();
        collect_members(&tree.root_node(), source, &mut out);
        Ok(out)
    }
}

/// Trait and interface method names, by trait name, in all three grammars:
/// Rust `trait_item`, Go `type_spec` with an `interface_type`, TypeScript
/// `interface_declaration`.
fn collect_members(
    node: &tree_sitter::Node,
    source: &str,
    out: &mut std::collections::HashMap<String, Vec<String>>,
) {
    let owner = match node.kind() {
        "trait_item" | "interface_declaration" => node.child_by_field_name("name"),
        "type_spec" => {
            let is_interface = node
                .child_by_field_name("type")
                .map(|t| t.kind() == "interface_type")
                .unwrap_or(false);
            if is_interface { node.child_by_field_name("name") } else { None }
        }
        _ => None,
    };
    if let Some(name_node) = owner {
        let mut methods = Vec::new();
        collect_method_names(node, source, &mut methods);
        out.insert(node_text(name_node, source), methods);
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_members(&child, source, out);
    }
}

fn collect_method_names(node: &tree_sitter::Node, source: &str, out: &mut Vec<String>) {
    if matches!(
        node.kind(),
        "function_signature_item" | "function_item" | "method_elem" | "method_spec" | "method_signature"
    ) {
        if let Some(n) = node.child_by_field_name("name") {
            out.push(node_text(n, source));
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_method_names(&child, source, out);
    }
}

/// Every identifier leaf in the tree, counted by text. Comments and string
/// literals are not identifiers, so a name mentioned in prose does not count.
fn collect_references(
    node: &tree_sitter::Node,
    source: &str,
    out: &mut std::collections::HashMap<String, usize>,
) {
    if node.child_count() == 0 {
        if matches!(
            node.kind(),
            "identifier"
                | "type_identifier"
                | "field_identifier"
                | "property_identifier"
                | "shorthand_property_identifier"
                | "shorthand_property_identifier_pattern"
                | "package_identifier"
        ) {
            *out.entry(node_text(*node, source)).or_insert(0) += 1;
        }
        return;
    }
    // `mod foo;` declares a module; it does not name anything. Counting it
    // would make every Rust layer read as referenced from `lib.rs`.
    let skip_name = node.kind() == "mod_item";
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if skip_name && node.child_by_field_name("name").map(|n| n.id()) == Some(child.id()) {
            continue;
        }
        collect_references(&child, source, out);
    }
}

// ── TypeScript Import Extraction ─────────────────────────

fn extract_ts_imports(
    root: &tree_sitter::Node,
    source: &str,
    from_file: &str,
) -> Result<Vec<ImportStatement>, AnalysisError> {
    let mut imports = Vec::new();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        match child.kind() {
            // import { Foo, Bar } from './foo.js'
            // import type { Baz } from '../bar.js'
            // import * as ns from 'pkg'
            "import_statement" => {
                // `import pg = require("pg")` has no `source` field: the
                // specifier hangs off an `import_require_clause`. It was read
                // by neither half — not a declaration here, and skipped as an
                // `import_statement` by the reference collector — so it was the
                // one import form nothing saw. (ADR-2609221430 §4.)
                let specifier = child
                    .child_by_field_name("source")
                    .or_else(|| import_require_specifier(&child));
                if let Some(src) = specifier {
                    let raw_path = unquote(node_text(src, source));
                    let names = extract_ts_import_names(&child, source);
                    imports.push(ImportStatement {
                        from_file: from_file.to_string(),
                        raw_path: raw_path.clone(),
                        resolved_path: raw_path,
                        names,
                        line: child.start_position().row + 1,
                    });
                }
            }
            // export { Foo } from './foo.js' (re-exports count as imports for graph building)
            "export_statement" => {
                if let Some(src) = child.child_by_field_name("source") {
                    let raw_path = unquote(node_text(src, source));
                    let names = extract_ts_export_clause_names(&child, source);
                    imports.push(ImportStatement {
                        from_file: from_file.to_string(),
                        raw_path: raw_path.clone(),
                        resolved_path: raw_path,
                        names,
                        line: child.start_position().row + 1,
                    });
                }
            }
            // Dynamic import: const { X } = await import('./foo.js')
            // or: import('./foo.js').then(...)
            "expression_statement" | "lexical_declaration" | "variable_declaration" => {
                collect_dynamic_imports(&child, source, from_file, &mut imports);
            }
            _ => {}
        }
    }

    Ok(imports)
}

/// The `"pkg"` of an `import x = require("pkg")`, if this is one.
fn import_require_specifier<'a>(node: &tree_sitter::Node<'a>) -> Option<tree_sitter::Node<'a>> {
    let mut cursor = node.walk();
    let clause = node.children(&mut cursor).find(|c| c.kind() == "import_require_clause")?;
    let mut inner = clause.walk();
    let found = clause.children(&mut inner).find(|c| matches!(c.kind(), "string" | "template_string"));
    found
}

/// Extract imported names from an import statement clause.
/// `import { Foo, Bar } from '...'` → ["Foo", "Bar"]
/// `import * as ns from '...'` → ["*"]
/// `import Foo from '...'` → ["default"]
fn extract_ts_import_names(node: &tree_sitter::Node, source: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "import_clause" {
            let mut ic = child.walk();
            for part in child.children(&mut ic) {
                match part.kind() {
                    "identifier" => {
                        // default import: import Foo from '...'
                        names.push("default".to_string());
                    }
                    "named_imports" => {
                        let mut nc = part.walk();
                        for spec in part.children(&mut nc) {
                            if spec.kind() == "import_specifier" {
                                if let Some(name) = spec.child_by_field_name("name") {
                                    names.push(node_text(name, source));
                                }
                            }
                        }
                    }
                    "namespace_import" => {
                        names.push("*".to_string());
                    }
                    _ => {}
                }
            }
        }
    }
    names
}

/// Extract names from an export clause: `export { Foo, Bar } from '...'`
fn extract_ts_export_clause_names(node: &tree_sitter::Node, source: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "export_clause" {
            let mut ec = child.walk();
            for spec in child.children(&mut ec) {
                if spec.kind() == "export_specifier" {
                    if let Some(name) = spec.child_by_field_name("name") {
                        names.push(node_text(name, source));
                    }
                }
            }
        }
        // export * from '...'
        if child.kind() == "*" {
            names.push("*".to_string());
        }
    }
    names
}

/// Recursively search for dynamic `import('...')` calls.
fn collect_dynamic_imports(
    node: &tree_sitter::Node,
    source: &str,
    from_file: &str,
    imports: &mut Vec<ImportStatement>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call_expression" {
            // Check if this is import('...')
            if let Some(func) = child.child_by_field_name("function") {
                if func.kind() == "import" {
                    // Get the argument
                    if let Some(args) = child.child_by_field_name("arguments") {
                        let mut ac = args.walk();
                        for arg in args.children(&mut ac) {
                            if arg.kind() == "string" || arg.kind() == "template_string" {
                                let raw_path = unquote(node_text(arg, source));
                                imports.push(ImportStatement {
                                    from_file: from_file.to_string(),
                                    raw_path: raw_path.clone(),
                                    resolved_path: raw_path,
                                    names: vec!["*".to_string()],
                                    line: child.start_position().row + 1,
                                });
                            }
                        }
                    }
                }
            }
        }
        // Recurse into child nodes
        collect_dynamic_imports(&child, source, from_file, imports);
    }
}

// ── Go Import Extraction ─────────────────────────────────

fn extract_go_imports(
    root: &tree_sitter::Node,
    source: &str,
    from_file: &str,
) -> Result<Vec<ImportStatement>, AnalysisError> {
    let mut imports = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "import_declaration" {
            // Single import: import "fmt"
            // Grouped import: import ( "fmt"; "net/http" )
            collect_go_import_specs(&child, source, from_file, &mut imports);
        }
    }

    // A Go import names a package, not a symbol, so the import line alone
    // cannot say which port an adapter uses. The symbols are in the body, as
    // `ports.CounterPort` (a qualified_type) or `ports.New()` (a
    // selector_expression). Collect them by package identifier and attach
    // them to the import they qualify. An import with no qualified use in
    // the file keeps `["*"]`, which is what it was before and is still the
    // right reading of a side-effect import.
    //
    // Without this, every port in an imported package read as used, and a
    // port nothing referenced could not be reported. Found by the Go case of
    // the per-language unused_ports fixture (ADR-2609121400).
    let mut uses: std::collections::HashMap<String, std::collections::BTreeSet<String>> =
        std::collections::HashMap::new();
    collect_go_qualified_uses(root, source, &mut uses);
    for imp in &mut imports {
        let alias = go_import_alias(&imp.raw_path);
        if let Some(names) = uses.get(&alias) {
            if !names.is_empty() {
                imp.names = names.iter().cloned().collect();
            }
        }
    }
    Ok(imports)
}

/// The identifier a Go file uses to refer to an imported package: the last
/// path segment.
fn go_import_alias(raw_path: &str) -> String {
    raw_path.rsplit('/').next().unwrap_or(raw_path).to_string()
}

/// Every `pkg.Name` in the file, grouped by `pkg`.
fn collect_go_qualified_uses(
    node: &tree_sitter::Node,
    source: &str,
    uses: &mut std::collections::HashMap<String, std::collections::BTreeSet<String>>,
) {
    let kind = node.kind();
    if (kind == "qualified_type" || kind == "selector_expression") && node.child_count() == 3 {
        let left = node.child(0).unwrap();
        let right = node.child(2).unwrap();
        let left_ok = matches!(left.kind(), "package_identifier" | "identifier");
        let right_ok = matches!(right.kind(), "type_identifier" | "field_identifier" | "identifier");
        if left_ok && right_ok {
            uses.entry(node_text(left, source).to_string())
                .or_default()
                .insert(node_text(right, source).to_string());
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_go_qualified_uses(&child, source, uses);
    }
}

fn collect_go_import_specs(
    node: &tree_sitter::Node,
    source: &str,
    from_file: &str,
    imports: &mut Vec<ImportStatement>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_spec" => {
                if let Some(path_node) = child.child_by_field_name("path") {
                    let raw_path = unquote(node_text(path_node, source));
                    // Go imports are whole-package; names are resolved at usage
                    imports.push(ImportStatement {
                        from_file: from_file.to_string(),
                        raw_path: raw_path.clone(),
                        resolved_path: raw_path,
                        names: vec!["*".to_string()],
                        line: child.start_position().row + 1,
                    });
                }
            }
            "import_spec_list" => {
                // Recurse into grouped imports
                collect_go_import_specs(&child, source, from_file, imports);
            }
            _ => {}
        }
    }
}

// ── Rust Import Extraction ───────────────────────────────

fn extract_rust_imports(
    root: &tree_sitter::Node,
    source: &str,
    from_file: &str,
) -> Result<Vec<ImportStatement>, AnalysisError> {
    let mut imports = Vec::new();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        match child.kind() {
            "use_declaration" => {
                let text = node_text(child, source).trim().to_string();
                let path = text
                    .strip_prefix("use ")
                    .unwrap_or(&text)
                    .trim_end_matches(';')
                    .trim();

                if let Some(brace_idx) = path.find('{') {
                    // Grouped use: `use crate::core::{ports, domain};`
                    // Expand into one import per item in the group
                    let base = path[..brace_idx].trim_end_matches("::").trim();
                    let group = path[brace_idx + 1..]
                        .trim_end_matches('}')
                        .trim();
                    for item in group.split(',') {
                        let item = item.trim();
                        if item.is_empty() {
                            continue;
                        }
                        // Each item might be `Name` or `submod::Name`
                        let full_path = format!("{}::{}", base, item);
                        let name = item.rsplit("::").next().unwrap_or(item).to_string();
                        imports.push(ImportStatement {
                            from_file: from_file.to_string(),
                            raw_path: full_path.clone(),
                            resolved_path: full_path,
                            names: vec![name],
                            line: child.start_position().row + 1,
                        });
                    }
                } else {
                    // Simple use: `use crate::core::ports::IStatePort;`
                    let name = path.rsplit("::").next().unwrap_or(path).to_string();
                    imports.push(ImportStatement {
                        from_file: from_file.to_string(),
                        raw_path: path.to_string(),
                        resolved_path: path.to_string(),
                        names: vec![name],
                        line: child.start_position().row + 1,
                    });
                }
            }
            "mod_item" if !has_body(&child) => {
                // mod foo; (external module declaration, not inline mod foo { ... })
                if let Some(name_node) = child.child_by_field_name("name") {
                    let mod_name = node_text(name_node, source);
                    imports.push(ImportStatement {
                        from_file: from_file.to_string(),
                        raw_path: format!("self::{}", mod_name),
                        resolved_path: format!("self::{}", mod_name),
                        names: vec![mod_name.clone()],
                        line: child.start_position().row + 1,
                    });
                }
            }
            _ => {}
        }
    }

    Ok(imports)
}

/// Check if a mod_item has a body (inline module) vs just `mod foo;`
fn has_body(node: &tree_sitter::Node) -> bool {
    let mut cursor = node.walk();
    let result = node.children(&mut cursor)
        .any(|c| c.kind() == "declaration_list");
    result
}

// ── TypeScript Export Extraction ─────────────────────────

fn extract_ts_exports(
    root: &tree_sitter::Node,
    source: &str,
    file: &str,
) -> Result<Vec<ExportDeclaration>, AnalysisError> {
    let mut exports = Vec::new();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        if child.kind() != "export_statement" {
            continue;
        }
        // Skip re-exports (export { ... } from '...') — those are imports, not local exports
        if child.child_by_field_name("source").is_some() {
            continue;
        }

        let hexa_public = has_hex_public_annotation(&child, source);
        let line = child.start_position().row + 1;

        // Check for `export default`
        let text = node_text(child, source);
        if text.starts_with("export default") {
            exports.push(ExportDeclaration {
                file: file.to_string(),
                name: "default".to_string(),
                line,
                hexa_public,
                kind: ExportKind::Default,
            });
            continue;
        }

        // Find the declaration inside the export
        let mut inner_cursor = child.walk();
        for inner in child.children(&mut inner_cursor) {
            match inner.kind() {
                "function_declaration" | "function_signature" => {
                    if let Some(n) = inner.child_by_field_name("name").map(|n| node_text(n, source)) {
                        exports.push(ExportDeclaration { file: file.to_string(), name: n, line, hexa_public, kind: ExportKind::Function });
                    }
                }
                "class_declaration" | "abstract_class_declaration" => {
                    if let Some(n) = inner.child_by_field_name("name").map(|n| node_text(n, source)) {
                        exports.push(ExportDeclaration { file: file.to_string(), name: n, line, hexa_public, kind: ExportKind::Type });
                    }
                }
                "interface_declaration" => {
                    if let Some(n) = inner.child_by_field_name("name").map(|n| node_text(n, source)) {
                        exports.push(ExportDeclaration { file: file.to_string(), name: n, line, hexa_public, kind: ExportKind::Type });
                    }
                }
                "type_alias_declaration" => {
                    if let Some(n) = inner.child_by_field_name("name").map(|n| node_text(n, source)) {
                        exports.push(ExportDeclaration { file: file.to_string(), name: n, line, hexa_public, kind: ExportKind::Type });
                    }
                }
                "enum_declaration" => {
                    if let Some(n) = inner.child_by_field_name("name").map(|n| node_text(n, source)) {
                        exports.push(ExportDeclaration { file: file.to_string(), name: n, line, hexa_public, kind: ExportKind::Type });
                    }
                }
                "lexical_declaration" => {
                    // export const foo = ..., bar = ... → ALL names
                    for n in extract_lexical_names(&inner, source) {
                        exports.push(ExportDeclaration { file: file.to_string(), name: n, line, hexa_public, kind: ExportKind::Value });
                    }
                }
                _ => {}
            }
        }
    }

    Ok(exports)
}

fn extract_lexical_names(node: &tree_sitter::Node, source: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            if let Some(name_node) = child.child_by_field_name("name") {
                names.push(node_text(name_node, source));
            }
        }
    }
    names
}

// ── Go Export Extraction ─────────────────────────────────

fn extract_go_exports(
    root: &tree_sitter::Node,
    source: &str,
    file: &str,
) -> Result<Vec<ExportDeclaration>, AnalysisError> {
    let mut exports = Vec::new();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = node_text(name_node, source);
                    if is_go_exported(&name) {
                        exports.push(ExportDeclaration {
                            file: file.to_string(),
                            name,
                            line: child.start_position().row + 1,
                            hexa_public: false,
                            kind: ExportKind::Function,
                        });
                    }
                }
            }
            "method_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = node_text(name_node, source);
                    if is_go_exported(&name) {
                        exports.push(ExportDeclaration {
                            file: file.to_string(),
                            name,
                            line: child.start_position().row + 1,
                            hexa_public: false,
                            kind: ExportKind::Method,
                        });
                    }
                }
            }
            "type_declaration" => {
                // type Foo struct { ... } or type Bar interface { ... }
                // `type Count = domain.Count` is a `type_alias`, not a
                // `type_spec`; it is an exported type all the same, and the
                // ports layer re-exports domain types exactly this way.
                let mut tc = child.walk();
                for spec in child.children(&mut tc) {
                    if spec.kind() == "type_spec" || spec.kind() == "type_alias" {
                        if let Some(name_node) = spec.child_by_field_name("name") {
                            let name = node_text(name_node, source);
                            if is_go_exported(&name) {
                                exports.push(ExportDeclaration {
                                    file: file.to_string(),
                                    name,
                                    line: spec.start_position().row + 1,
                                    hexa_public: false,
                                    kind: ExportKind::Type,
                                });
                            }
                        }
                    }
                }
            }
            "const_declaration" | "var_declaration" => {
                let mut tc = child.walk();
                for spec in child.children(&mut tc) {
                    if spec.kind() == "const_spec" || spec.kind() == "var_spec" {
                        if let Some(name_node) = spec.child_by_field_name("name") {
                            let name = node_text(name_node, source);
                            if is_go_exported(&name) {
                                exports.push(ExportDeclaration {
                                    file: file.to_string(),
                                    name,
                                    line: spec.start_position().row + 1,
                                    hexa_public: false,
                                    kind: ExportKind::Value,
                                });
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    Ok(exports)
}

/// Go exports are identified by capitalized names.
fn is_go_exported(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_uppercase())
}

// ── Rust Export Extraction ───────────────────────────────

fn extract_rust_exports(
    root: &tree_sitter::Node,
    source: &str,
    file: &str,
) -> Result<Vec<ExportDeclaration>, AnalysisError> {
    let mut exports = Vec::new();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        // Only consider items with `pub` visibility (not pub(crate) or pub(super))
        if !is_rust_pub(&child, source) {
            continue;
        }

        let named = |field: &str| child.child_by_field_name(field).map(|n| node_text(n, source));
        let (name, kind) = match child.kind() {
            "function_item" => (named("name"), ExportKind::Function),
            "struct_item" | "enum_item" | "trait_item" | "type_item" => (named("name"), ExportKind::Type),
            "const_item" | "static_item" => (named("name"), ExportKind::Value),
            "impl_item" => (named("type"), ExportKind::Impl),
            _ => (None, ExportKind::Value),
        };

        if let Some(n) = name {
            let hexa_public = has_hex_public_annotation(&child, source);
            exports.push(ExportDeclaration {
                file: file.to_string(),
                name: n,
                line: child.start_position().row + 1,
                hexa_public,
                kind,
            });
        }
    }

    Ok(exports)
}

/// Check if a Rust item has unrestricted `pub` visibility.
/// Rejects `pub(crate)`, `pub(super)`, `pub(in ...)`.
fn is_rust_pub(node: &tree_sitter::Node, source: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "visibility_modifier" {
            let text = node_text(child, source);
            // Plain "pub" is public; "pub(crate)", "pub(super)" etc. are restricted
            return text.trim() == "pub";
        }
    }
    false
}

// ── Annotation Detection ─────────────────────────────────

/// Check if the node (or its preceding sibling comment) contains `@hexa:public`.
fn has_hex_public_annotation(node: &tree_sitter::Node, source: &str) -> bool {
    // Check preceding sibling for comment with @hexa:public
    if let Some(prev) = node.prev_sibling() {
        if prev.kind() == "comment" || prev.kind() == "line_comment" || prev.kind() == "block_comment" {
            let text = node_text(prev, source);
            if text.contains("@hexa:public") {
                return true;
            }
        }
    }
    false
}

// ── Module references that are not import declarations ───────────────────────

/// A place where source code names a module without an import line.
///
/// ADR-2609211600. `use std::fs;` and `std::fs::read("x")` are the same claim
/// about what a file depends on, and a policy that judged only the first let
/// the second walk past a `deny` that named it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleReference {
    /// The path as written, with any leading `::` removed.
    pub raw_path: String,
    /// 1-based line.
    pub line: usize,
    /// What the reference is, for the caller's own rules.
    pub kind: ReferenceKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferenceKind {
    /// A path naming a module: `std::fs::read`, `sqlx::PgPool`, `#[tokio::main]`.
    Path,
    /// `extern crate x;`
    ExternCrate,
    /// A module specifier in an expression: `require("x")`, `import("x")`.
    Specifier,
    /// A load whose name is not a literal, so nothing can be judged about it.
    /// `raw_path` is the expression as written.
    ComputedLoad,
}

/// Every module reference in `source` that is not an import declaration.
///
/// Raw extraction only: whether `util::f()` names a dependency or a local
/// module is the caller's question, because only the caller knows what the
/// project declares. Returning local paths here and filtering there keeps this
/// function testable against the grammar alone.
pub fn extract_module_references(
    source: &str,
    lang: Language,
) -> Result<Vec<ModuleReference>, AnalysisError> {
    // A parse failure used to return an empty list, which reads exactly like a
    // file with nothing in it. ADR-2609122048: a tool that reports "nothing
    // found" must prove it looked, so the caller is told instead.
    let tree = parse_source(source, lang)?;
    let mut out = Vec::new();
    match lang {
        Language::Rust => collect_rust_references(tree.root_node(), source, &mut out),
        Language::TypeScript => collect_ts_references(tree.root_node(), source, &mut out),
        // Go cannot name a package without importing it, so there is nothing
        // here that `extract_imports` has not already seen.
        Language::Go | Language::Unknown => {}
    }
    Ok(out)
}

fn collect_rust_references(
    node: tree_sitter::Node,
    source: &str,
    out: &mut Vec<ModuleReference>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // Already an import. `extract_imports` reports it, and reporting it
            // twice would charge the grade twice for one line.
            "use_declaration" => continue,
            "extern_crate_declaration" => {
                // `extern crate sqlx;` or `extern crate sqlx as db;` — the
                // first identifier is the crate, the alias is a local name.
                let text = node_text(child, source);
                if let Some(name) = text
                    .trim_start_matches("extern")
                    .trim_start()
                    .trim_start_matches("crate")
                    .trim_start()
                    .split(|c: char| c.is_whitespace() || c == ';')
                    .find(|t| !t.is_empty())
                {
                    out.push(ModuleReference {
                        raw_path: name.to_string(),
                        line: child.start_position().row + 1,
                        kind: ReferenceKind::ExternCrate,
                    });
                }
                continue;
            }
            // Inside a macro, every argument is one `token_tree` of raw tokens
            // with no `scoped_identifier` among them, so `println!("{:?}",
            // std::fs::read("x"))` named a denied module and nothing saw it.
            // (ADR-2609221430 §1.) Hand-written code passed to a macro is read;
            // code a macro *generates* is not, and stays a documented limit.
            "token_tree" => {
                collect_rust_token_tree(child, source, out);
                continue;
            }
            // The outermost scoped path is the whole reference. Descending
            // would report `std::fs` again inside `std::fs::read`.
            "scoped_identifier" | "scoped_type_identifier" => {
                let raw = node_text(child, source);
                let raw = raw.trim().trim_start_matches("::").to_string();
                // `<sqlx::PgPool as Default>::default` is a scoped_identifier
                // whose text begins with `<`, so its first segment read as
                // `<sqlx` and matched nothing. The type inside the brackets is
                // a reference in its own right, so walk it. (§5.)
                if raw.starts_with('<') {
                    collect_rust_references(child, source, out);
                    continue;
                }
                if !raw.is_empty() {
                    out.push(ModuleReference {
                        raw_path: raw,
                        line: child.start_position().row + 1,
                        kind: ReferenceKind::Path,
                    });
                }
                // A generic argument inside a type path is its own reference:
                // `Vec<sqlx::PgPool>` names sqlx. Those sit in a type_arguments
                // child, which is not part of the path text above.
                let mut inner = child.walk();
                for grandchild in child.children(&mut inner) {
                    if grandchild.kind() == "type_arguments" {
                        collect_rust_references(grandchild, source, out);
                    }
                }
                continue;
            }
            _ => {}
        }
        collect_rust_references(child, source, out);
    }
}

/// Paths written inside a macro's argument list.
///
/// A `token_tree` is unparsed: `std`, `::`, `fs`, `::`, `read` arrive as five
/// sibling tokens. A path is therefore a *run* — an optional leading `::`, an
/// identifier, then one or more `:: identifier` pairs. A lone identifier is not
/// a path, so `println!("{}", x)` contributes nothing.
///
/// Whether a run names a dependency or a local module is still the caller's
/// question: `util::v()` and `Ordering::Less` are emitted here and filtered
/// there, the same as every other reference.
fn collect_rust_token_tree(node: tree_sitter::Node, source: &str, out: &mut Vec<ModuleReference>) {
    fn is_path_segment(kind: &str) -> bool {
        // `crate`, `self` and `super` are their own token kinds and are legal
        // path heads. They resolve inside the project, which the caller knows
        // and this function does not.
        matches!(kind, "identifier" | "type_identifier" | "crate" | "self" | "super" | "metavariable")
    }

    let mut cursor = node.walk();
    let children: Vec<tree_sitter::Node> = node.children(&mut cursor).collect();
    let mut i = 0;
    while i < children.len() {
        // Macros nest: `println!("{:?}", vec![std::fs::read("x")])`.
        if children[i].kind() == "token_tree" {
            collect_rust_token_tree(children[i], source, out);
            i += 1;
            continue;
        }
        let mut j = i;
        if children[j].kind() == "::" {
            j += 1;
        }
        if j >= children.len() || !is_path_segment(children[j].kind()) {
            i += 1;
            continue;
        }
        let line = children[j].start_position().row + 1;
        let mut segments = vec![node_text(children[j], source)];
        let mut k = j + 1;
        while k + 1 < children.len()
            && children[k].kind() == "::"
            && is_path_segment(children[k + 1].kind())
        {
            segments.push(node_text(children[k + 1], source));
            k += 2;
        }
        if segments.len() >= 2 {
            out.push(ModuleReference {
                raw_path: segments.join("::"),
                line,
                kind: ReferenceKind::Path,
            });
        }
        // Always advance: a single identifier that began no path still moves
        // the cursor, or this loop does not terminate.
        i = if k > i { k } else { i + 1 };
    }
}

fn collect_ts_references(node: tree_sitter::Node, source: &str, out: &mut Vec<ModuleReference>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        // `import ... from "x"` is a declaration `extract_imports` already
        // reports, and it contains no call expression, so it is skipped by
        // shape rather than by name. An `export` is NOT skipped: skipping the
        // statement skipped the body of every exported function with it, and
        // `export async function f() { await import("pg") }` is exactly the
        // load this is looking for.
        if child.kind() == "import_statement" {
            continue;
        }
        if child.kind() == "call_expression" {
            if let Some(func) = child.child(0) {
                let name = node_text(func, source);
                let name = name.trim();
                if name == "require" || name == "import" || func.kind() == "import" {
                    if let Some(args) = child.child_by_field_name("arguments") {
                        let mut arg_cursor = args.walk();
                        let first = args
                            .children(&mut arg_cursor)
                            .find(|n| !matches!(n.kind(), "(" | ")" | ","));
                        let line = child.start_position().row + 1;
                        match first {
                            // A template with no `${...}` in it is a constant
                            // spelled with backticks. It used to be reported as
                            // a computed load, which named a real limit that did
                            // not apply and let a denied package through as a
                            // warning. (ADR-2609221430 §4.)
                            Some(n)
                                if n.kind() == "string"
                                    || (n.kind() == "template_string"
                                        && !has_substitution(n)) =>
                            {
                                out.push(ModuleReference {
                                    raw_path: unquote(node_text(n, source)),
                                    line,
                                    kind: ReferenceKind::Specifier,
                                });
                            }
                            Some(n) => {
                                // A name assembled at runtime. Saying nothing
                                // here would be the silent skip this ADR
                                // exists to stop.
                                out.push(ModuleReference {
                                    raw_path: node_text(n, source),
                                    line,
                                    kind: ReferenceKind::ComputedLoad,
                                });
                            }
                            None => {}
                        }
                    }
                }
            }
        }
        collect_ts_references(child, source, out);
    }
}

// ── Helpers ──────────────────────────────────────────────

/// Whether a `template_string` interpolates anything.
fn has_substitution(node: tree_sitter::Node) -> bool {
    let mut cursor = node.walk();
    let found = node.children(&mut cursor).any(|c| c.kind() == "template_substitution");
    found
}

fn node_text(node: tree_sitter::Node, source: &str) -> String {
    source[node.byte_range()].to_string()
}

/// Remove surrounding quotes from a string literal.
fn unquote(s: String) -> String {
    let trimmed = s.trim().to_string();
    if (trimmed.starts_with('"') && trimmed.ends_with('"'))
        || (trimmed.starts_with('\'') && trimmed.ends_with('\''))
        || (trimmed.starts_with('`') && trimmed.ends_with('`'))
    {
        trimmed[1..trimmed.len() - 1].to_string()
    } else {
        trimmed
    }
}

// ── Tests ────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter() -> TreeSitterAdapter {
        TreeSitterAdapter::new()
    }

    // ── TypeScript ───────────────────────────────────

    #[test]
    fn ts_import_extraction() {
        let source = r#"
import { Foo } from './foo.js';
import type { Bar } from '../bar.js';
import * as baz from 'baz';
"#;
        let imports = adapter()
            .extract_imports(Path::new("src/main.ts"), source, Language::TypeScript)
            .unwrap();
        assert_eq!(imports.len(), 3);
        assert_eq!(imports[0].raw_path, "./foo.js");
        assert_eq!(imports[1].raw_path, "../bar.js");
        assert_eq!(imports[2].raw_path, "baz");
    }

    #[test]
    fn ts_reexport_counted_as_import() {
        let source = r#"export { Foo } from './foo.js';"#;
        let imports = adapter()
            .extract_imports(Path::new("src/index.ts"), source, Language::TypeScript)
            .unwrap();
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw_path, "./foo.js");
    }

    #[test]
    fn ts_export_extraction() {
        let source = r#"
export function hello() {}
export class MyClass {}
export interface IPort {}
export type Alias = string;
export const VALUE = 42;
"#;
        let exports = adapter()
            .extract_exports(Path::new("src/lib.ts"), source, Language::TypeScript)
            .unwrap();
        let names: Vec<&str> = exports.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"hello"));
        assert!(names.contains(&"MyClass"));
        assert!(names.contains(&"IPort"));
        assert!(names.contains(&"Alias"));
        assert!(names.contains(&"VALUE"));
    }

    // ── Go ───────────────────────────────────────────

    #[test]
    fn go_import_extraction() {
        let source = r#"
package main

import (
    "fmt"
    "net/http"
    "github.com/org/repo/internal/domain"
)
"#;
        let imports = adapter()
            .extract_imports(Path::new("cmd/main.go"), source, Language::Go)
            .unwrap();
        assert_eq!(imports.len(), 3);
        assert_eq!(imports[0].raw_path, "fmt");
        assert_eq!(imports[1].raw_path, "net/http");
        assert_eq!(imports[2].raw_path, "github.com/org/repo/internal/domain");
    }

    #[test]
    fn go_export_extraction() {
        let source = r#"
package domain

func NewEntity() Entity { return Entity{} }
func helper() {}

type Entity struct {
    Name string
}

type privateType struct {}
"#;
        let exports = adapter()
            .extract_exports(Path::new("internal/domain/entity.go"), source, Language::Go)
            .unwrap();
        let names: Vec<&str> = exports.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"NewEntity"));
        assert!(names.contains(&"Entity"));
        assert!(!names.contains(&"helper"));
        assert!(!names.contains(&"privateType"));
    }

    // ── Rust ─────────────────────────────────────────

    #[test]
    fn rust_import_extraction() {
        let source = r#"
use crate::core::ports::IStatePort;
use std::sync::Arc;
use super::helpers;
mod submodule;
"#;
        let imports = adapter()
            .extract_imports(Path::new("src/adapters/primary/cli.rs"), source, Language::Rust)
            .unwrap();
        assert!(imports.len() >= 3);
        assert!(imports.iter().any(|i| i.raw_path.contains("crate::core::ports")));
        assert!(imports.iter().any(|i| i.raw_path.contains("std::sync::Arc")));
    }

    #[test]
    fn rust_export_extraction() {
        let source = r#"
pub fn public_fn() {}
fn private_fn() {}
pub struct MyStruct;
pub(crate) struct CrateOnly;
pub trait MyTrait {}
"#;
        let exports = adapter()
            .extract_exports(Path::new("src/domain/types.rs"), source, Language::Rust)
            .unwrap();
        let names: Vec<&str> = exports.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"public_fn"));
        assert!(names.contains(&"MyStruct"));
        assert!(names.contains(&"MyTrait"));
        assert!(!names.contains(&"private_fn"));
        assert!(!names.contains(&"CrateOnly"));
    }

    // ── Annotation ───────────────────────────────────

    #[test]
    fn ts_hex_public_annotation() {
        let source = r#"
// @hexa:public
export const INTERNAL_API = true;
export const NORMAL = false;
"#;
        let exports = adapter()
            .extract_exports(Path::new("src/ports/api.ts"), source, Language::TypeScript)
            .unwrap();
        let annotated = exports.iter().find(|e| e.name == "INTERNAL_API");
        assert!(annotated.is_some());
        assert!(annotated.unwrap().hexa_public);

        let normal = exports.iter().find(|e| e.name == "NORMAL");
        assert!(normal.is_some());
        assert!(!normal.unwrap().hexa_public);
    }

    // ── Parity tests (gap-closing) ───────────────────

    #[test]
    fn ts_import_names_extracted() {
        let source = r#"
import { Foo, Bar } from './types.js';
import * as ns from './utils.js';
import Default from './default.js';
"#;
        let imports = adapter()
            .extract_imports(Path::new("src/main.ts"), source, Language::TypeScript)
            .unwrap();
        assert_eq!(imports.len(), 3);
        // Named imports
        assert!(imports[0].names.contains(&"Foo".to_string()));
        assert!(imports[0].names.contains(&"Bar".to_string()));
        // Namespace import
        assert_eq!(imports[1].names, vec!["*"]);
        // Default import
        assert_eq!(imports[2].names, vec!["default"]);
    }

    #[test]
    fn ts_reexport_names_extracted() {
        let source = r#"export { Foo, Bar } from './types.js';"#;
        let imports = adapter()
            .extract_imports(Path::new("src/index.ts"), source, Language::TypeScript)
            .unwrap();
        assert_eq!(imports.len(), 1);
        assert!(imports[0].names.contains(&"Foo".to_string()));
        assert!(imports[0].names.contains(&"Bar".to_string()));
    }

    #[test]
    fn ts_export_default() {
        let source = r#"
export default class MyApp {}
"#;
        let exports = adapter()
            .extract_exports(Path::new("src/app.ts"), source, Language::TypeScript)
            .unwrap();
        assert!(exports.iter().any(|e| e.name == "default"));
    }

    #[test]
    fn ts_export_multiple_const() {
        let source = r#"export const FOO = 1, BAR = 2, BAZ = 3;"#;
        let exports = adapter()
            .extract_exports(Path::new("src/config.ts"), source, Language::TypeScript)
            .unwrap();
        let names: Vec<&str> = exports.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"FOO"));
        assert!(names.contains(&"BAR"));
        assert!(names.contains(&"BAZ"));
    }

    #[test]
    fn ts_dynamic_import() {
        let source = r#"
const mod = await import('./dynamic.js');
"#;
        let imports = adapter()
            .extract_imports(Path::new("src/loader.ts"), source, Language::TypeScript)
            .unwrap();
        assert!(imports.iter().any(|i| i.raw_path == "./dynamic.js"));
        // Dynamic imports are namespace-like
        let dynamic = imports.iter().find(|i| i.raw_path == "./dynamic.js").unwrap();
        assert!(dynamic.names.contains(&"*".to_string()));
    }

    #[test]
    fn rust_grouped_use_expanded() {
        let source = r#"
use crate::core::{ports, domain};
use std::collections::{HashMap, HashSet};
"#;
        let imports = adapter()
            .extract_imports(Path::new("src/lib.rs"), source, Language::Rust)
            .unwrap();
        // Should expand into individual imports
        assert!(imports.iter().any(|i| i.names.contains(&"ports".to_string())));
        assert!(imports.iter().any(|i| i.names.contains(&"domain".to_string())));
        assert!(imports.iter().any(|i| i.names.contains(&"HashMap".to_string())));
        assert!(imports.iter().any(|i| i.names.contains(&"HashSet".to_string())));
        assert!(imports.len() >= 4);
    }

    #[test]
    fn rust_simple_use_names() {
        let source = r#"use crate::core::ports::IStatePort;"#;
        let imports = adapter()
            .extract_imports(Path::new("src/adapters/cli.rs"), source, Language::Rust)
            .unwrap();
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].names, vec!["IStatePort"]);
    }

    #[test]
    fn go_import_names_are_wildcard() {
        let source = r#"
package main
import "fmt"
"#;
        let imports = adapter()
            .extract_imports(Path::new("main.go"), source, Language::Go)
            .unwrap();
        assert_eq!(imports.len(), 1);
        // Go imports are whole-package, so names are wildcard
        assert_eq!(imports[0].names, vec!["*"]);
    }
}
