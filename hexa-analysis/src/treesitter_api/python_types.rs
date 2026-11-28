//! Python annotations as types (ADR-2611261303 §2, §4).
//!
//! An annotation is read from its text, not from the syntax tree, so that a
//! string annotation and the annotation it holds are read by the same code.
//! Anything this cannot read lowers to [`TypeRef::Unsupported`], which is an
//! error wherever it reaches the wire.

use std::collections::HashMap;

use super::super::ports::TypeRef;

/// Deepest nesting of one annotation, counting the strings inside it: the
/// parser and the lowerers recurse once per level.
const MAX_DEPTH: usize = 64;

/// Names with no schema: the ones §4 refuses, and their relatives.
const REFUSED: &[&str] = &[
    "Any", "Literal", "Annotated", "datetime", "date", "time", "timedelta", "UUID", "Decimal", "set", "Set",
    "frozenset", "FrozenSet", "bytes", "bytearray", "complex", "object",
];

/// Containers, which say nothing without what they contain.
const CONTAINERS: &[&str] =
    &["list", "List", "Sequence", "tuple", "Tuple", "dict", "Dict", "Mapping", "Optional", "Union"];

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Expr {
    /// `a`, or `a.b.c`.
    Name(Vec<String>),
    Subscript(Box<Expr>, Vec<Expr>),
    Union(Vec<Expr>),
    /// A string annotation: what it holds is read when it is lowered.
    Str(String),
    NoneLit,
    Ellipsis,
    /// Text that is not a type.
    Other(String),
}

impl Expr {
    fn text(&self) -> String {
        let list = |items: &[Expr]| items.iter().map(Expr::text).collect::<Vec<_>>();
        match self {
            Expr::Name(parts) => parts.join("."),
            Expr::Subscript(head, args) => format!("{}[{}]", head.text(), list(args).join(", ")),
            Expr::Union(members) => list(members).join(" | "),
            Expr::Str(s) => format!("\"{s}\""),
            Expr::NoneLit => "None".into(),
            Expr::Ellipsis => "...".into(),
            Expr::Other(t) => t.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Name(String),
    Str(String),
    Number(String),
    Ellipsis,
    Punct(char),
}

fn tokens(text: &str) -> Option<Vec<Token>> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() || c == '\\' {
            i += 1;
        } else if c == '#' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '"' || c == '\'' {
            let (s, next) = string_at(&chars, i)?;
            out.push(Token::Str(s));
            i = next;
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            // A prefixed string (`b"…"`, `f"…"`) is not an annotation.
            if matches!(chars.get(i), Some('"' | '\'')) && !matches!(word.as_str(), "r" | "u" | "R" | "U") {
                return None;
            }
            if !matches!(chars.get(i), Some('"' | '\'')) {
                out.push(Token::Name(word));
            }
        } else if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || matches!(chars[i], '_' | '.')) {
                i += 1;
            }
            out.push(Token::Number(chars[start..i].iter().collect()));
        } else if chars[i..].starts_with(&['.', '.', '.']) {
            out.push(Token::Ellipsis);
            i += 3;
        } else if matches!(c, '.' | '[' | ']' | '(' | ')' | ',' | '|' | '-') {
            out.push(Token::Punct(c));
            i += 1;
        } else {
            return None;
        }
    }
    Some(out)
}

