//! Guides at hexa's known dead ends, and the gaps they record (ADR-2610301500).
//! Pure: the caller prints, appends to `~/.hexa/gaps.jsonl` and runs the check.

/// One known dead end: what is inconsistent, the route around it, and where in
/// hexa the fix belongs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guide {
    pub id: String,
    /// The inconsistency, in one or two sentences.
    pub what: String,
    /// The route around it, in order. A step that begins `hexa ` is a command;
    /// its arguments are written `<placeholder>`, and ` — ` starts its note.
    pub around: Vec<String>,
    /// Where the fix belongs: `<path> — <what to change>`.
    pub fix: Vec<String>,
}

impl Guide {
    /// A guide with nothing to do, or nowhere to fix, is a message, not a guide.
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty() || !self.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
            return Err(format!("id `{}` must be kebab-case", self.id));
        }
        if self.what.trim().is_empty() {
            return Err(format!("{}: `what` is empty: say what is inconsistent", self.id));
        }
        if self.around.iter().all(|s| s.trim().is_empty()) {
            return Err(format!("{}: no `around` step: a guide says how to get past the dead end", self.id));
        }
        if self.fix.iter().all(|s| fix_path(s).is_empty()) {
            return Err(format!("{}: no `fix` path: a guide names where in hexa the fix belongs", self.id));
        }
        Ok(())
    }

    /// The guide as printed at the dead end.
    pub fn render(&self) -> String {
        let mut out = format!("guide {}: {}\n", self.id, self.what);
        out.push_str("  around it:\n");
        for (i, s) in self.around.iter().enumerate() {
            out.push_str(&format!("    {}. {}\n", i + 1, s));
        }
        out.push_str("  the fix belongs in:\n");
        for f in &self.fix {
            out.push_str(&format!("    {f}\n"));
        }
        out.push_str(&format!("  This is a gap in hexa, not a mistake of yours. `hexa gap pr {}` writes the pull request.\n", self.id));
        out
    }
}

/// The path of a `fix` entry: the text before ` — `.
pub fn fix_path(entry: &str) -> &str {
    entry.split(" \u{2014} ").next().unwrap_or("").trim()
}

/// For a step that begins `hexa `, the command words before its first
/// argument: `hexa spec accept <ADR>` gives `["spec", "accept"]`.
pub fn command_words(step: &str) -> Option<Vec<String>> {
    let command = step.split(" \u{2014} ").next()?.trim();
    let rest = command.strip_prefix("hexa ")?;
    let words: Vec<String> = rest
        .split_whitespace()
        .take_while(|w| !w.is_empty() && w.chars().all(|c| c.is_ascii_lowercase() || c == '-') && !w.starts_with('-'))
        .map(str::to_string)
        .collect();
    (!words.is_empty()).then_some(words)
}

/// Check every `hexa ` step of `guides` with `runs` (does `hexa <words> --help`
/// succeed?). Returns how many steps were checked and one line per failure.
pub fn check_commands(guides: &[Guide], runs: &dyn Fn(&[String]) -> bool) -> (usize, Vec<String>) {
    let mut checked = 0;
    let mut errors = Vec::new();
    for g in guides {
        for step in &g.around {
            if let Some(words) = command_words(step) {
                checked += 1;
                if !runs(&words) {
                    errors.push(format!("{}: `hexa {}` is not a hexa command (step: {step})", g.id, words.join(" ")));
                }
            }
        }
    }
    (checked, errors)
}

/// One printed guide, as recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub id: String,
    pub project: String,
    pub at: String,
}

/// One line of `hexa gap list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    pub count: usize,
    pub last_project: String,
    /// Every project it was hit in, first hit first.
    pub projects: Vec<String>,
    /// Recorded by `hexa gap add`, not in `guides()`.
    pub unlisted: bool,
}

/// Hits grouped by id, the most hit first, ties by id. `unlisted` names the
/// ids recorded by `hexa gap add`.
pub fn rank(hits: &[Hit], unlisted: &[String]) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::new();
    for h in hits {
        match rows.iter_mut().find(|r| r.id == h.id) {
            Some(r) => {
                r.count += 1;
                r.last_project = h.project.clone();
                if !r.projects.contains(&h.project) {
                    r.projects.push(h.project.clone());
                }
            }
            None => rows.push(Row {
                id: h.id.clone(),
                count: 1,
                last_project: h.project.clone(),
                projects: vec![h.project.clone()],
                unlisted: unlisted.contains(&h.id),
            }),
        }
    }
    rows.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.id.cmp(&b.id)));
    rows
}

/// The pull request for a gap: a title line, then the body.
pub fn pr_text(g: &Guide, row: Option<&Row>, unlisted: bool) -> String {
    let mut out = format!("Close the {} gap: {}\n\n", g.id, first_sentence(&g.what));
    out.push_str("## Driver\n\n");
    out.push_str(&format!("{}\n\n", g.what));
    match row {
        Some(r) => out.push_str(&format!("Hit {} time(s), in: {}.\n\n", r.count, r.projects.join(", "))),
        None => out.push_str("Not hit on this machine yet.\n\n"),
    }
    out.push_str("## What users do today\n\n");
    for (i, s) in g.around.iter().enumerate() {
        out.push_str(&format!("{}. {}\n", i + 1, s));
    }
    out.push_str("\n## The fix\n\n");
    for f in &g.fix {
        out.push_str(&format!("- {f}\n"));
    }
    out.push_str("\nThe fix retires this guide: remove it from `guides()` in `hexa-core/src/domain/guide.rs`.\n");
    if unlisted {
        out.push_str("\n## Until then, list it\n\nThis gap was recorded with `hexa gap add`. Add it to `guides()` in `hexa-core/src/domain/guide.rs`, and print it where the dead end is reached:\n\n```rust\n");
        out.push_str(&rust_entry(g));
        out.push_str("```\n");
    }
    out
}

