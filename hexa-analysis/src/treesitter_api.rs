//! What a source file says about the API, read with tree-sitter
//! (ADR-2610092245).
//!
//! The tag lives in a doc comment, the way `@hexa:public` does, so it reads
//! the same in Rust, Go and TypeScript and adds nothing to the code hexa
//! scaffolds. Everything language-specific is decided here, where the
//! grammar is: which parameter is a receiver or a context, how a `Result`, a
//! trailing `error` or a `Promise` unwraps, and what a field is called on the
//! wire (serde renames, Go `json:` tags, TypeScript property names). The
//! contract builder sees only [`TypeRef`]s and resolves names across files.
//!
//! A tag is a comment line that *begins* with the tag. A sentence that
//! mentions one, like this module's, is prose.

use tree_sitter::Node;

use super::ports::{
    capitalize, lower_camel, split_words, AnalysisError, ApiFacts, ApiMethodDecl, ApiPortDecl, ErrorChannel,
    FieldDecl, Language, TypeBody, TypeDecl, TypeRef, VariantDecl,
};

const API_TAG: &str = "@hexa:api";
const STATUS_TAG: &str = "@hexa:status";

/// Read `source` for its tagged ports, its type declarations and its stray tags.
pub fn extract(source: &str, lang: Language) -> Result<ApiFacts, AnalysisError> {
    if lang == Language::Unknown || (!source.contains(API_TAG) && !mentions_a_type(source, lang)) {
        return Ok(ApiFacts::default());
    }
    let grammar: tree_sitter::Language = match lang {
        Language::Rust => tree_sitter_rust::LANGUAGE.into(),
        Language::Go => tree_sitter_go::LANGUAGE.into(),
        Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Language::Unknown => return Ok(ApiFacts::default()),
    };
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&grammar).map_err(|e| AnalysisError::Other(e.to_string()))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| AnalysisError::Other("tree-sitter parse returned None".to_string()))?;
    let root = tree.root_node();
    let mut x = Extractor { src: source, facts: ApiFacts::default(), consumed: Vec::new() };
    x.walk(root, lang);

    // Every comment line that starts with the tag and was not read as part
    // of a port is a tag in the wrong place.
    let mut tagged = Vec::new();
    collect_tag_comments(root, source, &mut tagged);
    x.facts.stray_tags = tagged
        .into_iter()
        .filter(|(start, _)| !x.consumed.contains(start))
        .map(|(_, line)| line)
        .collect();
    x.facts.stray_tags.sort_unstable();
    Ok(x.facts)
}

/// A file with no tag still declares types a tagged port elsewhere may
/// name; a file declaring none can be skipped without parsing.
fn mentions_a_type(source: &str, lang: Language) -> bool {
    match lang {
        Language::Rust => ["struct ", "enum ", "type ", " as "].iter().any(|k| source.contains(k)),
        Language::Go => source.contains("type "),
        Language::TypeScript => ["interface ", "type ", " as "].iter().any(|k| source.contains(k)),
        Language::Unknown => false,
    }
}

// ── Comments ─────────────────────────────────────────────

fn is_comment(kind: &str) -> bool {
    matches!(kind, "comment" | "line_comment" | "block_comment")
}

/// A comment's lines with the comment syntax removed.
fn comment_lines(raw: &str) -> Vec<String> {
    raw.lines()
        .map(|l| {
            let mut t = l.trim();
            t = t.strip_suffix("*/").unwrap_or(t).trim_end();
            for p in ["/**", "/*", "///", "//!", "//", "*"] {
                if let Some(rest) = t.strip_prefix(p) {
                    t = rest;
                    break;
                }
            }
            t.trim().to_string()
        })
        .collect()
}

/// The text after `tag` when `line` begins with it as a whole word.
fn tag_rest<'a>(line: &'a str, tag: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(tag)?;
    (rest.is_empty() || rest.starts_with(char::is_whitespace)).then(|| rest.trim())
}

fn collect_tag_comments(n: Node, src: &str, out: &mut Vec<(usize, usize)>) {
    if is_comment(n.kind()) {
        for (i, l) in comment_lines(&src[n.byte_range()]).iter().enumerate() {
            if tag_rest(l, API_TAG).is_some() {
                out.push((n.start_byte(), n.start_position().row + 1 + i));
            }
        }
        return;
    }
    let mut c = n.walk();
    for ch in n.children(&mut c) {
        collect_tag_comments(ch, src, out);
    }
}

