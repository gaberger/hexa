//! Every command hexa's own source runs or recommends must exist in hexa.
//!
//! ADR-2609140844, decision 6. `shipped_docs_name_real_verbs.rs` already holds
//! this line for the documents. It could not hold it for the source, and the
//! source is where the rot was worse: `hexa hey` routed eleven intents to verbs
//! that had been deleted with the daemon (ADR-2608241500). Asking hexa "what's
//! broken?" ran `hexa brain validate`, which does not exist, and the miss path
//! then recommended `hexa brain enqueue`, which does not exist either.
//!
//! A document about a deleted verb is confidently wrong. A *classifier* that
//! routes to one is confidently wrong and then executes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

fn hexa_bin() -> PathBuf {
    let mut p = std::env::current_exe().expect("test exe");
    p.pop(); // deps/
    p.pop(); // debug/ or release/
    p.push("hexa");
    p
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("workspace root").to_path_buf()
}

/// Ask the binary itself, exactly as the document test does. A chain that does
/// not resolve exits non-zero under `--help`, so there is no list to keep in
/// sync with the clap tree.
fn resolves(chain: &[String]) -> bool {
    Command::new(hexa_bin())
        .args(chain)
        .arg("--help")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// The checkable verb chain at the head of a command string.
///
/// A placeholder (`<path>`), a flag (`--all`), a quoted argument or a path
/// (`.`) ends the chain — everything before it is what we can check.
fn chain(command: &str) -> Vec<String> {
    let mut out = Vec::new();
    for t in command.split_whitespace() {
        let checkable = !t.is_empty()
            && !t.starts_with('-')
            && t.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !checkable {
            break;
        }
        out.push(t.to_string());
    }
    out
}

fn rust_sources(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// Every `args: "…"` literal in the CLI source, mapped to the files it is in.
///
/// This is the executed set. `execute_intent` runs it as `hexa <args>`, so a
/// dead chain here is not a stale suggestion — it is a command that runs and
/// fails, while the classifier reports it understood the request.
fn executed_commands() -> BTreeMap<String, Vec<String>> {
    let root = workspace_root();
    let src = root.join("hexa-cli/src");
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for file in rust_sources(&src) {
        let Ok(body) = std::fs::read_to_string(&file) else { continue };
        let label = file.strip_prefix(&root).unwrap_or(&file).display().to_string();
        let mut rest = body.as_str();
        while let Some(i) = rest.find("args: \"") {
            let after = &rest[i + 7..];
            let Some(j) = after.find('"') else { break };
            let cmd = after[..j].trim().to_string();
            rest = &after[j + 1..];
            if !cmd.is_empty() {
                out.entry(cmd).or_default().push(label.clone());
            }
        }
    }
    out
}

/// The two files allowed to name a verb that does not exist, and why.
///
/// `doctor/mod.rs` carries a deny list *of* deleted verbs — naming them is its
/// job. `hook/mod.rs` matches the phrase a user types in a prompt
/// ("hexa skip plan"), which is an escape hatch, not a command.
const NAMING_DEAD_VERBS_ON_PURPOSE: &[&str] =
    &["hexa-cli/src/commands/doctor/mod.rs", "hexa-cli/src/commands/hook/mod.rs"];

/// Every double-quoted string literal in a Rust source file.
///
/// The obvious implementation — find a quote, find the next quote — is wrong,
/// and wrong in the exact place that matters. `println!("hexa brain enqueue -- \"<x>\"")`
/// has an escaped quote in the middle, so the naive scan stops early, truncates
/// the literal to something that no longer parses as a command, and the dead
/// recommendation walks straight past the check that exists to catch it.
fn string_literals(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = body.char_indices().peekable();
    while let Some((_, c)) = chars.next() {
        if c != '"' {
            continue;
        }
        let mut lit = String::new();
        let mut closed = false;
        while let Some((_, c)) = chars.next() {
            match c {
                '\\' => {
                    // Keep the escape as written; the token classifier reads it.
                    lit.push('\\');
                    if let Some((_, n)) = chars.next() {
                        lit.push(n);
                    }
                }
                '"' => {
                    closed = true;
                    break;
                }
                '\n' => break, // an unterminated literal is not a literal
                _ => lit.push(c),
            }
        }
        if closed && !lit.is_empty() {
            out.push(lit);
        }
    }
    out
}

/// English function words. None of them is a hexa verb, and none of them ever
/// will be, because hexa's verbs are all things you do.
///
/// This list is how a printed *sentence* is told apart from a printed
/// *command*. `hexa has not recorded a result for it` and
/// `hexa brain enqueue hexa-command` are both literals beginning with "hexa ";
/// only one of them is a claim that a command exists. Prose uses function
/// words. Command lines do not.
const FUNCTION_WORDS: &[&str] = &[
    "a", "about", "after", "all", "also", "an", "and", "any", "are", "at", "be", "been",
    "before", "but", "can", "cannot", "did", "does", "each", "every", "for", "from", "had",
    "has", "have", "here", "how", "in", "into", "is", "it", "its", "just", "may", "might",
    "must", "no", "not", "of", "on", "only", "onto", "or", "our", "over", "so", "than",
    "that", "the", "then", "there", "these", "they", "this", "those", "to", "under", "very",
    "was", "we", "were", "what", "when", "which", "who", "why", "will", "with", "would",
    "you", "your",
];

/// Does this literal read as a command line rather than as a sentence?
///
/// Every token must be something a shell would accept — a word, a flag, a
/// placeholder, a path, a quoted argument — and no bare word may be a function
/// word. A sentence fails on its first "the", its first capital, or its first
/// full stop.
fn looks_like_a_command(body: &str) -> bool {
    let mut words = 0usize;
    for t in body.split_whitespace() {
        if t.starts_with('-') {
            continue; // a flag; its value is checked as its own token
        }
        let t = t.trim_start_matches('\\');
        if t.starts_with('<') || t.starts_with('\'') || t.starts_with('"') || t == "." || t == ".." {
            continue; // a placeholder, a quoted argument, or a path
        }
        if !t.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '/' || c == '_') {
            return false; // a capital, a full stop, a comma — prose
        }
        if FUNCTION_WORDS.contains(&t) {
            return false;
        }
        words += 1;
    }
    words > 0
}

/// Every backticked `hexa …` span inside a string literal.
///
/// Backticks are how this codebase marks a command it is recommending, the
/// same convention the shipped documents use. A span inside backticks is a
/// claim regardless of what surrounds it.
fn backticked(lit: &str, out: &mut Vec<String>) {
    let mut rest = lit;
    while let Some(open) = rest.find('`') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('`') else { break };
        let span = after[..close].trim();
        rest = &after[close + 1..];
        if let Some(cmd) = span.strip_prefix("hexa ") {
            let cmd = cmd.trim();
            if !cmd.is_empty() {
                out.push(cmd.to_string());
            }
        }
    }
}

