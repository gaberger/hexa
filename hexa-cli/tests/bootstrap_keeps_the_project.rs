//! `hexa bootstrap` leaves an existing `.hexa/project.json` untouched and
//! never writes a model name. It once overwrote the file with three
//! hardcoded models, then reported those models missing and failed.

use std::process::Command;

#[test]
fn bootstrap_keeps_project_json_and_names_no_model() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".hexa")).unwrap();
    let before = r#"{
  "name": "keepme",
  "analyze": { "exclude": ["vendor"] },
  "inference": { "tier_models": {}, "budget_usd_per_day": 3.0 }
}
"#;
    let path = dir.path().join(".hexa/project.json");
    std::fs::write(&path, before).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(["bootstrap", "--skip-prereq", "--skip-models"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before, "project.json was rewritten:\n{text}");
    for name in ["qwen", "gemma", "devstral", "llama3", "llama-3"] {
        assert!(!text.to_lowercase().contains(name), "a model name leaked into bootstrap output: {name}\n{text}");
    }
    assert!(text.contains("kept as it is"), "{text}");
}

#[test]
fn bootstrap_creates_a_config_without_models_when_none_exists() {
    let dir = tempfile::tempdir().unwrap();
    let _ = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(["bootstrap", "--skip-prereq", "--skip-models"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    let written = std::fs::read_to_string(dir.path().join(".hexa/project.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&written).unwrap();
    assert_eq!(v["inference"]["tier_models"], serde_json::json!({}));
    assert!(v.get("bootstrap").is_none(), "no bootstrap block, no provider name: {written}");
}