/// The doc comment above a declaration, read for its tags.
#[derive(Default)]
struct Doc {
    text: String,
    api: Option<String>,
    statuses: Vec<String>,
    /// Start bytes of the comments that carried `@hexa:api`.
    tag_comments: Vec<usize>,
    /// Rust attributes between the comments and the item, in source order.
    attrs: Vec<String>,
}

/// The last row a node occupies. A Rust line comment includes its newline,
/// so it ends at column 0 of the row after it.
fn last_row(n: Node) -> usize {
    let end = n.end_position();
    if end.column == 0 && end.row > n.start_position().row {
        end.row - 1
    } else {
        end.row
    }
}

fn doc_of(node: Node, src: &str) -> Doc {
    let anchor = match node.parent() {
        Some(p) if p.kind() == "export_statement" => p,
        _ => node,
    };
    let mut comments = Vec::new();
    let mut attrs = Vec::new();
    let mut top = anchor.start_position().row;
    let mut cur = anchor;
    while let Some(prev) = cur.prev_sibling() {
        if prev.kind() == "attribute_item" {
            attrs.push(src[prev.byte_range()].to_string());
        } else if is_comment(prev.kind()) && last_row(prev) + 1 >= top {
            comments.push(prev);
        } else {
            break;
        }
        top = prev.start_position().row;
        cur = prev;
    }
    comments.reverse();
    attrs.reverse();

    let mut doc = Doc { attrs, ..Doc::default() };
    let mut paragraphs: Vec<Vec<String>> = vec![Vec::new()];
    for c in comments {
        for line in comment_lines(&src[c.byte_range()]) {
            if let Some(rest) = tag_rest(&line, API_TAG) {
                doc.api = Some(rest.to_string());
                doc.tag_comments.push(c.start_byte());
            } else if let Some(rest) = tag_rest(&line, STATUS_TAG) {
                doc.statuses.push(rest.to_string());
            } else if line.starts_with("@hexa:") {
                // Another hexa tag (`@hexa:public`): not prose.
            } else if line.is_empty() {
                paragraphs.push(Vec::new());
            } else if let Some(p) = paragraphs.last_mut() {
                p.push(line);
            }
        }
    }
    doc.text = paragraphs
        .into_iter()
        .filter(|p| !p.is_empty())
        .map(|p| p.join(" "))
        .collect::<Vec<_>>()
        .join("\n\n");
    doc
}

// ── Extraction ───────────────────────────────────────────

struct Extractor<'s> {
    src: &'s str,
    facts: ApiFacts,
    consumed: Vec<usize>,
}

fn line_of(n: Node) -> usize {
    n.start_position().row + 1
}

fn named_children(n: Node) -> Vec<Node> {
    let mut c = n.walk();
    n.named_children(&mut c).collect()
}

