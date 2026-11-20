//! A loopback HTTP/1.1 stub for ADR-2610281500's tests: records each
//! request, answers each with the next scripted reply (past the end, 500).

#![allow(dead_code)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Debug, Clone)]
pub struct Seen {
    pub method: String,
    pub path: String,
    /// Lower-case names.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Seen {
    pub fn header(&self, n: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == n).map(|(_, v)| v.as_str())
    }
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap()
    }
}

pub struct Stub {
    pub addr: std::net::SocketAddr,
    seen: Arc<Mutex<Vec<Seen>>>,
    replies: Arc<Mutex<VecDeque<(u16, String)>>>,
}

impl Stub {
    pub async fn start(replies: Vec<(u16, String)>) -> Stub {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let replies = Arc::new(Mutex::new(VecDeque::from(replies)));
        let (s, r) = (seen.clone(), replies.clone());
        tokio::spawn(async move {
            while let Ok((mut c, _)) = l.accept().await {
                let (s, r) = (s.clone(), r.clone());
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut tmp = [0u8; 8192];
                    let head_end = loop {
                        let n = match c.read(&mut tmp).await { Ok(0) | Err(_) => return, Ok(n) => n };
                        buf.extend_from_slice(&tmp[..n]);
                        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") { break i; }
                    };
                    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
                    let mut lines = head.split("\r\n");
                    let first: Vec<&str> = lines.next().unwrap_or("").split(' ').collect();
                    let headers: Vec<(String, String)> = lines
                        .filter_map(|l| l.split_once(':'))
                        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
                        .collect();
                    let len: usize = headers.iter().find(|(k, _)| k == "content-length").and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
                    let mut body = buf[head_end + 4..].to_vec();
                    while body.len() < len {
                        let n = match c.read(&mut tmp).await { Ok(0) | Err(_) => break, Ok(n) => n };
                        body.extend_from_slice(&tmp[..n]);
                    }
                    s.lock().unwrap().push(Seen {
                        method: first.first().unwrap_or(&"").to_string(),
                        path: first.get(1).unwrap_or(&"").to_string(),
                        headers,
                        body,
                    });
                    let (status, text) = r.lock().unwrap().pop_front().unwrap_or((500, "{}".into()));
                    let resp = format!(
                        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
                        text.len()
                    );
                    let _ = c.write_all(resp.as_bytes()).await;
                    let _ = c.shutdown().await;
                });
            }
        });
        Stub { addr, seen, replies }
    }
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }
    pub fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

/// An Anthropic Messages answer with this text.
pub fn anthropic_ok(text: &str) -> (u16, String) {
    (200, serde_json::json!({
        "id": "msg_1", "type": "message", "role": "assistant", "model": "m",
        "content": [{"type": "text", "text": text}],
        "stop_reason": "end_turn", "stop_sequence": null,
        "usage": {"input_tokens": 10, "output_tokens": 3}
    }).to_string())
}

/// An OpenAI chat completion answer with this text.
pub fn openai_ok(text: &str) -> (u16, String) {
    (200, serde_json::json!({
        "id": "c1", "object": "chat.completion", "model": "m",
        "choices": [{"index": 0, "finish_reason": "stop", "message": {"role": "assistant", "content": text}}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 3}
    }).to_string())
}
