//! `hexa verify <claim>` — adversarial verification of a natural-language claim.
//!
//! Operator types a claim about the repo or system; we attempt to FALSIFY
//! it. Returns one of:
//!   CONFIRMED   — adversarial check ran, came up empty (no counter-evidence)
//!   REFUTED     — counter-evidence found; the claim is false
//!   INCONCLUSIVE — couldn't run a falsifiable check (claim too vague,
//!                  needs context we don't have, or the check itself failed)
//!
//! Exit codes are 0 / 1 / 3 for the three verdicts and 4 for an error; 2 is
//! clap's usage error. `hexa verify --help` lists them.
//!
//! Two paths:
//!   1. Deterministic — common claim shapes (boundary check, file
//!      presence, secret absence, etc.) map to existing checks that can
//!      authoritatively pass/fail.
//!   2. LLM-driven — claims that don't match a deterministic check route
//!      through the adversarial-red persona via /api/inference/complete;
//!      verdict + evidence are parsed from the response.
//!
//! Builds on docs/adrs work earlier today: same adversarial-pattern that
//! verified 7 ADRs by running checks that COULD fail.

use anyhow::Result;
use clap::Parser;
use colored::Colorize;
use regex::Regex;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Parser)]
#[command(after_help = "Exit codes:\n  \
    0  CONFIRMED     a check that could have failed ran and found no counter-evidence\n  \
    1  REFUTED       counter-evidence was found; the claim is false\n  \
    2  usage error   the arguments could not be parsed\n  \
    3  INCONCLUSIVE  no check could settle it: the claim was too vague, the tree has no grade, or a check could not run\n  \
    4  error         verification itself failed (for example, inference was unreachable)")]
pub struct VerifyArgs {
    /// The claim to verify, in plain English.
    /// Examples:
    ///   hexa verify "domain layer has zero boundary violations"
    ///   hexa verify "no SQLite hub.db reference remains in code"
    ///   hexa verify "all Accepted ADRs have implementation files"
    pub claim: Vec<String>,

    /// Emit JSON instead of human-readable output.
    #[arg(long)]
    pub json: bool,

    /// Skip the deterministic-check phase; force LLM-only.
    #[arg(long)]
    pub llm_only: bool,
}

#[derive(Debug)]
enum Verdict {
    Confirmed,
    Refuted,
    Inconclusive,
}

impl Verdict {
    fn as_str(&self) -> &'static str {
        match self {
            Verdict::Confirmed => "CONFIRMED",
            Verdict::Refuted => "REFUTED",
            Verdict::Inconclusive => "INCONCLUSIVE",
        }
    }
    fn colored(&self) -> colored::ColoredString {
        match self {
            Verdict::Confirmed => "CONFIRMED".green().bold(),
            Verdict::Refuted => "REFUTED".red().bold(),
            Verdict::Inconclusive => "INCONCLUSIVE".yellow().bold(),
        }
    }
}

struct CheckResult {
    verdict: Verdict,
    summary: String,
    evidence: Vec<String>,
    method: &'static str, // "deterministic" | "adversarial-llm"
}

/// Exit codes, as `--help` documents them. INCONCLUSIVE is not 2, which clap
/// uses for a usage error, and an error is not 1, which is REFUTED.
const EXIT_CONFIRMED: i32 = 0;
const EXIT_REFUTED: i32 = 1;
const EXIT_INCONCLUSIVE: i32 = 3;
const EXIT_ERROR: i32 = 4;

pub async fn run(args: VerifyArgs) -> Result<()> {
    match verify(args).await {
        Ok(code) => crate::commands::events::exit(code),
        Err(e) => {
            eprintln!("Error: {e:#}");
            crate::commands::events::exit(EXIT_ERROR);
        }
    }
}