impl<'s> Extractor<'s> {
    fn text(&self, n: Node) -> &'s str {
        &self.src[n.byte_range()]
    }

    fn field_text(&self, n: Node, field: &str) -> Option<&'s str> {
        n.child_by_field_name(field).map(|f| self.text(f))
    }

    fn walk(&mut self, n: Node, lang: Language) {
        match (lang, n.kind()) {
            (Language::Rust, "trait_item") => self.rust_trait(n),
            (Language::Rust, "struct_item") => self.rust_struct(n),
            (Language::Rust, "enum_item") => self.rust_enum(n),
            (Language::Rust, "type_item") => self.alias(n, "type", lower_rust),
            (Language::Rust, "use_as_clause") => self.rust_use_as(n),
            (Language::Go, "type_spec") => self.go_type_spec(n),
            (Language::Go, "type_alias") => self.alias(n, "type", lower_go),
            (Language::TypeScript, "interface_declaration") => self.ts_interface(n),
            (Language::TypeScript, "type_alias_declaration") => self.ts_type_alias(n),
            (Language::TypeScript, "export_specifier") => self.ts_export_as(n),
            _ => {}
        }
        for ch in named_children(n) {
            self.walk(ch, lang);
        }
    }

    fn push_type(&mut self, name: &str, n: Node, body: TypeBody) {
        self.facts.types.push(TypeDecl { name: name.to_string(), line: line_of(n), body });
    }

    fn alias(&mut self, n: Node, field: &str, lower: fn(Node, &str) -> TypeRef) {
        let (Some(name), Some(ty)) = (self.field_text(n, "name"), n.child_by_field_name(field)) else {
            return;
        };
        let target = lower(ty, self.src);
        self.push_type(name, n, TypeBody::Alias(target));
    }

    /// Record a port when the interface or any of its methods is tagged.
    fn push_port(&mut self, name: &str, n: Node, doc: Doc, methods: Vec<(ApiMethodDecl, Vec<usize>)>) {
        if doc.api.is_none() && methods.iter().all(|(m, _)| m.tag.is_none()) {
            return;
        }
        self.consumed.extend(doc.tag_comments);
        let mut decls = Vec::new();
        for (m, tags) in methods {
            self.consumed.extend(tags);
            decls.push(m);
        }
        self.facts.ports.push(ApiPortDecl {
            name: name.to_string(),
            line: line_of(n),
            tag: doc.api,
            statuses: doc.statuses,
            methods: decls,
        });
    }

    fn method(
        &self,
        n: Node,
        name: &str,
        params: Vec<(String, TypeRef)>,
        (returns, error): (TypeRef, ErrorChannel),
    ) -> (ApiMethodDecl, Vec<usize>) {
        let doc = doc_of(n, self.src);
        let decl = ApiMethodDecl {
            name: name.to_string(),
            line: line_of(n),
            doc: doc.text,
            tag: doc.api,
            statuses: doc.statuses,
            params,
            returns,
            error,
        };
        (decl, doc.tag_comments)
    }

    // ── Rust ──

    fn rust_trait(&mut self, n: Node) {
        let Some(name) = self.field_text(n, "name") else { return };
        let mut methods = Vec::new();
        if let Some(body) = n.child_by_field_name("body") {
            for item in named_children(body) {
                if !matches!(item.kind(), "function_signature_item" | "function_item") {
                    continue;
                }
                let Some(mname) = self.field_text(item, "name") else { continue };
                let mut params = Vec::new();
                if let Some(ps) = item.child_by_field_name("parameters") {
                    for p in named_children(ps).into_iter().filter(|p| p.kind() == "parameter") {
                        let pname = self.field_text(p, "pattern").unwrap_or("");
                        let pname = pname.strip_prefix("mut ").unwrap_or(pname).trim().to_string();
                        let ty = p.child_by_field_name("type").map_or(TypeRef::Unit, |t| lower_rust(t, self.src));
                        params.push((pname, ty));
                    }
                }
                let ret = item.child_by_field_name("return_type");
                let signature = ret.map_or((TypeRef::Unit, ErrorChannel::None), |r| rust_return(r, self.src));
                methods.push(self.method(item, mname, params, signature));
            }
        }
        let doc = doc_of(n, self.src);
        self.push_port(name, n, doc, methods);
    }

    fn rust_struct(&mut self, n: Node) {
        let Some(name) = self.field_text(n, "name") else { return };
        let doc = doc_of(n, self.src);
        let rename_all = doc.attrs.iter().find_map(|a| serde_value(a, "rename_all"));
        let body = match n.child_by_field_name("body") {
            Some(b) if b.kind() == "field_declaration_list" => {
                let mut fields = Vec::new();
                let mut attrs: Vec<&str> = Vec::new();
                for ch in named_children(b) {
                    match ch.kind() {
                        "attribute_item" => attrs.push(self.text(ch)),
                        "field_declaration" => {
                            let skip = attrs.iter().any(|a| serde_flag(a, "skip") || serde_flag(a, "skip_serializing"));
                            let renamed = attrs.iter().find_map(|a| serde_value(a, "rename"));
                            attrs.clear();
                            if skip {
                                continue;
                            }
                            let (Some(fname), Some(ty)) = (self.field_text(ch, "name"), ch.child_by_field_name("type"))
                            else {
                                continue;
                            };
                            let wire_name = renamed.unwrap_or_else(|| rename(fname, rename_all.as_deref()));
                            fields.push(FieldDecl {
                                wire_name,
                                ty: lower_rust(ty, self.src),
                                optional: false,
                                line: line_of(ch),
                            });
                        }
                        _ => {}
                    }
                }
                TypeBody::Struct(fields)
            }
            Some(b) if b.kind() == "ordered_field_declaration_list" => {
                let types: Vec<Node> = named_children(b)
                    .into_iter()
                    .filter(|c| !matches!(c.kind(), "attribute_item" | "visibility_modifier"))
                    .collect();
                match types.as_slice() {
                    [one] => TypeBody::Newtype(lower_rust(*one, self.src)),
                    _ => TypeBody::Newtype(TypeRef::Unsupported(format!("tuple struct {name}"))),
                }
            }
            _ => TypeBody::Struct(Vec::new()),
        };
        self.push_type(name, n, body);
    }

    fn rust_enum(&mut self, n: Node) {
        let Some(name) = self.field_text(n, "name") else { return };
        let doc = doc_of(n, self.src);
        let rename_all = doc.attrs.iter().find_map(|a| serde_value(a, "rename_all"));
        let mut variants = Vec::new();
        if let Some(body) = n.child_by_field_name("body") {
            for v in named_children(body).into_iter().filter(|v| v.kind() == "enum_variant") {
                let Some(vname) = self.field_text(v, "name") else { continue };
                let vdoc = doc_of(v, self.src);
                let wire = vdoc
                    .attrs
                    .iter()
                    .find_map(|a| serde_value(a, "rename"))
                    .unwrap_or_else(|| rename(vname, rename_all.as_deref()));
                variants.push(VariantDecl {
                    name: wire,
                    status: vdoc.statuses.into_iter().next(),
                    unit: v.child_by_field_name("body").is_none(),
                });
            }
        }
        self.push_type(name, n, TypeBody::Enum(variants));
    }

    /// `pub use a::B as C;` — `C` is another name for `B`.
    fn rust_use_as(&mut self, n: Node) {
        let (Some(path), Some(alias)) = (self.field_text(n, "path"), self.field_text(n, "alias")) else {
            return;
        };
        let target = path.rsplit("::").next().unwrap_or(path).to_string();
        self.push_type(alias, n, TypeBody::Alias(TypeRef::Named(target)));
    }

    // ── Go ──

    fn go_type_spec(&mut self, n: Node) {
        let (Some(name), Some(ty)) = (self.field_text(n, "name"), n.child_by_field_name("type")) else {
            return;
        };
        match ty.kind() {
            "interface_type" => {
                let mut methods = Vec::new();
                for m in named_children(ty).into_iter().filter(|m| matches!(m.kind(), "method_elem" | "method_spec")) {
                    let Some(mname) = self.field_text(m, "name") else { continue };
                    let params = m.child_by_field_name("parameters").map(|p| go_params(p, self.src)).unwrap_or_default();
                    let signature = go_result(m.child_by_field_name("result"), self.src);
                    methods.push(self.method(m, mname, params, signature));
                }
                // The doc of a lone `type X interface` sits above `type`.
                let holder = match n.parent() {
                    Some(p) if p.kind() == "type_declaration" && named_children(p).len() == 1 => p,
                    _ => n,
                };
                let doc = doc_of(holder, self.src);
                self.push_port(name, n, doc, methods);
                self.push_type(name, n, TypeBody::Struct(Vec::new()));
            }
            "struct_type" => {
                let mut fields = Vec::new();
                let list = named_children(ty).into_iter().find(|c| c.kind() == "field_declaration_list");
                for f in list.map(named_children).unwrap_or_default() {
                    if f.kind() != "field_declaration" {
                        continue;
                    }
                    let Some(fty) = f.child_by_field_name("type") else { continue };
                    let tag = self.field_text(f, "tag").unwrap_or("");
                    let mut c = f.walk();
                    let names: Vec<&str> = f.children_by_field_name("name", &mut c).map(|nm| self.text(nm)).collect();
                    for fname in names {
                        if !fname.starts_with(|ch: char| ch.is_ascii_uppercase()) {
                            continue;
                        }
                        let (wire, omitempty) = match go_json_tag(tag) {
                            Some((w, _)) if w == "-" => continue,
                            Some((w, o)) if !w.is_empty() => (w, o),
                            Some((_, o)) => (fname.to_string(), o),
                            None => (fname.to_string(), false),
                        };
                        fields.push(FieldDecl {
                            wire_name: wire,
                            ty: lower_go(fty, self.src),
                            optional: omitempty,
                            line: line_of(f),
                        });
                    }
                }
                self.push_type(name, n, TypeBody::Struct(fields));
            }
            _ => {
                let inner = lower_go(ty, self.src);
                self.push_type(name, n, TypeBody::Newtype(inner));
            }
        }
    }

    // ── TypeScript ──

    fn ts_interface(&mut self, n: Node) {
        let (Some(name), Some(body)) = (self.field_text(n, "name"), n.child_by_field_name("body")) else {
            return;
        };
        let mut methods = Vec::new();
        for m in named_children(body).into_iter().filter(|m| m.kind() == "method_signature") {
            let Some(mname) = self.field_text(m, "name") else { continue };
            let params = m.child_by_field_name("parameters").map(|p| ts_params(p, self.src)).unwrap_or_default();
            let returns = m
                .child_by_field_name("return_type")
                .and_then(|a| named_children(a).into_iter().next())
                .map_or(TypeRef::Unsupported("no return type".into()), |t| ts_unwrap_promise(t, self.src));
            methods.push(self.method(m, mname, params, (returns, ErrorChannel::Opaque)));
        }
        let fields = ts_fields(body, self.src);
        let doc = doc_of(n, self.src);
        self.push_port(name, n, doc, methods);
        self.push_type(name, n, TypeBody::Struct(fields));
    }

    fn ts_type_alias(&mut self, n: Node) {
        let (Some(name), Some(value)) = (self.field_text(n, "name"), n.child_by_field_name("value")) else {
            return;
        };
        let body = if value.kind() == "object_type" {
            TypeBody::Struct(ts_fields(value, self.src))
        } else {
            TypeBody::Alias(lower_ts(value, self.src))
        };
        self.push_type(name, n, body);
    }

    /// `export { A as B }` — `B` is another name for `A`.
    fn ts_export_as(&mut self, n: Node) {
        let (Some(name), Some(alias)) = (self.field_text(n, "name"), self.field_text(n, "alias")) else {
            return;
        };
        self.push_type(alias, n, TypeBody::Alias(TypeRef::Named(name.to_string())));
    }
}

