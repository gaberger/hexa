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
        let specs = self.specs.iter().map(|s| Scenario { test: None, ..s.clone() }).collect();
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
