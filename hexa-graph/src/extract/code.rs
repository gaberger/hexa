//! AST entity + import extraction via tree-sitter.
//!
//! Self-contained (the engine must not depend on hexa-nexus): we load the same
//! native grammars hexa-nexus uses (Rust / TypeScript / Go) and pull out top-level
//! declarations as graph entities plus import statements as cross-file edges.
//! Patterns mirror `hexa-nexus/src/analysis/treesitter_adapter.rs`.

use tree_sitter::{Language as TsLanguage, Node, Parser};

use crate::ports::{Entity, FileExtract, ImportKind, NodeKind, RawImport};

/// Languages the AST extractor understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    TypeScript,
    Go,
    Rust,
    /// Python, by `.py` and `.pyi` (ADR-2611211324 §1).
    Python,
}

impl Language {
    /// Detect from file extension; `None` for unsupported files.
    pub fn from_path(path: &str) -> Option<Self> {
        if path.ends_with(".ts")
            || path.ends_with(".tsx")
            || path.ends_with(".js")
            || path.ends_with(".jsx")
            || path.ends_with(".mts")
            || path.ends_with(".cts")
            || path.ends_with(".mjs")
            || path.ends_with(".cjs")
        {
            Some(Language::TypeScript)
        } else if path.ends_with(".go") {
            Some(Language::Go)
        } else if path.ends_with(".rs") {
            Some(Language::Rust)
        } else if path.ends_with(".py") || path.ends_with(".pyi") {
            Some(Language::Python)
        } else {
            None
        }
    }

    fn grammar(self) -> Option<TsLanguage> {
        match self {
            Language::TypeScript => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
            Language::Go => Some(tree_sitter_go::LANGUAGE.into()),
            Language::Rust => Some(tree_sitter_rust::LANGUAGE.into()),
            Language::Python => Some(tree_sitter_python::LANGUAGE.into()),
        }
    }
}

/// Parse `source` and extract entities + imports. Returns an empty extract on
/// parse failure rather than erroring — a graph build over many files should be
/// resilient to one unparseable file.
pub fn extract_file(source: &str, lang: Language) -> FileExtract {
    let Some(grammar) = lang.grammar() else {
        return FileExtract::default();
    };
    let mut parser = Parser::new();
    if parser.set_language(&grammar).is_err() {
        return FileExtract::default();
    }
    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return FileExtract::default(),
    };
    let root = tree.root_node();
    match lang {
        Language::Rust => extract_rust(&root, source),
        Language::TypeScript => extract_ts(&root, source),
        Language::Go => extract_go(&root, source),
        Language::Python => extract_python(&root, source),
    }
}

fn text<'a>(node: Node, source: &'a str) -> &'a str {
    node.utf8_text(source.as_bytes()).unwrap_or("")
}

fn name_field(node: Node, source: &str) -> Option<String> {
    node.child_by_field_name("name")
        .map(|n| text(n, source).to_string())
        .filter(|s| !s.is_empty())
}

fn line_of(node: Node) -> usize {
    node.start_position().row + 1
}

// ── Rust ─────────────────────────────────────────────────

fn extract_rust(root: &Node, source: &str) -> FileExtract {
    let mut out = FileExtract::default();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        match child.kind() {
            "function_item" => push_named(&mut out.entities, child, source, NodeKind::Function),
            "struct_item" | "union_item" => {
                push_named(&mut out.entities, child, source, NodeKind::Struct)
            }
            "enum_item" => push_named(&mut out.entities, child, source, NodeKind::Enum),
            "trait_item" => push_named(&mut out.entities, child, source, NodeKind::Trait),
            "type_item" => push_named(&mut out.entities, child, source, NodeKind::Type),
            "const_item" | "static_item" => {
                push_named(&mut out.entities, child, source, NodeKind::Const)
            }
            "use_declaration" => collect_rust_use(child, source, &mut out.imports),
            "mod_item" if !rust_has_body(child) => {
                if let Some(name) = name_field(child, source) {
                    out.imports.push(RawImport {
                        raw_path: format!("self::{name}"),
                        names: vec![name],
                        line: line_of(child),
                        kind: ImportKind::ModDecl,
                    });
                }
            }
            _ => {}
        }
    }
    collect_rust_paths(*root, source, &mut out.imports);
    out
}

