//! Sealed tests for ADR-2610281516: inference is measured, cached, and repaired
//! rather than repeated.

use std::fs;
use std::future::Future;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use hexa_core::domain::design::{goldfish_prompt, needs_synthesis, stage_reviewers, Stage};
use hexa_core::ports::inference::{InferenceError, InferenceRequest, InferenceResponse};
use hexa_infer::complete::complete_raw_with;
use hexa_infer::ports::{completion_key, Backends, PromptCompressor};
use hexa_core::domain::messages::Message;

// ── Harness ──────────────────────────────────────────────

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn tmpdir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "hexa-adr1516-{tag}-{}-{nanos}-{n}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn envelope(result: &str, input: u64, output: u64, cache_read: u64, cost: &str) -> String {
    format!(
        "{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":{},\"usage\":{{\"input_tokens\":{input},\"output_tokens\":{output},\"cache_read_input_tokens\":{cache_read}}},\"total_cost_usd\":{cost}}}",
        json_str(result)
    )
}

const STUB: &str = r#"#!/bin/sh
D='__DIR__'
case " $* " in *" --version "*) echo "2.1.0 (Claude Code)"; exit 0;; esac
touch "$D/seq/$$"
n=$(ls "$D/seq" | wc -l | tr -d ' ')
f="$D/calls/$n.$$"
for a in "$@"; do printf '%s\0' "$a"; done > "$f.argv"
cat > "$f.stdin"
if grep -q SCEN-MARK "$f.stdin"; then
  k=$(( $(ls "$D/tests" | wc -l | tr -d ' ') + 1 ))
  cp "$f.stdin" "$D/tests/$k.stdin"
  if [ -f "$D/replies/tests-$k.json" ]; then cat "$D/replies/tests-$k.json"; else cat "$D/replies/tests.json"; fi
else
  cat "$D/replies/default.json"
fi
"#;

struct Env {
    root: PathBuf,
    home: PathBuf,
    stub: PathBuf,
}

impl Env {
    fn new(tag: &str) -> Self {
        let base = tmpdir(tag);
        let root = base.join("project");
        let home = base.join("home");
        let stub = base.join("stub");
        for d in [&root, &home] {
            fs::create_dir_all(d).unwrap();
        }
        for d in ["seq", "calls", "tests", "replies", "bin"] {
            fs::create_dir_all(stub.join(d)).unwrap();
        }
        let script = STUB.replace("__DIR__", stub.to_str().unwrap());
        let bin = stub.join("bin").join("claude");
        fs::write(&bin, script).unwrap();
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(stub.join("replies/default.json"), envelope("No findings.", 1, 1, 0, "0.0001")).unwrap();
        fs::write(stub.join("replies/tests.json"), envelope("No reply configured.", 1, 1, 0, "0.0001")).unwrap();
        let env = Env { root, home, stub };
        env.git(&["init", "-q"]);
        env.git(&["config", "user.name", "Tester"]);
        env.git(&["config", "user.email", "tester@example.org"]);
        env
    }

