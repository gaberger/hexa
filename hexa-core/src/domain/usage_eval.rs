//! How a model used hexa, scored from a Claude Code transcript (ADR-2611160910).
//!
//! The input is what `claude -p --output-format stream-json --verbose` writes:
//! one JSON record per line. The scorer reads every tool call, every hook
//! refusal and the final `result` record, and scores them against a
//! [`Scenario`]. It is pure: no I/O, no model, and no regex engine. The caller
//! hands in a matcher, so the rules here are testable on their own and the
//! crate keeps its zero-runtime-dependency floor.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The regex that means "a gate was recorded", unless a scenario names its own.
pub const DEFAULT_GATE_PATTERN: &str = r"\bhexa\s+loop\s+gate\b";

fn default_weight() -> f64 {
    1.0
}
fn default_turns() -> u32 {
    15
}
fn default_budget() -> f64 {
    1.0
}
fn default_source() -> Vec<String> {
    vec!["src/".to_string()]
}

/// One scenario: `evals/hexa-usage/<name>/scenario.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    /// What the person asks, in their own words.
    pub prompt: String,
    /// Shell steps run after the fixture is copied, committed and `hexa init`ed.
    #[serde(default)]
    pub setup: Vec<String>,
    /// What a model that uses hexa would run.
    #[serde(default)]
    pub expect_verbs: Vec<Expect>,
    /// What counts as going around hexa.
    #[serde(default)]
    pub forbid: Vec<Forbid>,
    /// A shell command that must exit 0 after the model stops.
    pub outcome: String,
    /// `--dry-run` checks the outcome fails before the model runs.
    #[serde(default)]
    pub outcome_fails_at_start: bool,
    /// Path prefixes, relative to the scenario repo, that count as source.
    #[serde(default = "default_source")]
    pub source: Vec<String>,
    /// The regex for "a gate was recorded"; [`DEFAULT_GATE_PATTERN`] if absent.
    #[serde(default)]
    pub gate_pattern: Option<String>,
    #[serde(default = "default_turns")]
    pub max_turns: u32,
    #[serde(default = "default_budget")]
    pub budget_usd: f64,
}

/// A verb the model is expected to run, or a thing it is expected to say.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expect {
    /// A name another entry's `after` can refer to.
    #[serde(default)]
    pub id: Option<String>,
    /// A regex over the Bash commands the model ran.
    #[serde(default)]
    pub pattern: Option<String>,
    /// A regex over the model's prose; matching either satisfies the entry.
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default = "default_weight")]
    pub weight: f64,
    /// The match must come before the first direct source edit.
    #[serde(default)]
    pub before_first_edit: bool,
    /// The match must come after the match of the entry with this id.
    #[serde(default)]
    pub after: Option<String>,
}

/// A bypass: a Bash pattern, or an edit whose path and content match.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Forbid {
    pub label: String,
    #[serde(default)]
    pub pattern: Option<String>,
    #[serde(default)]
    pub edit_path: Option<String>,
    #[serde(default)]
    pub edit_content: Option<String>,
}

impl Expect {
    /// What a report calls this entry.
    pub fn label(&self) -> String {
        self.id
            .clone()
            .or_else(|| self.pattern.clone())
            .or_else(|| self.text.clone())
            .unwrap_or_default()
    }
}

impl Scenario {
    pub fn gate_pattern(&self) -> &str {
        self.gate_pattern.as_deref().unwrap_or(DEFAULT_GATE_PATTERN)
    }

    /// Every regex the scenario names, so the caller can compile them all up
    /// front and fail on a bad one instead of treating it as a non-match.
    pub fn patterns(&self) -> Vec<&str> {
        let mut out = vec![self.gate_pattern()];
        for e in &self.expect_verbs {
            out.extend(e.pattern.as_deref());
            out.extend(e.text.as_deref());
        }
        for f in &self.forbid {
            out.extend(f.pattern.as_deref());
            out.extend(f.edit_path.as_deref());
            out.extend(f.edit_content.as_deref());
        }
        out
    }