// ── Rust types ───────────────────────────────────────────

fn type_args(n: Node) -> Vec<Node> {
    n.child_by_field_name("type_arguments")
        .map(named_children)
        .unwrap_or_default()
        .into_iter()
        .filter(|a| !matches!(a.kind(), "lifetime" | "type_binding"))
        .collect()
}

fn scalar(name: &str) -> Option<TypeRef> {
    Some(match name {
        "String" | "str" | "char" | "string" | "rune" => TypeRef::String,
        "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "u8" | "u16" | "u32" | "u64" | "u128" | "usize"
        | "int" | "int8" | "int16" | "int32" | "int64" | "uint" | "uint8" | "uint16" | "uint32" | "uint64"
        | "byte" | "uintptr" => TypeRef::Integer,
        "f32" | "f64" | "float32" | "float64" | "number" => TypeRef::Number,
        "bool" | "boolean" => TypeRef::Boolean,
        _ => return None,
    })
}

fn lower_rust(n: Node, src: &str) -> TypeRef {
    let text = &src[n.byte_range()];
    match n.kind() {
        "primitive_type" | "type_identifier" => scalar(text).unwrap_or_else(|| TypeRef::Named(text.to_string())),
        "scoped_type_identifier" => {
            let last = n.child_by_field_name("name").map_or(text, |l| &src[l.byte_range()]);
            scalar(last).unwrap_or_else(|| TypeRef::Named(last.to_string()))
        }
        "reference_type" => n.child_by_field_name("type").map_or(TypeRef::Unsupported(text.into()), |t| lower_rust(t, src)),
        "unit_type" => TypeRef::Unit,
        "array_type" => n
            .child_by_field_name("element")
            .map_or(TypeRef::Unsupported(text.into()), |e| TypeRef::Array(Box::new(lower_rust(e, src)))),
        "generic_type" => {
            let base = n.child_by_field_name("type").map_or("", |b| &src[b.byte_range()]);
            let base = base.rsplit("::").next().unwrap_or(base);
            let args = type_args(n);
            let arg = |i: usize| args.get(i).map_or(TypeRef::Unsupported(text.into()), |a| lower_rust(*a, src));
            match base {
                "Option" => TypeRef::Optional(Box::new(arg(0))),
                "Vec" | "VecDeque" | "HashSet" | "BTreeSet" => TypeRef::Array(Box::new(arg(0))),
                "HashMap" | "BTreeMap" => TypeRef::Map(Box::new(arg(1))),
                "Box" | "Arc" | "Rc" | "Cow" => arg(0),
                _ => TypeRef::Unsupported(text.to_string()),
            }
        }
        _ => TypeRef::Unsupported(text.to_string()),
    }
}