    fn git(&self, args: &[&str]) {
        let st = Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .output()
            .expect("git runs");
        assert!(st.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&st.stderr));
    }

    fn commit(&self) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "--allow-empty", "-m", "fixture"]);
    }

    fn write(&self, rel: &str, contents: &str) {
        let p = self.root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, contents).unwrap();
    }

    fn reply_default(&self, json: &str) {
        fs::write(self.stub.join("replies/default.json"), json).unwrap();
    }

    fn reply_tests(&self, k: usize, json: &str) {
        fs::write(self.stub.join(format!("replies/tests-{k}.json")), json).unwrap();
        fs::write(self.stub.join("replies/tests.json"), json).unwrap();
    }

    fn hexa(&self, args: &[&str], envs: &[(&str, &str)]) -> Output {
        let real_home = std::env::var("HOME").unwrap_or_default();
        let cargo_home = std::env::var("CARGO_HOME").unwrap_or_else(|_| format!("{real_home}/.cargo"));
        let rustup_home = std::env::var("RUSTUP_HOME").unwrap_or_else(|_| format!("{real_home}/.rustup"));
        let path = format!(
            "{}:{}",
            self.stub.join("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_hexa"));
        cmd.args(args)
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .env("HOME", &self.home)
            .env("PATH", path)
            .env("CARGO_HOME", cargo_home)
            .env("RUSTUP_HOME", rustup_home)
            .env_remove("HEXA_NO_CACHE");
        for (k, v) in envs {
            cmd.env(k, v);
        }
        cmd.output().expect("hexa runs")
    }

    /// Every recorded `claude` call, in order: (argv, stdin).
    fn calls(&self) -> Vec<(Vec<String>, String)> {
        let dir = self.stub.join("calls");
        let mut stems: Vec<(u64, String)> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| {
                let name = e.ok()?.file_name().to_string_lossy().to_string();
                let stem = name.strip_suffix(".stdin")?.to_string();
                let n = stem.split('.').next()?.parse().ok()?;
                Some((n, stem))
            })
            .collect();
        stems.sort();
        stems
            .into_iter()
            .map(|(_, stem)| {
                let argv = fs::read(dir.join(format!("{stem}.argv"))).unwrap_or_default();
                let argv = String::from_utf8_lossy(&argv)
                    .split('\0')
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect();
                let stdin = String::from_utf8_lossy(&fs::read(dir.join(format!("{stem}.stdin"))).unwrap()).to_string();
                (argv, stdin)
            })
            .collect()
    }

    /// The stdin of each test-writer call, in order.
    fn writer_calls(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut k = 1;
        loop {
            let p = self.stub.join(format!("tests/{k}.stdin"));
            match fs::read(&p) {
                Ok(b) => out.push(String::from_utf8_lossy(&b).to_string()),
                Err(_) => break,
            }
            k += 1;
        }
        out
    }

    fn spend_text(&self) -> String {
        let out = self.hexa(&["spend"], &[]);
        let mut text = String::from_utf8_lossy(&out.stdout).to_string();
        let hexa_dir = self.home.join(".hexa");
        let mut stack = vec![hexa_dir.clone()];
        while let Some(d) = stack.pop() {
            if d.starts_with(hexa_dir.join("cache")) {
                continue;
            }
            if let Ok(rd) = fs::read_dir(&d) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        stack.push(p);
                    } else if let Ok(s) = fs::read_to_string(&p) {
                        text.push('\n');
                        text.push_str(&s);
                    }
                }
            }
        }
        text
    }

    fn spend_cache_lines(&self) -> usize {
        let out = self.hexa(&["spend"], &[]);
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| l.contains("cache") && !l.contains("cache_read"))
            .count()
    }

    fn cache_files(&self) -> usize {
        fs::read_dir(self.home.join(".hexa/cache/completions"))
            .map(|rd| rd.flatten().filter(|e| e.path().is_file()).count())
            .unwrap_or(0)
    }

    fn report(&self) -> String {
        fs::read_to_string(self.root.join("docs/designs/goldfish_review_report.md")).unwrap_or_default()
    }

    fn draft(&self, contents: &str) -> &'static str {
        self.write("docs/designs/design_draft.md", contents);
        self.commit();
        "docs/designs/design_draft.md"
    }

    fn review_stage(&self, draft: &str, stage: &str, envs: &[(&str, &str)]) -> Output {
        self.hexa(&["design", "review", draft, "--stage", stage, "--model", "claude-text"], envs)
    }
}