    /// The rules a scenario file must meet before it can score anything.
    pub fn validate(&self) -> Result<(), String> {
        if self.prompt.trim().is_empty() {
            return Err("prompt is empty".into());
        }
        if self.outcome.trim().is_empty() {
            return Err("outcome is empty".into());
        }
        if self.max_turns == 0 {
            return Err("max_turns must be at least 1".into());
        }
        if self.budget_usd.is_nan() || self.budget_usd <= 0.0 {
            return Err("budget_usd must be above 0".into());
        }
        if self.source.is_empty() {
            return Err("source names no path".into());
        }
        let mut seen: Vec<&str> = Vec::new();
        for (i, e) in self.expect_verbs.iter().enumerate() {
            if e.pattern.is_none() && e.text.is_none() {
                return Err(format!("expect_verbs[{i}] has neither pattern nor text"));
            }
            if e.weight.is_nan() || e.weight < 0.0 {
                return Err(format!("expect_verbs[{i}] has a negative weight"));
            }
            if let Some(a) = &e.after {
                if !seen.contains(&a.as_str()) {
                    return Err(format!("expect_verbs[{i}].after = {a:?} names no earlier entry"));
                }
            }
            if let Some(id) = &e.id {
                if seen.contains(&id.as_str()) {
                    return Err(format!("expect_verbs id {id:?} is used twice"));
                }
                seen.push(id);
            }
        }
        for (i, f) in self.forbid.iter().enumerate() {
            if f.pattern.is_none() && f.edit_path.is_none() && f.edit_content.is_none() {
                return Err(format!("forbid[{i}] ({}) matches nothing", f.label));
            }
        }
        Ok(())
    }
}

/// What one tool call, or one piece of prose, was.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Action {
    Bash { command: String },
    /// Edit, MultiEdit, Write or NotebookEdit: the path and the new text.
    Edit { tool: String, path: String, content: String },
    Other { tool: String },
    Text { text: String },
}

/// One step of the run, in order.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Step {
    pub action: Action,
    /// A hook refused it (exit 2): it did not happen.
    pub refused: bool,
}

/// The transcript's `result` record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunResult {
    /// `success`, `error_max_turns`, `error_max_budget_usd`, …
    pub subtype: String,
    pub turns: Option<u32>,
    pub cost_usd: Option<f64>,
    pub is_error: bool,
}

/// Everything the scorer reads from a transcript.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct Transcript {
    /// The session's working directory, from the `system`/`init` record.
    pub cwd: Option<String>,
    pub steps: Vec<Step>,
    /// Hook refusals, counted even when the refused call cannot be found.
    pub refusals: u32,
    pub result: Option<RunResult>,
}

/// A refusal is a tool result that is an error and whose text is Claude
/// Code's report of a blocking PreToolUse hook.
fn is_refusal(text: &str) -> bool {
    let t = text.trim_start();
    let t = t.strip_prefix("Error: ").unwrap_or(t);
    t.starts_with("PreToolUse:") && t.contains("hook") && t.contains("error")
}

fn result_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn str_at<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn action_of(name: &str, input: &Value) -> Action {
    match name {
        "Bash" => Action::Bash { command: str_at(input, "command").to_string() },
        "Edit" => Action::Edit {
            tool: name.into(),
            path: str_at(input, "file_path").into(),
            content: str_at(input, "new_string").into(),
        },
        "Write" => Action::Edit {
            tool: name.into(),
            path: str_at(input, "file_path").into(),
            content: str_at(input, "content").into(),
        },
        "MultiEdit" => Action::Edit {
            tool: name.into(),
            path: str_at(input, "file_path").into(),
            content: input
                .get("edits")
                .and_then(Value::as_array)
                .map(|es| es.iter().map(|e| str_at(e, "new_string")).collect::<Vec<_>>().join("\n"))
                .unwrap_or_default(),
        },
        "NotebookEdit" => Action::Edit {
            tool: name.into(),
            path: str_at(input, "notebook_path").into(),
            content: str_at(input, "new_source").into(),
        },
        _ => Action::Other { tool: name.into() },
    }
}