/// Record every fully qualified path in an expression or a type
/// (`crate::domain::util::half(4)`, `super::a::Alpha`, `other_crate::f()`) as a
/// use. Such a call names an item without a `use`, and a consumer trace that
/// misses it calls a live module dead. `use` lines are collected separately.
fn collect_rust_paths(node: Node, source: &str, imports: &mut Vec<RawImport>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "use_declaration" => {}
            "scoped_identifier" | "scoped_type_identifier" => {
                if let Some(imp) = rust_path_use(child, source) {
                    let seen = imports
                        .iter()
                        .any(|i| i.kind == ImportKind::PathExpr && i.raw_path == imp.raw_path);
                    if !seen {
                        imports.push(imp);
                    }
                }
                // The outermost path is the whole path; its `path` child is a
                // prefix of it. Any other child can hold paths of its own.
                let prefix = child.child_by_field_name("path").map(|p| p.id());
                let mut inner = child.walk();
                for part in child.children(&mut inner) {
                    if Some(part.id()) != prefix {
                        collect_rust_paths(part, source, imports);
                    }
                }
            }
            _ => collect_rust_paths(child, source, imports),
        }
    }
}

/// A path use from one scoped path, or `None` for a path that cannot name a
/// project module (`std::…`, `Self::…`, `self::…`, a bare `a::b` relative to
/// a `use`). Whether a head identifier is a workspace crate is decided at
/// resolution, against the crate roots.
fn rust_path_use(node: Node, source: &str) -> Option<RawImport> {
    let raw: String = text(node, source).chars().filter(|c| !c.is_whitespace()).collect();
    let path = match raw.find('<') {
        Some(i) => raw[..i].trim_end_matches("::").to_string(),
        None => raw,
    };
    let segs: Vec<&str> = path.split("::").filter(|s| !s.is_empty()).collect();
    if segs.len() < 2 {
        return None;
    }
    let head = segs[0];
    if matches!(head, "self" | "Self" | "std" | "core" | "alloc") {
        return None;
    }
    let names: Vec<String> = segs[1..]
        .iter()
        .filter(|s| **s != "super")
        .map(|s| s.to_string())
        .collect();
    Some(RawImport {
        raw_path: path,
        names,
        line: line_of(node),
        kind: ImportKind::PathExpr,
    })
}

fn rust_has_body(node: Node) -> bool {
    let mut cursor = node.walk();
    let has = node
        .children(&mut cursor)
        .any(|c| c.kind() == "declaration_list");
    has
}

fn collect_rust_use(node: Node, source: &str, imports: &mut Vec<RawImport>) {
    let raw = text(node, source).trim();
    let path = raw
        .strip_prefix("use ")
        .unwrap_or(raw)
        .trim_end_matches(';')
        .trim();
    let line = line_of(node);
    if let Some(brace) = path.find('{') {
        let base = path[..brace].trim_end_matches("::").trim();
        let group = path[brace + 1..].trim_end_matches('}').trim();
        for item in group.split(',') {
            let item = item.trim();
            if item.is_empty() || item == "self" {
                continue;
            }
            let full = format!("{base}::{item}");
            let name = item.rsplit("::").next().unwrap_or(item).to_string();
            imports.push(RawImport {
                raw_path: full,
                names: vec![name],
                line,
                kind: ImportKind::Use,
            });
        }
    } else {
        let name = path.rsplit("::").next().unwrap_or(path).to_string();
        imports.push(RawImport {
            raw_path: path.to_string(),
            names: vec![name],
            line,
            kind: ImportKind::Use,
        });
    }
}

// ── TypeScript ───────────────────────────────────────────

fn extract_ts(root: &Node, source: &str) -> FileExtract {
    let mut out = FileExtract::default();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        match child.kind() {
            "import_statement" => collect_ts_import(child, source, &mut out.imports),
            "export_statement" => {
                if child.child_by_field_name("source").is_some() {
                    // re-export `export { X } from '...'` — treat as an import edge
                    collect_ts_import(child, source, &mut out.imports);
                } else {
                    let mut inner = child.walk();
                    for decl in child.children(&mut inner) {
                        ts_decl_entity(decl, source, &mut out.entities);
                    }
                }
            }
            _ => ts_decl_entity(child, source, &mut out.entities),
        }
    }
    // A CommonJS `require("x")` or a dynamic `import("x")` with one plain string
    // is a use of that file, wherever it is written (ADR-2611270420 §2). Without
    // it `graph consumers` called a required file safe to remove.
    collect_ts_dynamic_uses(*root, source, &mut out.imports);
    out
}

