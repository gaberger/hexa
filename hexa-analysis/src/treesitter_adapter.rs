//! Native tree-sitter adapter for import/export extraction.
//!
//! Implements `AstPort` using tree-sitter's Rust bindings with native grammar
//! libraries (no WASM). Supports TypeScript, Go, and Rust.
//!
//! ADR-034 Phase 2.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use tree_sitter::{Language as TsLanguage, Node as TsNode, Parser, Tree};

use super::ports::{
    AnalysisError, ApiFacts, AstPort, ExportDeclaration, ExportKind, ImportStatement, ItemCounts, Language,
    ModuleReference, ReferenceKind,
};

// ── Grammar Loading ──────────────────────────────────────

fn get_language(lang: Language) -> Result<TsLanguage, AnalysisError> {
    match lang {
        Language::TypeScript => Ok(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        Language::Go => Ok(tree_sitter_go::LANGUAGE.into()),
        Language::Rust => Ok(tree_sitter_rust::LANGUAGE.into()),
        Language::Python => Ok(tree_sitter_python::LANGUAGE.into()),
        Language::Unknown => Err(AnalysisError::Other(
            "cannot parse files with unknown language".to_string(),
        )),
    }
}

// ── Python (ADR-2611211324) ─────────────────────────────

/// Every import in a Python file, at any depth: an import under
/// `if TYPE_CHECKING:` or inside a function is still knowledge of the module
/// (§2). `import a.b` is `raw_path` "a.b" with no names; `from M import x, y`
/// is `raw_path` M as written (".ports", "..", "a.b") with the names it takes.
/// Which file a module is depends on the files there are, so resolution is
/// the analyzer's (`path_normalizer::resolve_python_import`).
fn extract_python_imports(root: &tree_sitter::Node, source: &str, from_file: &str) -> Result<Vec<ImportStatement>, AnalysisError> {
    let mut out = Vec::new();
    let mut stack = vec![*root];
    while let Some(n) = stack.pop() {
        match n.kind() {
            "import_statement" => {
                let mut cur = n.walk();
                for name in n.children_by_field_name("name", &mut cur) {
                    let module = match name.kind() {
                        "aliased_import" => name.child_by_field_name("name").map(|m| node_text(m, source)),
                        _ => Some(node_text(name, source)),
                    };
                    if let Some(module) = module {
                        out.push(ImportStatement {
                            from_file: from_file.to_string(),
                            raw_path: module.clone(),
                            resolved_path: module,
                            names: Vec::new(),
                            line: n.start_position().row + 1,
                        });
                    }
                }
            }
            "import_from_statement" => {
                let Some(module) = n.child_by_field_name("module_name").map(|m| node_text(m, source)) else { continue };
                let mut names = Vec::new();
                let mut cur = n.walk();
                for name in n.children_by_field_name("name", &mut cur) {
                    let taken = match name.kind() {
                        "aliased_import" => name.child_by_field_name("name").map(|m| node_text(m, source)),
                        _ => Some(node_text(name, source)),
                    };
                    names.extend(taken);
                }
                let mut cur = n.walk();
                if n.named_children(&mut cur).any(|c| c.kind() == "wildcard_import") {
                    names.push("*".to_string());
                }
                out.push(ImportStatement {
                    from_file: from_file.to_string(),
                    raw_path: module.clone(),
                    resolved_path: module,
                    names,
                    line: n.start_position().row + 1,
                });
            }
            _ => {
                let mut cur = n.walk();
                stack.extend(n.named_children(&mut cur));
            }
        }
    }
    out.sort_by_key(|i| i.line);
    Ok(out)
}

/// The top-level definition a statement makes: a decorated definition is its
/// definition.
fn python_definition(n: TsNode<'_>) -> TsNode<'_> {
    if n.kind() == "decorated_definition" {
        n.child_by_field_name("definition").unwrap_or(n)
    } else {
        n
    }
}

/// The names a top-level `x = …` or `x: T = …` assigns, with the assignment.
fn python_assignments<'a>(n: TsNode<'a>, source: &str) -> Vec<(String, TsNode<'a>)> {
    if n.kind() != "expression_statement" {
        return Vec::new();
    }
    named_children_of(n)
        .into_iter()
        .filter(|a| a.kind() == "assignment")
        .filter_map(|a| {
            let left = a.child_by_field_name("left")?;
            (left.kind() == "identifier").then(|| (node_text(left, source), a))
        })
        .collect()
}

/// A module's exports (§3): its `__all__` when it assigns one, else every
/// top-level `def`, `class` and assignment whose name does not start with `_`.
fn extract_python_exports(root: &tree_sitter::Node, source: &str, file: &str) -> Result<Vec<ExportDeclaration>, AnalysisError> {
    let lines: Vec<&str> = source.split('\n').collect();
    let mut defined: Vec<(String, usize, ExportKind, bool)> = Vec::new();
    let mut all: Option<(Vec<(String, usize)>, bool)> = None;
    for ch in named_children_of(*root) {
        let line = ch.start_position().row + 1;
        // A decorated definition starts at its first decorator, so its
        // comment run is the one above that (ADR-2611221644 §1); a comment
        // ending its own `def`/`class` line tags it as it would undecorated.
        let d = python_definition(ch);
        let tagged = python_comment_tagged(root, &lines, ch.start_position().row)
            || python_line_tagged(root, &lines, d.start_position().row);
        match d.kind() {
            "function_definition" => {
                if let Some(n) = d.child_by_field_name("name") {
                    defined.push((node_text(n, source), line, ExportKind::Function, tagged));
                }
            }
            "class_definition" => {
                if let Some(n) = d.child_by_field_name("name") {
                    defined.push((node_text(n, source), line, ExportKind::Type, tagged));
                }
            }
            _ => {
                for (name, a) in python_assignments(d, source) {
                    if name == "__all__" {
                        let listed = a
                            .child_by_field_name("right")
                            .filter(|r| matches!(r.kind(), "list" | "tuple"))
                            .map(|r| {
                                named_children_of(r)
                                    .into_iter()
                                    .filter(|s| s.kind() == "string")
                                    .map(|s| (node_text(s, source).trim_matches(|c| c == '"' || c == '\'').to_string(), line))
                                    .collect()
                            })
                            .unwrap_or_default();
                        // A tag above `__all__` marks every name it lists (§2).
                        all = Some((listed, tagged));
                    } else {
                        defined.push((name, line, ExportKind::Value, tagged));
                    }
                }
            }
        }
    }
    let export = |name: String, line: usize, kind: ExportKind, hexa_public: bool| ExportDeclaration { file: file.to_string(), name, line, hexa_public, kind };
    Ok(match all {
        Some((listed, all_tagged)) => listed
            .into_iter()
            .map(|(name, line)| {
                let found = defined.iter().find(|(n, _, _, _)| *n == name);
                let (line, kind, tagged) = found.map(|(_, l, k, t)| (*l, *k, *t)).unwrap_or((line, ExportKind::Value, false));
                export(name, line, kind, all_tagged || tagged)
            })
            .collect(),
        None => defined.into_iter().filter(|(n, _, _, _)| !n.starts_with('_')).map(|(n, l, k, t)| export(n, l, k, t)).collect(),
    })
}

/// The public top-level names `source` uses outside their own definitions, or
/// none when it assigns `__all__` (ADR-2611251938 §2). A use is an identifier
/// that reads the module's name: not an attribute or keyword of that spelling,
/// not a binding, not a local that shadows it. Comments and strings hold no
/// identifiers; an f-string's `{…}` is code, and counts.
fn python_home_uses(root: TsNode<'_>, source: &str) -> HashSet<String> {
    // Each public name, with the byte ranges of the statements defining it.
    let mut defined: HashMap<String, Vec<std::ops::Range<usize>>> = HashMap::new();
    for ch in named_children_of(root) {
        let d = python_definition(ch);
        let names: Vec<String> = match d.kind() {
            "function_definition" | "class_definition" => {
                d.child_by_field_name("name").map(|n| node_text(n, source)).into_iter().collect()
            }
            _ => python_assignments(d, source).into_iter().map(|(n, _)| n).collect(),
        };
        for name in names {
            if name == "__all__" {
                return HashSet::new();
            }
            if !name.starts_with('_') {
                defined.entry(name).or_default().push(ch.byte_range());
            }
        }
    }
    let mut used = HashSet::new();
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        if n.kind() == "identifier" {
            let name = node_text(n, source);
            let outside = defined.get(&name).is_some_and(|at| !at.iter().any(|r| r.contains(&n.start_byte())));
            if outside && python_reads(n) && !python_shadowed(n, &name, source) {
                used.insert(name);
            }
            continue;
        }
        if matches!(n.kind(), "import_statement" | "import_from_statement" | "global_statement" | "nonlocal_statement") {
            continue;
        }
        let mut cur = n.walk();
        stack.extend(n.named_children(&mut cur));
    }
    used
}