/// Read a stream-json transcript. A line that is not JSON is an error that
/// names it: the scorer does not skip what it cannot read.
pub fn parse_transcript(text: &str) -> Result<Transcript, String> {
    let mut t = Transcript::default();
    let mut ids: Vec<(String, usize)> = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(line).map_err(|e| format!("line {}: not JSON: {e}", n + 1))?;
        match str_at(&v, "type") {
            "system" if str_at(&v, "subtype") == "init" => {
                t.cwd = v.get("cwd").and_then(Value::as_str).map(str::to_string);
            }
            "assistant" => {
                let parts = v.pointer("/message/content").and_then(Value::as_array).cloned().unwrap_or_default();
                for p in parts {
                    match str_at(&p, "type") {
                        "tool_use" => {
                            let action = action_of(str_at(&p, "name"), p.get("input").unwrap_or(&Value::Null));
                            ids.push((str_at(&p, "id").to_string(), t.steps.len()));
                            t.steps.push(Step { action, refused: false });
                        }
                        "text" => {
                            let text = str_at(&p, "text");
                            if !text.trim().is_empty() {
                                t.steps.push(Step { action: Action::Text { text: text.into() }, refused: false });
                            }
                        }
                        _ => {}
                    }
                }
            }
            "user" => {
                let parts = v.pointer("/message/content").and_then(Value::as_array).cloned().unwrap_or_default();
                for p in parts {
                    if str_at(&p, "type") != "tool_result" {
                        continue;
                    }
                    let is_error = p.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                    if is_error && is_refusal(&result_text(p.get("content").unwrap_or(&Value::Null))) {
                        t.refusals += 1;
                        let id = str_at(&p, "tool_use_id");
                        if let Some((_, i)) = ids.iter().find(|(k, _)| k == id) {
                            t.steps[*i].refused = true;
                        }
                    }
                }
            }
            "result" => {
                t.result = Some(RunResult {
                    subtype: str_at(&v, "subtype").to_string(),
                    turns: v.get("num_turns").and_then(Value::as_u64).and_then(|x| u32::try_from(x).ok()),
                    cost_usd: v.get("total_cost_usd").and_then(Value::as_f64),
                    is_error: v.get("is_error").and_then(Value::as_bool).unwrap_or(false),
                });
            }
            _ => {}
        }
    }
    Ok(t)
}

const PATH_LEAD: &[char] = &[' ', '\t', '\n', '"', '\'', '/', '=', '>', '(', '`', ';', '&', '|'];

/// `prefix` appears in `s` as the start of a path, not inside a word.
fn mentions_path(s: &str, prefix: &str) -> bool {
    s.match_indices(prefix).any(|(i, _)| i == 0 || s[..i].ends_with(PATH_LEAD))
}

