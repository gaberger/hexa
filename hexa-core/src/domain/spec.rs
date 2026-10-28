//! An ADR's numbered decisions, stated as behaviour (ADR-2610221540).
//!
//! A spec maps each `§n` of an ADR's `## Decision` to `given / when / then`
//! scenarios, and each scenario to one test. Everything here is a value or
//! a pure judgement over values. Reading files, hashing and running tests
//! are the adapter's.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// `docs/specs/<ADR-id>.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Spec {
    pub adr: String,
    pub feature: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Decisions that state a rule over all inputs; each needs a property.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub universal: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exempt: Vec<Exemption>,
    /// Who accepted which content (§4). Not part of the accepted content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted: Option<Acceptance>,
    /// Test file → SHA-256 at sealing (§6). Not part of the accepted content.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sealed: BTreeMap<String, String>,
    /// The commit at which the generated tests were red (§5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub red: Option<String>,
    pub specs: Vec<Scenario>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scenario {
    pub id: String,
    /// The `§n` this scenario proves.
    pub decision: Vec<String>,
    pub kind: Kind,
    pub description: String,
    #[serde(default)]
    pub given: String,
    #[serde(default)]
    pub when: String,
    #[serde(default)]
    pub then: String,
    /// `<path>::<test_fn>`, once the test exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test: Option<String>,
    /// The commit at which this scenario's test existed and failed, when it
    /// differs from the spec's `red` (§5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub red: Option<String>,
}