async fn verify(args: VerifyArgs) -> Result<i32> {
    let claim = args.claim.join(" ").trim().to_string();
    if claim.is_empty() {
        anyhow::bail!("usage: hexa verify <claim>");
    }

    if !args.json {
        println!("{} {}", "claim:".cyan(), claim);
        println!();
    }

    let result = if args.llm_only {
        verify_via_llm(&claim).await?
    } else {
        match try_deterministic(&claim) {
            Some(r) => r,
            None => verify_via_llm(&claim).await?,
        }
    };

    if args.json {
        let body = serde_json::json!({
            "claim": claim,
            "verdict": result.verdict.as_str(),
            "summary": result.summary,
            "evidence": result.evidence,
            "method": result.method,
        });
        println!("{}", serde_json::to_string_pretty(&body)?);
    } else {
        println!("  verdict:  {}", result.verdict.colored());
        println!("  method:   {}", result.method);
        println!("  summary:  {}", result.summary);
        if !result.evidence.is_empty() {
            println!("  evidence:");
            for ev in &result.evidence {
                println!("    - {}", ev);
            }
        }
    }

    Ok(match result.verdict {
        Verdict::Confirmed => EXIT_CONFIRMED,
        Verdict::Refuted => EXIT_REFUTED,
        Verdict::Inconclusive => EXIT_INCONCLUSIVE,
    })
}

/// A check that matched the claim's shape but could not run. It settles
/// nothing, so it says so rather than reading its empty output as a verdict.
fn could_not_run(what: &str, why: String) -> CheckResult {
    CheckResult {
        verdict: Verdict::Inconclusive,
        summary: format!("{what} could not run, so the claim is neither confirmed nor refuted"),
        evidence: vec![why],
        method: "deterministic",
    }
}