/// `Result<T, E>` is `T` that fails with `E`; anything else cannot fail.
fn rust_return(n: Node, src: &str) -> (TypeRef, ErrorChannel) {
    if n.kind() == "generic_type" {
        let base = n.child_by_field_name("type").map_or("", |b| &src[b.byte_range()]);
        if base.rsplit("::").next() == Some("Result") {
            let args = type_args(n);
            let ok = args.first().map_or(TypeRef::Unit, |a| lower_rust(*a, src));
            let error = match args.get(1).map(|e| lower_rust(*e, src)) {
                Some(named @ TypeRef::Named(_)) => ErrorChannel::Typed(named),
                _ => ErrorChannel::Opaque,
            };
            return (ok, error);
        }
    }
    (lower_rust(n, src), ErrorChannel::None)
}

/// The value of `key = "…"` in a `#[serde(…)]` attribute.
fn serde_value(attr: &str, key: &str) -> Option<String> {
    serde_parts(attr).find_map(|part| {
        let (k, v) = part.split_once('=')?;
        (k.trim() == key).then(|| v.trim().trim_matches('"').to_string())
    })
}

fn serde_flag(attr: &str, key: &str) -> bool {
    serde_parts(attr).any(|part| part.trim() == key)
}

fn serde_parts(attr: &str) -> impl Iterator<Item = &str> {
    let inner = attr
        .trim()
        .strip_prefix("#[serde(")
        .and_then(|s| s.strip_suffix(")]"))
        .unwrap_or("");
    inner.split(',')
}