/// Every `require("literal")` and `import("literal")` under `node`. A call whose
/// argument is not one plain string names no file the graph can see.
fn collect_ts_dynamic_uses(node: Node, source: &str, imports: &mut Vec<RawImport>) {
    if node.kind() == "call_expression" {
        let callee = node.child_by_field_name("function");
        let is_use = callee.is_some_and(|f| match f.kind() {
            "import" => true,
            "identifier" => text(f, source) == "require",
            _ => false,
        });
        if is_use {
            if let Some(args) = node.child_by_field_name("arguments") {
                let mut ac = args.walk();
                let named: Vec<Node> = args.named_children(&mut ac).collect();
                if let [arg] = named.as_slice() {
                    if arg.kind() == "string" {
                        imports.push(RawImport {
                            raw_path: unquote(text(*arg, source)),
                            names: vec!["*".to_string()],
                            line: line_of(node),
                            kind: ImportKind::Use,
                        });
                    }
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_ts_dynamic_uses(child, source, imports);
    }
}

fn ts_decl_entity(node: Node, source: &str, entities: &mut Vec<Entity>) {
    let kind = match node.kind() {
        "function_declaration" | "generator_function_declaration" => NodeKind::Function,
        "class_declaration" | "abstract_class_declaration" => NodeKind::Class,
        "interface_declaration" => NodeKind::Interface,
        "type_alias_declaration" => NodeKind::Type,
        "enum_declaration" => NodeKind::Enum,
        "lexical_declaration" | "variable_declaration" => {
            collect_ts_variables(node, source, entities);
            return;
        }
        _ => return,
    };
    push_named(entities, node, source, kind);
}

fn collect_ts_variables(node: Node, source: &str, entities: &mut Vec<Entity>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            if let Some(name) = child.child_by_field_name("name") {
                let n = text(name, source).to_string();
                if !n.is_empty() {
                    entities.push(Entity {
                        name: n,
                        kind: NodeKind::Const,
                        line: line_of(child),
                    });
                }
            }
        }
    }
}

fn collect_ts_import(node: Node, source: &str, imports: &mut Vec<RawImport>) {
    let Some(src) = node.child_by_field_name("source") else {
        return;
    };
    let raw_path = unquote(text(src, source));
    let mut names = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_clause" => {
                let mut ic = child.walk();
                for part in child.children(&mut ic) {
                    match part.kind() {
                        "identifier" => names.push("default".to_string()),
                        "namespace_import" => names.push("*".to_string()),
                        "named_imports" => {
                            let mut nc = part.walk();
                            for spec in part.children(&mut nc) {
                                if spec.kind() == "import_specifier" {
                                    if let Some(name) = spec.child_by_field_name("name") {
                                        names.push(text(name, source).to_string());
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            "export_clause" => {
                let mut ec = child.walk();
                for spec in child.children(&mut ec) {
                    if spec.kind() == "export_specifier" {
                        if let Some(name) = spec.child_by_field_name("name") {
                            names.push(text(name, source).to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    imports.push(RawImport {
        raw_path,
        names,
        line: line_of(node),
        kind: ImportKind::Use,
    });
}

// ── Go ───────────────────────────────────────────────────

fn extract_go(root: &Node, source: &str) -> FileExtract {
    let mut out = FileExtract::default();
    let mut cursor = root.walk();
    let mut functions = Vec::new();
    for child in root.children(&mut cursor) {
        match child.kind() {
            "package_clause" => {
                let mut pc = child.walk();
                out.go_package = child
                    .children(&mut pc)
                    .find(|c| c.kind() == "package_identifier")
                    .map(|c| text(c, source).to_string());
            }
            "function_declaration" => {
                functions.extend(name_field(child, source));
                push_named(&mut out.entities, child, source, NodeKind::Function)
            }
            "method_declaration" => {
                out.go_methods.extend(name_field(child, source));
                push_named(&mut out.entities, child, source, NodeKind::Function)
            }
            "type_declaration" => collect_go_types(child, source, &mut out.entities),
            "const_declaration" | "var_declaration" => {
                collect_go_consts(child, source, &mut out.entities)
            }
            "import_declaration" => collect_go_imports(child, source, &mut out.imports),
            _ => {}
        }
    }
    // A function and a method of one name share an entity id; the function
    // is a real target, so that name stays linkable.
    out.go_methods.retain(|m| !functions.contains(m));
    let mut names = GoNames::default();
    collect_go_identifiers(*root, source, &mut names);
    out.go_literal_keys = names.keys.into_iter().collect();
    if !names.names.is_empty() || !out.go_literal_keys.is_empty() {
        out.imports.push(RawImport {
            raw_path: ".".to_string(),
            names: names.names.into_iter().collect(),
            line: 1,
            kind: ImportKind::SamePackage,
        });
    }
    out
}

/// What a Go file writes that can name a package-level entity of its own
/// package: bare identifiers, and the (type, key) of each bare key of a
/// literal of a named type, a use unless that type is a struct of the package.
#[derive(Default)]
struct GoNames {
    names: std::collections::BTreeSet<String>,
    keys: std::collections::BTreeSet<(String, String)>,
}

/// Every identifier a Go file writes that can name a package-level entity of
/// its own package, for its same-package references. A type qualified by
/// another package (`http.Handler`) and a struct literal's field key
/// (`struct{ N int }{N: 1}`) are not such names. The key of a literal of a
/// named type (`T{N: 1}`) is kept aside: whether it is a field depends on a
/// type another file of the package may declare. A local that shadows a
/// package-level name is still collected: over-reporting keeps the deletion
/// oracle conservative.
fn collect_go_identifiers(node: Node, source: &str, names: &mut GoNames) {
    match node.kind() {
        "identifier" | "type_identifier" => {
            let t = text(node, source);
            if !t.is_empty() {
                names.names.insert(t.to_string());
            }
            return;
        }
        "qualified_type" => return,
        "composite_literal" => {
            let ty = node.child_by_field_name("type");
            if let Some(t) = ty {
                collect_go_identifiers(t, source, names);
            }
            if let Some(body) = node.child_by_field_name("body") {
                collect_go_literal(body, ty, source, names);
            }
            return;
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_go_identifiers(child, source, names);
    }
}

/// How a composite literal's bare-identifier keys read, by its type.
enum GoKeys {
    /// An inline struct type, or elided from an unknown outer type.
    Fields,
    /// A map, slice or array: keys are expressions.
    Expressions,
    /// A named type: a field name only if the package declares it a struct.
    OfNamed(String),
}

fn go_keys_of(ty: Option<Node>, source: &str) -> GoKeys {
    let Some(t) = ty else { return GoKeys::Fields };
    match t.kind() {
        "map_type" | "slice_type" | "array_type" | "implicit_length_array_type" => GoKeys::Expressions,
        "type_identifier" => GoKeys::OfNamed(text(t, source).to_string()),
        "generic_type" => GoKeys::OfNamed(
            t.child_by_field_name("type").map(|n| text(n, source).to_string()).unwrap_or_default(),
        ),
        // Another package's type is never a struct this package declares.
        "qualified_type" => GoKeys::OfNamed(String::new()),
        _ => GoKeys::Fields,
    }
}

/// A composite literal's body, of type `ty` (`None` when elided from an
/// unknown outer type). A bare-identifier key is a field name for an inline
/// struct type, an expression for a map, slice or array, and for a named
/// type is left to the graph, which sees every file of the package.
fn collect_go_literal(body: Node, ty: Option<Node>, source: &str, names: &mut GoNames) {
    let keys = go_keys_of(ty, source);
    // The type an elided inner literal (`[]T{{...}}`) has.
    let elem = ty.and_then(|t| match t.kind() {
        "map_type" => t.child_by_field_name("value"),
        "slice_type" | "array_type" | "implicit_length_array_type" => {
            t.child_by_field_name("element")
        }
        _ => None,
    });
    let element = |el: Node, names: &mut GoNames| {
        let mut c = el.walk();
        for inner in el.children(&mut c) {
            if inner.kind() == "literal_value" {
                collect_go_literal(inner, elem, source, names);
            } else {
                collect_go_identifiers(inner, source, names);
            }
        }
    };
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        match child.kind() {
            "keyed_element" => {
                if let Some(key) = child.child_by_field_name("key") {
                    let bare = (key.named_child_count() == 1)
                        .then(|| key.named_child(0))
                        .flatten()
                        .filter(|k| k.kind() == "identifier");
                    match (&keys, bare) {
                        (GoKeys::Fields, Some(_)) => {}
                        (GoKeys::OfNamed(t), Some(k)) => {
                            names.keys.insert((t.clone(), text(k, source).to_string()));
                        }
                        _ => element(key, names),
                    }
                }
                if let Some(value) = child.child_by_field_name("value") {
                    element(value, names);
                }
            }
            "literal_element" => element(child, names),
            _ => collect_go_identifiers(child, source, names),
        }
    }
}

fn collect_go_types(node: Node, source: &str, entities: &mut Vec<Entity>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        // `type A int` / `type S struct{}` → type_spec; `type A = int` → type_alias.
        if child.kind() == "type_spec" || child.kind() == "type_alias" {
            let Some(name) = name_field(child, source) else {
                continue;
            };
            let kind = match child.child_by_field_name("type").map(|t| t.kind()) {
                Some("struct_type") => NodeKind::Struct,
                Some("interface_type") => NodeKind::Interface,
                _ => NodeKind::Type,
            };
            entities.push(Entity {
                name,
                kind,
                line: line_of(child),
            });
        }
    }
}

fn collect_go_consts(node: Node, source: &str, entities: &mut Vec<Entity>) {
    // A const group's specs are direct children; a var group's sit in a
    // `var_spec_list`. Every identifier a spec names is an entity
    // (ADR-2611221643 §3).
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "var_spec_list" => collect_go_consts(child, source, entities),
            "const_spec" | "var_spec" => {
                let mut nc = child.walk();
                for n in child.children_by_field_name("name", &mut nc) {
                    if n.kind() != "identifier" {
                        continue;
                    }
                    let name = text(n, source).to_string();
                    if name.is_empty() {
                        continue;
                    }
                    entities.push(Entity { name, kind: NodeKind::Const, line: line_of(child) });
                }
            }
            _ => {}
        }
    }
}

fn collect_go_imports(node: Node, source: &str, imports: &mut Vec<RawImport>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_spec" => {
                if let Some(path) = child.child_by_field_name("path") {
                    imports.push(RawImport {
                        raw_path: unquote(text(path, source)),
                        names: vec!["*".to_string()],
                        line: line_of(child),
                        kind: ImportKind::Use,
                    });
                }
            }
            "import_spec_list" => collect_go_imports(child, source, imports),
            _ => {}
        }
    }
}

// ── Python (ADR-2611211324) ──────────────────────────────

fn extract_python(root: &Node, source: &str) -> FileExtract {
    let mut out = FileExtract::default();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        let def = if child.kind() == "decorated_definition" { child.child_by_field_name("definition").unwrap_or(child) } else { child };
        match def.kind() {
            "function_definition" => push_named(&mut out.entities, def, source, NodeKind::Function),
            "class_definition" => push_named(&mut out.entities, def, source, NodeKind::Class),
            _ => {}
        }
    }
    collect_python_imports(*root, source, &mut out.imports);
    out
}

/// Every import, at any depth: one under `if TYPE_CHECKING:` is still one.
fn collect_python_imports(node: Node, source: &str, imports: &mut Vec<RawImport>) {
    let taken = |n: Node| -> Option<String> {
        match n.kind() {
            "aliased_import" => n.child_by_field_name("name").map(|m| text(m, source).to_string()),
            _ => Some(text(n, source).to_string()),
        }
    };
    match node.kind() {
        "import_statement" => {
            let mut cursor = node.walk();
            for name in node.children_by_field_name("name", &mut cursor) {
                if let Some(module) = taken(name) {
                    imports.push(RawImport { raw_path: module, names: vec!["*".to_string()], line: line_of(node), kind: ImportKind::Use });
                }
            }
        }
        "import_from_statement" => {
            if let Some(module) = node.child_by_field_name("module_name") {
                let mut cursor = node.walk();
                let mut names: Vec<String> = node.children_by_field_name("name", &mut cursor).filter_map(taken).collect();
                if names.is_empty() {
                    names.push("*".to_string());
                }
                imports.push(RawImport { raw_path: text(module, source).to_string(), names, line: line_of(node), kind: ImportKind::Use });
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                collect_python_imports(child, source, imports);
            }
        }
    }
}

// ── Shared helpers ───────────────────────────────────────

fn push_named(entities: &mut Vec<Entity>, node: Node, source: &str, kind: NodeKind) {
    if let Some(name) = name_field(node, source) {
        entities.push(Entity {
            name,
            kind,
            line: line_of(node),
        });
    }
}

fn unquote(s: &str) -> String {
    s.trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '`')
        .to_string()
}