/// Whether the identifier `n` reads a name, rather than naming an attribute, a
/// keyword argument, a definition, a parameter or the target of a binding.
fn python_reads(n: TsNode<'_>) -> bool {
    let Some(parent) = n.parent() else { return true };
    let is_field = |field: &str| parent.child_by_field_name(field).is_some_and(|f| f.id() == n.id());
    match parent.kind() {
        "attribute" => !is_field("attribute"),
        "keyword_argument" | "function_definition" | "class_definition" => !is_field("name"),
        _ => !python_binds(n),
    }
}

/// Whether the identifier `n` is bound where it stands: a parameter, or a
/// target of an assignment, a loop, `as` or `:=`.
fn python_binds(n: TsNode<'_>) -> bool {
    let Some(parent) = n.parent() else { return false };
    let is_field = |field: &str| parent.child_by_field_name(field).is_some_and(|f| f.id() == n.id());
    match parent.kind() {
        "parameters" | "lambda_parameters" | "as_pattern_target" => true,
        "typed_parameter" => !is_field("type"),
        "default_parameter" | "typed_default_parameter" | "named_expression" => is_field("name"),
        "assignment" | "for_statement" | "for_in_clause" => is_field("left"),
        // `a, b = …`, `(a, b) = …`, `*args`: bound if the whole pattern is.
        "pattern_list" | "tuple_pattern" | "list_pattern" | "list_splat_pattern" | "dictionary_splat_pattern" => python_binds(parent),
        _ => false,
    }
}

