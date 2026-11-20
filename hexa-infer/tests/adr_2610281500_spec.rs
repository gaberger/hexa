//! ADR-2610281500: managed mode. The settings (§1, §2) with an injected
//! environment and fd reader, and the routing (§3), retries and refusals
//! (§5) against a loopback stub. Process-wide activation is in
//! adr_2610281500_wiring_spec.rs, which sets real variables.
//!
//! Written before the code, from the ADR; not derived from it.

#[path = "support/stub_http.rs"]
mod stub_http;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use hexa_core::domain::messages::Message;
use hexa_core::domain::tools::{ToolDefinition, ToolInputSchema};
use hexa_core::ports::inference::{InferenceError, InferenceRequest, Priority};
use hexa_infer::managed::{code_for, Api, Managed, ManagedError, RETRY_DELAYS};
use stub_http::{anthropic_ok, openai_ok, Stub};

const TOKEN: &str = "tok-0123456789abcdef";

fn tiers() -> String {
    serde_json::json!({
        "t1": {"model": "loc-a", "api": "openai"},
        "t2": {"model": "claude-sonnet-5-5", "api": "anthropic"},
        "t2.5": {"model": "claude-sonnet-5-5", "api": "anthropic"},
        "t3": {"model": "claude-opus-5-5", "api": "anthropic"},
        "react": {"model": "claude-sonnet-5-5", "api": "anthropic"}
    })
    .to_string()
}

