//! A spec is drafted from the ADR and its ports alone, and its tests are
//! generated red before any code (ADR-2610221540 §2, §5).
//!
//! The author of the tests must not be the author of the code, or the tests
//! encode the code's reading of the decision. Two things enforce that here:
//! the spec author is a tool-less completion that sees the ADR and the
//! port/domain files the ADR names, and nothing else. And every generated
//! test must fail against the unimplemented port before it is kept.
//!
//! Inference is a double: it records every prompt and answers in order.

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

#[tokio::test]
async fn a_draft_sees_the_adr_and_its_ports_never_the_implementation() {
    let (_t, r) = repo();
    let (double, seen) = Double::new(&[drafted(true)]);
    let d = hexa_exec::spec_author::draft(&double, "a-local-model", &r, &id()).await.expect("drafted");
    assert_eq!(d.spec.adr, id());
    assert_eq!(d.spec.specs.len(), 4);
    assert!(d.findings.is_empty(), "{:?}", d.findings);
    let seen = seen.lock().unwrap().join("\n");
    assert!(seen.contains("`double` returns twice its input"), "the ADR is given");
    assert!(seen.contains("pub fn double(n: u32) -> u32"), "the port it names is given");
    assert!(!seen.contains(MARKER), "the use case it names is not: a port or domain file only");
    assert_eq!(d.given, vec!["src/ports.rs".to_string()], "and the record says exactly what was given");
}

#[tokio::test]
async fn a_draft_by_a_model_that_is_an_agent_is_refused() {
    let (_t, r) = repo();
    let (double, seen) = Double::new(&[]);
    let err = hexa_exec::spec_author::draft(&double, "claude-code", &r, &id()).await.unwrap_err();
    assert!(err.contains("agent"), "an agent can read the implementation for itself: {err}");
    assert!(seen.lock().unwrap().is_empty(), "nothing was sent");
}

#[tokio::test]
async fn a_drafted_spec_is_checked_for_coverage() {
    let (_t, r) = repo();
    let (double, _) = Double::new(&[drafted(false)]);
    let d = hexa_exec::spec_author::draft(&double, "a-local-model", &r, &id()).await.expect("drafted");
    assert!(
        d.findings.iter().any(|f| f.message().contains("§2") && f.message().contains("negative")),
        "a model's draft is checked like anyone's: {:?}",
        d.findings
    );
}

/// The drafted spec, accepted as a person would.
fn accepted(r: &Path) -> Spec {
    let text = drafted(true);
    let json = text.trim_start_matches("```json\n").trim_end_matches("\n```");
    let spec: Spec = serde_json::from_str(json).unwrap();
    hexa_exec::spec_author::accept(spec, "a reviewer", r)
}

const TESTS_RED: &str = "```rust\n\
/// S1: two doubles to four\n#[test]\nfn two_doubles_to_four() { assert_eq!(fx::double(2), 4); }\n\n\
/// S2: one does not stay one\n#[test]\nfn one_does_not_stay_one() { assert_ne!(fx::double(1), 1); }\n\n\
/// S3: three doubles to six\n#[test]\nfn three_doubles_to_six() { assert_eq!(fx::double(3), 6); }\n\n\
/// S4: five does not stay five\n#[test]\nfn five_does_not_stay_five() { assert_ne!(fx::double(5), 5); }\n```";

#[tokio::test]
async fn tests_that_fail_first_are_committed_and_their_commit_is_red() {
    let (_t, r) = repo();
    let spec = accepted(&r);
    let (double, seen) = Double::new(&[TESTS_RED.to_string()]);
    let path_env = std::env::var("PATH").unwrap_or_default();
    let g = hexa_exec::spec_author::generate_tests(&double, "a-local-model", &r, &spec, Path::new("."), &path_env)
        .await
        .expect("generated");
    assert_eq!(g.file, "tests/adr_2701010000_spec.rs");
    assert!(r.join(&g.file).exists());
    assert_eq!(g.refs.get("S1").map(String::as_str), Some("tests/adr_2701010000_spec.rs::two_doubles_to_four"));
    assert_eq!(g.refs.len(), 4, "one test per scenario: {:?}", g.refs);
    assert_eq!(g.red, git(&r, &["rev-parse", "--short", "HEAD"]), "the commit holding the red tests is recorded");
    assert!(git(&r, &["log", "-1", "--format=%s"]).contains("(failing)"));
    let seen = seen.lock().unwrap().join("\n");
    assert!(seen.contains("pub fn double(n: u32) -> u32") && !seen.contains(MARKER), "the test writer sees ports, not code");
}

#[tokio::test]
async fn a_generation_with_a_test_green_before_the_code_is_rejected() {
    let (_t, r) = repo();
    let spec = accepted(&r);
    let vacuous = TESTS_RED.replace("assert_ne!(fx::double(1), 1)", "let _ = 1");
    let (double, _) = Double::new(&[vacuous]);
    let path_env = std::env::var("PATH").unwrap_or_default();
    let head = git(&r, &["rev-parse", "HEAD"]);
    let err = hexa_exec::spec_author::generate_tests(&double, "a-local-model", &r, &spec, Path::new("."), &path_env)
        .await
        .unwrap_err();
    assert!(err.contains("S2") && err.contains("passes"), "a test green before the code tests nothing: {err}");
    assert!(!r.join("tests/adr_2701010000_spec.rs").exists(), "nothing is left in the tree");
    assert_eq!(git(&r, &["rev-parse", "HEAD"]), head, "nothing is committed");
}

#[tokio::test]
async fn a_generation_that_skips_a_scenario_is_rejected() {
    let (_t, r) = repo();
    let spec = accepted(&r);
    let missing = TESTS_RED.replace("/// S4: five does not stay five\n", "");
    let (double, _) = Double::new(&[missing]);
    let path_env = std::env::var("PATH").unwrap_or_default();
    let err = hexa_exec::spec_author::generate_tests(&double, "a-local-model", &r, &spec, Path::new("."), &path_env)
        .await
        .unwrap_err();
    assert!(err.contains("S4"), "every scenario gets its test: {err}");
}

#[tokio::test]
async fn generation_refuses_a_spec_nobody_accepted() {
    let (_t, r) = repo();
    let mut spec = accepted(&r);
    spec.accepted = None;
    let (double, seen) = Double::new(&[]);
    let err = hexa_exec::spec_author::generate_tests(&double, "a-local-model", &r, &spec, Path::new("."), "")
        .await
        .unwrap_err();
    assert!(err.contains("not accepted"), "{err}");
    assert!(seen.lock().unwrap().is_empty());
}
