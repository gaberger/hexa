//! Playbooks — the procedure a routed intent hands back (ADR-2609140844).
//!
//! `hexa hey` used to be a switchboard: text in, one command out. "Fix this
//! bug" is not one command. It is an ordered procedure — reproduce, trace
//! consumers, write the gate, fix under it, hunt what the fix missed, grade
//! the shape — and hexa already owns a verb for every step. Nothing bound
//! them into an order, so the order lived in `CLAUDE.md` as prose, and prose
//! cannot fail.
//!
//! A playbook is that order, as data. Each step names the verb it runs and
//! the condition that proves the step finished. The steps are copied
//! verbatim, because paraphrase is where drift enters.
//!
//! Decision 4 of the ADR is what keeps a playbook from becoming the spec
//! problem in a new hat: every playbook ends at `hexa analyze .`, and at
//! least one step is a proof step. `playbooks_end_at_the_gates` in
//! `hexa-cli/tests/playbooks_are_executable.rs` refuses any file that is not.

use crate::assets::Assets;

/// One ordered step of a playbook.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Step {
    /// What this step accomplishes, in the imperative.
    pub title: String,
    /// The command the step runs. Placeholders are `<angle-bracketed>`.
    pub run: String,
    /// The condition that proves the step finished.
    pub done_when: String,
}

/// A named, ordered procedure for one shape of task.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Playbook {
    pub name: String,
    pub summary: String,
    /// Whole words that route a request here. Matched against the request's
    /// word set, never as substrings: "is" must not fire on "this".
    pub triggers: Vec<String>,
    pub steps: Vec<Step>,
}

/// The embedded asset prefix. Playbooks ship in the binary beside the skills.
const PREFIX: &str = "playbooks/";

/// Every shipped playbook, in name order.
///
/// A malformed file is an error, not a skipped entry. Silently loading three
/// of four playbooks and reporting no problem is the defect ADR-2609122048
/// names, and this is the tool that enforces that lesson on everyone else.
pub fn load() -> anyhow::Result<Vec<Playbook>> {
    let mut out = Vec::new();
    for path in Assets::iter() {
        let Some(rest) = path.strip_prefix(PREFIX) else { continue };
        if !rest.ends_with(".json") {
            continue;
        }
        let body = Assets::get_str(&path)
            .ok_or_else(|| anyhow::anyhow!("playbook {path} is embedded but unreadable"))?;
        let pb: Playbook = serde_json::from_str(&body)
            .map_err(|e| anyhow::anyhow!("playbook {path} is malformed: {e}"))?;
        out.push(pb);
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// The request's words, lowercased, with punctuation dropped.
///
/// Trigger matching is whole-word so that a two-letter trigger cannot fire on
/// a fragment of a longer word.
pub fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect()
}

/// How many distinct triggers of `pb` appear as whole words in `text`.
pub fn score(text: &str, pb: &Playbook) -> usize {
    let present: std::collections::BTreeSet<String> = words(text).into_iter().collect();
    pb.triggers
        .iter()
        .map(|t| t.to_lowercase())
        .collect::<std::collections::BTreeSet<String>>()
        .iter()
        .filter(|t| present.contains(*t))
        .count()
}

/// The best-matching playbook for a request, with its score.
///
/// Ties break on name, so the same request always routes to the same
/// playbook. A request that triggers nothing returns `None` — that is a
/// result, not an error (decision 5).
pub fn best<'a>(text: &str, books: &'a [Playbook]) -> Option<(&'a Playbook, usize)> {
    books
        .iter()
        .map(|pb| (pb, score(text, pb)))
        .filter(|(_, s)| *s > 0)
        .max_by(|(a, sa), (b, sb)| sa.cmp(sb).then_with(|| b.name.cmp(&a.name)))
}

/// The playbook as the operator sees it.
///
/// Decision 3: the steps are copied, not summarised. The first line carries
/// `playbook: <name>` because that is what the gate greps for.
pub fn render(pb: &Playbook) -> String {
    let mut s = String::new();
    s.push_str(&format!("  → playbook: {}\n", pb.name));
    s.push_str(&format!("    {}\n\n", pb.summary));
    for (i, step) in pb.steps.iter().enumerate() {
        s.push_str(&format!("    {}. {}\n", i + 1, step.title));
        s.push_str(&format!("       run:  {}\n", step.run));
        s.push_str(&format!("       done: {}\n", step.done_when));
    }
    s.push_str("\n    Copy these steps verbatim. Do not reorder or paraphrase them.\n");
    s
}

/// What `hexa hey` prints when nothing matched.
///
/// Decision 5: it names what it considered and exits 0. Decision 6: every
/// command it names here exists.
pub fn no_match(text: &str, books: &[Playbook]) -> String {
    let names: Vec<&str> = books.iter().map(|p| p.name.as_str()).collect();
    format!(
        "  no playbook matched \"{}\"\n    considered: {}\n    Pick one directly with `hexa hey <playbook name> …`, or run a verb yourself — `hexa --help` lists them.\n",
        text,
        names.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shipped_playbook_loads() {
        let books = load().expect("playbooks load");
        assert!(
            books.len() >= 4,
            "found only {} playbooks; the loader or the asset prefix is broken",
            books.len()
        );
        let names: Vec<&str> = books.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"bug-fix"), "no bug-fix playbook: {names:?}");
    }

    #[test]
    fn words_are_whole_and_lowercased() {
        assert_eq!(words("Fix a BUG, where?"), vec!["fix", "a", "bug", "where"]);
    }

    #[test]
    fn a_trigger_never_fires_on_a_fragment() {
        let pb = Playbook {
            name: "t".into(),
            summary: String::new(),
            triggers: vec!["is".into()],
            steps: vec![],
        };
        // "this" contains "is". The word set must not.
        assert_eq!(score("this thing", &pb), 0);
        assert_eq!(score("is it broken", &pb), 1);
    }

    #[test]
    fn the_adr_gate_routes_to_bug_fix() {
        let books = load().expect("playbooks load");
        let (pb, s) = best("fix a bug where the scroll drifts", &books).expect("a match");
        assert_eq!(pb.name, "bug-fix", "scored {s}");
    }

    #[test]
    fn an_investigation_routes_to_investigation() {
        let books = load().expect("playbooks load");
        let (pb, _) = best("how do we cancel runs", &books).expect("a match");
        assert_eq!(pb.name, "investigation");
    }

    #[test]
    fn a_request_with_no_trigger_matches_nothing() {
        let books = load().expect("playbooks load");
        assert!(best("asdf qwer zxcv", &books).is_none());
    }

    #[test]
    fn matching_is_deterministic_across_runs() {
        let books = load().expect("playbooks load");
        let first = best("clean up the dead code", &books).map(|(p, _)| p.name.clone());
        for _ in 0..5 {
            let again = best("clean up the dead code", &books).map(|(p, _)| p.name.clone());
            assert_eq!(first, again, "the same request routed two ways");
        }
    }

    #[test]
    fn render_carries_the_name_the_gate_greps_for() {
        let books = load().expect("playbooks load");
        let pb = books.iter().find(|p| p.name == "bug-fix").expect("bug-fix");
        let out = render(pb);
        assert!(out.contains("playbook: bug-fix"), "render lost the name:\n{out}");
        // Verbatim: every step's own words survive.
        for step in &pb.steps {
            assert!(out.contains(&step.title), "step title was rewritten: {}", step.title);
            assert!(out.contains(&step.run), "step command was rewritten: {}", step.run);
        }
    }
}