/// Whether `name` at `n` is a local of a function, lambda or comprehension
/// around it, and so not the module's.
fn python_shadowed(n: TsNode<'_>, name: &str, source: &str) -> bool {
    let mut at = n;
    while let Some(scope) = at.parent() {
        let is_scope = matches!(
            scope.kind(),
            "function_definition" | "lambda" | "list_comprehension" | "set_comprehension" | "dictionary_comprehension" | "generator_expression"
        );
        // A default or an annotation is read outside the function it is on.
        let in_header = scope.kind() == "function_definition"
            && scope.child_by_field_name("body").is_some_and(|b| !b.byte_range().contains(&n.start_byte()));
        if is_scope && !in_header && python_scope_binds(scope, name, source) {
            return true;
        }
        at = scope;
    }
    false
}

/// Whether the scope binds `name` as its own local. `global name` says it does
/// not.
fn python_scope_binds(scope: TsNode<'_>, name: &str, source: &str) -> bool {
    let mut bound = false;
    let mut stack = named_children_of(scope);
    while let Some(n) = stack.pop() {
        match n.kind() {
            "global_statement" if named_children_of(n).iter().any(|i| node_text(*i, source) == name) => return false,
            // A nested definition binds its own name here, and nothing in it.
            "function_definition" | "class_definition" => {
                bound |= n.child_by_field_name("name").is_some_and(|i| node_text(i, source) == name);
            }
            // So does a lambda or a comprehension: its names are its own.
            "lambda" | "list_comprehension" | "set_comprehension" | "dictionary_comprehension" | "generator_expression" => {}
            "import_statement" | "import_from_statement" => bound |= python_import_binds(n, name, source),
            "identifier" => bound |= node_text(n, source) == name && python_binds(n),
            _ => stack.extend(named_children_of(n)),
        }
    }
    bound
}

