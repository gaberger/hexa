//! A generation that leaves a scenario without a test is rejected, naming the
//! scenario left out (ADR-2610221540 §5).
//!
//! `hexa spec tests` generates one test per scenario, each carrying the
//! scenario id in its doc comment. A generation missing a scenario gets
//! follow-ups before it is rejected (ADR-2610281516 §5). This backend answers
//! every call, the follow-ups included, and every answer leaves S4 out. The
//! rejection must still come, and it must name S4.
//!
//! Inference is a double: it records every prompt and keeps giving its last
//! answer once the list runs out, so no follow-up can exhaust it.
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

struct EveryReplyBackend {
    answers: Mutex<Vec<String>>,
    last: String,
    seen: Arc<Mutex<Vec<String>>>,
}

impl EveryReplyBackend {
    fn answering(answers: &[String]) -> (Self, Arc<Mutex<Vec<String>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let last = answers.last().cloned().unwrap_or_default();
        let mut a: Vec<String> = answers.to_vec();
        a.reverse();
        (Self { answers: Mutex::new(a), last, seen: seen.clone() }, seen)
    }
}

#[async_trait]
impl hexa_infer::ports::Backends for EveryReplyBackend {
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
        let answer = self.answers.lock().unwrap().pop().unwrap_or_else(|| self.last.clone());
        Ok(InferenceResponse {
            content: vec![ContentBlock::Text { text: answer }],
            model_used: request.model,
            stop_reason: StopReason::EndTurn,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            latency_ms: 0,
            telemetry: std::collections::HashMap::new(),
        })
    }

    fn record_spend(&self, _model: &str, _i: u64, _o: u64) {}
}

fn git_here(dir: &Path, args: &[&str]) -> String {
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

fn fixture_adr() -> String {
    format!("ADR-{}", "2701010000")
}

/// A repository holding an ADR that names its port, and a crate whose port
/// is a stub.
fn repo_with_stub_port() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let r = tmp.path().to_path_buf();
    std::fs::create_dir_all(r.join("docs/adrs")).unwrap();
    std::fs::create_dir_all(r.join("src")).unwrap();
    std::fs::write(
        r.join("docs/adrs").join(format!("{}-it-doubles.md", fixture_adr())),
        format!(
            "# {}: It doubles\n\n**Status:** Accepted\n\n## Decision\n\n\
             1. **`double` returns twice its input.** The port is `src/ports.rs`.\n\
             2. **`double` never returns its input unchanged, except at zero.** The port is `src/ports.rs`.\n\n\
             ## References\n\nnone\n",
            fixture_adr()
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
    git_here(&r, &["init", "-q"]);
    git_here(&r, &["add", "."]);
    git_here(&r, &["commit", "-qm", "init"]);
    (tmp, r)
}

fn scenario_json(sid: &str, d: &str, kind: &str) -> String {
    format!(
        r#"{{"id":"{sid}","decision":["{d}"],"kind":"{kind}","description":"x","given":"g","when":"w","then":"t"}}"#
    )
}

/// A four-scenario spec, accepted as a person would.
fn four_scenario_spec() -> Spec {
    let s = [
        scenario_json("S1", "§1", "example"),
        scenario_json("S2", "§1", "negative"),
        scenario_json("S3", "§2", "example"),
        scenario_json("S4", "§2", "negative"),
    ];
    let json = format!(
        "{{\"adr\":\"{}\",\"feature\":\"it doubles\",\"specs\":[{}]}}",
        fixture_adr(),
        s.join(",")
    );
    let spec: Spec = serde_json::from_str(&json).unwrap();
    hexa_exec::spec_author::accept(spec, "a reviewer")
}

/// Generated tests for S1, S2 and S3, each red against the stub port, and
/// none for S4.
const GENERATED_WITHOUT_S4: &str = "```rust\n\
/// S1: two doubles to four\n#[test]\nfn two_doubles_to_four() { assert_eq!(fx::double(2), 4); }\n\n\
/// S2: one does not stay one\n#[test]\nfn one_does_not_stay_one() { assert_ne!(fx::double(1), 1); }\n\n\
/// S3: three doubles to six\n#[test]\nfn three_doubles_to_six() { assert_eq!(fx::double(3), 6); }\n```";

/// S15b: A generation that leaves a scenario without a test is rejected
#[tokio::test]
async fn a_generation_that_never_tests_s4_is_rejected_naming_s4() {
    let (_t, r) = repo_with_stub_port();
    let spec = four_scenario_spec();
    // The first reply and both follow-ups all leave S4 out.
    let replies = vec![GENERATED_WITHOUT_S4.to_string(); 3];
    let (backend, seen) = EveryReplyBackend::answering(&replies);
    let path_env = std::env::var("PATH").unwrap_or_default();
    let head = git_here(&r, &["rev-parse", "HEAD"]);

    let result =
        hexa_exec::spec_author::generate_tests(&backend, "a-local-model", &r, &spec, Path::new("."), &path_env).await;
    let err = match result {
        Ok(_) => panic!("a generation with no test for S4 must be rejected"),
        Err(e) => e,
    };

    assert!(err.contains("S4"), "the refusal names the scenario left out: {err}");
    assert!(!seen.lock().unwrap().is_empty(), "the test writer was asked");
    assert!(!r.join("tests/adr_2701010000_spec.rs").exists(), "nothing is left in the tree");
    assert_eq!(git_here(&r, &["rev-parse", "HEAD"]), head, "nothing is committed");
}