/// A Rust name under serde's `rename_all`.
fn rename(name: &str, rule: Option<&str>) -> String {
    let words: Vec<String> = split_words(name);
    match rule {
        Some("camelCase") => lower_camel(name),
        Some("PascalCase") => words.iter().map(|w| capitalize(w)).collect(),
        Some("snake_case") => words.join("_"),
        Some("SCREAMING_SNAKE_CASE") => words.join("_").to_uppercase(),
        Some("kebab-case") => words.join("-"),
        Some("SCREAMING-KEBAB-CASE") => words.join("-").to_uppercase(),
        Some("lowercase") => name.to_lowercase(),
        Some("UPPERCASE") => name.to_uppercase(),
        _ => name.to_string(),
    }
}

// ── Go types ─────────────────────────────────────────────

fn lower_go(n: Node, src: &str) -> TypeRef {
    let text = &src[n.byte_range()];
    match n.kind() {
        "type_identifier" => match text {
            "error" | "any" => TypeRef::Unsupported(text.to_string()),
            _ => scalar(text).unwrap_or_else(|| TypeRef::Named(text.to_string())),
        },
        "qualified_type" => {
            let name = n.child_by_field_name("name").map_or(text, |l| &src[l.byte_range()]);
            TypeRef::Named(name.to_string())
        }
        "pointer_type" => named_children(n)
            .first()
            .map_or(TypeRef::Unsupported(text.into()), |t| TypeRef::Optional(Box::new(lower_go(*t, src)))),
        "slice_type" | "array_type" => n
            .child_by_field_name("element")
            .map_or(TypeRef::Unsupported(text.into()), |e| TypeRef::Array(Box::new(lower_go(e, src)))),
        "map_type" => n
            .child_by_field_name("value")
            .map_or(TypeRef::Unsupported(text.into()), |v| TypeRef::Map(Box::new(lower_go(v, src)))),
        "parenthesized_type" => named_children(n).first().map_or(TypeRef::Unsupported(text.into()), |t| lower_go(*t, src)),
        _ => TypeRef::Unsupported(text.to_string()),
    }
}

fn is_go_context(n: Node, src: &str) -> bool {
    n.kind() == "qualified_type" && &src[n.byte_range()] == "context.Context"
}

