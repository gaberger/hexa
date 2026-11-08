#![allow(unused, clippy::all)]
use hexa_cli::fmt::{append_telemetry, telemetry_badge, TelemetryAsset};
use std::collections::HashMap;

/// S1: Telemetry asset is loaded and parsed correctly
#[test]
fn s1_telemetry_asset_is_loaded_and_parsed_correctly() {
    let text = r#"{"compression_ratio": {"icon": "🧠", "label": "Context"}} "#;
    let asset = TelemetryAsset::parse(text);
    let entry = asset.get("compression_ratio").unwrap();
    assert_eq!(entry.icon, "🧠");
    assert_eq!(entry.label, "Context");
}

/// S1n: Malformed telemetry asset does not crash the binary
#[test]
fn s1n_malformed_telemetry_asset_does_not_crash_the_binary() {
    let text = r#"{ "compression_ratio": { "icon": "🧠", "label": "Context" "#;
    let asset = TelemetryAsset::parse(text);
    assert!(asset.get("compression_ratio").is_none());
}

/// S2: Render full telemetry for compression ratio and tokens saved
#[test]
fn s2_render_full_telemetry_for_compression_ratio_and_tokens_saved() {
    let mut telemetry = HashMap::new();
    telemetry.insert("compression_ratio".to_string(), "8.5x".to_string());
    telemetry.insert("tokens_saved".to_string(), "120k".to_string());

    let text = r#"{
        "compression_ratio": {"icon": "🧠", "label": "Context"},
        "tokens_saved": {"icon": "⚡️", "label": "Saved"}
    }"#;
    let asset = TelemetryAsset::parse(text);
    let badge = telemetry_badge(&telemetry, &asset);
    assert_eq!(badge, "🧠 Context: 8.5x ⚡️ Saved: 120k");
}

/// S3: Render partial telemetry (only tokens saved)
#[test]
fn s3_render_partial_telemetry_only_tokens_saved() {
    let mut telemetry = HashMap::new();
    telemetry.insert("tokens_saved".to_string(), "10k".to_string());

    let text = r#"{"tokens_saved": {"icon": "⚡️", "label": "Saved"}} "#;
    let asset = TelemetryAsset::parse(text);
    let badge = telemetry_badge(&telemetry, &asset);
    assert_eq!(badge, "⚡️ Saved: 10k");
}

/// S4: Render nothing for empty telemetry
#[test]
fn s4_render_nothing_for_empty_telemetry() {
    let telemetry = HashMap::new();
    let asset = TelemetryAsset::parse("");
    let badge = telemetry_badge(&telemetry, &asset);
    assert_eq!(badge, "");
}

/// S5: Ignore unknown telemetry keys
#[test]
fn s5_ignore_unknown_telemetry_keys() {
    let mut telemetry = HashMap::new();
    telemetry.insert("unknown_metric".to_string(), "100".to_string());

    let asset = TelemetryAsset::parse("");
    let badge = telemetry_badge(&telemetry, &asset);
    assert_eq!(badge, "");
}

/// S6: Telemetry badges are appended to the status line
#[test]
fn s6_telemetry_badges_are_appended_to_the_status_line() {
    let mut telemetry = HashMap::new();
    telemetry.insert("compression_ratio".to_string(), "8.5x".to_string());

    let text = r#"{"compression_ratio": {"icon": "🧠", "label": "Context"}} "#;
    let asset = TelemetryAsset::parse(text);
    let badge = telemetry_badge(&telemetry, &asset);
    let line = "Processing...";
    let updated = append_telemetry(line, &badge);
    assert_eq!(updated, "Processing... 🧠 Context: 8.5x");
}

/// S6n: Empty telemetry does not add trailing whitespace to status line
#[test]
fn s6n_empty_telemetry_does_not_add_trailing_whitespace_to_status_line() {
    let telemetry = HashMap::new();
    let asset = TelemetryAsset::parse("");
    let badge = telemetry_badge(&telemetry, &asset);
    let line = "Processing...";
    let updated = append_telemetry(line, &badge);
    assert_eq!(updated, "Processing...");
}