/// Every string literal in the CLI source that recommends a hexa command.
///
/// Two shapes count. A backticked span is always a recommendation. A bare
/// literal counts when the whole thing reads as a command line — which is what
/// `hexa brain enqueue hexa-command -- "<your-command>"` was, printed straight
/// at a user who could not run it.
fn recommended_commands() -> BTreeMap<String, Vec<String>> {
    let root = workspace_root();
    let src = root.join("hexa-cli/src");
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for file in rust_sources(&src) {
        let label = file.strip_prefix(&root).unwrap_or(&file).display().to_string();
        if NAMING_DEAD_VERBS_ON_PURPOSE.contains(&label.as_str()) {
            continue;
        }
        let Ok(body) = std::fs::read_to_string(&file) else { continue };
        for lit in string_literals(&body) {
            let lit = lit.as_str();
            let mut claims: Vec<String> = Vec::new();
            backticked(lit, &mut claims);
            if let Some(cmd) = lit.trim_start().strip_prefix("hexa ") {
                let cmd = cmd.trim_end_matches("\\n").trim();
                if !cmd.is_empty() && looks_like_a_command(cmd) {
                    claims.push(cmd.to_string());
                }
            }
            for c in claims {
                out.entry(c).or_default().push(label.clone());
            }
        }
    }
    out
}

fn assert_all_resolve(label: &str, found: &BTreeMap<String, Vec<String>>, floor: usize) {
    assert!(
        found.len() >= floor,
        "{label}: found only {} command(s); the extractor is broken, which would \
         make this test pass vacuously",
        found.len()
    );
    let mut dead: Vec<String> = Vec::new();
    for (cmd, files) in found {
        let c = chain(cmd);
        if c.is_empty() {
            continue;
        }
        if !resolves(&c) {
            let mut where_ = files.clone();
            where_.sort();
            where_.dedup();
            dead.push(format!("hexa {}  ← \"{}\"  in {}", c.join(" "), cmd, where_.join(", ")));
        }
    }
    assert!(
        dead.is_empty(),
        "{label}: {} command(s) that do not exist:\n  {}",
        dead.len(),
        dead.join("\n  ")
    );
}

/// The commands `hexa hey` actually runs.
#[test]
fn every_executed_command_exists() {
    assert_all_resolve("executed commands", &executed_commands(), 8);
}

/// The commands the CLI prints at a user.
#[test]
fn every_recommended_command_exists() {
    assert_all_resolve("recommended commands", &recommended_commands(), 5);
}

/// The exclusion list cannot rot into a blanket exemption: each file on it must
/// still exist, and the list must stay short enough to read.
#[test]
fn the_exclusion_list_is_still_honest() {
    let root = workspace_root();
    for f in NAMING_DEAD_VERBS_ON_PURPOSE {
        assert!(root.join(f).is_file(), "excluded file no longer exists: {f} — drop it from the list");
    }
    assert!(
        NAMING_DEAD_VERBS_ON_PURPOSE.len() <= 3,
        "the exclusion list has grown to {}; that is an exemption, not an exception",
        NAMING_DEAD_VERBS_ON_PURPOSE.len()
    );
}