fn go_params(list: Node, src: &str) -> Vec<(String, TypeRef)> {
    let mut out = Vec::new();
    for p in named_children(list) {
        if !matches!(p.kind(), "parameter_declaration" | "variadic_parameter_declaration") {
            continue;
        }
        let Some(ty) = p.child_by_field_name("type") else { continue };
        if is_go_context(ty, src) {
            continue;
        }
        let mut lowered = lower_go(ty, src);
        if p.kind() == "variadic_parameter_declaration" {
            lowered = TypeRef::Array(Box::new(lowered));
        }
        let mut c = p.walk();
        let names: Vec<String> = p.children_by_field_name("name", &mut c).map(|nm| src[nm.byte_range()].to_string()).collect();
        if names.is_empty() {
            out.push((String::new(), lowered.clone()));
        }
        for name in names {
            out.push((name, lowered.clone()));
        }
    }
    out
}

/// A trailing `error` makes the method fail; what is left is its answer.
fn go_result(result: Option<Node>, src: &str) -> (TypeRef, ErrorChannel) {
    let types: Vec<Node> = match result {
        None => Vec::new(),
        Some(r) if r.kind() == "parameter_list" => named_children(r)
            .into_iter()
            .filter_map(|p| p.child_by_field_name("type"))
            .collect(),
        Some(r) => vec![r],
    };
    let mut types = types;
    let fails = types.last().is_some_and(|t| &src[t.byte_range()] == "error");
    if fails {
        types.pop();
    }
    let error = if fails { ErrorChannel::Opaque } else { ErrorChannel::None };
    let returns = match types.as_slice() {
        [] => TypeRef::Unit,
        [one] => lower_go(*one, src),
        _ => TypeRef::Unsupported("several results".into()),
    };
    (returns, error)
}

/// `json:"name,omitempty"` → (`name`, true). `None` with no `json` key.
fn go_json_tag(tag: &str) -> Option<(String, bool)> {
    let body = tag.trim_matches('`');
    let start = body.find("json:\"")? + "json:\"".len();
    let end = body[start..].find('"')? + start;
    let mut parts = body[start..end].split(',');
    let name = parts.next().unwrap_or("").to_string();
    let omitempty = parts.any(|o| o == "omitempty");
    Some((name, omitempty))
}

// ── TypeScript types ─────────────────────────────────────

fn lower_ts(n: Node, src: &str) -> TypeRef {
    let text = &src[n.byte_range()];
    match n.kind() {
        "predefined_type" => match text {
            "void" | "undefined" => TypeRef::Unit,
            _ => scalar(text).unwrap_or_else(|| TypeRef::Unsupported(text.to_string())),
        },
        "type_identifier" => TypeRef::Named(text.to_string()),
        "nested_type_identifier" => {
            let name = n.child_by_field_name("name").map_or(text, |l| &src[l.byte_range()]);
            TypeRef::Named(name.to_string())
        }
        "array_type" => named_children(n)
            .first()
            .map_or(TypeRef::Unsupported(text.into()), |e| TypeRef::Array(Box::new(lower_ts(*e, src)))),
        "generic_type" => {
            let base = n.child_by_field_name("name").map_or("", |b| &src[b.byte_range()]);
            let args = type_args(n);
            let arg = |i: usize| args.get(i).map_or(TypeRef::Unsupported(text.into()), |a| lower_ts(*a, src));
            match base {
                "Array" | "ReadonlyArray" | "Set" => TypeRef::Array(Box::new(arg(0))),
                "Record" | "Map" => TypeRef::Map(Box::new(arg(1))),
                _ => TypeRef::Unsupported(text.to_string()),
            }
        }
        "parenthesized_type" => named_children(n).first().map_or(TypeRef::Unsupported(text.into()), |t| lower_ts(*t, src)),
        "union_type" => {
            let mut members = Vec::new();
            flatten_union(n, &mut members);
            let empty = |m: &Node| {
                let t = &src[m.byte_range()];
                matches!(t, "null" | "undefined")
            };
            let rest: Vec<Node> = members.iter().copied().filter(|m| !empty(m)).collect();
            match rest.as_slice() {
                [one] if rest.len() < members.len() => TypeRef::Optional(Box::new(lower_ts(*one, src))),
                _ => TypeRef::Unsupported(text.to_string()),
            }
        }
        _ => TypeRef::Unsupported(text.to_string()),
    }
}

fn flatten_union<'t>(n: Node<'t>, out: &mut Vec<Node<'t>>) {
    for m in named_children(n) {
        if m.kind() == "union_type" {
            flatten_union(m, out);
        } else {
            out.push(m);
        }
    }
}

