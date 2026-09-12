//! `hexa spend` sums the local inference log.

use std::process::Command;

#[test]
fn spend_sums_the_log_and_reports_the_budget() {
    let home = tempfile::tempdir().unwrap();
    let proj = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(proj.path().join(".hexa")).unwrap();
    std::fs::write(proj.path().join(".hexa/project.json"), r#"{ "inference": { "budget_usd_per_day": 2.0 } }"#).unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    std::fs::write(
        home.path().join("inference-log.jsonl"),
        format!(
            "{{\"model\":\"local\",\"input_tokens\":100,\"output_tokens\":50,\"source\":\"complete\",\"ts\":\"{now}\"}}\n\
             {{\"model\":\"claude-code\",\"input_tokens\":2000,\"output_tokens\":300,\"cost_usd\":0.25,\"source\":\"harden\",\"ts\":\"{now}\"}}\n"
        ),
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(["spend", "--json"])
        .env("HEXA_HOME", home.path())
        .current_dir(proj.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["today"]["calls"], 2);
    assert_eq!(v["today"]["input_tokens"], 2100);
    assert_eq!(v["today"]["priced_calls"], 1);
    assert!((v["today"]["cost_usd"].as_f64().unwrap() - 0.25).abs() < 1e-9);
    assert_eq!(v["budget_usd_per_day"], 2.0);
    assert_eq!(v["by_source_7_days"]["harden"]["calls"], 1);
}
