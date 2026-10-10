//! The API contract is a tagged driving port (ADR-2610092245).
//!
//! The oracle is not derived from the extractor: `expected.openapi.json` was
//! written by hand before any extractor existed, and the same port is written
//! three times — Rust, Go, TypeScript. Each must produce that one document.
//! Three parsers agreeing with a hand-written answer is hard to fake by
//! mirroring a bug.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/api")
}

fn hexa(cwd: &Path, args: &[&str]) -> Output {
    let home = tempfile::tempdir().unwrap();
    Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(args)
        .current_dir(cwd)
        .env("HOME", home.path())
        .output()
        .unwrap()
}

fn text(out: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

/// A writable copy of one fixture, so a case can break it.
fn copy_of(lang: &str) -> tempfile::TempDir {
    fn copy(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap().flatten() {
            let dest = to.join(e.file_name());
            if e.file_type().unwrap().is_dir() {
                copy(&e.path(), &dest);
            } else {
                std::fs::copy(e.path(), dest).unwrap();
            }
        }
    }
    let d = tempfile::tempdir().unwrap();
    copy(&fixtures().join(lang), d.path());
    d
}

fn edit(root: &Path, rel: &str, from: &str, to: &str) {
    let p = root.join(rel);
    let src = std::fs::read_to_string(&p).unwrap();
    assert!(src.contains(from), "{rel} does not contain {from:?}");
    std::fs::write(&p, src.replacen(from, to, 1)).unwrap();
}

/// The generator stamp names the hexa version, so it is not part of what a
/// document *says*.
fn without_stamp(mut v: serde_json::Value) -> serde_json::Value {
    if let Some(info) = v.get_mut("info").and_then(|i| i.as_object_mut()) {
        info.remove("x-generated-by");
    }
    v
}

fn spec_of(dir: &Path) -> serde_json::Value {
    let out_dir = tempfile::tempdir().unwrap();
    let out_file = out_dir.path().join("openapi.json");
    let out = hexa(dir, &["api", "spec", ".", "--out", out_file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    serde_json::from_str(&std::fs::read_to_string(&out_file).unwrap()).unwrap()
}

fn expected() -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(fixtures().join("expected.openapi.json")).unwrap()).unwrap()
}

fn assert_matches_expected(lang: &str) {
    let got = without_stamp(spec_of(&fixtures().join(lang)));
    let want = expected();
    assert_eq!(
        got,
        want,
        "{lang}:\n{}",
        serde_json::to_string_pretty(&got).unwrap()
    );
}

#[test]
fn the_rust_port_produces_the_expected_document() {
    assert_matches_expected("rust");
}

#[test]
fn the_go_port_produces_the_expected_document() {
    assert_matches_expected("go");
}

#[test]
fn the_typescript_port_produces_the_expected_document() {
    assert_matches_expected("ts");
}

#[test]
fn the_document_is_openapi_3_1_and_every_ref_resolves() {
    let v = spec_of(&fixtures().join("rust"));
    assert_eq!(v["openapi"], "3.1.0");
    assert!(v["info"]["x-generated-by"].as_str().unwrap_or("").starts_with("hexa"));
    fn refs(v: &serde_json::Value, out: &mut Vec<String>) {
        match v {
            serde_json::Value::Object(m) => {
                if let Some(r) = m.get("$ref").and_then(|r| r.as_str()) {
                    out.push(r.to_string());
                }
                m.values().for_each(|x| refs(x, out));
            }
            serde_json::Value::Array(a) => a.iter().for_each(|x| refs(x, out)),
            _ => {}
        }
    }
    let mut all = Vec::new();
    refs(&v, &mut all);
    assert!(!all.is_empty());
    for r in all {
        let name = r.strip_prefix("#/components/schemas/").expect("a local schema ref");
        assert!(v["components"]["schemas"][name].is_object(), "{r} does not resolve");
    }
}

#[test]
fn an_untagged_method_is_not_exposed() {
    let v = spec_of(&fixtures().join("go"));
    let ids: Vec<String> = v["paths"]
        .as_object()
        .unwrap()
        .values()
        .flat_map(|p| p.as_object().unwrap().values())
        .map(|op| op["operationId"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(ids.len(), 4, "{ids:?}");
    assert!(!ids.iter().any(|i| i == "stats"), "{ids:?}");
}

#[test]
fn a_path_segment_with_no_parameter_fails_and_names_the_line() {
    let d = copy_of("rust");
    edit(d.path(), "src/ports/mod.rs", "GET /bookmarks/{id}", "GET /bookmarks/{bookmark}");
    let out = hexa(d.path(), &["api", "spec", "."]);
    assert_ne!(out.status.code(), Some(0), "{}", text(&out));
    let all = text(&out);
    assert!(all.contains("src/ports/mod.rs:"), "{all}");
    assert!(all.contains("{bookmark}"), "{all}");
    assert!(!d.path().join("openapi.json").exists(), "a failed contract writes no document");
}

#[test]
fn routes_differing_only_in_parameter_name_are_one_route() {
    let d = copy_of("rust");
    edit(
        d.path(),
        "src/ports/mod.rs",
        "    /// Not part of the API: no tag.",
        "    /// @hexa:api GET /bookmarks/{key}\n    fn find(&self, key: &str) -> Result<BookmarkValue, ApiError>;\n\n    /// Not part of the API: no tag.",
    );
    let out = hexa(d.path(), &["api", "spec", "."]);
    assert_ne!(out.status.code(), Some(0), "{}", text(&out));
    let all = text(&out);
    assert!(all.contains("already declared"), "{all}");
    assert!(!d.path().join("openapi.json").exists(), "a failed contract writes no document");
}

#[test]
fn an_unresolvable_type_fails_and_is_never_an_empty_schema() {
    let d = copy_of("go");
    edit(d.path(), "internal/domain/bookmark.go", "Title   string     `json:\"title\"`", "Title   Headline   `json:\"title\"`");
    let out = hexa(d.path(), &["api", "spec", "."]);
    assert_ne!(out.status.code(), Some(0), "{}", text(&out));
    let all = text(&out);
    assert!(all.contains("internal/domain/bookmark.go:"), "{all}");
    assert!(all.contains("Headline"), "{all}");
}

#[test]
fn a_tag_on_a_handler_is_a_rule_error_in_the_grade() {
    let d = copy_of("rust");
    std::fs::create_dir_all(d.path().join("src/adapters/primary")).unwrap();
    std::fs::write(
        d.path().join("src/adapters/primary/http.rs"),
        "use crate::ports::BookmarkApi;\n\n/// @hexa:api GET /health\npub fn health() -> &'static str { \"ok\" }\n",
    )
    .unwrap();
    let out = hexa(d.path(), &["analyze", ".", "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    let errors = v["api"]["errors"].as_array().cloned().unwrap_or_default();
    assert!(
        errors.iter().any(|e| e["file"] == "src/adapters/primary/http.rs"),
        "{}",
        text(&out)
    );
    assert_eq!(v["score_components"]["api_errors"], 1, "{}", text(&out));
}

#[test]
fn a_tagged_port_no_primary_adapter_drives_is_reported() {
    let d = copy_of("rust");
    let unserved = |root: &Path| {
        let out = hexa(root, &["analyze", ".", "--json"]);
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
        v["api"]["unserved"].clone()
    };
    assert_eq!(unserved(d.path()), serde_json::json!(["BookmarkApi"]));

    std::fs::create_dir_all(d.path().join("src/adapters/primary")).unwrap();
    std::fs::write(
        d.path().join("src/adapters/primary/http.rs"),
        "use crate::ports::BookmarkApi;\n\npub fn serve(_api: &dyn BookmarkApi) {}\n",
    )
    .unwrap();
    assert_eq!(unserved(d.path()), serde_json::json!([]));
}

#[test]
fn check_passes_on_a_fresh_spec_and_fails_on_drift() {
    let d = copy_of("ts");
    let out = hexa(d.path(), &["api", "spec", "."]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    assert!(d.path().join("openapi.json").is_file());

    let out = hexa(d.path(), &["api", "check", "."]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));

    edit(d.path(), "src/core/ports/bookmark-api.ts", "DELETE /bookmarks/{id}", "DELETE /links/{id}");
    let out = hexa(d.path(), &["api", "check", "."]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    let all = text(&out);
    assert!(all.contains("/links/{id}"), "the drift names the operation: {all}");
}

#[test]
fn a_project_with_no_tags_is_a_vacuous_contract() {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join("src/ports")).unwrap();
    std::fs::write(d.path().join("src/ports/mod.rs"), "pub trait Store { fn load(&self) -> u32; }\n").unwrap();
    for verb in ["spec", "check", "list"] {
        let out = hexa(d.path(), &["api", verb, "."]);
        assert_eq!(out.status.code(), Some(2), "api {verb}: {}", text(&out));
    }
}

#[test]
fn list_prints_one_line_per_operation() {
    let out = hexa(&fixtures().join("rust"), &["api", "list", "."]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().filter(|l| l.contains("BookmarkApi::")).collect();
    assert_eq!(lines.len(), 4, "{stdout}");
    assert!(lines.iter().any(|l| l.contains("DELETE") && l.contains("/bookmarks/{id}") && l.contains("src/ports/mod.rs:")), "{stdout}");
}

#[test]
fn ci_runs_the_drift_check_on_a_committed_document() {
    let api_line = |root: &Path| {
        let out = hexa(root, &["ci"]);
        text(&out).lines().find(|l| l.contains("API contract")).unwrap_or("").to_string()
    };
    let d = copy_of("go");
    assert!(hexa(d.path(), &["api", "spec", "."]).status.success());
    assert!(api_line(d.path()).contains("pass"), "{}", api_line(d.path()));

    edit(d.path(), "internal/ports/bookmark_api.go", "GET /bookmarks/{id}", "GET /links/{id}");
    assert!(api_line(d.path()).contains("fail"), "{}", api_line(d.path()));
}