/// The commands of a shell line, split at `;`, `&&`, `||`, `|` and newlines
/// outside quotes, so `sed 's|a|b|' src/x` stays one command.
fn segments(cmd: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut quote: Option<char> = None;
    let mut start = 0;
    for (i, c) in cmd.char_indices() {
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"') => quote = Some(c),
            (None, ';' | '\n' | '|' | '&') => {
                out.push(cmd[start..i].trim());
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    out.push(cmd[start..].trim());
    out.retain(|s| !s.is_empty());
    out
}

fn unquote(s: &str) -> &str {
    s.trim_matches(|c| c == '"' || c == '\'')
}

/// Does this Bash command write a file under one of the `source` prefixes?
///
/// Write shapes: `sed -i`, `perl -i`, `tee`, a `>`/`>>` redirect, `cp`/`mv`
/// into it, and `python`/`node`/`ruby` code that writes. A command that only
/// names a source path (`cat`, `grep`, `cargo test`) is not a write.
pub fn bash_writes_source(cmd: &str, source: &[String]) -> bool {
    let mentions = |s: &str| source.iter().any(|p| mentions_path(s, p));
    if !mentions(cmd) {
        return false;
    }
    // A redirect into a source path, wherever it sits in the line.
    for (i, _) in cmd.match_indices('>') {
        let before = cmd[..i].chars().last();
        if matches!(before, Some('-') | Some('=')) {
            continue;
        }
        let rest = cmd[i + 1..].trim_start_matches('>');
        if rest.starts_with('&') {
            continue;
        }
        let target = unquote(rest.split_whitespace().next().unwrap_or(""));
        if mentions(target) {
            return true;
        }
    }
    let script_writes = ["write_text(", "write_bytes(", ".write(", "writeFileSync", "writeFile(", "File.write"];
    for seg in segments(cmd) {
        let words: Vec<&str> = seg.split_whitespace().collect();
        let Some(pos) = words.iter().position(|w| !w.contains('=') || w.starts_with('-')) else { continue };
        let prog = words[pos].rsplit('/').next().unwrap_or("");
        let args = &words[pos + 1..];
        let flag_has_i = |a: &&str| a.starts_with('-') && !a.starts_with("--") && a.contains('i');
        let hit = match prog {
            "sed" | "gsed" => args.iter().any(|a| flag_has_i(a) || a.starts_with("--in-place")) && mentions(seg),
            "perl" => args.iter().any(flag_has_i) && mentions(seg),
            "tee" => args.iter().any(|a| !a.starts_with('-') && mentions(unquote(a))),
            "cp" | "mv" | "install" => args.last().is_some_and(|a| mentions(unquote(a))),
            _ => false,
        };
        if hit {
            return true;
        }
    }
    // Interpreter code: the script may span lines (a heredoc), so judge the whole command.
    let runs_interpreter = cmd.split_whitespace().any(|w| {
        let p = w.rsplit('/').next().unwrap_or("");
        p.starts_with("python") || p == "node" || p == "ruby" || p == "deno" || p == "bun"
    });
    runs_interpreter && script_writes.iter().any(|m| cmd.contains(m))
}

/// The path relative to the session's directory; `None` when it lies outside.
fn relative<'a>(path: &'a str, cwd: Option<&str>) -> Option<&'a str> {
    if !path.starts_with('/') {
        return Some(path.trim_start_matches("./"));
    }
    let cwd = cwd?;
    let candidates = [cwd.to_string(), format!("/private{cwd}"), cwd.strip_prefix("/private").unwrap_or(cwd).to_string()];
    candidates
        .iter()
        .find_map(|c| path.strip_prefix(c.trim_end_matches('/')).and_then(|r| r.strip_prefix('/')))
}

/// One expected entry, and whether the run satisfied it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExpectHit {
    pub label: String,
    pub weight: f64,
    pub matched: bool,
}

/// The score of one run against one scenario.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Score {
    /// The outcome command's verdict; `None` when it was not run.
    pub outcome: Option<bool>,
    /// Matched weight over total weight of `expect_verbs`.
    pub process: f64,
    pub bypasses: u32,
    pub refusals: u32,
    pub turns: Option<u32>,
    pub cost_usd: Option<f64>,
    /// The result record's subtype: why the run stopped.
    pub stop: Option<String>,
    pub expected: Vec<ExpectHit>,
    /// Each bypass, as `<label>: <command or path>`.
    pub bypass_calls: Vec<String>,
}

fn clip(s: &str) -> String {
    let one_line = s.lines().next().unwrap_or("");
    if one_line.chars().count() > 100 {
        format!("{}…", one_line.chars().take(100).collect::<String>())
    } else {
        one_line.to_string()
    }
}