/// A title's worth of `what`: up to its first colon or full stop.
fn first_sentence(s: &str) -> &str {
    let end = [s.find(": "), s.find(". ")].into_iter().flatten().min().unwrap_or(s.len());
    s[..end].trim_end_matches('.')
}

fn rust_entry(g: &Guide) -> String {
    let list = |v: &[String]| v.iter().map(|s| format!("        {s:?},\n")).collect::<String>();
    format!(
        "guide(\n    {:?},\n    {:?},\n    &[\n{}    ],\n    &[\n{}    ],\n),\n",
        g.id,
        g.what,
        list(&g.around),
        list(&g.fix)
    )
}

fn guide(id: &str, what: &str, around: &[&str], fix: &[&str]) -> Guide {
    Guide {
        id: id.to_string(),
        what: what.to_string(),
        around: around.iter().map(|s| s.to_string()).collect(),
        fix: fix.iter().map(|s| s.to_string()).collect(),
    }
}

/// Every known dead end. A fix that closes one removes it from here.
pub fn guides() -> Vec<Guide> {
    vec![
        guide(
            "no-completion-model",
            "no completion model is reachable: `hexa spec draft` and `hexa spec tests` need one, no local server answers, no API key or inference URL is set, and the frontier CLI is refused because it can read the repository (ADR-2610221540 §2).",
            &[
                "hexa config inference add <template> <url> \u{2014} if you have a completion model, register it and run the command again",
                "Otherwise write docs/specs/<ADR>.json by hand from the ADR and the port and domain files it names, never the implementation: the author must not see the code",
                "hexa spec check <ADR> \u{2014} every numbered decision has an example and a negative",
                "hexa spec accept <ADR> \u{2014} a person reads every scenario and accepts it",
                "For the tests, follow the link-hand-written-tests guide (`hexa gap pr link-hand-written-tests` prints it)",
            ],
            &[
                "hexa-cli/src/commands/spec_ledger.rs \u{2014} `hexa spec draft --by-hand <ADR>`: write the skeleton from the ADR's decisions for a person to fill",
                "hexa-exec/src/spec_author.rs \u{2014} the drafter and the test writer",
            ],
        ),
        guide(
            "link-hand-written-tests",
            "tests written by hand have no verb that links them to their spec: `hexa spec tests` generates them with a model, and `hexa spec accept` after a change drops the links and points at it.",
            &[
                "Write <crate>/tests/adr_<id>_spec.rs from the accepted scenarios only, one #[test] per scenario",
                "Set each scenario's `test` in docs/specs/<ADR>.json to <file>::<fn>; acceptance does not cover it, so it still holds",
                "Run the tests and see every one fail; one that passes now is a mirror (the guard-test-is-a-mirror guide)",
                "git commit the ADR, the spec and the failing tests: that commit is the red commit",
                "Then, and only then, set the spec's `red` to it: git rev-parse --short HEAD",
                "hexa spec seal <ADR> \u{2014} freezes the test file; build until `hexa spec verify --prove <ADR>` passes",
            ],
            &["hexa-cli/src/commands/spec_ledger.rs \u{2014} `hexa spec link <ADR> --file <tests>`: match tests to scenarios, require each red, commit, record red, seal"],
        ),
        guide(
            "guard-test-is-a-mirror",
            "a negative scenario that guards behaviour already in place passes before the code exists, and `hexa spec verify --prove` calls it a mirror.",
            &[
                "Have the guard's test also exercise the new behaviour it sits beside, so it fails until that is built and still asserts the guard",
                "hexa spec verify --prove <ADR> \u{2014} every scenario red at its red commit",
            ],
            &["hexa-cli/src/commands/spec_ledger.rs \u{2014} a `guard` scenario kind that `--prove` requires green at the red commit"],
        ),
    ]
}

/// The listed guide with this id.
pub fn find(id: &str) -> Option<Guide> {
    guides().into_iter().find(|g| g.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_words_stop_at_the_first_argument() {
        assert_eq!(command_words("hexa spec accept <ADR> \u{2014} note"), Some(vec!["spec".into(), "accept".into()]));
        assert_eq!(command_words("hexa spec verify --prove <ADR>"), Some(vec!["spec".into(), "verify".into()]));
        assert_eq!(command_words("git commit the tests"), None);
        assert_eq!(command_words("hexa <x>"), None);
    }

    #[test]
    fn rank_counts_and_orders() {
        let h = |id: &str, p: &str| Hit { id: id.into(), project: p.into(), at: String::new() };
        let rows = rank(&[h("b", "x"), h("a", "x"), h("a", "y"), h("a", "x")], &["b".into()]);
        assert_eq!(rows[0].id, "a");
        assert_eq!((rows[0].count, rows[0].last_project.as_str()), (3, "x"));
        assert_eq!(rows[0].projects, vec!["x".to_string(), "y".to_string()]);
        assert!(rows[1].unlisted && !rows[0].unlisted);
    }
}
