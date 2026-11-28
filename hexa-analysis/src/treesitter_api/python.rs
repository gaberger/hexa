//! What a Python file says about the API (ADR-2611261303).
//!
//! A port is a class, a tag is a `#` comment in the run above it or above its
//! first decorator, a signature is its annotations, and a schema is a
//! `@dataclass`, a `TypedDict` or a Pydantic `BaseModel`. What this cannot
//! read is reported, never guessed.

use tree_sitter::{Node, Point};

use super::super::ports::{ApiMethodDecl, ApiPortDecl, ErrorChannel, FieldDecl, TypeBody, TypeRef, WrittenParam};
use super::python_types::{parse, Expr, Names};
use super::{line_of, named_children, tag_rest, Doc, Extractor, API_TAG, STATUS_TAG};

/// A file worth parsing though it has no tag: it may declare, or rename, a
/// type a tagged port elsewhere names.
pub(super) fn mentions_a_type(source: &str) -> bool {
    ["class ", "NewType", " as ", "TypeAlias", "type "].iter().any(|k| source.contains(k))
}

/// A `#` comment's text.
pub(super) fn comment_text(raw: &str) -> String {
    raw.trim().trim_start_matches('#').trim().to_string()
}

pub(super) fn read(x: &mut Extractor, root: Node) {
    let mut reader = Reader { lines: x.src.split('\n').collect(), root, names: Names::default(), x };
    reader.imports(root);
    reader.declarations(root, true);
}

/// Statements that hold statements of the scope they are in.
fn is_block(kind: &str) -> bool {
    matches!(
        kind,
        "if_statement" | "elif_clause" | "else_clause" | "try_statement" | "except_clause" | "except_group_clause"
            | "finally_clause" | "with_statement" | "for_statement" | "while_statement" | "match_statement"
            | "case_clause" | "block"
    )
}

/// The definition a node is, under any decorators.
fn definition(n: Node) -> Node {
    match n.kind() {
        "decorated_definition" => n.child_by_field_name("definition").unwrap_or(n),
        _ => n,
    }
}

/// What a class annotation declares.
struct Field {
    ty: TypeRef,
    /// `Some(true)` for `Required[T]`, `Some(false)` for `NotRequired[T]`.
    marked: Option<bool>,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Dataclass,
    TypedDict { total: bool },
    Model,
}

struct Reader<'a, 'x, 's, 't> {
    x: &'a mut Extractor<'x>,
    lines: Vec<&'s str>,
    root: Node<'t>,
    names: Names,
}

