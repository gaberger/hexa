//! The register gate: operator-facing prose has a word count.
//!
//! `hexa bro` got this first (ADR-2609140925, decision 4), because its whole
//! job is to be understood by someone who just walked back in. The same rule
//! belongs on every string hexa hands a person and calls guidance — a playbook
//! step is read at the start of a task by someone deciding what to do, which is
//! exactly when a 40-word sentence costs most.
//!
//! Shared by `#[path]` rather than through the library, so the check is a test
//! concern and never becomes a public surface of `hexa-cli`.

#![allow(dead_code)]

/// The longest a sentence may be. Past this, a reader is holding too much.
pub const MAX_WORDS: usize = 25;

/// The prose sentences of a block of text.
///
/// A line that ends in a full stop is prose. A line that does not is a literal
/// being quoted — a command, a title, a commit subject — and hexa does not get
/// to rewrite those to fit a word count.
pub fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if !t.ends_with('.') {
            continue;
        }
        for piece in t.trim_end_matches('.').split(". ") {
            let s = piece.trim();
            if !s.is_empty() {
                out.push(s.to_string());
            }
        }
    }
    out
}

/// Every sentence is short enough, and there are some.
///
/// The floor is the half that matters: text that says nothing passes a word
/// count perfectly.
pub fn assert_plain(label: &str, text: &str, floor: usize) {
    let found = sentences(text);
    assert!(
        found.len() >= floor,
        "{label}: found only {} sentence(s); text that says nothing passes every \
         other check here.\n{text}",
        found.len()
    );
    let long: Vec<String> = found
        .iter()
        .filter(|s| s.split_whitespace().count() > MAX_WORDS)
        .map(|s| format!("{} words: {s}", s.split_whitespace().count()))
        .collect();
    assert!(
        long.is_empty(),
        "{label}: {} sentence(s) over {MAX_WORDS} words:\n  {}",
        long.len(),
        long.join("\n  ")
    );
}