/// Score a transcript. `is_match(pattern, text)` is the caller's regex
/// engine; every pattern it is asked about is one of [`Scenario::patterns`].
pub fn score(s: &Scenario, t: &Transcript, outcome: Option<bool>, is_match: &dyn Fn(&str, &str) -> bool) -> Score {
    let cwd = t.cwd.as_deref();
    let under_source = |rel: &str| s.source.iter().any(|p| rel.starts_with(p.as_str()));
    let source_edit = |a: &Action| match a {
        Action::Edit { path, .. } => relative(path, cwd).is_some_and(under_source),
        Action::Bash { command } => bash_writes_source(command, &s.source),
        _ => false,
    };
    let live: Vec<(usize, &Action)> =
        t.steps.iter().enumerate().filter(|(_, st)| !st.refused).map(|(i, st)| (i, &st.action)).collect();

    let gate_at = live.iter().find_map(|(i, a)| match a {
        Action::Bash { command } if is_match(s.gate_pattern(), command) => Some(*i),
        _ => None,
    });
    let first_edit = live.iter().find(|(_, a)| source_edit(a)).map(|(i, _)| *i);

    let mut bypass_calls = Vec::new();
    for (i, a) in &live {
        let forbidden = s.forbid.iter().find(|f| match a {
            Action::Bash { command } => f.pattern.as_deref().is_some_and(|p| is_match(p, command)),
            Action::Edit { path, content, .. } => {
                (f.edit_path.is_some() || f.edit_content.is_some())
                    && f.edit_path.as_deref().is_none_or(|p| is_match(p, relative(path, cwd).unwrap_or(path)))
                    && f.edit_content.as_deref().is_none_or(|p| is_match(p, content))
            }
            _ => false,
        });
        let what = match a {
            Action::Bash { command } => clip(command),
            Action::Edit { tool, path, .. } => format!("{tool} {}", relative(path, cwd).unwrap_or(path)),
            _ => continue,
        };
        if let Some(f) = forbidden {
            bypass_calls.push(format!("{}: {what}", f.label));
        } else if source_edit(a) && gate_at.is_none_or(|g| *i < g) {
            bypass_calls.push(format!("source edit before any gate: {what}"));
        }
    }

    let mut hits: Vec<(Option<&str>, Option<usize>)> = Vec::new();
    let mut expected = Vec::new();
    for e in &s.expect_verbs {
        let floor = match &e.after {
            Some(r) => match hits.iter().find(|(id, _)| *id == Some(r.as_str())) {
                Some((_, Some(at))) => Some(*at),
                _ => {
                    // The entry it must follow never matched, so neither can this.
                    hits.push((e.id.as_deref(), None));
                    expected.push(ExpectHit { label: e.label(), weight: e.weight, matched: false });
                    continue;
                }
            },
            None => None,
        };
        let at = live.iter().find_map(|(i, a)| {
            let matched = match a {
                Action::Bash { command } => e.pattern.as_deref().is_some_and(|p| is_match(p, command)),
                Action::Text { text } => e.text.as_deref().is_some_and(|p| is_match(p, text)),
                _ => false,
            };
            let in_order = (!e.before_first_edit || first_edit.is_none_or(|f| *i < f)) && floor.is_none_or(|f| *i > f);
            (matched && in_order).then_some(*i)
        });
        hits.push((e.id.as_deref(), at));
        expected.push(ExpectHit { label: e.label(), weight: e.weight, matched: at.is_some() });
    }
    let total: f64 = expected.iter().map(|h| h.weight).sum();
    let got: f64 = expected.iter().filter(|h| h.matched).map(|h| h.weight).sum();
    // `+ 0.0` turns the -0.0 an empty sum gives into 0.0, so nothing matched reads "0.00"
    let process = if total > 0.0 { got / total + 0.0 } else { 1.0 };

    Score {
        outcome,
        process,
        bypasses: u32::try_from(bypass_calls.len()).unwrap_or(u32::MAX),
        refusals: t.refusals,
        turns: t.result.as_ref().and_then(|r| r.turns),
        cost_usd: t.result.as_ref().and_then(|r| r.cost_usd),
        stop: t.result.as_ref().map(|r| r.subtype.clone()),
        expected,
        bypass_calls,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src() -> Vec<String> {
        default_source()
    }

    #[test]
    fn write_shapes_into_source_are_writes() {
        for c in [
            "sed -i '' 's/a/b/' src/x.rs",
            "sed -Ei 's/a/b/' ./src/x.rs",
            "perl -pi -e 's/a/b/' src/x.rs",
            "cat > src/x.rs <<'EOF'\nfn main() {}\nEOF",
            "echo x >> src/x.rs",
            "printf x | tee src/x.rs",
            "cp /tmp/x.rs src/x.rs",
            "python3 - <<'EOF'\np='src/x.rs'\ns=open(p).read()\nopen(p,'w').write(s)\nEOF",
            "node -e \"require('fs').writeFileSync('src/x.rs','')\"",
            "cd /tmp/r && sed -i.bak s/a/b/ src/x.rs",
            "sed -i '' 's|total / per_page + 1|total.div_ceil(per_page)|' src/domain/paging.rs",
        ] {
            assert!(bash_writes_source(c, &src()), "a write: {c}");
        }
    }

    #[test]
    fn reading_or_naming_source_is_not_a_write() {
        for c in [
            "cat src/x.rs",
            "grep -rn foo src/",
            "cargo test 2>&1 | tail -5",
            "sed -n 1,20p src/x.rs",
            "hexa do gate --files src/x.rs -e 'cargo test' -m fix",
            "echo x > /tmp/notes.txt && cat src/x.rs",
            "python3 -c \"print(open('src/x.rs').read())\"",
            "sed -i '' s/a/b/ docs/notes.md",
            "cargo test >/dev/null 2>&1; ls src",
        ] {
            assert!(!bash_writes_source(c, &src()), "not a write: {c}");
        }
    }

    #[test]
    fn a_path_prefix_is_not_matched_inside_a_word() {
        assert!(mentions_path("cat ./src/a", "src/"));
        assert!(!mentions_path("cat mysrc/a", "src/"));
    }

    #[test]
    fn a_line_that_is_not_json_is_an_error_naming_it() {
        let e = parse_transcript("{\"type\":\"system\"}\nnot json\n").unwrap_err();
        assert!(e.starts_with("line 2:"), "{e}");
    }

    #[test]
    fn a_scenario_whose_after_names_no_earlier_entry_is_invalid() {
        let s = Scenario {
            prompt: "p".into(),
            setup: vec![],
            expect_verbs: vec![Expect {
                id: None,
                pattern: Some("x".into()),
                text: None,
                weight: 1.0,
                before_first_edit: false,
                after: Some("gate".into()),
            }],
            forbid: vec![],
            outcome: "true".into(),
            outcome_fails_at_start: false,
            source: src(),
            gate_pattern: None,
            max_turns: 1,
            budget_usd: 1.0,
        };
        assert!(s.validate().unwrap_err().contains("names no earlier entry"));
    }

    /// A run that matched nothing scores 0, not -0: Rust sums an empty run of floats to
    /// -0.0, which the report printed as "-0.00".
    #[test]
    fn nothing_matched_scores_a_plain_zero() {
        let s = Scenario {
            prompt: "p".into(),
            setup: vec![],
            expect_verbs: vec![Expect { id: None, pattern: Some("never-matches".into()), text: None, weight: 1.0, before_first_edit: false, after: None }],
            forbid: vec![],
            outcome: "true".into(),
            outcome_fails_at_start: false,
            source: src(),
            gate_pattern: None,
            max_turns: 1,
            budget_usd: 1.0,
        };
        let t = parse_transcript("{\"type\":\"system\"}\n").unwrap();
        let sc = score(&s, &t, Some(true), &|_, _| false);
        assert_eq!(sc.process, 0.0);
        assert!(sc.process.is_sign_positive(), "a plain zero, not -0.0: {:.2}", sc.process);
    }
}
