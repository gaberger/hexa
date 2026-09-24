//! What the discovered paths add up to: whether any model is reachable, which
//! models are on offer, and whether a given model is served. Pure — the
//! discovery itself is `crate::discover`.

use crate::ports::{Coverage, Found};

/// Does a frontier CLI answer for this model id?
fn frontier_serves(model: &str) -> bool {
    let m = model.to_ascii_lowercase();
    m.starts_with("claude") || m.starts_with("anthropic/") || m.starts_with("us.anthropic.")
}

/// Resolve `model` against the discovered paths. Never a substring match:
/// `registry::serving` makes the same point about routing, and a diagnosis
/// that guesses is the thing this replaces.
pub fn serves(found: &[Found], model: &str) -> Coverage {
    let reachable = || found.iter().filter(|f| f.reachable == Some(true));
    for f in reachable() {
        if f.models.iter().any(|m| m == model) {
            return Coverage::Served(f.name.clone());
        }
        if f.kind == "frontier" && frontier_serves(model) {
            return Coverage::Served(f.name.clone());
        }
    }
    // Nothing listed it. Only say so when every reachable path was asked.
    match reachable().find(|f| f.models.is_empty() && f.kind != "frontier") {
        Some(f) => Coverage::Unverified(f.name.clone()),
        None => Coverage::NotServed,
    }
}

/// Every model the reachable paths list, for the line that says what is
/// actually on offer.
pub fn served_models(found: &[Found]) -> Vec<String> {
    let mut out: Vec<String> = found
        .iter()
        .filter(|f| f.reachable == Some(true))
        .flat_map(|f| f.models.iter().cloned())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Is there any path to a model?
pub fn any_path(found: &[Found]) -> bool {
    found.iter().any(Found::open)
}

/// The open paths, in words: "local server, anthropic, claude".
pub fn path_words(found: &[Found]) -> String {
    let names: Vec<String> = found
        .iter()
        .filter(|f| f.open())
        .map(|f| match f.kind {
            "local" => "local server".to_string(),
            "frontier" => "claude".to_string(),
            _ => f.name.clone(),
        })
        .collect();
    if names.is_empty() { "none".to_string() } else { names.join(", ") }
}
