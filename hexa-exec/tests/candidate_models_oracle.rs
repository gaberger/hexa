//! Oracle for ADR-2606072044 step 1 — candidate model resolution precedence.
//! Independent of the impl file the agent edits (direct_react.rs).
use hexa_exec::direct_react::candidate_models;

fn s(xs: &[&str]) -> Vec<String> {
    xs.iter().map(|x| x.to_string()).collect()
}

#[test]
fn explicit_model_wins() {
    assert_eq!(candidate_models(Some("foo"), &s(&["a", "b"])), s(&["foo"]));
}
#[test]
fn configured_list_used_when_no_explicit() {
    assert_eq!(candidate_models(None, &s(&["a", "b"])), s(&["a", "b"]));
}
// The singular-key fallback that used to be asserted here is now read, not
// resolved: it is `hexa_infer::react_models_in_config`, gated by
// `hexa-infer/tests/one_reader_for_react_models.rs`. The end-to-end assertion
// for it is unchanged in the sibling oracle,
// `react_models_config_oracle.rs::single_from_config`.
/// Nothing configured means nothing to run, not a model of hexa's choosing.
///
/// This used to assert a hardcoded pair of model ids. Founding goal G1 says no
/// non-test file outside `hexa-infer` may name a provider or a model, and a
/// hardcoded last resort is the clearest way to break it: the operator cannot
/// re-point it by editing configuration, and cannot see that it happened,
/// because a run on the wrong model looks exactly like a run on the right one.
///
/// An empty list makes the caller say `NO_MODEL_CONFIGURED` and name the key
/// that is missing.
#[test]
fn nothing_configured_yields_no_candidates() {
    assert_eq!(candidate_models(None, &[]), Vec::<String>::new());
}
