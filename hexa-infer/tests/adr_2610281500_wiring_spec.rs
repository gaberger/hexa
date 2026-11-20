//! ADR-2610281500 §3, §4: with real `HEXA_MANAGED*` variables and a real
//! token fd, `RegistryBackends` (what every completion runs on) and the
//! tier lookups defer to managed mode. Its own test binary: it sets
//! process-wide variables, so its tests run one at a time.
//!
//! Written before the code, from the ADR; not derived from it.

#[path = "support/stub_http.rs"]
mod stub_http;

use std::io::Write;
use std::os::fd::IntoRawFd;

use hexa_core::domain::messages::Message;
use hexa_core::ports::inference::{InferenceRequest, Priority};
use hexa_infer::ports::Backends;
use hexa_infer::wiring::RegistryBackends;
use stub_http::{anthropic_ok, Stub};

const TOKEN: &str = "tok-wiring-0123456789";

fn token_fd() -> i32 {
    let (r, w) = std::io::pipe().unwrap();
    let mut w = w;
    w.write_all(TOKEN.as_bytes()).unwrap();
    drop(w);
    r.into_raw_fd()
}

#[tokio::test]
async fn every_completion_and_tier_lookup_defers_to_managed_mode() {
    let s = Stub::start(vec![anthropic_ok("ok"), anthropic_ok("ok")]).await;
    let state = tempfile::tempdir().unwrap();
    let fd = token_fd();
    // The only test in this binary: no other thread reads the environment.
    {
        std::env::set_var("HEXA_MANAGED", "1");
        std::env::set_var("HEXA_MANAGED_URL", s.url());
        std::env::set_var("HEXA_MANAGED_TOKEN_FD", fd.to_string());
        std::env::set_var("HEXA_STATE_DIR", state.path());
        std::env::set_var(
            "HEXA_MANAGED_TIERS",
            r#"{"t1":{"model":"m-one","api":"openai"},"t2":{"model":"m-two","api":"anthropic"},"t2.5":{"model":"m-two","api":"anthropic"},"t3":{"model":"m-three","api":"anthropic"},"react":{"model":"m-react","api":"anthropic"}}"#,
        );
        // Settings managed mode must ignore.
        std::env::set_var("HEXA_COMPLETION_COMMAND", "/bin/false");
        std::env::set_var("ANTHROPIC_BASE_URL", "http://127.0.0.1:1");
        std::env::set_var("ANTHROPIC_API_KEY", "sk-ant-should-not-be-used");
        std::env::set_var("HEXA_OLLAMA_HOST", "http://127.0.0.1:1");
    }
    assert_eq!(hexa_infer::tiers::tier_model("t2").as_deref(), Some("m-two"));
    assert_eq!(hexa_infer::tiers::tier_model("t3").as_deref(), Some("m-three"));
    assert_eq!(hexa_infer::tiers::react_models(), vec!["m-react".to_string()]);

    let req = |model: &str| InferenceRequest {
        model: model.into(),
        system_prompt: String::new(),
        messages: vec![Message::user("hi")],
        tools: vec![],
        max_tokens: 10,
        temperature: 0.0,
        thinking_budget: None,
        cache_control: false,
        priority: Priority::default(),
        grammar: None,
    };
    RegistryBackends.complete(req("m-two")).await.unwrap();
    // The token was read once: a second call still works.
    RegistryBackends.complete(req("m-react")).await.unwrap();
    let seen = s.seen();
    assert_eq!(seen.len(), 2);
    assert!(seen.iter().all(|r| r.header("x-api-key") == Some(TOKEN)));
    // The fd is closed once read.
    assert!(std::fs::metadata(format!("/proc/self/fd/{fd}")).is_err(), "the token fd is still open");
    // Dumpable is off.
    // SAFETY: a plain prctl read.
    assert_eq!(unsafe { libc_prctl_get_dumpable() }, 0);
}

unsafe extern "C" {
    fn prctl(option: i32, ...) -> i32;
}

/// PR_GET_DUMPABLE is 3.
unsafe fn libc_prctl_get_dumpable() -> i32 {
    unsafe { prctl(3) }
}