impl<'a, 'x: 's, 's, 't> Reader<'a, 'x, 's, 't> {
    fn text(&self, n: Node) -> &'x str {
        self.x.text(n)
    }

    fn fault(&mut self, line: usize, message: String) {
        self.x.facts.faults.push((line, message));
    }

    /// The module's `from m import X as Y` statements. `Y` is read as `X` in
    /// this file, and declared as another name for it for the files that
    /// import `Y` from here.
    fn imports(&mut self, n: Node) {
        for ch in named_children(n) {
            if ch.kind() == "import_from_statement" {
                let mut c = ch.walk();
                let items: Vec<Node> = ch.children_by_field_name("name", &mut c).collect();
                for item in items.into_iter().filter(|i| i.kind() == "aliased_import") {
                    let (Some(name), Some(alias)) = (item.child_by_field_name("name"), item.child_by_field_name("alias"))
                    else {
                        continue;
                    };
                    let name = self.text(name).rsplit('.').next().unwrap_or("").trim().to_string();
                    let alias = self.text(alias).to_string();
                    if name != alias {
                        self.x.push_type(&alias, item, TypeBody::Alias(TypeRef::Named(name.clone())));
                        self.names.aliases.insert(alias, name);
                    }
                }
            } else if is_block(ch.kind()) {
                self.imports(ch);
            }
        }
    }

    /// Every class, for the ports; the module's own for the types.
    fn declarations(&mut self, n: Node, module: bool) {
        for ch in named_children(n) {
            let d = definition(ch);
            match d.kind() {
                "class_definition" => {
                    self.port(ch, d);
                    if module {
                        self.class_type(ch, d);
                    }
                    self.declarations(d, false);
                }
                "function_definition" => self.declarations(d, false),
                "expression_statement" if module => self.assigned_type(ch),
                "type_alias_statement" if module => {
                    let written = self.text(ch).trim_start_matches("type").trim_start();
                    let name: String = written.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                    self.x.push_type(&name, ch, TypeBody::Foreign("a type alias".into()));
                }
                _ => self.declarations(ch, module && is_block(ch.kind())),
            }
        }
    }

    /// `X = NewType("X", T)` is the type it wraps. Another way of naming a
    /// type is declared too, so a use of it cannot resolve to a class of the
    /// same name elsewhere.
    fn assigned_type(&mut self, statement: Node) {
        let Some(a) = named_children(statement).into_iter().find(|c| c.kind() == "assignment") else { return };
        let (Some(left), Some(right)) = (a.child_by_field_name("left"), a.child_by_field_name("right")) else {
            return;
        };
        if left.kind() != "identifier" {
            return;
        }
        let name = self.text(left);
        let called = (right.kind() == "call").then(|| self.called(right)).flatten();
        let body = match called.as_deref() {
            Some("NewType") => {
                let args = right.child_by_field_name("arguments").map(named_children).unwrap_or_default();
                let wrapped = args.iter().filter(|a| a.kind() != "comment").nth(1);
                TypeBody::Newtype(wrapped.map_or_else(
                    || TypeRef::Unsupported(self.text(right).to_string()),
                    |t| self.annotation(*t),
                ))
            }
            Some("TypedDict" | "NamedTuple" | "Enum") => TypeBody::Foreign("a type made by a call".into()),
            _ => {
                let marked = a.child_by_field_name("type").is_some_and(|t| self.text(t).ends_with("TypeAlias"));
                let looks = matches!(right.kind(), "subscript" | "binary_operator")
                    && name.starts_with(|c: char| c.is_uppercase())
                    && name.contains(|c: char| c.is_lowercase());
                if !marked && !looks {
                    return;
                }
                TypeBody::Foreign("a type alias".into())
            }
        };
        self.x.push_type(name, statement, body);
    }

    fn expr(&self, n: Node) -> Expr {
        let text = self.text(n);
        parse(text).unwrap_or_else(|| Expr::Other(text.to_string()))
    }

    fn annotation(&self, n: Node) -> TypeRef {
        self.names.lower(&self.expr(n))
    }

    // ── Comments ──

    /// The comment node a line holds and nothing else, if it does.
    fn own_comment(&self, row: usize) -> Option<Node<'t>> {
        let line = self.lines.get(row)?;
        let column = line.len() - line.trim_start().len();
        if !line[column..].starts_with('#') {
            return None;
        }
        let at = Point { row, column };
        self.root.descendant_for_point_range(at, at).filter(|n| n.kind() == "comment")
    }

    /// The comment run directly above `row`, read for its tags. Lines are read
    /// from the source: tree-sitter-python can file a column-0 comment inside
    /// the block before it.
    fn run_above(&self, row: usize) -> Doc {
        let mut comments = Vec::new();
        let mut r = row;
        while r > 0 {
            let Some(c) = self.own_comment(r - 1) else { break };
            comments.push(c);
            r -= 1;
        }
        comments.reverse();
        let mut doc = Doc::default();
        let mut prose = Vec::new();
        for c in comments {
            let line = comment_text(self.text(c));
            if let Some(rest) = tag_rest(&line, API_TAG) {
                doc.api = Some(rest.to_string());
                doc.tag_comments.push(c.start_byte());
            } else if let Some(rest) = tag_rest(&line, STATUS_TAG) {
                doc.statuses.push(rest.to_string());
            } else if !line.starts_with("@hexa:") {
                prose.push(line);
            }
        }
        doc.text = prose.join("\n").trim().to_string();
        doc
    }

    // ── Ports ──

    fn port(&mut self, outer: Node, class: Node) {
        let Some(name) = class.child_by_field_name("name").map(|n| self.text(n)) else { return };
        let doc = self.run_above(outer.start_position().row);
        let mut methods = Vec::new();
        let mut tagged = false;
        let body = class.child_by_field_name("body").map(named_children).unwrap_or_default();
        for member in body {
            let function = definition(member);
            if function.kind() != "function_definition" {
                continue;
            }
            let run = self.run_above(member.start_position().row);
            if run.api.is_none() {
                continue;
            }
            tagged = true;
            self.x.consumed.extend(run.tag_comments.iter().copied());
            methods.extend(self.method(member, function, run));
        }
        if doc.api.is_none() && !tagged {
            return;
        }
        if class.has_error() {
            self.fault(line_of(class), format!("`{name}` does not parse, so hexa cannot read all of its API"));
        }
        self.x.consumed.extend(doc.tag_comments);
        self.x.facts.ports.push(ApiPortDecl {
            name: name.to_string(),
            line: line_of(class),
            tag: doc.api,
            statuses: doc.statuses,
            methods,
        });
    }

    /// The last name of each decorator on `outer`: `@a.b(c)` is `b`.
    fn decorators<'n>(&self, outer: Node<'n>) -> Vec<(String, Node<'n>)> {
        if outer.kind() != "decorated_definition" {
            return Vec::new();
        }
        named_children(outer)
            .into_iter()
            .filter(|d| d.kind() == "decorator")
            .map(|d| {
                let written = self.text(d).trim_start_matches('@');
                let path = written.split(['(', '#']).next().unwrap_or("").trim();
                let parts: Vec<&str> = path.split('.').map(str::trim).collect();
                let name = match parts.as_slice() {
                    [one] => self.names.real(one).to_string(),
                    _ => parts.last().copied().unwrap_or("").to_string(),
                };
                (name, d)
            })
            .collect()
    }

    /// A tagged method, or nothing after saying why it cannot be read.
    fn method(&mut self, outer: Node, function: Node, run: Doc) -> Option<ApiMethodDecl> {
        let name = function.child_by_field_name("name").map(|n| self.text(n))?;
        let line = line_of(function);
        let decorators: Vec<String> = self.decorators(outer).into_iter().map(|(n, _)| n).collect();
        for d in &decorators {
            let what = match d.as_str() {
                "overload" => "an overload",
                "property" | "cached_property" | "abstractproperty" | "setter" | "getter" | "deleter" => "a property",
                _ => continue,
            };
            self.fault(line, format!("`{name}` is {what}, which is not an operation; `@hexa:api` tags a method"));
            return None;
        }
        let (params, written, mut ok) = self.parameters(function, name, decorators.iter().any(|d| d == "staticmethod"));
        let returns = match function.child_by_field_name("return_type") {
            Some(t) => self.returned(t),
            None => {
                self.fault(line, format!("`{name}` has no return annotation; write `-> None` if it returns nothing"));
                ok = false;
                TypeRef::Unit
            }
        };
        if !ok {
            return None;
        }
        let mut c = function.walk();
        let is_async = function.children(&mut c).any(|t| t.kind() == "async");
        let doc = if run.text.is_empty() { self.docstring(function) } else { run.text };
        Some(ApiMethodDecl {
            name: name.to_string(),
            line,
            doc,
            tag: run.api,
            statuses: run.statuses,
            params,
            returns,
            // Every Python method can raise, and nothing types what.
            error: ErrorChannel::Opaque,
            written,
            is_async,
        })
    }

    /// `Awaitable[T]` and `Coroutine[_, _, T]` answer with `T`.
    fn returned(&self, n: Node) -> TypeRef {
        let written = self.expr(n);
        let e = match self.names.unquote(&written) {
            Ok(e) => e,
            Err(why) => return why,
        };
        if let Some([one]) = self.names.applied(&e, &["Awaitable"]) {
            return self.names.lower(one);
        }
        if let Some([_, _, last]) = self.names.applied(&e, &["Coroutine"]) {
            return self.names.lower(last);
        }
        self.names.lower(&e)
    }

    /// The parameters the API sees, and whether every one could be read. The
    /// first of a method that is not static is its receiver.
    fn parameters(&mut self, function: Node, method: &str, is_static: bool) -> (Vec<(String, TypeRef)>, Vec<WrittenParam>, bool) {
        let line = line_of(function);
        let mut params = Vec::new();
        let mut written = Vec::new();
        let mut ok = true;
        let mut receiver = !is_static;
        let list = function.child_by_field_name("parameters").map(named_children).unwrap_or_default();
        for p in list {
            match p.kind() {
                "comment" | "positional_separator" => continue,
                "keyword_separator" => {
                    // Nothing positional comes before it: there is no receiver.
                    receiver = false;
                    continue;
                }
                _ => {}
            }
            // The name, or the `*args` pattern, whichever kind of parameter.
            let target = match p.kind() {
                "default_parameter" | "typed_default_parameter" => p.child_by_field_name("name"),
                "typed_parameter" => named_children(p).into_iter().next(),
                _ => Some(p),
            };
            let Some(target) = target else { continue };
            // tree-sitter reads a bare `*` with an annotation-free list as
            // a separator already; a splat takes any number of values.
            if matches!(target.kind(), "list_splat_pattern" | "dictionary_splat_pattern") {
                let text = self.text(target);
                self.fault(line, format!("`{method}` takes `{text}`, which names no parameter of the API"));
                ok = false;
                receiver = false;
                continue;
            }
            if std::mem::take(&mut receiver) {
                continue;
            }
            let name = self.text(target).to_string();
            let Some(annotation) = p.child_by_field_name("type") else {
                self.fault(line, format!("parameter `{name}` of `{method}` has no annotation"));
                ok = false;
                continue;
            };
            let ty = self.annotation(annotation);
            let none_default = p.child_by_field_name("value").is_some_and(|v| v.kind() == "none");
            let ty = match ty {
                TypeRef::Unit => TypeRef::Unsupported("None".into()),
                TypeRef::Optional(_) if !none_default => {
                    self.fault(
                        line,
                        format!(
                            "parameter `{name}` of `{method}` allows None without a None default; the contract cannot say it must be sent and may be null"
                        ),
                    );
                    ty
                }
                other => other,
            };
            written.push(WrittenParam { name: name.clone(), written: self.text(annotation).to_string(), context: false });
            params.push((name, ty));
        }
        (params, written, ok)
    }

    /// The method's docstring as `inspect.cleandoc` cleans it, or nothing.
    fn docstring(&self, function: Node) -> String {
        let first = function
            .child_by_field_name("body")
            .and_then(|b| named_children(b).into_iter().find(|c| c.kind() != "comment"))
            .filter(|s| s.kind() == "expression_statement")
            .and_then(|s| named_children(s).into_iter().next());
        let Some(node) = first else { return String::new() };
        let parts = match node.kind() {
            "string" => vec![node],
            "concatenated_string" => named_children(node).into_iter().filter(|c| c.kind() == "string").collect(),
            _ => return String::new(),
        };
        let mut text = String::new();
        for part in parts {
            let Some(value) = string_value(self.text(part)) else { return String::new() };
            text.push_str(&value);
        }
        clean_doc(&text)
    }

    // ── Schema classes ──

    fn class_type(&mut self, outer: Node, class: Node) {
        let Some(name) = class.child_by_field_name("name").map(|n| self.text(n)) else { return };
        let body = self.class_body(outer, class);
        self.x.push_type(name, class, body);
    }

    fn class_body(&mut self, outer: Node, class: Node) -> TypeBody {
        let arguments = class.child_by_field_name("superclasses").map(named_children).unwrap_or_default();
        let mut bases = Vec::new();
        let mut keywords = Vec::new();
        for a in arguments.into_iter().filter(|a| a.kind() != "comment") {
            match a.kind() {
                "keyword_argument" | "dictionary_splat" | "list_splat" => keywords.push(a),
                _ => {
                    let e = self.expr(a);
                    bases.push(self.names.head(&e).map_or_else(|| self.text(a).to_string(), str::to_string));
                }
            }
        }
        let decorators = self.decorators(outer);
        let dataclass = decorators.iter().any(|(n, _)| n == "dataclass");
        let typed_dict = bases.iter().any(|b| b == "TypedDict");
        let model = bases.iter().any(|b| b == "BaseModel");
        if !(dataclass || typed_dict || model) {
            return if bases.is_empty() {
                TypeBody::Foreign("a class that is not a dataclass, a TypedDict or a Pydantic BaseModel".into())
            } else {
                TypeBody::Derived(bases)
            };
        }
        let refused = |why: String| TypeBody::Refused(why);
        if let Some(other) = bases.iter().find(|b| !matches!(b.as_str(), "BaseModel" | "TypedDict")) {
            return refused(format!("it has the base `{other}`, which may change what is sent"));
        }
        if bases.len() > 1 || (dataclass && !bases.is_empty()) {
            return refused("it is more than one kind of schema class".into());
        }
        if let Some((other, _)) = decorators.iter().find(|(n, _)| n != "dataclass") {
            return refused(format!("it has the decorator `@{other}`, which may change what is sent"));
        }
        if decorators.iter().any(|(_, d)| self.keyword_names(*d).iter().any(|k| k == "config")) {
            return refused("its `@dataclass` is given a `config`".into());
        }
        let mut total = true;
        for k in keywords {
            let key = k.child_by_field_name("name").map_or("", |n| self.text(n));
            let value = k.child_by_field_name("value").map_or("", |n| self.text(n));
            match (key, typed_dict, model) {
                ("total", true, _) if matches!(value, "True" | "False") => total = value == "True",
                ("frozen", _, true) => {}
                _ => return refused(format!("the class keyword `{}` may change what is sent", self.text(k))),
            }
        }
        if class.has_error() {
            return refused("it does not parse".into());
        }
        let kind = if typed_dict {
            Kind::TypedDict { total }
        } else if model {
            Kind::Model
        } else {
            Kind::Dataclass
        };
        let mut fields: Vec<FieldDecl> = Vec::new();
        let body = class.child_by_field_name("body").map(named_children).unwrap_or_default();
        for statement in body {
            if let Err(why) = self.member(statement, kind, &mut fields) {
                return refused(why);
            }
        }
        TypeBody::Struct(fields)
    }

    /// The names of the keyword arguments of the call a decorator makes.
    fn keyword_names(&self, decorator: Node) -> Vec<String> {
        named_children(decorator)
            .into_iter()
            .filter(|c| c.kind() == "call")
            .filter_map(|c| c.child_by_field_name("arguments"))
            .flat_map(named_children)
            .filter_map(|a| a.child_by_field_name("name").map(|n| self.text(n).to_string()))
            .collect()
    }

    /// One statement of a schema class's body: a field, or something that
    /// does or does not change what is sent. `Err` refuses the class.
    fn member(&mut self, statement: Node, kind: Kind, fields: &mut Vec<FieldDecl>) -> Result<(), String> {
        let d = definition(statement);
        match d.kind() {
            "function_definition" => {
                let changes = |n: &str| matches!(n, "computed_field" | "field_serializer" | "model_serializer");
                match self.decorators(statement).iter().find(|(n, _)| changes(n)) {
                    Some((n, _)) => Err(format!("it has a `@{n}` method, which changes what is sent")),
                    None => Ok(()),
                }
            }
            "class_definition" => {
                if d.child_by_field_name("name").map(|n| self.text(n)) == Some("Config") {
                    self.config_class(d)
                } else {
                    Ok(())
                }
            }
            "expression_statement" => {
                for a in named_children(statement) {
                    if matches!(a.kind(), "assignment" | "augmented_assignment") {
                        self.class_assignment(a, kind, fields)?;
                    }
                }
                Ok(())
            }
            k if is_block(k) => {
                // What a branch declares may or may not be there.
                for inner in named_children(statement) {
                    let mut declared = Vec::new();
                    self.member(inner, kind, &mut declared)?;
                    if !declared.is_empty() {
                        return Err("it declares a field conditionally".into());
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn class_assignment(&mut self, a: Node, kind: Kind, fields: &mut Vec<FieldDecl>) -> Result<(), String> {
        let Some(left) = a.child_by_field_name("left") else { return Ok(()) };
        if left.kind() != "identifier" {
            return Ok(());
        }
        let name = self.text(left).to_string();
        let right = a.child_by_field_name("right");
        // A pydantic dataclass takes its configuration under the second name.
        if name == "model_config" || name == "__pydantic_config__" {
            return match right {
                Some(value) if a.kind() != "assignment" || !self.only_frozen(value) => {
                    Err("its `model_config` sets something other than `frozen`".into())
                }
                _ => Ok(()),
            };
        }
        let Some(annotation) = a.child_by_field_name("type") else { return Ok(()) };
        let Some(field) = self.field_type(annotation, kind) else { return Ok(()) };
        let value = right.map(|v| self.field_value(v));
        let none_default = value == Some(Some(true));
        let Field { mut ty, marked } = field;
        let required = match kind {
            Kind::TypedDict { total } => marked.unwrap_or(total),
            _ => !(matches!(ty, TypeRef::Optional(_)) && none_default),
        };
        let mut fault = None;
        if name.starts_with('_') {
            ty = TypeRef::Unsupported(format!("{name}, a field whose name begins with `_`"));
        } else if value == Some(None) {
            let written = right.map_or("", |v| self.text(v));
            ty = TypeRef::Unsupported(format!("{name} = {written}, a field value hexa cannot read"));
        } else if required && matches!(ty, TypeRef::Optional(_)) {
            fault = Some(format!(
                "field `{name}` allows None and is required; the contract cannot say it is present and may be null"
            ));
        }
        let decl = FieldDecl { wire_name: name, ty, optional: !required, line: line_of(a), fault };
        match fields.iter_mut().find(|f| f.wire_name == decl.wire_name) {
            Some(first) => *first = decl,
            None => fields.push(decl),
        }
        Ok(())
    }

    /// The type of an annotated field, or `None` when the annotation does not
    /// declare one: `ClassVar`, `InitVar` and `KW_ONLY`.
    fn field_type(&self, annotation: Node, kind: Kind) -> Option<Field> {
        let written = self.expr(annotation);
        let e = match self.names.unquote(&written) {
            Ok(e) => e,
            Err(why) => return Some(Field { ty: why, marked: None }),
        };
        if matches!(self.names.head(&e), Some("ClassVar" | "InitVar" | "KW_ONLY")) {
            return None;
        }
        if matches!(kind, Kind::TypedDict { .. }) {
            for (head, required) in [("Required", true), ("NotRequired", false)] {
                if let Some([inner]) = self.names.applied(&e, &[head]) {
                    return Some(Field { ty: self.wire_type(inner), marked: Some(required) });
                }
            }
        }
        Some(Field { ty: self.wire_type(&e), marked: None })
    }

    fn wire_type(&self, e: &Expr) -> TypeRef {
        match self.names.lower(e) {
            TypeRef::Unit => TypeRef::Unsupported("None".into()),
            t => t,
        }
    }

    /// The last name of what a call calls.
    fn called(&self, call: Node) -> Option<String> {
        let function = call.child_by_field_name("function")?;
        self.names.head(&self.expr(function)).map(str::to_string)
    }

    /// Whether a field's value is one that cannot rename or drop it: a
    /// literal, `None`, `field(…)` or `Field(…)` with only a default and a
    /// description. `Some(true)` when the default it gives is `None`;
    /// `None` when it cannot be read.
    fn field_value(&self, v: Node) -> Option<bool> {
        match v.kind() {
            "none" => Some(true),
            "call" => {
                let pydantic = match self.called(v)?.as_str() {
                    "field" => false,
                    "Field" => true,
                    _ => return None,
                };
                let arguments = v.child_by_field_name("arguments").filter(|a| a.kind() == "argument_list")?;
                let mut positional = 0;
                let mut none_default = false;
                for a in named_children(arguments).into_iter().filter(|a| a.kind() != "comment") {
                    match a.kind() {
                        "keyword_argument" => {
                            let key = self.text(a.child_by_field_name("name")?);
                            match key {
                                "default" => none_default = a.child_by_field_name("value")?.kind() == "none",
                                "default_factory" => {}
                                "description" if pydantic => {}
                                _ => return None,
                            }
                        }
                        "list_splat" | "dictionary_splat" => return None,
                        // Pydantic's first positional argument is the default.
                        _ if pydantic && positional == 0 => {
                            positional = 1;
                            none_default = a.kind() == "none";
                        }
                        _ => return None,
                    }
                }
                Some(none_default)
            }
            _ => is_literal(v).then_some(false),
        }
    }

    /// Whether a `model_config` value sets nothing but `frozen`.
    fn only_frozen(&self, v: Node) -> bool {
        match v.kind() {
            "call" => {
                matches!(self.called(v).as_deref(), Some("ConfigDict" | "SettingsConfigDict" | "dict"))
                    && v.child_by_field_name("arguments").is_some_and(|args| {
                        named_children(args).into_iter().filter(|a| a.kind() != "comment").all(|a| {
                            a.kind() == "keyword_argument"
                                && a.child_by_field_name("name").is_some_and(|n| self.text(n) == "frozen")
                        })
                    })
            }
            "dictionary" => named_children(v).into_iter().filter(|a| a.kind() != "comment").all(|pair| {
                pair.child_by_field_name("key")
                    .is_some_and(|k| string_value(self.text(k)).as_deref() == Some("frozen"))
            }),
            _ => false,
        }
    }

    /// A `class Config` may set `frozen` and nothing else.
    fn config_class(&self, class: Node) -> Result<(), String> {
        let only_frozen = class.child_by_field_name("body").is_some_and(|body| {
            named_children(body).into_iter().all(|s| match s.kind() {
                "comment" | "pass_statement" => true,
                "expression_statement" => named_children(s).into_iter().all(|c| match c.kind() {
                    "string" | "ellipsis" => true,
                    "assignment" => c.child_by_field_name("left").is_some_and(|l| self.text(l) == "frozen"),
                    _ => false,
                }),
                _ => false,
            })
        });
        if only_frozen && class.child_by_field_name("superclasses").is_none() {
            Ok(())
        } else {
            Err("its `class Config` sets something other than `frozen`".into())
        }
    }
}

fn is_literal(n: Node) -> bool {
    match n.kind() {
        "integer" | "float" | "true" | "false" | "none" | "ellipsis" => true,
        "string" => !named_children(n).iter().any(|c| c.kind() == "interpolation"),
        "unary_operator" => n.child_by_field_name("argument").is_some_and(|a| matches!(a.kind(), "integer" | "float")),
        "concatenated_string" | "list" | "tuple" | "set" | "dictionary" | "pair" | "parenthesized_expression" => {
            named_children(n).into_iter().filter(|c| c.kind() != "comment").all(is_literal)
        }
        _ => false,
    }
}

/// What a string literal holds. `None` for bytes and f-strings, which are not
/// text. A `\N{…}` escape is left as written.
fn string_value(literal: &str) -> Option<String> {
    let start = literal.find(['"', '\''])?;
    let prefix = literal[..start].to_ascii_lowercase();
    if prefix.contains(['b', 'f', 't']) {
        return None;
    }
    let rest = &literal[start..];
    let width = if rest.starts_with("\"\"\"") || rest.starts_with("'''") { 3 } else { 1 };
    let inner = rest.get(width..rest.len().checked_sub(width)?)?.replace("\r\n", "\n");
    if prefix.contains('r') {
        return Some(inner);
    }
    let mut out = String::new();
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let Some(e) = chars.next() else {
            out.push('\\');
            break;
        };
        match e {
            '\n' => {}
            '\\' | '\'' | '"' => out.push(e),
            'a' => out.push('\u{7}'),
            'b' => out.push('\u{8}'),
            'f' => out.push('\u{c}'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            'v' => out.push('\u{b}'),
            '0'..='7' => {
                let mut code = e.to_digit(8).unwrap_or(0);
                for _ in 0..2 {
                    let Some(d) = chars.peek().and_then(|d| d.to_digit(8)) else { break };
                    code = code * 8 + d;
                    chars.next();
                }
                out.extend(char::from_u32(code));
            }
            'x' | 'u' | 'U' => {
                let digits = match e {
                    'x' => 2,
                    'u' => 4,
                    _ => 8,
                };
                let hex: String = (0..digits).filter_map(|_| chars.next()).collect();
                out.extend(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32));
            }
            other => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    Some(out)
}

/// `inspect.cleandoc`: tabs expanded, the first line stripped, the indent the
/// later lines share removed, and blank lines at either end dropped.
fn clean_doc(doc: &str) -> String {
    let mut lines: Vec<String> = doc.split('\n').map(expand_tabs).collect();
    let indent = |l: &str| l.len() - l.trim_start_matches(' ').len();
    let margin = lines.iter().skip(1).filter(|l| !l.trim_start_matches(' ').is_empty()).map(|l| indent(l)).min();
    for (i, line) in lines.iter_mut().enumerate() {
        if i == 0 {
            *line = line.trim_start().to_string();
        } else if let Some(margin) = margin {
            *line = line.get(margin..).unwrap_or("").to_string();
        }
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    let blank = lines.iter().take_while(|l| l.is_empty()).count();
    lines[blank..].join("\n")
}

/// Python's `str.expandtabs()` for one line: tab stops every eight columns.
fn expand_tabs(line: &str) -> String {
    let mut out = String::new();
    let mut column = 0;
    for c in line.chars() {
        if c == '\t' {
            let spaces = 8 - column % 8;
            out.extend(std::iter::repeat_n(' ', spaces));
            column += spaces;
        } else {
            out.push(c);
            column += 1;
        }
    }
    out
}
