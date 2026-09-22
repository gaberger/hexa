//! `inference.react_models` has one reader, and every verb sees the same list.
//!
//! Two readers of that key existed. `hexa_infer::react_models` read the plural
//! array; `hexa_exec::direct_react::react_models_from_config_value` read the
//! plural array **and** a singular `inference.react_model`, from its own copy
//! of the file. So a project configured with the singular key alone had a
//! do-loop that ran and a `hexa hey` that reported nothing configured — the
//! same configuration meaning two things depending on who asked.
//!
//! That is the shape of the memory-scope bug (ADR-2609211200) in a different
//! key: no error, and the wrong answer looks like a normal answer.
//!
//! The reader under test is pure and takes the parsed document, so no test
//! here writes the process environment or the working directory
//! (ADR-2609131749).

use hexa_infer::react_models_in_config;

fn cfg(body: &str) -> serde_json::Value {
    serde_json::from_str(body).expect("fixture parses")
}

#[test]
fn the_plural_key_is_read_in_order() {
    let v = react_models_in_config(&cfg(
        r#"{ "inference": { "react_models": ["first", "second"] } }"#,
    ));
    assert_eq!(v, vec!["first".to_string(), "second".to_string()]);
}

#[test]
fn the_singular_key_is_read_too() {
    // The half only the do-loop's reader had. A project that set this alone
    // was invisible to every other verb.
    let v = react_models_in_config(&cfg(r#"{ "inference": { "react_model": "only" } }"#));
    assert_eq!(v, vec!["only".to_string()], "the singular key is configuration too");
}

#[test]
fn the_plural_key_wins_when_both_are_set() {
    let v = react_models_in_config(&cfg(
        r#"{ "inference": { "react_models": ["list"], "react_model": "single" } }"#,
    ));
    assert_eq!(v, vec!["list".to_string()], "a list is more specific than a single");
}

#[test]
fn nothing_configured_is_an_empty_list_and_never_a_guess() {
    // A model id defaulted here is one the operator never chose and cannot
    // change by editing configuration — founding goal G1 in one line. The
    // empty list makes the caller say what is missing.
    for body in [
        r#"{}"#,
        r#"{ "inference": {} }"#,
        r#"{ "inference": { "react_models": [] } }"#,
        r#"{ "inference": { "react_models": "not-an-array" } }"#,
        r#"{ "inference": { "react_model": "" } }"#,
    ] {
        assert!(
            react_models_in_config(&cfg(body)).is_empty(),
            "expected no models from {body}"
        );
    }
}

#[test]
fn a_malformed_entry_is_skipped_rather_than_failing_the_read() {
    let v = react_models_in_config(&cfg(
        r#"{ "inference": { "react_models": ["good", 7, null, "", "also-good"] } }"#,
    ));
    assert_eq!(
        v,
        vec!["good".to_string(), "also-good".to_string()],
        "one bad entry must not hide the models around it"
    );
}
