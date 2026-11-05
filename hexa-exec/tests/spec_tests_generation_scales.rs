//! Spec test generation at the size of a real spec, and what the test writer
//! is told. Not sealed: the sealed file for ADR-2610221540 is
//! a_spec_is_drafted_and_its_tests_generated_red.rs, and these tests extend it
//! without changing it. The helpers are copied from it for that reason.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use hexa_core::domain::messages::{ContentBlock, StopReason};
use hexa_core::ports::inference::{InferenceError, InferenceRequest, InferenceResponse};
use hexa_core::ports::spec::Spec;

const MARKER: &str = "IMPLEMENTATION-MARKER-7f3a";

struct Double {
    answers: Mutex<Vec<String>>,
    seen: Arc<Mutex<Vec<String>>>,
}

impl Double {
    fn new(answers: &[String]) -> (Self, Arc<Mutex<Vec<String>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut a: Vec<String> = answers.to_vec();
        a.reverse();
        (Self { answers: Mutex::new(a), seen: seen.clone() }, seen)
    }
}

#[async_trait]
impl hexa_infer::ports::Backends for Double {
    async fn complete(&self, request: InferenceRequest) -> Result<InferenceResponse, InferenceError> {
        assert!(request.tools.is_empty(), "the spec author and test writer have no tools");
        let mut text = request.system_prompt.clone();
        for m in &request.messages {
            for c in &m.content {
                if let ContentBlock::Text { text: t } = c {
                    text.push('\n');
                    text.push_str(t);
                }
            }
        }
        self.seen.lock().unwrap().push(text);
        let answer = self.answers.lock().unwrap().pop().expect("an answer for every call");
        Ok(InferenceResponse {
            content: vec![ContentBlock::Text { text: answer }],
            model_used: request.model,
            stop_reason: StopReason::EndTurn,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            latency_ms: 0,
        })
    }

    fn record_spend(&self, _model: &str, _i: u64, _o: u64) {}
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn id() -> String {
    format!("ADR-{}", "2701010000")
}

/// A repository holding an ADR that names its port and its use case, a
/// crate whose port is a stub, and a use case carrying a marker.
fn repo() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let r = tmp.path().to_path_buf();
    std::fs::create_dir_all(r.join("docs/adrs")).unwrap();
    std::fs::create_dir_all(r.join("src")).unwrap();
    std::fs::write(
        r.join("docs/adrs").join(format!("{}-it-doubles.md", id())),
        format!(
            "# {}: It doubles\n\n**Status:** Accepted\n\n## Decision\n\n\
             1. **`double` returns twice its input.** The port is `src/ports.rs`.\n\
             2. **`double` never returns its input unchanged, except at zero.** The use case is `src/usecase.rs`.\n\n\
             ## References\n\nnone\n",
            id()
        ),
    )
    .unwrap();
    std::fs::write(r.join("Cargo.toml"), "[package]\nname = \"fx\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[workspace]\n").unwrap();
    std::fs::write(r.join("src/lib.rs"), "pub mod ports;\npub mod usecase;\npub use ports::double;\n").unwrap();
    std::fs::write(r.join("src/ports.rs"), "/// Twice `n`.\npub fn double(n: u32) -> u32 {\n    let _ = n;\n    todo!(\"the decision is not built yet\")\n}\n").unwrap();
    std::fs::write(r.join("src/usecase.rs"), format!("// {MARKER}\npub fn helper() -> u32 {{ 7 }}\n")).unwrap();
    std::fs::write(r.join(".gitignore"), "target/\n").unwrap();
    git(&r, &["init", "-q"]);
    git(&r, &["add", "."]);
    git(&r, &["commit", "-qm", "init"]);
    (tmp, r)
}