/// `git` with `args` in the current directory: its stdout when it exited 0,
/// or why not. A caller that reads exit 1 as meaningful asks for it.
fn git_ok(args: &[&str], also_ok: &[i32]) -> std::result::Result<(i32, String), String> {
    let out = Command::new("git").args(args).output().map_err(|e| format!("`git {}` did not start: {e}", args.join(" ")))?;
    let code = out.status.code().unwrap_or(-1);
    if code == 0 || also_ok.contains(&code) {
        Ok((code, String::from_utf8_lossy(&out.stdout).into_owned()))
    } else {
        Err(format!(
            "`git {}` exited {code}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

// ── Deterministic checks ────────────────────────────────────────────────────
//
// Each check function:
//   - returns None if the claim doesn't match its pattern (try next check)
//   - returns Some(CheckResult) with verdict + evidence when it matched
//
// Add a new check by adding a function and calling it from try_deterministic.

fn try_deterministic(claim: &str) -> Option<CheckResult> {
    let lower = claim.to_lowercase();

    [
        check_boundary_violations as fn(&str, &str) -> Option<CheckResult>,
        check_secret_in_tree,
        check_no_substring,
        check_files_under,
        check_adr_status,
    ]
    .iter()
    .find_map(|f| f(claim, &lower))
}

/// Matches: "domain ... boundary violations" / "hexagonal layering"
fn check_boundary_violations(_claim: &str, lower: &str) -> Option<CheckResult> {
    if !(lower.contains("boundary") || lower.contains("hexagonal") || lower.contains("layering")) {
        return None;
    }
    if !(lower.contains("violation") || lower.contains("zero") || lower.contains("no ")) {
        return None;
    }

    // This binary, never whatever `hexa` is first on PATH: an older install
    // there grades by older rules, and its verdict would be reported as ours.
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => return Some(could_not_run("hexa analyze", format!("this binary's path is unknown: {e}"))),
    };
    let out = match Command::new(&exe).args(["analyze", ".", "--json"]).output() {
        Ok(o) => o,
        Err(e) => return Some(could_not_run("hexa analyze", format!("`{} analyze` did not start: {e}", exe.display()))),
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let json: Option<serde_json::Value> = text.find('{').and_then(|i| serde_json::from_str(&text[i..]).ok());
    let Some(json) = json else {
        return Some(could_not_run(
            "hexa analyze",
            format!(
                "`hexa analyze . --json` exited {} with no report: {}",
                out.status.code().unwrap_or(-1),
                String::from_utf8_lossy(&out.stderr).lines().next().unwrap_or("").trim()
            ),
        ));
    };
    // A tree with no grade has no boundary result: nothing was read.
    let graded = json.get("graded").and_then(|g| g.as_bool()) == Some(true);
    let violations = json.pointer("/score_components/violations").and_then(|v| v.as_u64());
    let (true, Some(violations)) = (graded, violations) else {
        return Some(CheckResult {
            verdict: Verdict::Inconclusive,
            summary: "hexa analyze . produced no grade for this tree, so it says nothing about its boundaries".into(),
            evidence: vec![json
                .get("not_graded_reason")
                .and_then(|r| r.as_str())
                .unwrap_or("no boundary analysis ran")
                .to_string()],
            method: "deterministic",
        });
    };
    let score = json.get("score").and_then(|s| s.as_u64()).map(|s| s.to_string()).unwrap_or_else(|| "?".into());

    Some(CheckResult {
        verdict: if violations == 0 { Verdict::Confirmed } else { Verdict::Refuted },
        summary: format!("hexa analyze . reports {violations} boundary violation(s) (score {score}/100)"),
        evidence: vec![
            "`hexa analyze . --json` — full hexagonal layering check across the workspace".into(),
            format!("score {score}/100"),
        ],
        method: "deterministic",
    })
}

/// Matches: "no secrets" / "no .env" / "no api key in"
fn check_secret_in_tree(_claim: &str, lower: &str) -> Option<CheckResult> {
    let mentions_secret = lower.contains("secret") || lower.contains(".env")
        || lower.contains("api key") || lower.contains("apikey");
    let negation = lower.contains("no ") || lower.contains("zero")
        || lower.contains("absent") || lower.contains("without");
    if !(mentions_secret && negation) {
        return None;
    }

    let listed = match git_ok(&["ls-files"], &[]) {
        Ok((_, s)) => s,
        Err(why) => return Some(could_not_run("git ls-files", why)),
    };
    let files: Vec<&str> = listed.lines().collect();
    let mut hits = Vec::new();
    for f in &files {
        if f.ends_with(".env") || f == &".env" || f.ends_with("/credentials.json") {
            hits.push(format!("tracked path: {}", f));
        }
    }
    Some(CheckResult {
        verdict: if hits.is_empty() { Verdict::Confirmed } else { Verdict::Refuted },
        summary: format!(
            "tracked tree scan: {} suspicious path(s) ({} total files)",
            hits.len(),
            files.len()
        ),
        evidence: if hits.is_empty() {
            vec!["no `.env`, `credentials.json` tracked".into()]
        } else { hits },
        method: "deterministic",
    })
}

/// Matches "no X in code" / "no X remains" — grep for literal token X.
fn check_no_substring(claim: &str, _lower: &str) -> Option<CheckResult> {
    // Matched case-insensitively, captured as written: `git grep` is
    // case-sensitive, and the lowercased token found nothing for
    // "CounterStore", which confirmed a false claim.
    let re = Regex::new(r#"(?i)\bno [`"'']?([\w.\-/]{3,})[`"'']? (in|remains|present|exists)"#).ok()?;
    let cap = re.captures(claim)?;
    let token = cap.get(1)?.as_str().to_string();
    // Skip if the deterministic-secret check already covers it
    if matches!(token.to_lowercase().as_str(), "secret" | "secrets" | ".env" | "api" | "key") {
        return None;
    }

    // `git grep` exits 0 with matches, 1 with none, and anything else when it
    // could not search. Only 1 is "no match".
    let (code, body) = match git_ok(&["grep", "-l", "-I", "-F", "--", &token], &[1]) {
        Ok(r) => r,
        Err(why) => return Some(could_not_run(&format!("git grep `{token}`"), why)),
    };
    let lines: Vec<&str> = body.lines().take(10).collect();
    let n = body.lines().count();
    if (code == 0) != (n > 0) {
        return Some(could_not_run(
            &format!("git grep `{token}`"),
            format!("`git grep` exited {code} but listed {n} file(s)"),
        ));
    }

    Some(CheckResult {
        verdict: if n == 0 { Verdict::Confirmed } else { Verdict::Refuted },
        summary: format!(
            "git grep `{}`: {} file(s) match (claim was: {})",
            token, n, claim
        ),
        evidence: if n == 0 {
            vec![format!("`git grep -l -F {}` exited 1: no tracked file matches", token)]
        } else {
            lines.iter().map(|l| format!("hit: {}", l)).collect()
        },
        method: "deterministic",
    })
}

/// Matches: "files under <dir>" / "X files in <dir>"
fn check_files_under(_claim: &str, lower: &str) -> Option<CheckResult> {
    let re = Regex::new(r"(?:files? (?:under|in) |present in )([\w./\-]+)").ok()?;
    let cap = re.captures(lower)?;
    let dir = cap.get(1)?.as_str();
    if !Path::new(dir).exists() {
        return None;
    }
    // Use git ls-files for an authoritative tracked-file count under dir.
    // Outside a repository there is no count, and no verdict.
    let listed = match git_ok(&["ls-files", "--", dir], &[]) {
        Ok((_, s)) => s,
        Err(why) => return Some(could_not_run("git ls-files", why)),
    };
    let count = listed.lines().filter(|l| !l.is_empty()).count();
    Some(CheckResult {
        verdict: if count > 0 { Verdict::Confirmed } else { Verdict::Refuted },
        summary: format!("{} contains {} file(s)", dir, count),
        evidence: vec![format!("walked `{}` — {} regular files", dir, count)],
        method: "deterministic",
    })
}

/// Matches: "all <status> ADRs have <something>" / "<N> Proposed ADRs"
fn check_adr_status(_claim: &str, lower: &str) -> Option<CheckResult> {
    if !lower.contains("adr") { return None; }
    let entries = std::fs::read_dir("docs/adrs").ok()?;
    let mut totals = std::collections::HashMap::<String, u32>::new();
    // Handle both `**Status:** Accepted` and `Status: Accepted` (with bold
    // variants). The trailing `\*\*` after the colon was missing from the
    // first cut → 0 ADRs matched. Use (?m) so ^ matches each line. Built once:
    // compiling it per file is the same pattern every time.
    let re = Regex::new(r"(?im)^\s*\*{0,2}Status\*{0,2}:\s*\*{0,2}\s*(\w+)").ok()?;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) != Some("md") { continue; }
        if let Ok(text) = std::fs::read_to_string(&p) {
            if let Some(c) = re.captures(&text) {
                let s = c.get(1)?.as_str().to_lowercase();
                let bucket = match s.as_str() {
                    "accepted" => "accepted",
                    "proposed" => "proposed",
                    "rejected" => "rejected",
                    "superseded" => "superseded",
                    "deprecated" => "deprecated",
                    _ => "other",
                };
                *totals.entry(bucket.to_string()).or_insert(0) += 1;
            }
        }
    }
    let summary = {
        let mut parts: Vec<(String, u32)> = totals.iter().map(|(k,v)| (k.clone(), *v)).collect();
        parts.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        parts.into_iter().map(|(k,v)| format!("{} {}", v, k)).collect::<Vec<_>>().join(", ")
    };
    Some(CheckResult {
        verdict: Verdict::Inconclusive,
        summary: format!("ADR census: {}", summary),
        evidence: vec!["docs/adrs/ scanned for Status: field".into()],
        method: "deterministic",
    })
}

// ── LLM-driven adversarial verification ─────────────────────────────────────

/// Words in the claim worth grepping for: backticked spans, quoted strings,
/// anything with a path separator or an underscore, and CamelCase names.
/// Ordinary prose is dropped, because grepping for "the" proves nothing.
fn claim_tokens(claim: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |t: &str| {
        let t = t.trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '/' && c != '.');
        if t.len() >= 3 && !out.iter().any(|e| e == t) {
            out.push(t.to_string());
        }
    };
    // Backticked and quoted spans first: the author marked them as literal.
    for quote in ['`', '"', '\''] {
        let parts: Vec<&str> = claim.split(quote).collect();
        for (i, part) in parts.iter().enumerate() {
            if i % 2 == 1 {
                push(part);
            }
        }
    }
    for w in claim.split_whitespace() {
        let bare = w.trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '/' && c != '.');
        let interesting = bare.contains('/')
            || bare.contains('_')
            || (bare.contains('.') && !bare.ends_with('.'))
            || bare.chars().any(|c| c.is_ascii_uppercase());
        if interesting {
            push(bare);
        }
    }
    out.truncate(6);
    out
}

/// What the repository actually says about those words.
///
/// Each line is `path:line:text` from `git grep`, capped. This is the whole
/// point: the verifier used to send the claim alone to a model with no files
/// and no tools, and the model either refused or guessed. One answer cited
/// `cargo graph`, a command that does not exist and was never run
/// (ADR-2609122048).
fn gather_evidence(claim: &str) -> (Vec<String>, Vec<String>) {
    let mut lines = Vec::new();
    let mut searched = Vec::new();
    for token in claim_tokens(claim) {
        // Exit 1 is "no match"; any other failure searched nothing, and an
        // empty result from it is not evidence of absence.
        let text = match git_ok(&["grep", "-n", "-I", "-F", "--", &token], &[1]) {
            Ok((_, t)) => t,
            Err(_) => continue,
        };
        searched.push(token.clone());
        let hits: Vec<&str> = text.lines().take(8).collect();
        if hits.is_empty() {
            lines.push(format!("git grep -F {token} — no match in tracked files"));
        } else {
            let total = text.lines().count();
            lines.push(format!("git grep -F {token} — {total} match(es):"));
            for h in hits {
                lines.push(format!("  {}", h.chars().take(200).collect::<String>()));
            }
        }
        if lines.len() > 40 {
            break;
        }
    }
    (lines, searched)
}

async fn verify_via_llm(claim: &str) -> Result<CheckResult> {
    let system = "You are an adversarial verifier evaluating a claim about a software repository. \
                  Output EXACTLY THREE LINES in this format and nothing else:\n\n\
                  VERDICT: <CONFIRMED or REFUTED or INCONCLUSIVE>\n\
                  SUMMARY: <one-line reason>\n\
                  EVIDENCE: <file path / ADR id / command name / or the word unknown>\n\n\
                  Use CONFIRMED only if you have actively considered how the claim could be false \
                  and found no counter-example in the evidence. Use REFUTED if the evidence names a \
                  specific counter-example. Use INCONCLUSIVE whenever the evidence does not settle it. \
                  EVIDENCE must quote a line you were given. Never name a command that is not in the \
                  evidence; you cannot run anything.\n\n\
                  Examples:\n\n\
                  VERDICT: CONFIRMED\n\
                  SUMMARY: zero hub.db references found in tracked source files\n\
                  EVIDENCE: git grep -l 'hub.db' returned no matches\n\n\
                  VERDICT: REFUTED\n\
                  SUMMARY: hexa-cli/src/commands/legacy.rs still uses SQLite\n\
                  EVIDENCE: hexa-cli/src/commands/legacy.rs:42\n\n\
                  VERDICT: INCONCLUSIVE\n\
                  SUMMARY: claim depends on runtime behavior we cannot inspect statically\n\
                  EVIDENCE: unknown\n\n\
                  Begin your reply with the literal word VERDICT. No preamble.";

    // Structured three-line output is T1 work. The model id comes from
    // `.hexa/project.json` — it was pinned to a specific one here, which is the
    // G1 failure: a caller that names a model cannot be re-pointed by editing
    // configuration. HEXA_VERIFY_MODEL still overrides for a one-off.
    let model = match std::env::var("HEXA_VERIFY_MODEL") {
        Ok(m) if !m.is_empty() => m,
        _ => hexa_infer::tier_model("t1").ok_or_else(|| {
            anyhow::anyhow!(
                "no T1 model configured — set inference.tier_models in .hexa/project.json, \
                 or pass one with HEXA_VERIFY_MODEL"
            )
        })?,
    };

    // Gather first, ask second. A verifier states no verdict it did not earn.
    let (gathered, searched) = gather_evidence(claim);
    if gathered.is_empty() {
        return Ok(CheckResult {
            verdict: Verdict::Inconclusive,
            summary: "nothing in the claim was concrete enough to search for".to_string(),
            evidence: vec![
                "no file, path, identifier or quoted term to grep".to_string(),
                "name one, or use a claim shape the deterministic checks cover".to_string(),
            ],
            method: "no-evidence",
        });
    }

    let user = format!(
        "Claim: {claim}\n\nEvidence gathered from the repository with git grep.          This is all you have; you cannot run commands. Base the verdict only on          these lines, and cite one of them.\n\n{}",
        gathered.join("\n")
    );
    let content = hexa_infer::complete_text(&model, system, &user, 300)
        .await
        .map_err(|e| anyhow::anyhow!("verification inference: {e}"))?;
    let content = Regex::new(r"(?s)<think>.*?</think>")
        .unwrap()
        .replace_all(&content, "")
        .trim()
        .to_string();

    let verdict = Regex::new(r"(?im)^VERDICT:\s*(CONFIRMED|REFUTED|INCONCLUSIVE)").unwrap();
    let summary = Regex::new(r"(?im)^SUMMARY:\s*(.+)$").unwrap();
    let evidence = Regex::new(r"(?im)^EVIDENCE:\s*(.+)$").unwrap();

    let v_str = verdict.captures(&content)
        .and_then(|c| c.get(1)).map(|m| m.as_str().to_uppercase())
        .unwrap_or_else(|| "INCONCLUSIVE".into());
    let s_str = summary.captures(&content)
        .and_then(|c| c.get(1)).map(|m| m.as_str().trim().to_string())
        .unwrap_or_else(|| "model did not produce a SUMMARY line".into());
    let e_str = evidence.captures(&content)
        .and_then(|c| c.get(1)).map(|m| m.as_str().trim().to_string())
        .unwrap_or_else(|| "unknown".into());

    let mut shown: Vec<String> = vec![format!("searched: {}", searched.join(", "))];
    shown.extend(gathered.into_iter().take(6));
    if !e_str.eq_ignore_ascii_case("unknown") {
        shown.insert(0, format!("cited: {e_str}"));
    }

    Ok(CheckResult {
        verdict: match v_str.as_str() {
            "CONFIRMED" => Verdict::Confirmed,
            "REFUTED"   => Verdict::Refuted,
            _           => Verdict::Inconclusive,
        },
        summary: s_str,
        evidence: shown,
        method: "grep-then-judge",
    })
}

#[cfg(test)]
mod evidence_tests {
    use super::*;

    #[test]
    fn prose_alone_gives_nothing_to_search_for() {
        assert!(claim_tokens("this thing is mostly good and quite nice").is_empty());
    }

    #[test]
    fn a_path_an_identifier_and_a_quoted_term_are_all_picked_up() {
        let t = claim_tokens("`crate_roots` lives in hexa-graph/src/lib.rs and returns a HashMap");
        assert!(t.iter().any(|x| x == "crate_roots"), "{t:?}");
        assert!(t.iter().any(|x| x == "hexa-graph/src/lib.rs"), "{t:?}");
        assert!(t.iter().any(|x| x == "HashMap"), "{t:?}");
    }

    /// The defect: with nothing gathered, the verifier used to hand the bare
    /// claim to a model, which answered REFUTED and cited a command it had
    /// never run. No evidence now means no verdict (ADR-2609122048).
    #[test]
    fn no_searchable_term_means_no_verdict() {
        assert!(claim_tokens("it works well enough for now").is_empty());
    }
}
