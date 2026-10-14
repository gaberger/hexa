//! Oracle for ADR-2606072044 step 2 — parse the candidate list out of config.
//! Composes candidate_models (step 1). Independent of direct_react.rs.
use hexa_exec::direct_react::react_models_from_config_value;
use serde_json::json;

fn s(xs: &[&str]) -> Vec<String> {
    xs.iter().map(|x| x.to_string()).collect()
}

#[test]
fn list_from_config() {
    let cfg = json!({ "inference": { "react_models": ["a", "b"] } });
    assert_eq!(react_models_from_config_value(&cfg, None), s(&["a", "b"]));
}
#[test]
fn single_from_config() {
    let cfg = json!({ "inference": { "react_model": "x" } });
    assert_eq!(react_models_from_config_value(&cfg, None), s(&["x"]));
}
#[test]
fn explicit_overrides_config() {
    let cfg = json!({ "inference": { "react_models": ["a", "b"] } });
    assert_eq!(react_models_from_config_value(&cfg, Some("z")), s(&["z"]));
}
/// An empty config yields no candidates. See the sibling oracle in
/// `candidate_models_oracle.rs`: a hardcoded last-resort model is founding
/// goal G1's failure case, because configuration can no longer re-point it.
#[test]
fn empty_config_yields_no_candidates() {
    let cfg = json!({});
    assert_eq!(react_models_from_config_value(&cfg, None), Vec::<String>::new());
}

/// The do-loop's documented last step is the project's T2 model
/// (`resolve_react_model`), and `NO_MODEL_CONFIGURED` tells the operator to
/// set exactly that key. A project that did so got an empty candidate list,
/// and `hexa do run` panicked in `select_best_of_n` ("outcomes must be
/// non-empty") — observed with the v26.10.0 binary on 2026-10-14.
#[test]
fn the_t2_tier_is_the_last_candidate_when_no_react_model_is_set() {
    let cfg = json!({ "inference": { "tier_models": { "t2": "m2" } } });
    assert_eq!(react_models_from_config_value(&cfg, None), s(&["m2"]));
    let both = json!({ "inference": { "react_models": ["a"], "tier_models": { "t2": "m2" } } });
    assert_eq!(react_models_from_config_value(&both, None), s(&["a"]), "a react list still wins");
}
