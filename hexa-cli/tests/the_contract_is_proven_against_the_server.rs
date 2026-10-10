//! The contract is proven against the server (ADR-2610092329).
//!
//! The server here is written with `std::net` in this file, from the
//! contract's own words, not from hexa's code: a bookmarks API kept in
//! memory. Each mode breaks it in one way the ADR says must be caught. The
//! contract is the Rust fixture from ADR-2610092245.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Conforming,
    /// `tags` comes back as one comma-joined string.
    TagsAsString,
    /// The list answers 418, which no operation declares.
    Teapot,
    /// Every bookmark carries a `createdAt` the schema does not name.
    ExtraField,
    /// Only URLs under https://accepted.example/ are saved; anything else is 400.
    StrictUrl,
    /// Everything is 404 — declared, and proves nothing.
    AllNotFound,
    /// The create keeps no tags, so the list by tag comes back empty — a
    /// valid answer that says nothing about the items it would hold.
    ForgetsTags,
}

struct Server {
    base: String,
}

fn serve(mode: Mode) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let store: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let store = store.clone();
            std::thread::spawn(move || handle(stream, mode, &store));
        }
    });
    Server { base }
}

fn handle(stream: TcpStream, mode: Mode, store: &Mutex<Vec<Value>>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    let mut length = 0usize;
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                length = v.trim().parse().unwrap_or(0);
            }
        }
    }
    let mut body = vec![0u8; length];
    let _ = reader.read_exact(&mut body);
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();
    let (status, reply) = route(mode, store, &method, &target, &body);
    let text = reply.map(|v| v.to_string()).unwrap_or_default();
    let reason = match status { 200 => "OK", 201 => "Created", 204 => "No Content", 400 => "Bad Request", 404 => "Not Found", 418 => "I'm a teapot", _ => "Other" };
    let mut out = stream;
    let _ = write!(
        out,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
        text.len()
    );
}

fn route(mode: Mode, store: &Mutex<Vec<Value>>, method: &str, target: &str, body: &[u8]) -> (u16, Option<Value>) {
    if mode == Mode::AllNotFound {
        return (404, Some(json!({"error": "not found"})));
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let shape = |b: &Value| {
        let mut b = b.clone();
        match mode {
            Mode::TagsAsString => {
                let joined: Vec<String> = b["tags"].as_array().unwrap().iter().map(|t| t.as_str().unwrap().to_string()).collect();
                b["tags"] = json!(joined.join(","));
            }
            Mode::ExtraField => b["createdAt"] = json!(1_700_000_000_000u64),
            _ => {}
        }
        b
    };
    let mut bookmarks = store.lock().unwrap();
    match (method, path) {
        ("POST", "/bookmarks") => {
            let Ok(req) = serde_json::from_slice::<Value>(body) else { return (400, Some(json!({"error": "json"}))) };
            let url = req["url"].as_str().unwrap_or("");
            if mode == Mode::StrictUrl && !url.starts_with("https://accepted.example/") {
                return (400, Some(json!({"error": "url refused"})));
            }
            let b = json!({
                "id": format!("bm-{}", bookmarks.len() + 1),
                "url": url,
                "title": req["title"],
                "tags": if mode == Mode::ForgetsTags { json!([]) } else { req["tags"].clone() },
                "savedAt": "2026-10-09T00:00:00Z",
            });
            bookmarks.push(b.clone());
            (201, Some(shape(&b)))
        }
        ("GET", "/bookmarks") => {
            if mode == Mode::Teapot {
                return (418, None);
            }
            let Some(tag) = query.split('&').find_map(|kv| kv.strip_prefix("tag=")) else {
                return (400, Some(json!({"error": "a tag is required"})));
            };
            let found: Vec<Value> = bookmarks
                .iter()
                .filter(|b| b["tags"].as_array().unwrap().iter().any(|t| t == tag))
                .map(shape)
                .collect();
            (200, Some(json!(found)))
        }
        ("GET", p) if p.starts_with("/bookmarks/") => {
            let id = &p["/bookmarks/".len()..];
            match bookmarks.iter().find(|b| b["id"] == id) {
                Some(b) => (200, Some(shape(b))),
                None => (404, Some(json!({"error": "not found"}))),
            }
        }
        ("DELETE", p) if p.starts_with("/bookmarks/") => {
            let id = p["/bookmarks/".len()..].to_string();
            bookmarks.retain(|b| b["id"] != id.as_str());
            (204, None)
        }
        _ => (404, None),
    }
}

fn contract() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/api/rust")
}