fn env(over: &[(&str, Option<&str>)], url: &str) -> BTreeMap<String, String> {
    let mut m: BTreeMap<String, String> = [
        ("HEXA_MANAGED", "1"),
        ("HEXA_MANAGED_URL", url),
        ("HEXA_MANAGED_TOKEN_FD", "7"),
        ("HEXA_STATE_DIR", "/work/.parley/hexa"),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    m.insert("HEXA_MANAGED_TIERS".into(), tiers());
    for (k, v) in over {
        match v {
            Some(v) => m.insert(k.to_string(), v.to_string()),
            None => m.remove(*k),
        };
    }
    m
}

fn parse(e: &BTreeMap<String, String>, fd_bytes: &[u8]) -> Result<Option<Managed>, ManagedError> {
    let bytes = fd_bytes.to_vec();
    Managed::from_env(&|k: &str| e.get(k).cloned(), &move |fd: i32| {
        assert_eq!(fd, 7, "the fd named by HEXA_MANAGED_TOKEN_FD");
        Ok(bytes.clone())
    })
}

fn managed(url: &str) -> Managed {
    parse(&env(&[], url), format!("{TOKEN}\n").as_bytes()).unwrap().unwrap()
}

// ----------------------------------------------------------- settings ----

#[test]
fn only_hexa_managed_1_enters_managed_mode() {
    for v in [None, Some("0"), Some("true"), Some("yes"), Some(""), Some(" 1")] {
        let e = env(&[("HEXA_MANAGED", v)], "http://127.0.0.1:8790");
        assert!(parse(&e, b"x").unwrap().is_none(), "{v:?}");
    }
}

#[test]
fn the_settings_parse() {
    let m = managed("http://127.0.0.1:8790");
    assert_eq!(m.url, "http://127.0.0.1:8790");
    assert_eq!(m.token, TOKEN, "one trailing newline trimmed");
    assert_eq!(m.state_dir, PathBuf::from("/work/.parley/hexa"));
    assert_eq!(m.deadline, None);
    assert_eq!(m.tier("t2").unwrap().model, "claude-sonnet-5-5");
    assert_eq!(m.tier("t2").unwrap().api, Api::Anthropic);
    assert_eq!(m.tier("t1").unwrap().api, Api::OpenAi);
    assert_eq!(m.tier("t2.5").unwrap().model, "claude-sonnet-5-5");
    assert!(m.tier("t9").is_none());
    let d = parse(&env(&[("HEXA_MANAGED_DEADLINE", Some("1800000000"))], "http://x"), b"t").unwrap().unwrap();
    assert_eq!(d.deadline, Some(1_800_000_000));
    // A trailing slash on the URL is accepted and dropped.
    assert_eq!(managed("http://127.0.0.1:8790/").url, "http://127.0.0.1:8790");
}

#[test]
fn the_token_never_comes_from_the_environment() {
    let e = env(&[("HEXA_MANAGED_TOKEN_FD", None), ("HEXA_MANAGED_TOKEN", Some(TOKEN))], "http://x");
    let err = parse(&e, b"").unwrap_err();
    assert_eq!(err.var, "HEXA_MANAGED_TOKEN_FD");
}

/// Overrides, the fd's bytes, and the variable the error must name.
type Case<'a> = (Vec<(&'a str, Option<&'a str>)>, &'a [u8], &'a str);

#[test]
fn each_bad_setting_is_named() {
    let url = "http://127.0.0.1:8790";
    let cases: Vec<Case> = vec![
        (vec![("HEXA_MANAGED_URL", None)], b"t", "HEXA_MANAGED_URL"),
        (vec![("HEXA_MANAGED_URL", Some("ftp://x"))], b"t", "HEXA_MANAGED_URL"),
        (vec![("HEXA_MANAGED_URL", Some("http://x/v1"))], b"t", "HEXA_MANAGED_URL"),
        (vec![("HEXA_MANAGED_URL", Some("http://x/?a=b"))], b"t", "HEXA_MANAGED_URL"),
        (vec![("HEXA_MANAGED_TOKEN_FD", Some("2"))], b"t", "HEXA_MANAGED_TOKEN_FD"),
        (vec![("HEXA_MANAGED_TOKEN_FD", Some("x"))], b"t", "HEXA_MANAGED_TOKEN_FD"),
        (vec![], b"", "HEXA_MANAGED_TOKEN_FD"),
        (vec![], b"\n", "HEXA_MANAGED_TOKEN_FD"),
        (vec![("HEXA_MANAGED_TIERS", None)], b"t", "HEXA_MANAGED_TIERS"),
        (vec![("HEXA_MANAGED_TIERS", Some("not json"))], b"t", "HEXA_MANAGED_TIERS"),
        (vec![("HEXA_STATE_DIR", None)], b"t", "HEXA_STATE_DIR"),
        (vec![("HEXA_STATE_DIR", Some("relative/dir"))], b"t", "HEXA_STATE_DIR"),
        (vec![("HEXA_MANAGED_DEADLINE", Some("soon"))], b"t", "HEXA_MANAGED_DEADLINE"),
        (vec![("HEXA_MANAGED_DELEGATE", Some("claude"))], b"t", "HEXA_MANAGED_DELEGATE"),
    ];
    for (over, fd, var) in cases {
        let err = parse(&env(&over, url), fd).expect_err(&format!("{over:?}"));
        assert_eq!(err.var, var, "{over:?}: {err:?}");
        assert!(!err.reason.is_empty());
        assert!(!format!("{err}").contains(TOKEN), "a message never shows the token");
    }
    // A token over 4 KiB.
    let big = vec![b'a'; 4097];
    assert_eq!(parse(&env(&[], url), &big).unwrap_err().var, "HEXA_MANAGED_TOKEN_FD");
}

#[test]
fn the_tier_map_needs_exactly_the_five_tiers_each_well_formed() {
    let mut v: serde_json::Value = serde_json::from_str(&tiers()).unwrap();
    v.as_object_mut().unwrap().remove("react");
    let missing = v.to_string();
    let mut extra: serde_json::Value = serde_json::from_str(&tiers()).unwrap();
    extra["t4"] = serde_json::json!({"model": "m", "api": "anthropic"});
    let extra = extra.to_string();
    for bad in [
        missing,
        extra,
        tiers().replace("\"openai\"", "\"gemini\""),
        tiers().replace("\"loc-a\"", "\"\""),
        r#"{"t1": "loc-a"}"#.to_string(),
        "[]".to_string(),
    ] {
        let err = parse(&env(&[("HEXA_MANAGED_TIERS", Some(&bad))], "http://x"), b"t").unwrap_err();
        assert_eq!(err.var, "HEXA_MANAGED_TIERS", "{bad}");
    }
}

// ------------------------------------------------------------ routing ----

fn req(model: &str, tools: bool) -> InferenceRequest {
    InferenceRequest {
        model: model.into(),
        system_prompt: "sys".into(),
        messages: vec![Message::user("hi")],
        tools: if tools {
            vec![ToolDefinition {
                name: "read".into(),
                description: "read a file".into(),
                input_schema: ToolInputSchema { schema_type: "object".into(), properties: serde_json::json!({}), required: vec![] },
            }]
        } else {
            vec![]
        },
        max_tokens: 100,
        temperature: 0.0,
        thinking_budget: None,
        cache_control: false,
        priority: Priority::default(),
        grammar: None,
    }
}

#[tokio::test]
async fn an_anthropic_tier_goes_to_messages_with_x_api_key() {
    let s = Stub::start(vec![anthropic_ok("hello")]).await;
    let m = managed(&s.url());
    let r = m.complete(req("claude-sonnet-5-5", false)).await.unwrap();
    assert_eq!(r.input_tokens, 10);
    let seen = &s.seen()[0];
    assert_eq!((seen.method.as_str(), seen.path.as_str()), ("POST", "/v1/messages"));
    assert_eq!(seen.header("x-api-key"), Some(TOKEN));
    assert_eq!(seen.header("authorization"), None);
    assert_eq!(seen.json()["model"], "claude-sonnet-5-5");
}

#[tokio::test]
async fn an_openai_tier_goes_to_chat_completions_with_bearer() {
    let s = Stub::start(vec![openai_ok("hello")]).await;
    let m = managed(&s.url());
    m.complete(req("loc-a", false)).await.unwrap();
    let seen = &s.seen()[0];
    assert_eq!((seen.method.as_str(), seen.path.as_str()), ("POST", "/v1/chat/completions"));
    assert_eq!(seen.header("authorization"), Some(format!("Bearer {TOKEN}").as_str()));
    assert_eq!(seen.header("x-api-key"), None);
}

#[tokio::test]
async fn a_tool_using_request_goes_to_the_managed_url_too() {
    let s = Stub::start(vec![openai_ok("ok"), anthropic_ok("ok")]).await;
    let m = managed(&s.url());
    m.complete(req("loc-a", true)).await.unwrap();
    m.complete(req("claude-sonnet-5-5", true)).await.unwrap();
    let seen = s.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].path, "/v1/chat/completions");
    assert!(seen[0].json()["tools"].is_array());
    assert_eq!(seen[1].path, "/v1/messages");
    assert!(seen[1].json()["tools"].is_array());
}