fn ok(out: &Output, what: &str) {
    assert!(
        out.status.success(),
        "{what} failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn has_json_output_flag(argv: &[String]) -> bool {
    argv.iter().any(|a| a == "--output-format=json")
        || argv.windows(2).any(|w| w[0] == "--output-format" && w[1] == "json")
}

// ── Spec-tests harness ───────────────────────────────────

const ADR_ID: &str = "ADR-2610290001";

fn spec_env(tag: &str) -> Env {
    let env = Env::new(tag);
    env.write(
        "Cargo.toml",
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n",
    );
    env.write(".gitignore", "target/\n");
    env.write(
        "src/lib.rs",
        "//! The demo crate.\n\npub mod secret;\n\n/// The answer.\npub fn answer() -> u32 {\n    unimplemented!(\"built later\")\n}\n",
    );
    env.write("src/secret.rs", "pub fn secret(x: u32) -> u32 { x } // IMPL-SECRET\n");
    env.write(
        &format!("docs/adrs/{ADR_ID}-demo-answers.md"),
        &format!(
            "---\nid: {ADR_ID}\nstatus: proposed\ndate: 2026-10-29\n---\n# {ADR_ID}: the demo answers\n\n## Decision\n\n1. **The demo answers.** `answer` in `src/lib.rs` returns the answer.\n\n## Implementation\n\n`src/lib.rs`\n\n## Gate\n\n`hexa spec verify {ADR_ID}`\n"
        ),
    );
    env.commit();
    let scenarios = r#"```json
[
  {"id":"S01","decision":["§1"],"kind":"example","description":"SCEN-MARK one","given":"the crate","when":"answer is called","then":"it returns 1"},
  {"id":"S02","decision":["§1"],"kind":"negative","description":"SCEN-MARK two","given":"the crate","when":"answer is called","then":"it does not return 0"},
  {"id":"S03","decision":["§1"],"kind":"property","description":"SCEN-MARK three","given":"the crate","when":"answer is called","then":"it is stable"},
  {"id":"S04","decision":["§1"],"kind":"example","description":"SCEN-MARK four","given":"the crate","when":"answer is called","then":"it returns 4"}
]
```"#;
    env.reply_default(&envelope(scenarios, 10, 10, 0, "0.001"));
    let out = env.hexa(&["spec", "draft", ADR_ID, "--model", "claude-text"], &[("HEXA_NO_CACHE", "1")]);
    ok(&out, "hexa spec draft");
    let out = env.hexa(&["spec", "accept", ADR_ID], &[]);
    ok(&out, "hexa spec accept");
    env.commit();
    env
}

const ALL: [&str; 4] = ["S01", "S02", "S03", "S04"];
const E0502_HELPER: &str = "#[allow(dead_code)]\nfn helper() {\n    let mut v = vec![1u32];\n    let first = &v[0];\n    v.push(2);\n    println!(\"{first}\");\n}\n";
const SECRET_HELPER: &str = "#[allow(dead_code)]\nfn helper_two() -> u32 {\n    demo::secret::secret(\"text\")\n}\n";

fn writer_file(ids: &[&str], unpassable: Option<&str>, extra: &str, mark: &str) -> String {
    let mut out = format!("// {mark}\n\n");
    for id in ids {
        let n: u32 = id[1..].parse().unwrap();
        let body = if unpassable == Some(*id) {
            "assert!(false, \"UNPASSABLE-MARK\");".to_string()
        } else {
            format!("assert_eq!(demo::answer(), {n});")
        };
        out.push_str(&format!(
            "/// {id}: SCEN-MARK scenario {id}\n#[test]\nfn {}() {{\n    {body}\n}}\n\n",
            id.to_lowercase()
        ));
    }
    out.push_str(extra);
    out
}

fn writer_reply(file: &str) -> String {
    envelope(&format!("```rust\n{file}\n```"), 100, 100, 0, "0.01")
}

fn run_spec_tests(env: &Env) -> Output {
    env.hexa(&["spec", "tests", ADR_ID, "--model", "claude-text"], &[("HEXA_NO_CACHE", "1")])
}

fn accepted_test_file(env: &Env) -> Option<String> {
    let rd = fs::read_dir(env.root.join("tests")).ok()?;
    rd.flatten()
        .filter_map(|e| fs::read_to_string(e.path()).ok())
        .find(|s| s.contains("/// S04"))
}

// ── In-process backend for S08 ───────────────────────────

struct Counting {
    calls: AtomicUsize,
}

impl Backends for Counting {
    #[allow(unreachable_code)]
    fn complete<'life0, 'async_trait>(
        &'life0 self,
        request: InferenceRequest,
    ) -> Pin<Box<dyn Future<Output = Result<InferenceResponse, InferenceError>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            drop(request);
            let r: Result<InferenceResponse, InferenceError> = panic!("COUNTING-BACKEND-STOP");
            r
        })
    }

    fn record_spend(&self, _model: &str, _input_tokens: u64, _output_tokens: u64) {}
}

fn block_on<F: Future>(f: F) -> F::Output {
    struct Unpark(std::thread::Thread);
    impl std::task::Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut f = std::pin::pin!(f);
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::park();
    }
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}

// ── §1 ───────────────────────────────────────────────────

/// S01: claude-text reads the reply and the usage from claude's JSON output and records both in the spend log.
#[test]
fn s01_claude_text_reads_json_reply_and_usage() {
    let env = Env::new("s01");
    env.reply_default(r#"{"result":"hello","usage":{"input_tokens":120,"output_tokens":7,"cache_read_input_tokens":100},"total_cost_usd":0.0042}"#);
    let draft = env.draft("# Design\n\nsay hello\n");
    let out = env.review_stage(draft, "5", &[("HEXA_NO_CACHE", "1")]);
    ok(&out, "hexa design review");
    let calls = env.calls();
    assert_eq!(calls.len(), 1, "one completion for stage 5");
    let argv = &calls[0].0;
    assert!(argv.iter().any(|a| a == "-p"), "claude invoked with -p: {argv:?}");
    assert!(has_json_output_flag(argv), "claude invoked with --output-format json: {argv:?}");
    let report = env.report();
    assert!(!report.contains("total_cost_usd"), "the reply is the result, not the envelope");
    let spend = env.spend_text();
    assert!(spend.contains("120"), "input tokens 120 recorded: {spend}");
    assert!(spend.contains('7'), "output tokens 7 recorded: {spend}");
    assert!(spend.contains("100"), "cache-read tokens 100 recorded: {spend}");
    assert!(spend.contains("0.0042"), "cost 0.0042 USD recorded: {spend}");
}

/// S02: The prompt reaches claude on stdin.
#[test]
fn s02_prompt_reaches_claude_on_stdin() {
    let env = Env::new("s02");
    env.reply_default(&envelope("No findings.", 10, 2, 0, "0.001"));
    let draft = env.draft("# Design\n\nPROMPT-MARKER-1\n");
    let out = env.review_stage(draft, "5", &[("HEXA_NO_CACHE", "1")]);
    ok(&out, "hexa design review");
    let calls = env.calls();
    assert!(!calls.is_empty(), "claude was called");
    assert!(calls.iter().any(|(_, stdin)| stdin.contains("PROMPT-MARKER-1")), "the prompt travelled on stdin");
}

/// S03: The prompt is never passed as an argument, and a prompt over 128 KiB still goes through.
#[test]
fn s03_large_prompt_not_in_argv() {
    let env = Env::new("s03");
    env.reply_default(&envelope("No findings.", 10, 2, 0, "0.001"));
    let mut doc = String::from("# Design\n\n");
    let mut i: u64 = 0;
    while doc.len() < 200 * 1024 - 32 {
        doc.push_str(&format!(
            "Line {i:06} of the oversized draft, word{} and word{}.\n",
            i * 7919 % 100_003,
            i * 104_729 % 99_991
        ));
        i += 1;
    }
    doc.push_str("TAIL-MARKER\n");
    let draft = env.draft(&doc);
    let out = env.review_stage(draft, "5", &[("HEXA_NO_CACHE", "1")]);
    ok(&out, "hexa design review with a 200 KiB draft");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("Argument list too long"), "no argument-length error");
    let calls = env.calls();
    assert!(!calls.is_empty(), "claude was called");
    let body = doc.trim_end();
    for (argv, stdin) in &calls {
        for a in argv {
            assert!(!a.contains("TAIL-MARKER"), "argv carries the prompt tail");
            let mut off = 0;
            while off + 1024 <= body.len() {
                assert!(!a.contains(&body[off..off + 1024]), "argv carries a 1 KiB slice of the prompt");
                off += 1024;
            }
        }
        assert!(stdin.contains(body), "stdin holds the whole prompt");
    }
}

/// S04: The raw JSON envelope is not returned as the reply, and a zero-token entry is not recorded as if usage had been read.
#[test]
fn s04_envelope_not_reply_and_usage_recorded() {
    let env = Env::new("s04");
    env.reply_default(&envelope("- answer", 50, 5, 0, "0.0007"));
    let draft = env.draft("# Design\n\nA small draft.\n");
    let out = env.review_stage(draft, "5", &[("HEXA_NO_CACHE", "1")]);
    ok(&out, "hexa design review");
    let report = env.report();
    assert!(report.contains("answer"), "the reply's result reached the report: {report}");
    assert!(!report.contains("usage"), "the envelope is not the reply");
    assert!(!report.contains("total_cost_usd"), "the envelope is not the reply");
    let spend = env.spend_text();
    assert!(spend.contains("50"), "the spend entry records 50 input tokens, not 0: {spend}");
}

/// S05: For every prompt, the prompt travels on stdin and never in argv.
#[test]
fn s05_property_prompt_on_stdin_never_argv() {
    for (n, size) in [24usize, 70 * 1024, 1024 * 1024].into_iter().enumerate() {
        let env = Env::new(&format!("s05-{n}"));
        env.reply_default(&envelope("No findings.", 10, 2, 0, "0.001"));
        let marker = format!("UNIQ-{n}-QZX");
        let mut p = marker.clone();
        let mut i = 0u64;
        while p.len() < size {
            p.push_str(&format!(" w{i}"));
            i += 1;
        }
        p.truncate(size.max(marker.len()));
        let draft = env.draft(&p);
        let out = env.review_stage(draft, "5", &[("HEXA_NO_CACHE", "1")]);
        ok(&out, "hexa design review");
        let calls = env.calls();
        assert!(!calls.is_empty(), "claude was called for size {size}");
        for (argv, stdin) in &calls {
            assert!(stdin.contains(p.as_str()), "stdin holds the prompt in full (size {size})");
            assert!(argv.iter().all(|a| !a.contains(&marker)), "argv carries the marker (size {size})");
        }
    }
}

// ── §2 ───────────────────────────────────────────────────

/// S06: completion_key is a stable sha256 of model, system and user prompt.
#[test]
fn s06_completion_key_stable_sha256() {
    let a = completion_key("m1", "sys", "usr");
    let b = completion_key("m1", "sys", "usr");
    assert_eq!(a, b);
    assert!(is_hex64(&a), "64 hex characters: {a}");
}

/// S07: A second identical tool-less completion is answered from the cache and recorded as a cache hit.
#[test]
fn s07_identical_completion_answered_from_cache() {
    let env = Env::new("s07");
    env.reply_default(&envelope("- R", 30, 3, 0, "0.001"));
    let draft = env.draft("# Design\n\nCache me.\n");
    let out = env.review_stage(draft, "5", &[]);
    ok(&out, "first review");
    let first = env.report();
    let out = env.review_stage(draft, "5", &[]);
    ok(&out, "second review");
    let second = env.report();
    assert_eq!(env.calls().len(), 1, "the backend is called once");
    for r in [&first, &second] {
        assert!(r.contains("S5-1") && r.contains('R'), "both replies are R: {r}");
    }
    assert_eq!(env.cache_files(), 1, "exactly one cache file for the key");
    let spend = env.spend_text();
    assert!(spend.contains("cache"), "the second entry has source cache: {spend}");
}

struct NoOpCompressor;
#[async_trait::async_trait]
impl hexa_infer::ports::PromptCompressor for NoOpCompressor {
    async fn compress(&self, _model: &str, messages: Vec<hexa_core::domain::messages::Message>) -> Result<(Vec<hexa_core::domain::messages::Message>, std::collections::HashMap<String, String>), String> {
        Ok((messages, std::collections::HashMap::new()))
    }
}

/// S08: A request that carries tools is never cached.
#[test]
fn s08_request_with_tools_never_cached() {
    let home = tmpdir("s08-home");
    // SAFETY: only this test changes the process environment.
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::remove_var("HEXA_NO_CACHE");
    }
    let key = completion_key("m-tools", "sys", "usr");
    assert!(is_hex64(&key));
    let cache = home.join(".hexa/cache/completions");
    fs::create_dir_all(&cache).unwrap();
    fs::write(cache.join(&key), "CACHED").unwrap();
    let before = fs::read_dir(&cache).unwrap().count();

    let backend = Counting { calls: AtomicUsize::new(0) };
    let req = serde_json::json!({
        "model": "m-tools",
        "system": "sys",
        "max_tokens": 64,
        "messages": [{"role": "user", "content": "usr"}],
        "tools": [{"name": "read_file", "description": "read a file", "input_schema": {"type": "object", "properties": {}}}]
    });
    for _ in 0..2 {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| block_on(complete_raw_with(&backend, &NoOpCompressor, &req))));
        if let Ok(Ok(v)) = r {
            assert!(!v.to_string().contains("CACHED"), "a tooled request was answered from the cache");
        }
    }
    assert_eq!(backend.calls.load(Ordering::SeqCst), 2, "the backend is called twice");
    assert_eq!(fs::read_dir(&cache).unwrap().count(), before, "nothing is put into the cache");
}

/// S09: The bypasses force a fresh completion: HEXA_NO_CACHE=1 does, and `--fresh` on `hexa design` and `hexa drift` does.
#[test]
fn s09_bypasses_force_fresh_completion() {
    let env = Env::new("s09");
    env.reply_default(&envelope("No findings.", 30, 3, 0, "0.001"));
    env.write("src/lib.rs", "pub fn f() {}\n");
    env.write("docs/notes.md", "# Notes\n\nThe crate is `src/lib.rs`.\n");
    let draft = env.draft("# Design\n\nBypass me.\n");

    ok(&env.review_stage(draft, "5", &[]), "review to fill the cache");
    ok(&env.review_stage(draft, "5", &[]), "cached review");
    assert_eq!(env.calls().len(), 1, "the cache holds the reply");
    let baseline = env.spend_cache_lines();

    ok(&env.review_stage(draft, "5", &[("HEXA_NO_CACHE", "1")]), "review with HEXA_NO_CACHE=1");
    assert_eq!(env.calls().len(), 2, "HEXA_NO_CACHE=1 calls the backend");
    assert_eq!(env.spend_cache_lines(), baseline, "no cache entry under HEXA_NO_CACHE=1");

    let out = env.hexa(&["design", "review", draft, "--stage", "5", "--model", "claude-text", "--fresh"], &[]);
    ok(&out, "hexa design review --fresh");
    assert_eq!(env.calls().len(), 3, "--fresh on hexa design calls the backend");
    assert_eq!(env.spend_cache_lines(), baseline, "no cache entry under hexa design --fresh");

    ok(&env.hexa(&["drift", "--model", "claude-text"], &[]), "drift to fill the cache");
    let after_drift = env.calls().len();
    assert!(after_drift > 3, "drift called the backend");
    ok(&env.hexa(&["drift", "--model", "claude-text"], &[]), "cached drift");
    assert_eq!(env.calls().len(), after_drift, "drift is answered from the cache");
    let drift_baseline = env.spend_cache_lines();
    ok(&env.hexa(&["drift", "--model", "claude-text", "--fresh"], &[]), "hexa drift --fresh");
    assert!(env.calls().len() > after_drift, "--fresh on hexa drift calls the backend");
    assert_eq!(env.spend_cache_lines(), drift_baseline, "no cache entry under hexa drift --fresh");
}

/// S10: Different inputs never share a key, including when text moves across the field boundaries.
#[test]
fn s10_distinct_inputs_distinct_keys() {
    let keys = [
        completion_key("m1", "s", "u"),
        completion_key("m2", "s", "u"),
        completion_key("m1", "s2", "u"),
        completion_key("m1", "s", "u2"),
    ];
    for i in 0..keys.len() {
        for j in i + 1..keys.len() {
            assert_ne!(keys[i], keys[j], "keys {i} and {j} collide");
        }
    }
    assert_ne!(completion_key("ab", "c", "d"), completion_key("a", "bc", "d"));
}

/// S11: The key is a function of exactly the three inputs.
#[test]
fn s11_property_key_function_of_three_inputs() {
    let parts = ["", "a", "ab", "b", "\0", "a\0b", "ü", "\n", "sys", "x y"];
    let mut triples = Vec::new();
    for m in parts {
        for s in parts {
            for u in ["", "a", "b", "ab"] {
                triples.push((m, s, u));
            }
        }
    }
    let keys: Vec<String> = triples.iter().map(|(m, s, u)| completion_key(m, s, u)).collect();
    for k in &keys {
        assert!(is_hex64(k), "64 hex characters: {k}");
    }
    for i in 0..triples.len() {
        assert_eq!(keys[i], completion_key(triples[i].0, triples[i].1, triples[i].2));
        for j in i + 1..triples.len() {
            let same = triples[i] == triples[j];
            assert_eq!(keys[i] == keys[j], same, "{:?} vs {:?}", triples[i], triples[j]);
        }
    }
}

// ── §3 ───────────────────────────────────────────────────

/// S12: goldfish_prompt places framing, then the document, then the sources, then the standard, then the persona.
#[test]
fn s12_goldfish_prompt_order() {
    let sources = vec![("src/a.rs".to_string(), "SOURCE-A".to_string())];
    let out = goldfish_prompt("PERSONA-P", Some("STANDARD-S"), "DOCUMENT-D", &sources, 1000);
    for m in ["PERSONA-P", "STANDARD-S", "DOCUMENT-D", "SOURCE-A"] {
        assert_eq!(out.matches(m).count(), 1, "{m} occurs exactly once");
    }
    let d = out.find("DOCUMENT-D").unwrap();
    let s = out.find("SOURCE-A").unwrap();
    let st = out.find("STANDARD-S").unwrap();
    let p = out.find("PERSONA-P").unwrap();
    assert!(d < s && s < st && st < p, "order: document < source < standard < persona");
    let framing = &out[..d];
    assert!(framing.contains("hexa"), "hexa's framing comes first: {framing}");
    assert!(!out.starts_with("DOCUMENT-D"));
}

/// S13: Reordering does not change the content: nothing is added or dropped, and the cut is still stated.
#[test]
fn s13_reordering_keeps_content_and_cut() {
    let source = format!("SRC-MARK{}", "Q".repeat(42));
    assert_eq!(source.len(), 50);
    let sources = vec![("src/a.rs".to_string(), source)];
    let out = goldfish_prompt("P", None, "D-MARK", &sources, 10);
    let src = out.find("SRC-MARK").expect("the cut source is present");
    assert!(out.contains("D-MARK"));
    assert!(out.trim_end().ends_with('P'), "the persona comes last, after SRC-MARK");
    assert!(out.rfind('P').unwrap() > src);
    assert!(!out.contains("# Standard"), "no standard section");
    assert!(out.contains("SRC-MARKQQ"), "the first 10 bytes are kept");
    assert!(!out.contains("SRC-MARKQQQ"), "the text past the cap does not appear");
    assert!(out.contains("50"), "the cut states the original 50 bytes");
}

/// S14: The persona and the standard never come before the document or the sources.
#[test]
fn s14_persona_standard_after_document_sources() {
    let sources = vec![("src/q.rs".to_string(), "SRC-Q".to_string())];
    let out = goldfish_prompt("PERSONA-Q", Some("STD-Q"), "DOC-Q", &sources, 1000);
    let doc = out.find("DOC-Q").unwrap();
    let src = out.find("SRC-Q").unwrap();
    for m in ["PERSONA-Q", "STD-Q"] {
        let pos = out.find(m).unwrap();
        assert!(pos > doc, "{m} before the document");
        assert!(pos > src, "{m} before the sources");
    }
}

/// S15: Reviewers of the same review share a prefix that covers the document and every source.
#[test]
fn s15_property_shared_prefix_covers_document_and_sources() {
    let docs = ["d", "# Draft\n\nShort doc text DOCX.", "Line one\nLine two\nLine three with words."];
    let source_sets: Vec<Vec<(String, String)>> = vec![
        vec![],
        vec![("src/a.rs".into(), "fn a() { 1 }".into())],
        vec![
            ("src/a.rs".into(), "fn alpha() -> u32 { 42 } // alpha body text".into()),
            ("src/b.rs".into(), "pub struct Beta { field: String }".into()),
        ],
    ];
    let pairs: [(&str, Option<&str>); 3] = [("PER-A", Some("STD-A")), ("PER-B", None), ("PER-C", Some("STD-C"))];
    for d in docs {
        for sources in &source_sets {
            for cap in [5usize, 20, 1000] {
                for i in 0..pairs.len() {
                    for j in i + 1..pairs.len() {
                        let a = goldfish_prompt(pairs[i].0, pairs[i].1, d, sources, cap);
                        let b = goldfish_prompt(pairs[j].0, pairs[j].1, d, sources, cap);
                        let lcp = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
                        let prefix = &a[..lcp];
                        assert!(prefix.contains(d), "shared prefix lacks the document (cap {cap})");
                        for (_, contents) in sources {
                            let cut = &contents[..contents.len().min(cap)];
                            assert!(prefix.contains(cut), "shared prefix lacks source {cut:?} (cap {cap})");
                        }
                    }
                }
            }
        }
    }
}

// ── §4 ───────────────────────────────────────────────────

/// S16: Only multi-reviewer stages need synthesis.
#[test]
fn s16_only_multi_reviewer_stages_need_synthesis() {
    assert!(!needs_synthesis(Stage::ZoomOut));
    assert!(!needs_synthesis(Stage::Comprehension));
    assert!(!needs_synthesis(Stage::IntermediateZoomOut));
    assert!(needs_synthesis(Stage::Critic));
    assert!(needs_synthesis(Stage::ImplementationReadiness));
}

/// S17: A single reviewer's list items become the stage's findings, numbered as before.
#[test]
fn s17_single_reviewer_items_are_findings() {
    let env = Env::new("s17");
    env.reply_default(&envelope("- first\n- second", 30, 5, 0, "0.001"));
    let draft = env.draft("# Design\n\nComprehend me.\n");
    ok(&env.review_stage(draft, "6", &[("HEXA_NO_CACHE", "1")]), "stage 6 review");
    assert_eq!(env.calls().len(), 1, "no synthesizer completion for stage 6");
    let report = env.report();
    let line = |id: &str| report.lines().find(|l| l.contains(id)).map(str::to_string);
    let one = line("S6-1").expect("finding S6-1");
    let two = line("S6-2").expect("finding S6-2");
    assert!(one.contains("first"), "S6-1 is \"first\": {one}");
    assert!(two.contains("second"), "S6-2 is \"second\": {two}");
    assert!(!report.contains("S6-3"));
}

/// S18: A single-reviewer stage never calls the synthesizer, and "No findings." yields none.
#[test]
fn s18_full_review_eleven_completions() {
    let env = Env::new("s18");
    env.reply_default(&envelope("No findings.", 30, 3, 0, "0.001"));
    let draft = env.draft("# Design\n\nReview me fully.\n");
    let out = env.hexa(&["design", "review", draft, "--full", "--model", "claude-text"], &[("HEXA_NO_CACHE", "1")]);
    ok(&out, "full review");
    assert_eq!(env.calls().len(), 11, "9 reviewers and 2 synthesizers");
    let report = env.report();
    assert!(!report.contains("S8-1"), "stage 8 has zero findings");
}

/// S19: needs_synthesis agrees with the reviewer count.
#[test]
fn s19_property_needs_synthesis_matches_reviewer_count() {
    for s in [
        Stage::ZoomOut,
        Stage::Comprehension,
        Stage::Critic,
        Stage::IntermediateZoomOut,
        Stage::ImplementationReadiness,
    ] {
        assert_eq!(needs_synthesis(s), stage_reviewers(s).len() > 1, "{s:?}");
    }
}

// ── §5 ───────────────────────────────────────────────────

/// S20: A compile failure is repaired with one follow-up that carries the writer's reply and the compiler's messages.
#[test]
fn s20_compile_failure_repaired_by_one_follow_up() {
    let env = spec_env("s20");
    env.reply_tests(1, &writer_reply(&writer_file(&ALL, None, E0502_HELPER, "FIRST-REPLY-MARK")));
    env.reply_tests(2, &writer_reply(&writer_file(&ALL, None, "", "REPAIR-MARK")));
    let out = run_spec_tests(&env);
    let calls = env.writer_calls();
    assert_eq!(calls.len(), 2, "one generation and exactly one follow-up");
    assert!(calls[1].contains("FIRST-REPLY-MARK"), "the follow-up carries the writer's reply");
    assert!(calls[1].contains("E0502"), "the follow-up carries the compiler's E0502 message");
    ok(&out, "hexa spec tests");
    assert!(accepted_test_file(&env).is_some_and(|f| f.contains("REPAIR-MARK")), "the repaired file is accepted");
}

/// S21: A missing scenario or an unpassable site is repaired with its own error text.
#[test]
fn s21_missing_scenario_or_unpassable_site_repaired() {
    let env = spec_env("s21a");
    env.reply_tests(1, &writer_reply(&writer_file(&["S01", "S02", "S03"], None, "", "FIRST-REPLY-MARK")));
    env.reply_tests(2, &writer_reply(&writer_file(&ALL, None, "", "REPAIR-MARK")));
    let _ = run_spec_tests(&env);
    let calls = env.writer_calls();
    assert!(calls.len() >= 2, "a follow-up is sent for the missing scenario");
    assert!(calls[1].contains("FIRST-REPLY-MARK"), "the follow-up carries the writer's reply");
    assert!(calls[1].contains("S04"), "the follow-up names the missing scenario S04");

    let env = spec_env("s21b");
    env.reply_tests(1, &writer_reply(&writer_file(&ALL, Some("S01"), "", "FIRST-REPLY-MARK")));
    env.reply_tests(2, &writer_reply(&writer_file(&ALL, None, "", "REPAIR-MARK")));
    let _ = run_spec_tests(&env);
    let calls = env.writer_calls();
    assert!(calls.len() >= 2, "a follow-up is sent for the unpassable site");
    assert!(calls[1].contains("FIRST-REPLY-MARK"), "the follow-up carries the writer's reply");
    assert!(
        calls[1].matches("assert!(false").count() >= 2,
        "the follow-up identifies the unpassable site beyond quoting the reply"
    );
}

/// S22: There are at most two repairs, and every repaired file passes every check again from the start.
#[test]
fn s22_at_most_two_repairs_each_rechecked() {
    let env = spec_env("s22");
    env.reply_tests(1, &writer_reply(&writer_file(&ALL, None, E0502_HELPER, "FIRST-REPLY-MARK")));
    env.reply_tests(2, &writer_reply(&writer_file(&["S01", "S02", "S03"], None, "", "SECOND-REPLY-MARK")));
    env.reply_tests(3, &writer_reply(&writer_file(&ALL, None, E0502_HELPER, "THIRD-REPLY-MARK")));
    let out = run_spec_tests(&env);
    let calls = env.writer_calls();
    assert_eq!(calls.len(), 3, "a generation and exactly two follow-ups");
    assert!(calls[2].contains("SECOND-REPLY-MARK"), "the second follow-up carries the first repair");
    assert!(calls[2].contains("S04"), "the first repair is rejected for the missing scenario");
    assert!(!calls[2].contains("E0502"), "the first repair compiled; its rejection is not a compile error");
    assert!(!out.status.success(), "the generation is rejected");
}

/// S23: The follow-up never carries source from outside the test file.
#[test]
fn s23_follow_up_carries_no_outside_source() {
    let env = spec_env("s23");
    let extra = format!("{E0502_HELPER}\n{SECRET_HELPER}");
    env.reply_tests(1, &writer_reply(&writer_file(&ALL, None, &extra, "FIRST-REPLY-MARK")));
    env.reply_tests(2, &writer_reply(&writer_file(&ALL, None, "", "REPAIR-MARK")));
    let _ = run_spec_tests(&env);
    let calls = env.writer_calls();
    assert!(calls.len() >= 2, "a follow-up is sent");
    assert!(!calls[1].contains("IMPL-SECRET"), "the follow-up leaks src/secret.rs");
    assert!(!calls[1].contains("pub fn secret(x: u32)"), "the follow-up leaks src/secret.rs");
    assert!(calls[1].contains("E0502"), "the follow-up carries the test-file error messages");
}

/// S24: Over any sequence of rejections, follow-ups are bounded and confined to the test file.
#[test]
fn s24_property_follow_ups_bounded_and_confined() {
    let secret_extra = format!("{E0502_HELPER}\n{SECRET_HELPER}");
    let sequences: Vec<(Vec<String>, bool)> = vec![
        (
            vec![
                writer_file(&["S01", "S02", "S03"], None, "", "R1"),
                writer_file(&ALL, None, &secret_extra, "R2"),
                writer_file(&ALL, Some("S02"), "", "R3"),
                writer_file(&ALL, None, "", "R4"),
            ],
            false,
        ),
        (
            vec![writer_file(&ALL, None, &secret_extra, "R1"), writer_file(&ALL, None, "", "R2")],
            true,
        ),
    ];
    for (n, (replies, should_accept)) in sequences.into_iter().enumerate() {
        let env = spec_env(&format!("s24-{n}"));
        for (k, r) in replies.iter().enumerate() {
            env.reply_tests(k + 1, &writer_reply(r));
        }
        let out = run_spec_tests(&env);
        let calls = env.writer_calls();
        assert!(calls.len() <= 3, "at most two follow-ups (sequence {n})");
        for c in calls.iter().skip(1) {
            assert!(!c.contains("IMPL-SECRET"), "a follow-up leaks project source (sequence {n})");
            assert!(!c.contains("pub fn secret(x: u32)"), "a follow-up leaks project source (sequence {n})");
        }
        if should_accept {
            ok(&out, "hexa spec tests");
            let file = accepted_test_file(&env).expect("an accepted test file");
            for id in ALL {
                assert!(file.contains(&format!("/// {id}:")), "accepted file names {id}");
            }
            assert!(!file.contains("assert!(false"), "accepted file holds no unpassable site");
        } else {
            assert!(!out.status.success(), "sequence {n} ends in rejection");
            assert!(
                accepted_test_file(&env).map_or(true, |f| !f.contains("R4")),
                "a file beyond the second repair is never accepted"
            );
        }
    }
}

#[allow(dead_code)]
fn _paths_used(_: &Path) {}