/// A method's answer: `Promise<T>` is `T`.
fn ts_unwrap_promise(n: Node, src: &str) -> TypeRef {
    if n.kind() == "generic_type" && n.child_by_field_name("name").map(|b| &src[b.byte_range()]) == Some("Promise") {
        return type_args(n).first().map_or(TypeRef::Unit, |a| lower_ts(*a, src));
    }
    lower_ts(n, src)
}

fn ts_annotation(n: Node, src: &str) -> TypeRef {
    n.child_by_field_name("type")
        .and_then(|a| named_children(a).into_iter().next())
        .map_or(TypeRef::Unsupported("untyped".into()), |t| lower_ts(t, src))
}

fn ts_params(list: Node, src: &str) -> Vec<(String, TypeRef)> {
    named_children(list)
        .into_iter()
        .filter(|p| matches!(p.kind(), "required_parameter" | "optional_parameter"))
        .map(|p| {
            let name = p.child_by_field_name("pattern").map_or("", |n| &src[n.byte_range()]).to_string();
            let ty = ts_annotation(p, src);
            let ty = if p.kind() == "optional_parameter" { TypeRef::Optional(Box::new(ty)) } else { ty };
            (name, ty)
        })
        .collect()
}

fn ts_fields(body: Node, src: &str) -> Vec<FieldDecl> {
    named_children(body)
        .into_iter()
        .filter(|p| p.kind() == "property_signature")
        .filter_map(|p| {
            let name = &src[p.child_by_field_name("name")?.byte_range()];
            let mut c = p.walk();
            let optional = p.children(&mut c).any(|ch| ch.kind() == "?");
            Some(FieldDecl {
                wire_name: name.trim_matches(|q| q == '"' || q == '\'').to_string(),
                ty: ts_annotation(p, src),
                optional,
                line: line_of(p),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_split_into_the_same_words_in_every_case() {
        assert_eq!(lower_camel("list_by_tag"), "listByTag");
        assert_eq!(lower_camel("ListByTag"), "listByTag");
        assert_eq!(lower_camel("listByTag"), "listByTag");
        assert_eq!(lower_camel("GetURL"), "getUrl");
        assert_eq!(lower_camel("ID"), "id");
        assert_eq!(rename("saved_at", Some("camelCase")), "savedAt");
        assert_eq!(rename("saved_at", Some("kebab-case")), "saved-at");
        assert_eq!(rename("saved_at", None), "saved_at");
    }

    #[test]
    fn a_tag_must_begin_the_line() {
        assert_eq!(tag_rest("@hexa:api GET /x", API_TAG), Some("GET /x"));
        assert_eq!(tag_rest("@hexa:api", API_TAG), Some(""));
        assert_eq!(tag_rest("@hexa:apis", API_TAG), None);
        assert_eq!(tag_rest("reads the @hexa:api tag", API_TAG), None);
    }

    #[test]
    fn prose_mentioning_the_tag_is_not_a_stray_tag() {
        let src = "/// Reads the `@hexa:api` tag.\npub fn f() {}\nconst S: &str = \"@hexa:api GET /\";\n";
        let facts = extract(src, Language::Rust).unwrap();
        assert!(facts.stray_tags.is_empty(), "{:?}", facts.stray_tags);
    }

    #[test]
    fn a_tag_on_a_free_function_is_stray() {
        let src = "/// @hexa:api GET /health\npub fn health() {}\n";
        let facts = extract(src, Language::Rust).unwrap();
        assert_eq!(facts.stray_tags, vec![1]);
        assert!(facts.ports.is_empty());
    }

    #[test]
    fn go_json_tags_decide_the_wire_name() {
        assert_eq!(go_json_tag("`json:\"note,omitempty\"`"), Some(("note".into(), true)));
        assert_eq!(go_json_tag("`json:\"-\"`"), Some(("-".into(), false)));
        assert_eq!(go_json_tag("`db:\"x\"`"), None);
    }

    #[test]
    fn serde_attributes_are_read_by_key() {
        let a = "#[serde(rename_all = \"camelCase\")]";
        assert_eq!(serde_value(a, "rename_all").as_deref(), Some("camelCase"));
        assert_eq!(serde_value(a, "rename"), None);
        assert!(serde_flag("#[serde(skip)]", "skip"));
        assert!(!serde_flag("#[serde(skip_serializing_if = \"x\")]", "skip"));
    }
}