/// Whether the import statement `n` binds `name`: the alias, else the name it
/// takes, else the first part of the module `import a.b` names.
fn python_import_binds(n: TsNode<'_>, name: &str, source: &str) -> bool {
    let mut cur = n.walk();
    let bound = n.children_by_field_name("name", &mut cur).any(|item| {
        let text = match item.child_by_field_name("alias") {
            Some(alias) => node_text(alias, source),
            None => node_text(item, source),
        };
        text.split('.').next() == Some(name)
    });
    bound
}

/// Whether the run of `#` comment lines directly above `row` carries
/// `@hexa:public` (ADR-2611221644 §1). Each line of the run holds only a
/// comment, so a blank line or a code line ends it, and a `#` comment that
/// ends a code line belongs to that line. Lines are read from the source:
/// tree-sitter-python can file a column-0 comment inside the block before
/// it, so siblings would miss it. Each `#` must start a comment node, not
/// sit inside a string. A comment ending the item's own first line is the
/// item's too.
fn python_comment_tagged(root: &tree_sitter::Node, lines: &[&str], row: usize) -> bool {
    if python_line_tagged(root, lines, row) {
        return true;
    }
    let mut r = row;
    while r > 0 {
        r -= 1;
        let line = lines.get(r).copied().unwrap_or("");
        let col = line.len() - line.trim_start().len();
        if !line[col..].starts_with('#') {
            return false;
        }
        let at = tree_sitter::Point { row: r, column: col };
        if root.descendant_for_point_range(at, at).is_none_or(|n| n.kind() != "comment") {
            return false;
        }
        if line.contains("@hexa:public") {
            return true;
        }
    }
    false
}

/// Whether the `#` comment that ends line `row` carries `@hexa:public`: the
/// first `#` that starts a comment node, not one inside a string.
fn python_line_tagged(root: &tree_sitter::Node, lines: &[&str], row: usize) -> bool {
    let own = lines.get(row).copied().unwrap_or("");
    for (col, _) in own.match_indices('#') {
        let at = tree_sitter::Point { row, column: col };
        if root.descendant_for_point_range(at, at).is_some_and(|n| n.kind() == "comment") {
            return own[col..].contains("@hexa:public");
        }
    }
    false
}

