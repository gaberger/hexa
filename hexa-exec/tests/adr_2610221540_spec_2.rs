//! A generation that leaves a scenario without a test is rejected
//! (ADR-2610221540 §5).
//!
//! `hexa spec tests` generates one test per scenario, each carrying the
//! scenario id in its doc comment. A generation that leaves a scenario with no
//! test would let a decision go unproven under a green run. So it is refused,
//! and the refusal names the scenario left out.
//!
//! Inference is a double: it records every prompt and answers in order.
//!
//! Ids are assembled at run time: the citation checker scans source, and a
//! literal id here would claim a decision exists.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use hexa_core::domain::messages::{ContentBlock, StopReason};
use hexa_core::ports::inference::{InferenceError, InferenceRequest, InferenceResponse};
use hexa_core::ports::spec::Spec;

struct RecordingBackend {
    answers: Mutex<Vec<String>>,
    seen: Arc<Mutex<Vec<String>>>,
}

impl RecordingBackend {
    fn with_answers(answers: &[String]) -> (Self, Arc<Mutex<Vec<String>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut a: Vec<String> = answers.to_vec();
        a.reverse();
        (Self { answers: Mutex::new(a), seen: seen.clone() }, seen)
    }
}

#[async_trait]
impl hexa_infer::ports::Backends for RecordingBackend {
    async fn complete(&self, request: InferenceRequest) -> Result<InferenceResponse, InferenceError> {
        assert!(request.tools.is_empty(), "the test writer has no tools");
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

fn run_git(dir: &Path, args: &[&str]) -> String {
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

fn adr_id() -> String {
    format!("ADR-{}", "2701010000")
}

/// A repository holding an ADR that names its port, and a crate whose port
/// is a stub.
fn stub_repo() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let r = tmp.path().to_path_buf();
    std::fs::create_dir_all(r.join("docs/adrs")).unwrap();
    std::fs::create_dir_all(r.join("src")).unwrap();
    std::fs::write(
        r.join("docs/adrs").join(format!("{}-it-doubles.md", adr_id())),
        format!(
            "# {}: It doubles\n\n**Status:** Accepted\n\n## Decision\n\n\
             1. **`double` returns twice its input.** The port is `src/ports.rs`.\n\
             2. **`double` never returns its input unchanged, except at zero.** The port is `src/ports.rs`.\n\n\
             ## References\n\nnone\n",
            adr_id()
        ),
    )
    .unwrap();
    std::fs::write(
        r.join("Cargo.toml"),
        "[package]\nname = \"fx\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[workspace]\n",
    )
    .unwrap();
    std::fs::write(r.join("src/lib.rs"), "pub mod ports;\npub use ports::double;\n").unwrap();
    std::fs::write(
        r.join("src/ports.rs"),
        "/// Twice `n`.\npub fn double(n: u32) -> u32 {\n    let _ = n;\n    todo!(\"the decision is not built yet\")\n}\n",
    )
    .unwrap();
    std::fs::write(r.join(".gitignore"), "target/\n").unwrap();
    run_git(&r, &["init", "-q"]);
    run_git(&r, &["add", "."]);
    run_git(&r, &["commit", "-qm", "init"]);
    (tmp, r)
}

fn one_scenario(sid: &str, d: &str, kind: &str) -> String {
    format!(
        r#"{{"id":"{sid}","decision":["{d}"],"kind":"{kind}","description":"x","given":"g","when":"w","then":"t"}}"#
    )
}

/// A four-scenario spec, accepted as a person would.
fn accepted_spec() -> Spec {
    let s = [
        one_scenario("S1", "§1", "example"),
        one_scenario("S2", "§1", "negative"),
        one_scenario("S3", "§2", "example"),
        one_scenario("S4", "§2", "negative"),
    ];
    let json = format!(
        "{{\"adr\":\"{}\",\"feature\":\"it doubles\",\"specs\":[{}]}}",
        adr_id(),
        s.join(",")
    );
    let spec: Spec = serde_json::from_str(&json).unwrap();
    hexa_exec::spec_author::accept(spec, "a reviewer")
}

/// Generated tests for S1, S2 and S3, each red against the stub port, and
/// none for S4.
const TESTS_WITHOUT_S4: &str = "```rust\n\
/// S1: two doubles to four\n#[test]\nfn two_doubles_to_four() { assert_eq!(fx::double(2), 4); }\n\n\
/// S2: one does not stay one\n#[test]\nfn one_does_not_stay_one() { assert_ne!(fx::double(1), 1); }\n\n\
/// S3: three doubles to six\n#[test]\nfn three_doubles_to_six() { assert_eq!(fx::double(3), 6); }\n```";

/// S15b: A generation that leaves a scenario without a test is rejected
#[tokio::test]
async fn a_generation_missing_the_test_for_s4_is_rejected_and_names_s4() {
    let (_t, r) = stub_repo();
    let spec = accepted_spec();
    let (backend, _seen) = RecordingBackend::with_answers(&[TESTS_WITHOUT_S4.to_string()]);
    let path_env = std::env::var("PATH").unwrap_or_default();
    let head = run_git(&r, &["rev-parse", "HEAD"]);

    let result = hexa_exec::spec_author::generate_tests(
        &backend,
        "a-local-model",
        &r,
        &spec,
        Path::new("."),
        &path_env,
    )
    .await;

    let err = match result {
        Ok(_) => panic!("a generation with no test for S4 must be rejected"),
        Err(e) => e,
    };
    assert!(err.contains("S4"), "the rejection names the scenario left without a test: {err}");
    assert_eq!(run_git(&r, &["rev-parse", "HEAD"]), head, "a rejected generation commits nothing");
}