fn scenario(sid: &str, d: &str, kind: &str) -> String {
    format!(r#"{{"id":"{sid}","decision":["{d}"],"kind":"{kind}","description":"x","given":"g","when":"w","then":"t"}}"#)
}

fn drafted(both_negatives: bool) -> String {
    let mut s = vec![scenario("S1", "§1", "example"), scenario("S2", "§1", "negative"), scenario("S3", "§2", "example")];
    if both_negatives {
        s.push(scenario("S4", "§2", "negative"));
    }
    format!("```json\n{{\"adr\":\"{}\",\"feature\":\"it doubles\",\"specs\":[{}]}}\n```", id(), s.join(","))
}


/// The drafted spec, accepted as a person would.
fn accepted(r: &Path) -> Spec {
    let text = drafted(true);
    let json = text.trim_start_matches("```json\n").trim_end_matches("\n```");
    let spec: Spec = serde_json::from_str(json).unwrap();
    { let _ = r; hexa_exec::spec_author::accept(spec, "a reviewer") }
}

const TESTS_RED: &str = "```rust\n\
/// S1: two doubles to four\n#[test]\nfn two_doubles_to_four() { assert_eq!(fx::double(2), 4); }\n\n\
/// S2: one does not stay one\n#[test]\nfn one_does_not_stay_one() { assert_ne!(fx::double(1), 1); }\n\n\
/// S3: three doubles to six\n#[test]\nfn three_doubles_to_six() { assert_eq!(fx::double(3), 6); }\n\n\
/// S4: five does not stay five\n#[test]\nfn five_does_not_stay_five() { assert_ne!(fx::double(5), 5); }\n```";

/// An accepted spec of `n` scenarios, S1..Sn, over the ADR's two decisions.
fn accepted_n(n: usize) -> Spec {
    let s: Vec<String> = (1..=n)
        .map(|i| scenario(&format!("S{i}"), if i % 2 == 0 { "§2" } else { "§1" }, if i % 3 == 0 { "negative" } else { "example" }))
        .collect();
    let json = format!(r#"{{"adr":"{}","feature":"doubling","description":"x","specs":[{}]}}"#, id(), s.join(","));
    let spec: Spec = serde_json::from_str(&json).unwrap();
    hexa_exec::spec_author::accept(spec, "a reviewer")
}

/// A rust block with one red test for each of `ids`.
fn red_tests(ids: std::ops::RangeInclusive<usize>) -> String {
    let tests: String = ids
        .map(|i| format!("/// S{i}: {i} doubles\n#[test]\nfn doubles_{i}() {{ assert_eq!(fx::double({i}), {}); }}\n\n", 2 * i))
        .collect();
    format!("```rust\nuse fx::double as _d;\n\n{tests}```")
}

/// A spec too large for one answer is written in batches, each its own module
/// in the one file, and every scenario still gets its test (strata's
/// SQLite storage spec: one call for 29 scenarios kept only the last four).
#[tokio::test]
async fn a_large_spec_is_generated_in_batches_one_module_each() {
    let (_t, r) = repo();
    let spec = accepted_n(12);
    let (double, seen) = Double::new(&[red_tests(1..=10), red_tests(11..=12)]);
    let path_env = std::env::var("PATH").unwrap_or_default();
    let g = hexa_exec::spec_author::generate_tests(&double, "a-local-model", &r, &spec, Path::new("."), &path_env)
        .await
        .expect("generated in batches");
    assert_eq!(g.refs.len(), 12, "one test per scenario: {:?}", g.refs);
    assert_eq!(g.refs.get("S11").map(String::as_str), Some("tests/adr_2701010000_spec.rs::part_2::doubles_11"));
    assert_eq!(g.refs.get("S1").map(String::as_str), Some("tests/adr_2701010000_spec.rs::part_1::doubles_1"));
    let file = std::fs::read_to_string(r.join(&g.file)).unwrap();
    assert!(file.contains("mod part_1 {") && file.contains("mod part_2 {"), "{file}");
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2, "one call per batch");
    assert!(seen[0].contains("\"S10\"") && !seen[0].contains("\"S11\""), "the first batch holds S1..S10 only");
    assert!(seen[1].contains("\"S11\"") && !seen[1].contains("\"S10\""), "the second batch holds S11..S12 only");
}

/// A note in its own fence before the code is not taken for the code.
#[tokio::test]
async fn the_largest_fenced_block_is_the_test_file() {
    let (_t, r) = repo();
    let spec = accepted(&r);
    let answer = format!("```text\nThese tests use fx::double.\n```\n\n{TESTS_RED}");
    let (double, _) = Double::new(&[answer]);
    let path_env = std::env::var("PATH").unwrap_or_default();
    let g = hexa_exec::spec_author::generate_tests(&double, "a-local-model", &r, &spec, Path::new("."), &path_env)
        .await
        .expect("the rust block is found");
    assert_eq!(g.refs.len(), 4, "{:?}", g.refs);
}

/// A rejected generation says where its raw answer is, so a short or
/// malformed answer can be read rather than guessed at.
#[tokio::test]
async fn a_rejected_generation_keeps_the_raw_answer() {
    let (_t, r) = repo();
    let spec = accepted(&r);
    let missing = TESTS_RED.replace("/// S4: five does not stay five\n", "");
    let (double, _) = Double::new(std::slice::from_ref(&missing));
    let path_env = std::env::var("PATH").unwrap_or_default();
    let err = hexa_exec::spec_author::generate_tests(&double, "a-local-model", &r, &spec, Path::new("."), &path_env)
        .await
        .unwrap_err();
    let path = err
        .split("raw answer: ")
        .nth(1)
        .map(|p| p.lines().next().unwrap_or("").trim().to_string())
        .unwrap_or_else(|| panic!("the error names the raw answer: {err}"));
    assert_eq!(std::fs::read_to_string(&path).expect("the raw answer is kept"), missing);
}

/// The writer sees the ADR, whose Decision names the API the tests call, and
/// still not the implementation (ADR-2610221540 §2: the ADR and its ports).
#[tokio::test]
async fn the_test_writer_sees_the_adr_and_not_the_code() {
    let (_t, r) = repo();
    let spec = accepted(&r);
    let (double, seen) = Double::new(&[TESTS_RED.to_string()]);
    let path_env = std::env::var("PATH").unwrap_or_default();
    hexa_exec::spec_author::generate_tests(&double, "a-local-model", &r, &spec, Path::new("."), &path_env)
        .await
        .expect("generated");
    let seen = seen.lock().unwrap().join("\n");
    assert!(seen.contains("`double` returns twice its input"), "the ADR's decision text reaches the writer");
    assert!(!seen.contains(MARKER), "the implementation does not");
}

/// A scenario that is a whole suite is a module of tests (strata's SQLite
/// storage spec: "passes the conformance suite" expands a macro of 26 tests).
/// The module is red when any of its tests fails, and an empty one proves
/// nothing.
#[tokio::test]
async fn a_scenario_can_be_a_module_of_tests() {
    let (_t, r) = repo();
    let spec = accepted(&r);
    let answer = TESTS_RED.replace(
        "/// S1: two doubles to four\n#[test]\nfn two_doubles_to_four() { assert_eq!(fx::double(2), 4); }\n",
        "/// S1: small numbers double\nmod small_numbers_double {\n    #[test]\n    fn two() { assert_eq!(fx::double(2), 4); }\n    #[test]\n    fn three() { assert_eq!(fx::double(3), 6); }\n}\n",
    );
    let (double, _) = Double::new(&[answer]);
    let path_env = std::env::var("PATH").unwrap_or_default();
    let g = hexa_exec::spec_author::generate_tests(&double, "a-local-model", &r, &spec, Path::new("."), &path_env)
        .await
        .expect("a module scenario is generated");
    assert_eq!(g.refs.get("S1").map(String::as_str), Some("tests/adr_2701010000_spec.rs::small_numbers_double::"));
    assert_eq!(g.refs.len(), 4);
}

#[tokio::test]
async fn an_empty_module_scenario_is_rejected() {
    let (_t, r) = repo();
    let spec = accepted(&r);
    let answer = TESTS_RED.replace(
        "/// S1: two doubles to four\n#[test]\nfn two_doubles_to_four() { assert_eq!(fx::double(2), 4); }\n",
        "/// S1: small numbers double\nmod small_numbers_double {\n}\n",
    );
    let (double, _) = Double::new(&[answer]);
    let path_env = std::env::var("PATH").unwrap_or_default();
    let err = hexa_exec::spec_author::generate_tests(&double, "a-local-model", &r, &spec, Path::new("."), &path_env)
        .await
        .unwrap_err();
    assert!(err.contains("S1") && err.contains("0 tests"), "an empty module tests nothing: {err}");
}

/// The writer is told which crates the tests may use, so it does not guess
/// at one the test crate does not have (strata: serde_json, then duckdb).
#[tokio::test]
async fn the_test_writer_is_told_the_crates_it_may_use() {
    let (_t, r) = repo();
    let manifest = std::fs::read_to_string(r.join("Cargo.toml")).unwrap();
    std::fs::write(
        r.join("Cargo.toml"),
        manifest.replace("[workspace]", "[dependencies]\n\n[dev-dependencies]\n\n[workspace]"),
    )
    .unwrap();
    std::fs::write(
        r.join("Cargo.toml"),
        std::fs::read_to_string(r.join("Cargo.toml"))
            .unwrap()
            .replace("[dev-dependencies]\n", "[dev-dependencies]\n# a comment\n"),
    )
    .unwrap();
    git(&r, &["commit", "-qam", "deps"]);
    let spec = accepted(&r);
    let (double, seen) = Double::new(&[TESTS_RED.to_string()]);
    let path_env = std::env::var("PATH").unwrap_or_default();
    hexa_exec::spec_author::generate_tests(&double, "a-local-model", &r, &spec, Path::new("."), &path_env)
        .await
        .expect("generated");
    let seen = seen.lock().unwrap().join("\n");
    assert!(
        seen.contains("The tests may use only these crates: `fx`, std."),
        "the writer is told the crates it may use: {seen}"
    );
}

/// The writer is told the crate's Rust edition: strata's 2024 crates got tests
/// naming a variable `gen`, a reserved keyword there, and none compiled.
#[tokio::test]
async fn the_test_writer_is_told_the_rust_edition() {
    let (_t, r) = repo();
    let spec = accepted(&r);
    let (double, seen) = Double::new(&[TESTS_RED.to_string()]);
    let path_env = std::env::var("PATH").unwrap_or_default();
    hexa_exec::spec_author::generate_tests(&double, "a-local-model", &r, &spec, Path::new("."), &path_env)
        .await
        .expect("generated");
    let seen = seen.lock().unwrap().join("\n");
    assert!(seen.contains("The crate is Rust edition 2021"), "the writer is told the edition: {seen}");
}