/// The content of the string opening at `at`, and the index after it.
fn string_at(chars: &[char], at: usize) -> Option<(String, usize)> {
    let quote = chars[at];
    let triple = chars.get(at + 1) == Some(&quote) && chars.get(at + 2) == Some(&quote);
    let width = if triple { 3 } else { 1 };
    let mut i = at + width;
    let mut out = String::new();
    while i < chars.len() {
        if chars[i] == '\\' {
            out.push(*chars.get(i + 1)?);
            i += 2;
        } else if chars[i] == quote && (!triple || (chars.get(i + 1) == Some(&quote) && chars.get(i + 2) == Some(&quote))) {
            return Some((out, i + width));
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    None
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
}

impl Parser {
    fn eat(&mut self, c: char) -> bool {
        let found = self.tokens.get(self.at) == Some(&Token::Punct(c));
        self.at += usize::from(found);
        found
    }

    fn union(&mut self, depth: usize) -> Option<Expr> {
        let mut members = vec![self.primary(depth)?];
        while self.eat('|') {
            members.push(self.primary(depth)?);
        }
        if members.len() == 1 {
            members.pop()
        } else {
            Some(Expr::Union(members))
        }
    }

    /// Comma-separated expressions up to `close`, and whether a comma was read.
    fn list(&mut self, close: char, depth: usize) -> Option<(Vec<Expr>, bool)> {
        let mut items = Vec::new();
        let mut comma = false;
        while !self.eat(close) {
            items.push(self.union(depth)?);
            if self.eat(',') {
                comma = true;
            } else {
                return self.eat(close).then_some((items, comma));
            }
        }
        Some((items, comma))
    }

    fn primary(&mut self, depth: usize) -> Option<Expr> {
        if depth > MAX_DEPTH {
            return None;
        }
        let token = self.tokens.get(self.at)?.clone();
        self.at += 1;
        let mut expr = match token {
            Token::Name(n) if n == "None" => Expr::NoneLit,
            Token::Name(n) => Expr::Name(vec![n]),
            Token::Str(mut s) => {
                while let Some(Token::Str(more)) = self.tokens.get(self.at) {
                    s.push_str(more);
                    self.at += 1;
                }
                Expr::Str(s)
            }
            Token::Number(n) => Expr::Other(n),
            Token::Ellipsis => Expr::Ellipsis,
            Token::Punct('-') => Expr::Other(format!("-{}", self.primary(depth + 1)?.text())),
            Token::Punct('[') => {
                let (items, _) = self.list(']', depth + 1)?;
                Expr::Other(format!("[{}]", items.iter().map(Expr::text).collect::<Vec<_>>().join(", ")))
            }
            Token::Punct('(') => {
                let (mut items, comma) = self.list(')', depth + 1)?;
                match (items.len(), comma) {
                    (1, false) => items.pop()?,
                    _ => Expr::Other("a tuple".into()),
                }
            }
            Token::Punct(_) => return None,
        };
        let mut trailers = 0;
        loop {
            trailers += 1;
            if trailers > MAX_DEPTH {
                return None;
            }
            if self.eat('.') {
                let Some(Token::Name(next)) = self.tokens.get(self.at).cloned() else { return None };
                self.at += 1;
                let Expr::Name(mut parts) = expr else { return None };
                parts.push(next);
                expr = Expr::Name(parts);
            } else if self.eat('[') {
                let (args, _) = self.list(']', depth + 1)?;
                expr = Expr::Subscript(Box::new(expr), args);
            } else {
                return Some(expr);
            }
        }
    }
}

pub(super) fn parse(text: &str) -> Option<Expr> {
    let mut p = Parser { tokens: tokens(text)?, at: 0 };
    // A top-level comma makes a tuple, which is not a type.
    let expr = p.union(0)?;
    (p.at == p.tokens.len()).then_some(expr)
}

/// The names one file reads its annotations with.
#[derive(Default)]
pub(super) struct Names {
    /// `from m import X as Y`: `Y` is `X` in this file.
    pub(super) aliases: HashMap<String, String>,
}

impl Names {
    pub(super) fn real<'a>(&'a self, name: &'a str) -> &'a str {
        self.aliases.get(name).map_or(name, String::as_str)
    }

    /// What a name or a subscripted name is called: `typing.List[int]` and
    /// an imported-as `L[int]` are both `List`.
    pub(super) fn head<'a>(&'a self, e: &'a Expr) -> Option<&'a str> {
        match e {
            Expr::Name(parts) => match parts.as_slice() {
                [one] => Some(self.real(one)),
                _ => parts.last().map(String::as_str),
            },
            Expr::Subscript(head, _) => self.head(head),
            _ => None,
        }
    }

    /// `e`, or what it holds when it is a string. `Err` is a string that does
    /// not parse.
    pub(super) fn unquote(&self, e: &Expr) -> Result<Expr, TypeRef> {
        let mut e = e.clone();
        for _ in 0..8 {
            let Expr::Str(text) = &e else { return Ok(e) };
            match parse(text) {
                Some(inner) => e = inner,
                None => break,
            }
        }
        Err(TypeRef::Unsupported(format!("the annotation {} does not parse as a type", e.text())))
    }

    /// The subscript arguments of `e` when its head is one of `heads`.
    pub(super) fn applied<'e>(&self, e: &'e Expr, heads: &[&str]) -> Option<&'e [Expr]> {
        let Expr::Subscript(head, args) = e else { return None };
        self.head(head).is_some_and(|h| heads.contains(&h)).then_some(args.as_slice())
    }

    pub(super) fn lower(&self, e: &Expr) -> TypeRef {
        self.lower_at(e, 0)
    }

    /// A type inside another: `None` alone is no type there.
    fn element(&self, e: &Expr, depth: usize) -> TypeRef {
        match self.lower_at(e, depth + 1) {
            TypeRef::Unit => TypeRef::Unsupported("None".into()),
            t => t,
        }
    }

    fn lower_at(&self, e: &Expr, depth: usize) -> TypeRef {
        if depth > MAX_DEPTH {
            return TypeRef::Unsupported("an annotation nested too deeply".into());
        }
        let e = match self.unquote(e) {
            Ok(e) => e,
            Err(why) => return why,
        };
        let unsupported = || TypeRef::Unsupported(e.text());
        match &e {
            Expr::NoneLit => TypeRef::Unit,
            Expr::Name(_) => match self.head(&e) {
                Some("str") => TypeRef::String,
                Some("int") => TypeRef::Integer,
                Some("float") => TypeRef::Number,
                Some("bool") => TypeRef::Boolean,
                Some(n) if REFUSED.contains(&n) || CONTAINERS.contains(&n) => unsupported(),
                Some(n) => TypeRef::Named(n.to_string()),
                None => unsupported(),
            },
            Expr::Subscript(head, args) => match (self.head(head), args.as_slice()) {
                (Some("Optional"), [one]) => self.union(&[one.clone(), Expr::NoneLit], &e, depth),
                (Some("Union"), members) => self.union(members, &e, depth),
                (Some("list" | "List" | "Sequence"), [one]) => TypeRef::Array(Box::new(self.element(one, depth))),
                (Some("tuple" | "Tuple"), [one, Expr::Ellipsis]) => TypeRef::Array(Box::new(self.element(one, depth))),
                (Some("dict" | "Dict" | "Mapping"), [key, value])
                    if self.lower_at(key, depth + 1) == TypeRef::String =>
                {
                    TypeRef::Map(Box::new(self.element(value, depth)))
                }
                _ => unsupported(),
            },
            Expr::Union(members) => self.union(members, &e, depth),
            Expr::Str(_) | Expr::Ellipsis | Expr::Other(_) => unsupported(),
        }
    }

    /// `T | None` in any spelling is an optional `T`; two types are none.
    fn union(&self, members: &[Expr], whole: &Expr, depth: usize) -> TypeRef {
        let mut types = Vec::new();
        let mut nullable = false;
        let mut pending: Vec<(Expr, usize)> = members.iter().rev().map(|m| (m.clone(), depth)).collect();
        while let Some((member, depth)) = pending.pop() {
            if depth > MAX_DEPTH {
                return TypeRef::Unsupported("an annotation nested too deeply".into());
            }
            let member = match self.unquote(&member) {
                Ok(m) => m,
                Err(why) => return why,
            };
            if member == Expr::NoneLit {
                nullable = true;
            } else if let Expr::Union(inner) = &member {
                pending.extend(inner.iter().rev().map(|m| (m.clone(), depth + 1)));
            } else if let Some(inner) = self.applied(&member, &["Union"]) {
                pending.extend(inner.iter().rev().map(|m| (m.clone(), depth + 1)));
            } else if let Some([one]) = self.applied(&member, &["Optional"]) {
                nullable = true;
                pending.push((one.clone(), depth + 1));
            } else {
                types.push((member, depth));
            }
        }
        match types.as_slice() {
            [(one, at)] if nullable => TypeRef::Optional(Box::new(self.element(one, *at))),
            [(one, at)] => self.lower_at(one, at + 1),
            [] => TypeRef::Unit,
            _ => TypeRef::Unsupported(whole.text()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lower(text: &str) -> TypeRef {
        Names::default().lower(&parse(text).unwrap_or(Expr::Other(text.into())))
    }

    #[test]
    fn a_deeply_nested_annotation_is_refused_not_a_stack_overflow() {
        let n = 50_000;
        for text in [
            format!("{}str{}", "list[".repeat(n), "]".repeat(n)),
            format!("{}str{}", "(".repeat(n), ")".repeat(n)),
            format!("{}1", "-".repeat(n)),
            format!("a{}", "[int]".repeat(n)),
        ] {
            assert_eq!(parse(&text), None);
        }
        let flat = format!("str{}", " | None".repeat(n));
        assert_eq!(lower(&flat), TypeRef::Optional(Box::new(TypeRef::String)));
    }

    #[test]
    fn what_is_not_a_type_is_unsupported() {
        for text in ["str, int", "Foo()", "list[str] x", "b\"str\"", "\"list[\"", "1", "[str]", "(str, int)", "dict[int, str]"] {
            assert!(matches!(lower(text), TypeRef::Unsupported(_)), "{text}: {:?}", lower(text));
        }
        assert_eq!(lower("\"\\\"str\\\"\""), TypeRef::String);
        assert_eq!(lower("(str)"), TypeRef::String);
    }

    #[test]
    fn a_local_import_alias_reads_as_what_it_names() {
        let mut names = Names::default();
        names.aliases.insert("L".into(), "List".into());
        names.aliases.insert("Cash".into(), "Money".into());
        let t = names.lower(&parse("L[Cash]").unwrap());
        assert_eq!(t, TypeRef::Array(Box::new(TypeRef::Named("Money".into()))));
    }
}