/// Counts for the layer inventory: a class whose bases name `Protocol` or
/// `ABC` is an interface, any other class a type, and one with other bases
/// an implementation; top-level functions are functions.
fn python(root: TsNode<'_>, src: &str, c: &mut ItemCounts) {
    for ch in named_children_of(root) {
        let d = python_definition(ch);
        match d.kind() {
            "function_definition" => c.functions += 1,
            "class_definition" => {
                let bases: Vec<String> = d
                    .child_by_field_name("superclasses")
                    .map(|s| named_children_of(s).into_iter().map(|b| node_text(b, src)).collect())
                    .unwrap_or_default();
                let abstract_base = |b: &String| {
                    let last = b.rsplit('.').next().unwrap_or(b);
                    matches!(last, "Protocol" | "ABC") || last.starts_with("Protocol[")
                };
                if bases.iter().any(abstract_base) {
                    c.interfaces += 1;
                } else {
                    c.types += 1;
                    if bases.iter().any(|b| !b.starts_with("metaclass") && b != "object") {
                        *c.implementations.get_or_insert(0) += 1;
                    }
                }
            }
            _ => {}
        }
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
        Language::Python => python(tree.root_node(), source, &mut c),
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
    fn module_references(&self, source: &str, lang: Language) -> Result<Vec<ModuleReference>, AnalysisError> {
        extract_module_references(source, lang)
    }

    fn module_paths(
        &self,
        path: &Path,
        source: &str,
        lang: Language,
    ) -> Result<Vec<ImportStatement>, AnalysisError> {
        if lang != Language::Rust {
            return Ok(Vec::new());
        }
        let tree = parse_source(source, lang)?;
        Ok(extract_rust_module_paths(&tree.root_node(), source, &path.to_string_lossy()))
    }

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
            Language::Python => extract_python_imports(&root, source, &from_file),
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
            Language::Python => extract_python_exports(&root, source, &file),
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

    fn extract_api(&self, _path: &Path, source: &str, lang: Language) -> Result<ApiFacts, AnalysisError> {
        super::treesitter_api::extract(source, lang)
    }

    fn names_used_at_home(&self, source: &str, lang: Language) -> Result<HashSet<String>, AnalysisError> {
        if lang != Language::Python {
            return Ok(HashSet::new());
        }
        let tree = parse_source(source, lang)?;
        Ok(python_home_uses(tree.root_node(), source))
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

    // CommonJS `require("x")`, wherever it is written (ADR-2611270420 §2).
    collect_require_calls(root, source, from_file, &mut imports);

    Ok(imports)
}

/// Every `require("literal")` call under `node`, in document order: an import
/// edge to what the string names. A call whose argument is anything but one
/// plain string (`require(name)`, `require("./" + dir)`) names no file hexa
/// can see and is not guessed at (ADR-2611270420 §2).
fn collect_require_calls(node: &tree_sitter::Node, source: &str, from_file: &str, imports: &mut Vec<ImportStatement>) {
    if node.kind() == "call_expression" {
        if let Some(func) = node.child_by_field_name("function") {
            if func.kind() == "identifier" && node_text(func, source) == "require" {
                if let Some(args) = node.child_by_field_name("arguments") {
                    let mut ac = args.walk();
                    let named: Vec<_> = args.named_children(&mut ac).collect();
                    if let [arg] = named.as_slice() {
                        if arg.kind() == "string" {
                            let raw_path = unquote(node_text(*arg, source));
                            imports.push(ImportStatement {
                                from_file: from_file.to_string(),
                                raw_path: raw_path.clone(),
                                resolved_path: raw_path,
                                names: require_names(node, source),
                                line: node.start_position().row + 1,
                            });
                        }
                    }
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_require_calls(&child, source, from_file, imports);
    }
}

/// The names a `require(...)` call takes from the module: `const { a, b } =
/// require(..)` names `a` and `b`; `require(..).a` names `a`; anything else
/// (`const m = require(..)`, a bare call) takes the module whole, as an
/// `import * as` does.
fn require_names(call: &tree_sitter::Node, source: &str) -> Vec<String> {
    let whole = || vec!["*".to_string()];
    let Some(parent) = call.parent() else { return whole() };
    match parent.kind() {
        "variable_declarator" => {
            let Some(pattern) = parent.child_by_field_name("name") else { return whole() };
            if pattern.kind() != "object_pattern" {
                return whole();
            }
            let mut names = Vec::new();
            let mut cursor = pattern.walk();
            for part in pattern.named_children(&mut cursor) {
                match part.kind() {
                    "shorthand_property_identifier_pattern" => names.push(node_text(part, source)),
                    "pair_pattern" => match part.child_by_field_name("key") {
                        Some(k) => names.push(unquote(node_text(k, source))),
                        None => return whole(),
                    },
                    "object_assignment_pattern" => match part.child_by_field_name("left") {
                        Some(l) => names.push(node_text(l, source)),
                        None => return whole(),
                    },
                    _ => return whole(),
                }
            }
            if names.is_empty() { whole() } else { names }
        }
        "member_expression" => match parent.child_by_field_name("property") {
            Some(p) => vec![node_text(p, source)],
            None => whole(),
        },
        _ => whole(),
    }
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
        if child.kind() == "mod_item" && !has_body(&child) {
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
    }

    // Every other way the file reaches a module, outside test code: `use`
    // at any depth (a function body's too), `pub use`, and inline paths
    // (`crate::store::save()`, `hexa_exec::local_store::x()`). Only top-level
    // `use` was read, from its text — so `pub use` and `use x as y` kept a
    // raw path that resolved to nothing, and the rest were not read at all.
    let mut seen = std::collections::HashSet::new();
    collect_rust_edges(*root, source, from_file, Want::Uses, &mut imports, &mut seen);
    Ok(imports)
}

/// What [`collect_rust_edges`] reports: declarations (`use`), or inline
/// module paths. Kept apart because they have different readers — the domain
/// import policy reads declarations and filters inline paths itself.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Want {
    Uses,
    Paths,
}

/// Inline module paths in a Rust file, outside test code: `crate::x::y()`,
/// `hexa_exec::local_store::z()` — reaching a module with no `use` line.
fn extract_rust_module_paths(root: &tree_sitter::Node, source: &str, from_file: &str) -> Vec<ImportStatement> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    collect_rust_edges(*root, source, from_file, Want::Paths, &mut out, &mut seen);
    out
}

/// `use` declarations and inline module paths under `node`, skipping any
/// item marked `#[cfg(test)]` or `#[test]`. One import per distinct path.
fn collect_rust_edges(
    node: tree_sitter::Node,
    source: &str,
    from_file: &str,
    want: Want,
    out: &mut Vec<ImportStatement>,
    seen: &mut std::collections::HashSet<String>,
) {
    fn push(
        raw: String,
        name: Option<String>,
        line: usize,
        from_file: &str,
        out: &mut Vec<ImportStatement>,
        seen: &mut std::collections::HashSet<String>,
    ) {
        if seen.insert(raw.clone()) {
            out.push(ImportStatement {
                from_file: from_file.to_string(),
                resolved_path: raw.clone(),
                raw_path: raw,
                names: name.into_iter().collect(),
                line,
            });
        }
    }
    let mut skip_next = false;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let line = child.start_position().row + 1;
        match child.kind() {
            "attribute_item" => {
                let t: String = node_text(child, source).chars().filter(|c| !c.is_whitespace()).collect();
                if t.starts_with("#[cfg(test)") || t == "#[test]" {
                    skip_next = true;
                }
                continue;
            }
            _ if std::mem::take(&mut skip_next) => continue,
            "use_declaration" => {
                if want != Want::Uses {
                    continue;
                }
                if let Some(arg) = child.child_by_field_name("argument") {
                    for (path, name) in expand_rust_use(&node_text(arg, source)) {
                        push(path, Some(name), line, from_file, out, seen);
                    }
                }
                continue;
            }
            "scoped_identifier" | "scoped_type_identifier" => {
                let raw = node_text(child, source).trim().trim_start_matches("::").to_string();
                if raw.starts_with('<') {
                    collect_rust_edges(child, source, from_file, want, out, seen);
                    continue;
                }
                // A module path starts with `crate`, `self`, `super` or a
                // snake_case crate name. `Vec::new`, `Self::x`,
                // `Ordering::Less` start with a type and name no module.
                let head = raw.split("::").next().unwrap_or("");
                let module_like = matches!(head, "crate" | "self" | "super")
                    || head.starts_with(|c: char| c.is_ascii_lowercase() || c == '_');
                if want == Want::Paths && module_like && raw.contains("::") {
                    push(raw, None, line, from_file, out, seen);
                }
                let mut inner = child.walk();
                for grandchild in child.children(&mut inner) {
                    if grandchild.kind() == "type_arguments" {
                        collect_rust_edges(grandchild, source, from_file, want, out, seen);
                    }
                }
                continue;
            }
            _ => {}
        }
        collect_rust_edges(child, source, from_file, want, out, seen);
    }
}

/// A `use` argument, expanded: `a::{b, c::{d, e as f}}` is `a::b`, `a::c::d`,
/// `a::c::e`, each with the name it binds. `self` inside a group is the
/// group's own path; `as` aliases are dropped from the path.
fn expand_rust_use(arg: &str) -> Vec<(String, String)> {
    fn split_top(s: &str) -> Vec<&str> {
        let (mut depth, mut start, mut out) = (0i32, 0usize, Vec::new());
        for (i, c) in s.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => depth -= 1,
                ',' if depth == 0 => {
                    out.push(&s[start..i]);
                    start = i + 1;
                }
                _ => {}
            }
        }
        out.push(&s[start..]);
        out
    }
    fn walk(prefix: &str, item: &str, out: &mut Vec<(String, String)>) {
        let item = item.trim();
        if item.is_empty() {
            return;
        }
        if let Some(open) = item.find('{') {
            let base = item[..open].trim().trim_end_matches("::");
            let inner = item[open + 1..].trim_end().trim_end_matches('}');
            let joined = join(prefix, base);
            for part in split_top(inner) {
                walk(&joined, part, out);
            }
            return;
        }
        let path_part = item.split(" as ").next().unwrap_or(item).trim();
        if path_part == "self" {
            let name = prefix.rsplit("::").next().unwrap_or(prefix).to_string();
            out.push((prefix.to_string(), name));
            return;
        }
        let full = join(prefix, path_part);
        let name = full.rsplit("::").next().unwrap_or(&full).to_string();
        out.push((full, name));
    }
    fn join(prefix: &str, rest: &str) -> String {
        match (prefix.is_empty(), rest.is_empty()) {
            (true, _) => rest.to_string(),
            (_, true) => prefix.to_string(),
            _ => format!("{prefix}::{rest}"),
        }
    }
    let mut out = Vec::new();
    walk("", arg.trim().trim_start_matches("::"), &mut out);
    out
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
        // `module.exports = …`, `module.exports.a = …`, `exports.a = …`
        // (ADR-2611270420 §4).
        if child.kind() == "expression_statement" {
            extract_commonjs_exports(&child, source, file, &mut exports);
            continue;
        }
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

/// The exports a CommonJS assignment statement makes. `module.exports = { a, b:
/// x, c() {} }` exports `a`, `b` and `c` (a spread says nothing about names and
/// adds none); `module.exports = f` exports the module's default; `exports.a =`
/// and `module.exports.a =` export `a`.
fn extract_commonjs_exports(stmt: &tree_sitter::Node, source: &str, file: &str, out: &mut Vec<ExportDeclaration>) {
    let mut cursor = stmt.walk();
    let Some(assign) = stmt.named_children(&mut cursor).find(|c| c.kind() == "assignment_expression") else { return };
    let (Some(left), Some(right)) = (assign.child_by_field_name("left"), assign.child_by_field_name("right")) else { return };
    if left.kind() != "member_expression" {
        return;
    }
    let hexa_public = has_hex_public_annotation(stmt, source);
    let line = stmt.start_position().row + 1;
    let kind_of = |value: &tree_sitter::Node| match value.kind() {
        "function_expression" | "arrow_function" | "function" | "generator_function" => ExportKind::Function,
        "class" | "class_declaration" => ExportKind::Type,
        _ => ExportKind::Value,
    };
    let push = |out: &mut Vec<ExportDeclaration>, name: String, kind: ExportKind| {
        out.push(ExportDeclaration { file: file.to_string(), name, line, hexa_public, kind });
    };
    let text = node_text(left, source);
    if text == "module.exports" {
        if right.kind() == "object" {
            let mut oc = right.walk();
            for prop in right.named_children(&mut oc) {
                match prop.kind() {
                    "shorthand_property_identifier" => push(out, node_text(prop, source), ExportKind::Value),
                    "pair" => {
                        if let Some(k) = prop.child_by_field_name("key") {
                            let kind = prop.child_by_field_name("value").map(|v| kind_of(&v)).unwrap_or(ExportKind::Value);
                            push(out, unquote(node_text(k, source)), kind);
                        }
                    }
                    "method_definition" => {
                        if let Some(n) = prop.child_by_field_name("name") {
                            push(out, node_text(n, source), ExportKind::Function);
                        }
                    }
                    _ => {}
                }
            }
        } else {
            push(out, "default".to_string(), ExportKind::Default);
        }
        return;
    }
    let name = text
        .strip_prefix("module.exports.")
        .or_else(|| text.strip_prefix("exports."))
        .filter(|n| !n.is_empty() && n.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$'));
    if let Some(name) = name {
        push(out, name.to_string(), kind_of(&right));
    }
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
                            hexa_public: go_doc_tagged(&child, source),
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
                            hexa_public: go_doc_tagged(&child, source),
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
                // A group's doc comment covers every spec in it (ADR-2611221553 §2).
                let group = go_doc_tagged(&child, source);
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
                                    hexa_public: group || go_doc_tagged(&spec, source),
                                    kind: ExportKind::Type,
                                });
                            }
                        }
                    }
                }
            }
            "const_declaration" | "var_declaration" => {
                let group = go_doc_tagged(&child, source);
                // A const group's specs are direct children; a var group's sit
                // one level down, in a `var_spec_list` (ADR-2611221609 §1).
                let mut specs = Vec::new();
                let mut tc = child.walk();
                for c in child.children(&mut tc) {
                    if c.kind() == "var_spec_list" {
                        let mut lc = c.walk();
                        specs.extend(c.children(&mut lc).filter(|s| s.kind() == "var_spec"));
                    } else if c.kind() == "const_spec" || c.kind() == "var_spec" {
                        specs.push(c);
                    }
                }
                for spec in specs {
                    // Every name a const or var spec declares (`const A, B = 1, 2`):
                    // the identifiers only, not the commas (ADR-2611221643 §1).
                    let names: Vec<String> = {
                        let mut nc = spec.walk();
                        spec.children_by_field_name("name", &mut nc)
                            .filter(|n| n.kind() == "identifier")
                            .map(|n| node_text(n, source))
                            .collect()
                    };
                    let tagged = group || go_doc_tagged(&spec, source);
                    for name in names.into_iter().filter(|n| is_go_exported(n)) {
                        exports.push(ExportDeclaration {
                            file: file.to_string(),
                            name,
                            line: spec.start_position().row + 1,
                            hexa_public: tagged,
                            kind: ExportKind::Value,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    Ok(exports)
}

/// Whether the Go doc comment of `node` carries `@hexa:public` (ADR-2611221553
/// §1): the run of comment lines directly above it, each on the line just
/// above the next, so a blank line or any other node ends the run. A Go doc
/// comment is several `//` lines, each its own node, and the tag may be on any.
///
/// A line is read whole (ADR-2611221934): every comment on it is gathered, so
/// a line of comments after code (`A = 1 /* x */ // …`) is that code's, and a
/// line holding only comments (`/* … */ // …`) is one doc line.
fn go_doc_tagged(node: &tree_sitter::Node, source: &str) -> bool {
    let mut next_row = node.start_position().row;
    let mut prev = node.prev_sibling();
    while let Some(last) = prev {
        if last.kind() != "comment" || last.end_position().row + 1 != next_row {
            return false;
        }
        // The comments on this line, back to the first of them.
        let mut line = vec![last];
        let mut first = last;
        while let Some(q) = first.prev_sibling() {
            if q.kind() == "comment" && q.end_position().row == first.start_position().row {
                line.push(q);
                first = q;
            } else {
                break;
            }
        }
        // Code before the first comment on its line makes the whole line that
        // code's line comments (`A = 1 // …`, `const ( // …`, `) // …`), not a
        // doc line: the newline between them is hidden in the tree, so rows
        // are the only way to tell.
        if first.prev_sibling().is_some_and(|q| q.kind() != "comment" && q.end_position().row == first.start_position().row) {
            return false;
        }
        if line.iter().any(|c| node_text(*c, source).contains("@hexa:public")) {
            return true;
        }
        next_row = first.start_position().row;
        prev = first.prev_sibling();
    }
    false
}

/// Go exports are identified by capitalized names: the Go spec's rule is a
/// first character that is a Unicode upper-case letter, so `Äpfel` is exported
/// as `Apple` is (an ASCII-only check missed it).
fn is_go_exported(name: &str) -> bool {
    name.starts_with(|c: char| c.is_uppercase())
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
        Language::Go | Language::Python | Language::Unknown => {}
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