impl Scenario {
    /// This scenario's red commit, else the spec's. Empty is none.
    pub fn red_commit<'a>(&'a self, spec: &'a Spec) -> Option<&'a str> {
        self.red.as_deref().or(spec.red.as_deref()).filter(|r| !r.trim().is_empty())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Example,
    Negative,
    Property,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Exemption {
    pub decision: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Acceptance {
    pub by: String,
    pub sha256: String,
}

impl Spec {
    /// The spec as a person reads and accepts it: the behaviour. Not the
    /// record of acceptance, the seal, the red commit or the test refs, which
    /// are all written after acceptance and must not void it.
    pub fn accepted_content(&self) -> Spec {
        let specs = self.specs.iter().map(|s| Scenario { test: None, red: None, ..s.clone() }).collect();
        Spec { accepted: None, sealed: BTreeMap::new(), red: None, specs, ..self.clone() }
    }

    /// The test files the scenarios name, which sealing hashes.
    pub fn test_files(&self) -> BTreeSet<String> {
        self.specs
            .iter()
            .filter_map(|s| s.test.as_deref().and_then(test_ref))
            .map(|(path, _)| path.to_string())
            .collect()
    }
}

/// `<path>.rs::<test_fn>` split after the file. The function side may carry
/// a module path (`src/x.rs::tests::f`) for a test inside a library.
pub fn test_ref(r: &str) -> Option<(&str, &str)> {
    let at = r.find(".rs::")? + 3;
    let (path, f) = (&r[..at], &r[at + 2..]);
    (path.len() > 3 && !f.is_empty()).then_some((path, f))
}

/// The `§n` of each numbered item at the left margin of `## Decision`.
pub fn numbered_decisions(adr_markdown: &str) -> Vec<String> {
    let Some(after) = adr_markdown.split("\n## Decision").nth(1) else { return Vec::new() };
    let section = after.split("\n## ").next().unwrap_or(after);
    section
        .lines()
        .filter_map(|l| {
            let (n, rest) = l.split_once(". ")?;
            (!n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) && !rest.is_empty()).then(|| format!("§{n}"))
        })
        .collect()
}

/// What `hexa spec check` refuses (§3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finding {
    /// No scenario cites the decision and nothing exempts it.
    Uncovered(String),
    NoExample(String),
    NoNegative(String),
    /// Declared universal, and no property scenario cites it.
    NoProperty(String),
    /// A scenario cites a `§n` the ADR does not make.
    UnknownDecision { scenario: String, decision: String },
    ExemptWithoutReason(String),
    DuplicateScenario(String),
    /// The spec names a different ADR than the one it is checked against.
    WrongAdr { expected: String, found: String },
}

impl Finding {
    pub fn message(&self) -> String {
        match self {
            Finding::Uncovered(d) => format!("{d} uncovered: no scenario cites it and no exemption names it"),
            Finding::NoExample(d) => format!("{d} has no example scenario"),
            Finding::NoNegative(d) => format!("{d} has no negative scenario: nothing says what it must refuse"),
            Finding::NoProperty(d) => format!("{d} is universal and has no property scenario"),
            Finding::UnknownDecision { scenario, decision } => {
                format!("{scenario} cites {decision}, which the ADR does not decide")
            }
            Finding::ExemptWithoutReason(d) => format!("{d} is exempted without a reason"),
            Finding::DuplicateScenario(id) => format!("scenario id {id} is used twice"),
            Finding::WrongAdr { expected, found } => format!("the spec names {found}, not {expected}"),
        }
    }
}

/// Every way `spec` fails to cover `decisions` (§3). Empty means covered.
pub fn coverage(adr_id: &str, decisions: &[String], spec: &Spec) -> Vec<Finding> {
    let mut out = Vec::new();
    if spec.adr != adr_id {
        out.push(Finding::WrongAdr { expected: adr_id.to_string(), found: spec.adr.clone() });
    }
    let mut seen = BTreeSet::new();
    for s in &spec.specs {
        if !seen.insert(s.id.as_str()) {
            out.push(Finding::DuplicateScenario(s.id.clone()));
        }
        for d in &s.decision {
            if !decisions.contains(d) {
                out.push(Finding::UnknownDecision { scenario: s.id.clone(), decision: d.clone() });
            }
        }
    }
    for e in &spec.exempt {
        if e.reason.trim().is_empty() {
            out.push(Finding::ExemptWithoutReason(e.decision.clone()));
        }
    }
    for d in decisions {
        if spec.exempt.iter().any(|e| &e.decision == d && !e.reason.trim().is_empty()) {
            continue;
        }
        let citing: Vec<Kind> = spec.specs.iter().filter(|s| s.decision.contains(d)).map(|s| s.kind).collect();
        if citing.is_empty() {
            out.push(Finding::Uncovered(d.clone()));
            continue;
        }
        if !citing.contains(&Kind::Example) {
            out.push(Finding::NoExample(d.clone()));
        }
        if !citing.contains(&Kind::Negative) {
            out.push(Finding::NoNegative(d.clone()));
        }
        if spec.universal.contains(d) && !citing.contains(&Kind::Property) {
            out.push(Finding::NoProperty(d.clone()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sc(id: &str, d: &[&str], kind: Kind) -> Scenario {
        Scenario {
            id: id.into(),
            decision: d.iter().map(|s| s.to_string()).collect(),
            kind,
            description: String::new(),
            given: String::new(),
            when: String::new(),
            then: String::new(),
            test: None,
            red: None,
        }
    }

    fn spec(specs: Vec<Scenario>) -> Spec {
        Spec {
            adr: "A".into(),
            feature: "f".into(),
            description: String::new(),
            universal: vec![],
            exempt: vec![],
            accepted: None,
            sealed: BTreeMap::new(),
            red: None,
            specs,
        }
    }

    #[test]
    fn numbered_items_at_the_margin_are_decisions_and_nothing_else_is() {
        let md = "# A\n\n## Context\n\n1. not a decision\n\n## Decision\n\n1. **One.** x\n   2. nested, not one\n2. **Two.**\n\n   - a bullet\n10. **Ten.**\n\n## Consequences\n\n3. not one\n";
        assert_eq!(numbered_decisions(md), vec!["§1", "§2", "§10"]);
        assert!(numbered_decisions("# A\n\n## Context\n\n1. x\n").is_empty());
    }

    #[test]
    fn coverage_needs_an_example_and_a_negative_per_decision() {
        let ds = vec!["§1".to_string(), "§2".to_string()];
        let ok = spec(vec![sc("S1", &["§1", "§2"], Kind::Example), sc("S2", &["§1", "§2"], Kind::Negative)]);
        assert!(coverage("A", &ds, &ok).is_empty());
        let half = spec(vec![sc("S1", &["§1"], Kind::Example)]);
        let f = coverage("A", &ds, &half);
        assert!(f.contains(&Finding::NoNegative("§1".into())));
        assert!(f.contains(&Finding::Uncovered("§2".into())));
    }

    #[test]
    fn accepted_content_ignores_what_is_written_after_acceptance() {
        let mut a = spec(vec![sc("S1", &["§1"], Kind::Example)]);
        let b = a.clone();
        a.accepted = Some(Acceptance { by: "p".into(), sha256: "h".into() });
        a.sealed.insert("t.rs".into(), "h".into());
        a.red = Some("abc1234".into());
        a.specs[0].test = Some("tests/a.rs::f".into());
        a.specs[0].red = Some("def5678".into());
        assert_eq!(a.accepted_content(), b.accepted_content());
        a.specs[0].then = "something else".into();
        assert_ne!(a.accepted_content(), b.accepted_content(), "behaviour is what was accepted");
    }

    #[test]
    fn a_test_ref_splits_after_the_file() {
        assert_eq!(test_ref("tests/a.rs::f"), Some(("tests/a.rs", "f")));
        assert_eq!(test_ref("src/x.rs::tests::f"), Some(("src/x.rs", "tests::f")));
        assert_eq!(test_ref("tests/a.rs"), None);
        assert_eq!(test_ref(".rs::f"), None);
        assert_eq!(test_ref("tests/a.rs::"), None);
    }
}

/// Which bytes of Rust source are code, not a string, char or comment.
/// Brace matching and macro search read only code bytes.
fn code_mask(src: &str) -> Vec<bool> {
    let b = src.as_bytes();
    let mut mask = vec![true; b.len()];
    let mut i = 0;
    while i < b.len() {
        let start = i;
        if b[i] == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
            let mut depth = 0;
            while i < b.len() {
                if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
                    depth += 1;
                    i += 2;
                } else if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
        } else if b[i] == b'r' && (b.get(i + 1) == Some(&b'"') || b.get(i + 1) == Some(&b'#'))
            && (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_'))
        {
            let mut j = i + 1;
            let mut hashes = 0;
            while b.get(j) == Some(&b'#') {
                hashes += 1;
                j += 1;
            }
            if b.get(j) != Some(&b'"') {
                i += 1;
                continue;
            }
            j += 1;
            loop {
                if j >= b.len() {
                    break;
                }
                if b[j] == b'"' && (0..hashes).all(|k| b.get(j + 1 + k) == Some(&b'#')) {
                    j += 1 + hashes;
                    break;
                }
                j += 1;
            }
            i = j;
        } else if b[i] == b'"' {
            i += 1;
            while i < b.len() && b[i] != b'"' {
                if b[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i += 1;
        } else if b[i] == b'\'' {
            // A char literal: 'x', '\n', '\'', '\u{..}'; otherwise a lifetime.
            let close = if b.get(i + 1) == Some(&b'\\') {
                (i + 3..b.len().min(i + 12)).find(|&k| b[k] == b'\'')
            } else {
                let ch_len = src[i + 1..].chars().next().map(char::len_utf8).unwrap_or(1);
                (b.get(i + 1 + ch_len) == Some(&b'\'')).then_some(i + 1 + ch_len)
            };
            match close {
                Some(k) => i = k + 1,
                None => {
                    i += 1;
                    continue;
                }
            }
        } else {
            i += 1;
            continue;
        }
        for m in mask.iter_mut().take(i.min(b.len())).skip(start) {
            *m = false;
        }
    }
    mask
}

/// The index just past the bracket matching the one at `open`.
fn matching(b: &[u8], mask: &[bool], open: usize) -> usize {
    let (o, c) = (b[open], match b[open] { b'{' => b'}', b'(' => b')', _ => b']' });
    let mut depth = 0usize;
    for i in open..b.len() {
        if !mask[i] {
            continue;
        }
        if b[i] == o {
            depth += 1;
        } else if b[i] == c {
            depth -= 1;
            if depth == 0 {
                return i + 1;
            }
        }
    }
    b.len()
}

/// One item of a Rust file or block: its leading docs and attributes, its
/// header (up to its body), and its body's span if it has one.
struct Item {
    /// From its first doc comment or attribute to its end.
    span: (usize, usize),
    lead: Vec<String>,
    header: String,
    body: Option<(usize, usize)>,
    text: String,
    test_only: bool,
}

/// The items of `src[from..to]`, in order.
fn items(src: &str, mask: &[bool], from: usize, to: usize) -> Vec<Item> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = from;
    let mut lead: Vec<String> = Vec::new();
    let mut lead_start: Option<usize> = None;
    let mut test_only = false;
    while i < to {
        if b[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if !mask[i] {
            // A comment: keep `///` docs for the item that follows.
            let mut j = i;
            while j < to && !mask[j] {
                j += 1;
            }
            let c = src[i..j].trim();
            if c.starts_with("///") && !c.starts_with("////") {
                lead_start.get_or_insert(i);
                for l in c.lines() {
                    lead.push(l.trim().to_string());
                }
            }
            i = j;
            continue;
        }
        if b[i] == b'#' {
            let inner = b.get(i + 1) == Some(&b'!');
            let open = if inner { i + 2 } else { i + 1 };
            if b.get(open) == Some(&b'[') {
                let end = matching(b, mask, open);
                let attr = src[i..end].trim().to_string();
                if !inner {
                    lead_start.get_or_insert(i);
                    let compact: String = attr.chars().filter(|c| !c.is_whitespace()).collect();
                    test_only |= compact == "#[cfg(test)]" || compact == "#[test]";
                    lead.push(attr);
                }
                i = end;
                continue;
            }
        }
        // The item: up to a `;` or a body `{ … }` at depth 0.
        let start = i;
        let mut j = i;
        let mut body = None;
        let mut end = to;
        while j < to {
            if !mask[j] {
                j += 1;
                continue;
            }
            match b[j] {
                b'(' | b'[' => j = matching(b, mask, j),
                b';' => {
                    end = j + 1;
                    break;
                }
                b'{' => {
                    let close = matching(b, mask, j);
                    body = Some((j, close));
                    end = close;
                    // `pub const X: T = T { … };` and `struct S { … }` alike.
                    let rest = src[close..to].trim_start();
                    if rest.starts_with(';') {
                        end = close + (src[close..to].len() - rest.len()) + 1;
                    }
                    break;
                }
                _ => j += 1,
            }
        }
        let header = match body {
            Some((open, _)) => src[start..open].trim_end().to_string(),
            None => src[start..end].trim_end().to_string(),
        };
        let span = (lead_start.take().unwrap_or(start), end);
        out.push(Item { span, lead: std::mem::take(&mut lead), header, body, text: src[start..end].trim().to_string(), test_only });
        test_only = false;
        i = end.max(start + 1);
    }
    out
}

/// The words of a header before its signature's punctuation.
fn header_words(header: &str) -> Vec<String> {
    header
        .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '!'))
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

fn kind_of(header: &str) -> &'static str {
    let words = header_words(header);
    const KINDS: [&str; 12] =
        ["fn", "impl", "trait", "struct", "enum", "union", "mod", "type", "const", "static", "use", "macro_rules!"];
    for w in &words {
        if let Some(k) = KINDS.iter().find(|k| **k == w) {
            // `const fn` and `unsafe impl` read on to the real kind.
            if *k == "const" && words.iter().any(|x| x == "fn") {
                return "fn";
            }
            return k;
        }
    }
    ""
}

fn render(lead: &[String], line: &str, indent: &str) -> String {
    let mut out = String::new();
    for l in lead {
        out.push_str(indent);
        out.push_str(l);
        out.push('\n');
    }
    out.push_str(indent);
    out.push_str(line);
    out.push('\n');
    out
}

/// The members of a trait or impl body, signatures only. `all` keeps every
/// fn (a trait, or an impl of one); otherwise only `pub` ones.
fn members(src: &str, mask: &[bool], body: (usize, usize), all: bool) -> String {
    let mut out = String::new();
    for it in items(src, mask, body.0 + 1, body.1 - 1) {
        if it.test_only {
            continue;
        }
        let public = it.header.starts_with("pub");
        if !(all || public) {
            continue;
        }
        match (kind_of(&it.header), it.body) {
            ("fn", Some(_)) => out.push_str(&render(&it.lead, &format!("{} {{ … }}", it.header), "    ")),
            ("fn" | "type" | "const", None) => out.push_str(&render(&it.lead, &it.text, "    ")),
            ("type" | "const", Some(_)) => out.push_str(&render(&it.lead, &it.text, "    ")),
            _ => {}
        }
    }
    out
}

/// A Rust file's public API: its `pub` items with every function body
/// replaced by `{ … }`, private items and `mod tests` removed. What the test
/// writer is shown of a file that is not a port or the domain.
pub fn public_signatures(source: &str) -> String {
    let mask = code_mask(source);
    let mut out: Vec<String> = Vec::new();
    for it in items(source, &mask, 0, source.len()) {
        if it.test_only {
            continue;
        }
        let public = it.header.starts_with("pub");
        let rendered = match (kind_of(&it.header), it.body) {
            ("fn", Some(_)) if public => render(&it.lead, &format!("{} {{ … }}", it.header), ""),
            ("trait", Some(body)) if public => {
                render(&it.lead, &format!("{} {{\n{}}}", it.header, members(source, &mask, body, true)), "")
            }
            ("impl", Some(body)) => {
                let of_trait = header_words(&it.header).iter().any(|w| w == "for");
                let inner = members(source, &mask, body, of_trait);
                if inner.is_empty() && !of_trait {
                    continue;
                }
                render(&it.lead, &format!("{} {{\n{inner}}}", it.header), "")
            }
            ("mod", None) if public => render(&it.lead, &it.text, ""),
            ("struct" | "enum" | "union" | "type" | "const" | "static" | "use", _) if public => {
                render(&it.lead, &it.text, "")
            }
            _ => continue,
        };
        out.push(rendered);
    }
    out.join("\n")
}

/// Sites in a generated test file that make a test unable to pass, as
/// `(1-based line, the text found)`: every `todo!(`, `unimplemented!(` and
/// `assert!(false`, and every `panic!(` that is the first statement of a
/// function or closure body.
pub fn unpassable_sites(test_source: &str) -> Vec<(usize, String)> {
    let src = test_source;
    let b = src.as_bytes();
    let mask = code_mask(src);
    let line_of = |i: usize| src[..i].matches('\n').count() + 1;
    let word_start = |i: usize| i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_');
    let mut out = Vec::new();
    for (i, _) in src.char_indices() {
        if !mask[i] || !word_start(i) {
            continue;
        }
        let rest = &src[i..];
        for m in ["todo!(", "unimplemented!(", "assert!(false"] {
            if rest.starts_with(m) {
                out.push((line_of(i), m.to_string()));
            }
        }
        if rest.starts_with("panic!(") {
            // The previous code byte must open a body: `{` of a fn or a closure.
            let prev = (0..i).rev().find(|&k| mask[k] && !b[k].is_ascii_whitespace());
            let Some(open) = prev.filter(|&k| b[k] == b'{') else { continue };
            let seg_start = (0..open).rev().find(|&k| mask[k] && matches!(b[k], b'{' | b'}' | b';')).map(|k| k + 1).unwrap_or(0);
            let seg: String = (seg_start..open).filter(|&k| mask[k]).map(|k| b[k] as char).collect();
            let seg = seg.trim();
            let is_fn = header_words(seg).iter().any(|w| w == "fn");
            let is_closure = seg.ends_with('|') || seg.rsplit_once('|').is_some_and(|(_, r)| r.trim_start().starts_with("->"));
            if is_fn || is_closure {
                out.push((line_of(i), "panic!(".to_string()));
            }
        }
    }
    out
}

/// The accepted spec with each unchanged scenario's `test` and `red` carried
/// over from `previous` (ADR-2610281628 §1).
pub fn rebind(previous: &Spec, accepted: &Spec) -> Spec {
    let same = |a: &Scenario, b: &Scenario| {
        a.decision == b.decision
            && a.kind == b.kind
            && a.description == b.description
            && a.given == b.given
            && a.when == b.when
            && a.then == b.then
    };
    let mut out = accepted.clone();
    for s in out.specs.iter_mut() {
        match previous.specs.iter().find(|p| p.id == s.id && same(p, s)) {
            Some(p) if p.test.is_some() => {
                s.test = p.test.clone();
                // Proved at the spec-level red: that red, written onto it.
                s.red = p.red.clone().or_else(|| previous.red.clone());
            }
            _ => {
                s.test = None;
                s.red = None;
            }
        }
    }
    out
}

/// `source` without the `#[test]` functions named, each with its doc
/// comments and attributes (§2).
/// The names of the `#[test]` functions in `source`, read as code: text in
/// a string or a comment is not a test.
pub fn test_fns(source: &str) -> Vec<String> {
    let mask = code_mask(source);
    items(source, &mask, 0, source.len())
        .into_iter()
        .filter(|it| {
            kind_of(&it.header) == "fn"
                && it.lead.iter().any(|a| {
                    let c: String = a.chars().filter(|c| !c.is_whitespace()).collect();
                    c == "#[test]" || (c.starts_with("#[") && c.ends_with("::test]"))
                })
        })
        .filter_map(|it| {
            let words = header_words(&it.header);
            words.iter().position(|w| w == "fn").and_then(|i| words.get(i + 1)).cloned()
        })
        .collect()
}

pub fn without_tests(source: &str, names: &[String]) -> String {
    if names.is_empty() {
        return source.to_string();
    }
    let mask = code_mask(source);
    let mut cut: Vec<(usize, usize)> = Vec::new();
    for it in items(source, &mask, 0, source.len()) {
        let is_test = it.lead.iter().any(|a| {
            let c: String = a.chars().filter(|c| !c.is_whitespace()).collect();
            c == "#[test]" || (c.starts_with("#[") && c.ends_with("::test]"))
        });
        if !is_test || kind_of(&it.header) != "fn" {
            continue;
        }
        let words = header_words(&it.header);
        let name = words.iter().position(|w| w == "fn").and_then(|i| words.get(i + 1));
        if name.is_some_and(|n| names.iter().any(|x| x == n)) {
            // The item and the line break that ends it.
            let end = if source[it.span.1..].starts_with('\n') { it.span.1 + 1 } else { it.span.1 };
            cut.push((it.span.0, end));
        }
    }
    let mut out = String::with_capacity(source.len());
    let mut at = 0;
    for (a, b) in cut {
        out.push_str(&source[at..a]);
        at = b;
    }
    out.push_str(&source[at..]);
    out
}