#[tokio::test]
async fn a_model_outside_the_map_is_refused_without_a_call() {
    let s = Stub::start(vec![]).await;
    let m = managed(&s.url());
    assert!(m.complete(req("gpt-9", false)).await.is_err());
    assert!(s.seen().is_empty());
}

#[test]
fn the_retry_waits_are_the_adrs() {
    assert_eq!(RETRY_DELAYS, [2, 4, 8, 16, 32].map(Duration::from_secs));
}

#[tokio::test]
async fn a_5xx_is_retried_then_served() {
    let s = Stub::start(vec![(503, "{}".into()), (500, "{}".into()), anthropic_ok("ok")]).await;
    let m = managed(&s.url()).with_retry_delays(vec![Duration::from_millis(1); 5]);
    m.complete(req("claude-sonnet-5-5", false)).await.unwrap();
    assert_eq!(s.seen().len(), 3);
}

#[tokio::test]
async fn retries_stop_after_five() {
    let s = Stub::start(vec![(503, "{}".into()); 10]).await;
    let m = managed(&s.url()).with_retry_delays(vec![Duration::from_millis(1); 5]);
    assert!(m.complete(req("claude-sonnet-5-5", false)).await.is_err());
    assert_eq!(s.seen().len(), 6, "one try and five retries");
}

#[tokio::test]
async fn a_connection_error_is_retried() {
    // Nothing listens on this port.
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let m = managed(&format!("http://127.0.0.1:{port}")).with_retry_delays(vec![Duration::from_millis(1); 5]);
    let at = std::time::Instant::now();
    assert!(m.complete(req("claude-sonnet-5-5", false)).await.is_err());
    assert!(at.elapsed() < Duration::from_secs(10));
}

#[tokio::test]
async fn a_401_or_402_is_not_retried_and_has_its_code() {
    for (status, code) in [(401u16, "auth"), (402, "budget")] {
        let s = Stub::start(vec![(status, "{\"type\":\"error\"}".into()), anthropic_ok("ok")]).await;
        let m = managed(&s.url()).with_retry_delays(vec![Duration::from_millis(1); 5]);
        let e = m.complete(req("claude-sonnet-5-5", false)).await.unwrap_err();
        assert_eq!(s.seen().len(), 1, "{status} was retried");
        assert_eq!(code_for(&e), code);
    }
    assert_eq!(code_for(&InferenceError::Network("x".into())), "model_error");
    assert_eq!(code_for(&InferenceError::ApiError { status: 400, body: String::new() }), "model_error");
}