fn api_test(base: &str, extra: &[&str]) -> (Option<i32>, String) {
    let home = tempfile::tempdir().unwrap();
    let mut args = vec!["api", "test", ".", "--base-url", base];
    args.extend_from_slice(extra);
    let out: Output = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(&args)
        .current_dir(contract())
        .env("HOME", home.path())
        .output()
        .unwrap();
    let all = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    (out.status.code(), all)
}

#[test]
fn a_conforming_server_proves_every_operation() {
    let s = serve(Mode::Conforming);
    let (code, all) = api_test(&s.base, &[]);
    assert_eq!(code, Some(0), "{all}");
    assert!(all.contains("4 proven"), "{all}");
}

#[test]
fn a_field_of_the_wrong_type_is_a_violation_at_its_path() {
    let s = serve(Mode::TagsAsString);
    let (code, all) = api_test(&s.base, &[]);
    assert_eq!(code, Some(1), "{all}");
    assert!(all.contains("POST /bookmarks"), "{all}");
    assert!(all.contains("$.tags"), "{all}");
}

#[test]
fn an_undeclared_status_is_a_violation() {
    let s = serve(Mode::Teapot);
    let (code, all) = api_test(&s.base, &[]);
    assert_eq!(code, Some(1), "{all}");
    assert!(all.contains("418"), "{all}");
    assert!(all.contains("GET /bookmarks"), "{all}");
}

#[test]
fn a_property_the_schema_does_not_name_is_a_violation() {
    let s = serve(Mode::ExtraField);
    let (code, all) = api_test(&s.base, &[]);
    assert_eq!(code, Some(1), "{all}");
    assert!(all.contains("createdAt"), "{all}");
}

#[test]
fn a_refused_input_is_unproven_until_an_example_is_given() {
    let s = serve(Mode::StrictUrl);
    let (code, all) = api_test(&s.base, &[]);
    // Nothing was created, so the list is empty and proves nothing either: 2.
    assert_eq!(code, Some(2), "the create, the empty list and the reads that need an id are unproven: {all}");
    assert!(all.contains("unproven"), "{all}");
    assert!(all.contains("--examples"), "{all}");

    let dir = tempfile::tempdir().unwrap();
    let examples = dir.path().join("examples.json");
    std::fs::write(&examples, r#"{"url": "https://accepted.example/hexa"}"#).unwrap();
    let (code, all) = api_test(&s.base, &["--examples", examples.to_str().unwrap()]);
    assert_eq!(code, Some(0), "{all}");
}

#[test]
fn a_server_that_refuses_everything_proves_nothing() {
    let s = serve(Mode::AllNotFound);
    let (code, all) = api_test(&s.base, &[]);
    assert_eq!(code, Some(2), "{all}");
}

#[test]
fn nothing_listening_is_never_a_pass() {
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let (code, all) = api_test(&format!("http://127.0.0.1:{port}"), &[]);
    assert_ne!(code, Some(0), "{all}");
    assert!(all.contains("unreachable"), "{all}");
}

#[test]
fn a_project_with_no_tags_has_no_contract_to_test() {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join("src/ports")).unwrap();
    std::fs::write(d.path().join("src/ports/mod.rs"), "pub trait Store { fn load(&self) -> u32; }\n").unwrap();
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(["api", "test", ".", "--base-url", "http://127.0.0.1:9"])
        .current_dir(d.path())
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn an_empty_list_proves_nothing_about_its_items() {
    let s = serve(Mode::ForgetsTags);
    let (code, all) = api_test(&s.base, &[]);
    assert_eq!(code, Some(1), "{all}");
    // The list's own line, not the read by id.
    let list = all.lines().find(|l| l.contains(" GET /bookmarks ")).unwrap_or_default();
    assert!(list.contains("unproven"), "the empty list must be unproven: {all}");
    assert!(all.contains("empty list"), "and say why: {all}");
}
