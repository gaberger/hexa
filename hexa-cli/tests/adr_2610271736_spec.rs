//! Integration tests for ADR-2610271736: Elephant-Goldfish design review and drift.
//!
//! Pure scenarios call `hexa_core::domain::design` directly. CLI scenarios run the
//! `hexa` binary in a throwaway project. The fake completion route is set up in one
//! place, `Project`. Every completion is assumed to go to the executable named by
//! `HEXA_COMPLETION_COMMAND`. That executable gets the prompt on stdin and the model
//! in `HEXA_COMPLETION_MODEL` (or its arguments), and prints its reply on stdout.
//! If the harness hook differs, adapt only `Project`.

use hexa_core::domain::design::{self, Pin, Preset, Stage};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

const COMMIT: &str = "a8380ab5f0aba392c97e31c79f9a6666b3f09f14";
const REPO: &str = "https://github.com/google/elephant-goldfish-model";
const FAKE_ROUTE_ENV: &str = "HEXA_COMPLETION_COMMAND";
const ALL_STAGES: [Stage; 5] = [
    Stage::ZoomOut,
    Stage::Comprehension,
    Stage::Critic,
    Stage::IntermediateZoomOut,
    Stage::ImplementationReadiness,
];

// ---------------------------------------------------------------- SHA-256

fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    let bitlen = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bitlen.to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }
    h.iter().map(|x| format!("{:08x}", x)).collect()
}

fn sha(b: &[u8]) -> String {
    sha256_hex(b)
}

// ---------------------------------------------------------------- tiny JSON

#[derive(Debug, Clone)]
enum Json {
    Null,
    Bool(bool),
    Num(String),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(kv) => kv.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
}

struct JsonParser<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> JsonParser<'a> {
    fn ws(&mut self) {
        while self.i < self.s.len() && (self.s[self.i] as char).is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn value(&mut self) -> Json {
        self.ws();
        match self.s[self.i] {
            b'{' => {
                self.i += 1;
                let mut kv = Vec::new();
                loop {
                    self.ws();
                    if self.s[self.i] == b'}' {
                        self.i += 1;
                        break;
                    }
                    let k = self.string();
                    self.ws();
                    assert_eq!(self.s[self.i], b':');
                    self.i += 1;
                    let v = self.value();
                    kv.push((k, v));
                    self.ws();
                    if self.s[self.i] == b',' {
                        self.i += 1;
                    }
                }
                Json::Obj(kv)
            }
            b'[' => {
                self.i += 1;
                let mut v = Vec::new();
                loop {
                    self.ws();
                    if self.s[self.i] == b']' {
                        self.i += 1;
                        break;
                    }
                    v.push(self.value());
                    self.ws();
                    if self.s[self.i] == b',' {
                        self.i += 1;
                    }
                }
                Json::Arr(v)
            }
            b'"' => Json::Str(self.string()),
            b't' => {
                self.i += 4;
                Json::Bool(true)
            }
            b'f' => {
                self.i += 5;
                Json::Bool(false)
            }
            b'n' => {
                self.i += 4;
                Json::Null
            }
            _ => {
                let st = self.i;
                while self.i < self.s.len() && b"+-0123456789.eE".contains(&self.s[self.i]) {
                    self.i += 1;
                }
                Json::Num(String::from_utf8_lossy(&self.s[st..self.i]).into_owned())
            }
        }
    }
    fn string(&mut self) -> String {
        assert_eq!(self.s[self.i], b'"');
        self.i += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let c = self.s[self.i];
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let e = self.s[self.i];
                    self.i += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'u' => {
                            let hex = std::str::from_utf8(&self.s[self.i..self.i + 4]).unwrap();
                            self.i += 4;
                            let cp = u32::from_str_radix(hex, 16).unwrap();
                            let ch = char::from_u32(cp).unwrap_or('?');
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                        other => out.push(other),
                    }
                }
                other => out.push(other),
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }
}

fn parse_json(text: &str) -> Json {
    JsonParser { s: text.as_bytes(), i: 0 }.value()
}

fn pin_from_upstream(json: &Json) -> Pin {
    let repository = ["repository", "repo", "url", "upstream"]
        .iter()
        .find_map(|k| json.get(k).and_then(|v| v.as_str()))
        .unwrap_or_default()
        .to_string();
    let commit = json.get("commit").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let hash_of = |v: &Json| -> String {
        if let Some(s) = v.as_str() {
            return s.to_string();
        }
        ["sha256", "hash", "digest"]
            .iter()
            .find_map(|k| v.get(k).and_then(|x| x.as_str()))
            .unwrap_or_default()
            .to_string()
    };
    let mut files = Vec::new();
    match json.get("files") {
        Some(Json::Obj(kv)) => {
            for (k, v) in kv {
                files.push((k.clone(), hash_of(v)));
            }
        }
        Some(Json::Arr(items)) => {
            for it in items {
                let path = it.get("path").and_then(|p| p.as_str()).unwrap_or_default().to_string();
                files.push((path, hash_of(it)));
            }
        }
        _ => panic!("UPSTREAM.json has no `files` entry"),
    }
    Pin { repository, commit, files }
}

// ---------------------------------------------------------------- RNG

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn coin(&mut self) -> bool {
        self.next() & 1 == 1
    }
    fn ascii(&mut self, max_len: usize) -> String {
        const A: &[u8] = b"abcdefghijklmnopqrstuvwxyz ABCXYZ0123456789\n.,;:-_/";
        let n = self.below(max_len + 1);
        (0..n).map(|_| A[self.below(A.len())] as char).collect()
    }
}

// ---------------------------------------------------------------- assets

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn repo_root() -> PathBuf {
    manifest_dir().parent().unwrap().to_path_buf()
}

fn egm_dir() -> PathBuf {
    manifest_dir().join("assets").join("egm")
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>, skip_git: bool) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if skip_git && p.file_name().map(|n| n == ".git").unwrap_or(false) {
            continue;
        }
        if p.is_dir() {
            walk(root, &p, out, skip_git);
        } else {
            let rel = p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            out.push(rel);
        }
    }
}

fn agent_stems() -> Vec<String> {
    let mut v = Vec::new();
    for e in std::fs::read_dir(egm_dir().join("agents")).expect("vendored agents/ directory").flatten() {
        let p = e.path();
        if p.extension().map(|x| x == "md").unwrap_or(false) {
            v.push(p.file_stem().unwrap().to_string_lossy().into_owned());
        }
    }
    v.sort();
    v
}

/// A distinctive line of a persona file: its longest non-frontmatter line.
fn persona_fingerprint(stem: &str) -> String {
    let text = std::fs::read_to_string(egm_dir().join("agents").join(format!("{stem}.md")))
        .unwrap_or_else(|_| panic!("persona agents/{stem}.md"));
    text.lines()
        .map(str::trim)
        .filter(|l| !l.starts_with("---"))
        .max_by_key(|l| l.len())
        .unwrap_or_default()
        .to_string()
}

/// The single persona a prompt carries, if exactly one agent persona is in it.
fn sole_persona(prompt: &str) -> Option<String> {
    let found: Vec<String> = agent_stems()
        .into_iter()
        .filter(|s| {
            let f = persona_fingerprint(s);
            !f.is_empty() && prompt.contains(&f)
        })
        .collect();
    if found.len() == 1 { found.into_iter().next() } else { None }
}

fn stages_of_stem(stem: &str) -> Vec<Stage> {
    ALL_STAGES.iter().copied().filter(|s| design::stage_reviewers(*s).contains(&stem)).collect()
}

/// Stems that a stage runs and no other stage runs.
fn exclusive_stems(stage: Stage) -> Vec<String> {
    design::stage_reviewers(stage)
        .into_iter()
        .filter(|s| stages_of_stem(s) == vec![stage])
        .map(String::from)
        .collect()
}

// ---------------------------------------------------------------- project harness

static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct Call {
    prompt: String,
    meta: String,
    start: u128,
    end: u128,
}

struct Project {
    base: PathBuf,
    root: PathBuf,
    log: PathBuf,
    script: PathBuf,
}

impl Project {
    fn new() -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!("hexa-egm-it-{}-{n}-{nanos}", std::process::id()));
        let root = base.join("project");
        let log = base.join("log");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&log).unwrap();
        let p = Project { script: base.join("fake-route.sh"), base, root, log };
        p.write("CLAUDE.md", "# Project\n\nA test project.\n");
        p.write("src/lib.rs", "pub fn original() {}\n");
        p.set_reply("echo '- Canned finding: the design is unclear about errors.'");
        p
    }

    fn write(&self, rel: &str, contents: &str) {
        let p = self.root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, contents).unwrap();
    }

    fn write_base(&self, rel: &str, contents: &str) -> PathBuf {
        let p = self.base.join(rel);
        std::fs::write(&p, contents).unwrap();
        p
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.root.join(rel)).unwrap_or_else(|_| panic!("expected file {rel}"))
    }

    /// `body` is shell that prints the reply; `$PROMPT` is the prompt file.
    fn set_reply(&self, body: &str) {
        let script = format!(
            "#!/bin/sh\nLOG='{log}'\nid=\"$(date +%s%N)-$$\"\nstart=$(date +%s%N)\nPROMPT=\"$LOG/$id.prompt\"\ncat > \"$PROMPT\"\n{{ printf '%s\\n' \"$@\"; printf 'model=%s\\n' \"${{HEXA_COMPLETION_MODEL:-}}\"; }} > \"$LOG/$id.meta\"\n{body}\necho \"$start $(date +%s%N)\" > \"$LOG/$id.time\"\n",
            log = self.log.display()
        );
        std::fs::write(&self.script, script).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&self.script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn hexa(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_hexa"))
            .args(args)
            .current_dir(&self.root)
            .env(FAKE_ROUTE_ENV, &self.script)
            .env("HEXA_TODAY", "2026-10-27")
            .output()
            .expect("run hexa")
    }

    fn hexa_ok(&self, args: &[&str]) -> Output {
        let out = self.hexa(args);
        assert!(
            out.status.success(),
            "hexa {:?} failed: {}\nstdout: {}",
            args,
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
        out
    }

    fn calls(&self) -> Vec<Call> {
        let mut ids: Vec<String> = std::fs::read_dir(&self.log)
            .unwrap()
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                n.strip_suffix(".prompt").map(String::from)
            })
            .collect();
        ids.sort();
        let mut calls: Vec<Call> = ids
            .into_iter()
            .map(|id| {
                let r = |ext: &str| std::fs::read_to_string(self.log.join(format!("{id}.{ext}"))).unwrap_or_default();
                let time = r("time");
                let mut t = time.split_whitespace().map(|x| x.parse::<u128>().unwrap_or(0));
                let start = t.next().unwrap_or(0);
                let end = t.next().unwrap_or(u128::MAX);
                Call { prompt: r("prompt"), meta: r("meta"), start, end }
            })
            .collect();
        calls.sort_by_key(|c| c.start);
        calls
    }

    fn clear_log(&self) {
        for e in std::fs::read_dir(&self.log).unwrap().flatten() {
            let _ = std::fs::remove_file(e.path());
        }
    }

    fn snapshot(&self) -> BTreeMap<String, Vec<u8>> {
        let mut files = Vec::new();
        walk(&self.root, &self.root, &mut files, true);
        files
            .into_iter()
            .map(|f| {
                let b = std::fs::read(self.root.join(&f)).unwrap();
                (f, b)
            })
            .collect()
    }

    fn git(&self, args: &[&str]) -> String {
        let out = Command::new("git").args(args).current_dir(&self.root).output().expect("git");
        assert!(out.status.success(), "git {:?}: {}", args, String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).into_owned()
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

fn changed(before: &BTreeMap<String, Vec<u8>>, after: &BTreeMap<String, Vec<u8>>) -> BTreeSet<String> {
    let mut s = BTreeSet::new();
    for (k, v) in after {
        if before.get(k) != Some(v) {
            s.insert(k.clone());
        }
    }
    for k in before.keys() {
        if !after.contains_key(k) {
            s.insert(k.clone());
        }
    }
    s
}

/// Text of every Markdown file created or modified, other than `exclude`.
fn new_reports(before: &BTreeMap<String, Vec<u8>>, after: &BTreeMap<String, Vec<u8>>, exclude: &str) -> String {
    let mut text = String::new();
    for f in changed(before, after) {
        if f != exclude && f.ends_with(".md") {
            if let Some(b) = after.get(&f) {
                text.push_str(&String::from_utf8_lossy(b));
                text.push('\n');
            }
        }
    }
    assert!(!text.is_empty(), "no report was written");
    text
}

fn stage_heading_pos(report: &str, n: u8) -> Option<usize> {
    let mut off = 0;
    for line in report.split_inclusive('\n') {
        let l = line.to_lowercase();
        if l.trim_start().starts_with('#') && l.contains(&format!("stage {n}")) {
            return Some(off);
        }
        off += line.len();
    }
    None
}

const DRAFT_REL: &str = "docs/designs/design_draft.md";

fn with_draft(p: &Project) -> String {
    let draft = "# Design: widget cache\n\nThe cache lives in `src/lib.rs`.\nDRAFT-MARKER-QQQ\n\n## Behaviour\nIt evicts the oldest entry.\n";
    p.write(DRAFT_REL, draft);
    draft.to_string()
}

fn handwritten_report(p: &Project, rel: &str, draft_rel: &str) {
    p.write(
        rel,
        &format!(
            "# Design review\n\nDraft: `{draft_rel}`\nPersonas: critic\nEGM commit: {COMMIT}\n\n## Stage 6\n\n- S6-1: AAA the error path is undefined.\n- S6-2: BBB the name is vague.\n\n## Stage 7\n\n- S7-1: CCC the cache has two owners.\n"
        ),
    );
}

fn drift_report(p: &Project) -> String {
    let dir = p.root.join("docs/analysis");
    let mut found = Vec::new();
    for e in std::fs::read_dir(&dir).expect("docs/analysis exists").flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if n.ends_with("_DRIFT_REPORT.md") {
            found.push(e.path());
        }
    }
    assert_eq!(found.len(), 1, "exactly one drift report expected, got {:?}", found);
    std::fs::read_to_string(&found[0]).unwrap()
}

fn drift_project() -> Project {
    let p = Project::new();
    p.write("README.md", "# Widget\n\nREADME-CONTENT-MARK says widgets are blue.\n");
    p.write("ARCHITECTURE.md", "# Architecture\n\nARCH-CONTENT-MARK one crate.\n");
    p.write("docs/guide.md", "# Guide\n\nGUIDE-CONTENT-MARK run it.\n");
    p
}

const DRIFT_README_ONLY: &str =
    "if grep -q README-CONTENT-MARK \"$PROMPT\"; then echo '- DIVERGENCE-README: README says blue, code says red.'; else echo 'No divergences.'; fi";

fn assert_writes_allowed(changes: &BTreeSet<String>, artifact_dir: &str) {
    let prefix = format!("{}/", artifact_dir.trim_end_matches('/'));
    for f in changes {
        assert!(
            f.starts_with(&prefix) || f.starts_with("docs/analysis/"),
            "{f} was written outside {prefix} and docs/analysis/"
        );
    }
}

// ================================================================ §1

fn two_file_pin() -> (Pin, Vec<u8>, Vec<u8>) {
    let critic = b"critic persona".to_vec();
    let license = b"Apache License 2.0".to_vec();
    let pin = Pin {
        repository: REPO.into(),
        commit: COMMIT.into(),
        files: vec![
            ("agents/critic.md".into(), sha(&critic)),
            ("LICENSE".into(), sha(&license)),
        ],
    };
    (pin, critic, license)
}

/// S01: A vendored copy that matches its pin verifies
#[test]
fn s01_matching_vendored_copy_verifies() {
    let (pin, critic, license) = two_file_pin();
    let present = vec!["agents/critic.md".to_string(), "LICENSE".to_string()];
    let read = |p: &str| match p {
        "agents/critic.md" => Some(critic.clone()),
        "LICENSE" => Some(license.clone()),
        _ => None,
    };
    assert_eq!(design::verify_pin(&pin, &present, &read, &sha), Ok(()));
}

/// S02: A file whose hash differs from its pin is refused and named
#[test]
fn s02_hash_mismatch_is_named() {
    let (pin, _critic, license) = two_file_pin();
    let present = vec!["agents/critic.md".to_string(), "LICENSE".to_string()];
    let read = |p: &str| match p {
        "agents/critic.md" => Some(b"tampered persona".to_vec()),
        "LICENSE" => Some(license.clone()),
        _ => None,
    };
    let err = design::verify_pin(&pin, &present, &read, &sha).expect_err("mismatch must be refused");
    assert!(err.iter().any(|e| e.contains("agents/critic.md")), "{err:?}");
    assert!(!err.iter().any(|e| e.contains("LICENSE")), "{err:?}");
}

/// S03: A recorded file that is missing is refused and named
#[test]
fn s03_missing_file_is_named() {
    let (pin, _critic, license) = two_file_pin();
    let present = vec!["LICENSE".to_string()];
    let read = |p: &str| match p {
        "LICENSE" => Some(license.clone()),
        _ => None,
    };
    let err = design::verify_pin(&pin, &present, &read, &sha).expect_err("missing file must be refused");
    assert!(err.iter().any(|e| e.contains("agents/critic.md")), "{err:?}");
}

/// S04: A file present but not recorded in the pin is refused and named
#[test]
fn s04_unrecorded_file_is_named() {
    let license = b"Apache License 2.0".to_vec();
    let pin = Pin {
        repository: REPO.into(),
        commit: COMMIT.into(),
        files: vec![("LICENSE".into(), sha(&license))],
    };
    let present = vec!["LICENSE".to_string(), "agents/extra.md".to_string()];
    let read = |p: &str| match p {
        "LICENSE" => Some(license.clone()),
        "agents/extra.md" => Some(b"extra".to_vec()),
        _ => None,
    };
    let err = design::verify_pin(&pin, &present, &read, &sha).expect_err("unrecorded file must be refused");
    assert!(err.iter().any(|e| e.contains("agents/extra.md")), "{err:?}");
}

/// S05: verify_pin names exactly the set of mismatched, missing and unrecorded files
#[test]
fn s05_verify_pin_names_exactly_the_bad_files() {
    let pool = ["LICENSE", "agents/critic.md", "agents/goldfish.md", "resources/x.md", "skills/a/SKILL.md", "README.md"];
    let content = |p: &str| format!("content of {p}").into_bytes();
    let mut rng = Rng(0x9e3779b97f4a7c15);
    for _ in 0..300 {
        let mut files = Vec::new();
        let mut expected = BTreeSet::new();
        let mut present = Vec::new();
        for p in pool {
            let recorded = rng.coin();
            let is_present = rng.coin();
            if is_present {
                present.push(p.to_string());
            }
            if recorded {
                let good = rng.below(3) != 0;
                let h = if good { sha(&content(p)) } else { sha(b"something else") };
                files.push((p.to_string(), h));
                if !is_present || !good {
                    expected.insert(p.to_string());
                }
            } else if is_present {
                expected.insert(p.to_string());
            }
        }
        let pin = Pin { repository: REPO.into(), commit: COMMIT.into(), files };
        let present_set: BTreeSet<String> = present.iter().cloned().collect();
        let read = |p: &str| if present_set.contains(p) { Some(content(p)) } else { None };
        let got = design::verify_pin(&pin, &present, &read, &sha);
        if expected.is_empty() {
            assert_eq!(got, Ok(()), "pin {:?} present {:?}", pin, present);
        } else {
            let err = got.expect_err("expected refusal");
            let named: BTreeSet<String> = pool
                .iter()
                .filter(|p| err.iter().any(|e| e == *p || e.contains(*p)))
                .map(|p| p.to_string())
                .collect();
            assert_eq!(named, expected, "pin {:?} present {:?} err {:?}", pin, present, err);
        }
    }
}

/// S06: The checked-in vendored copy matches UPSTREAM.json
#[test]
fn s06_checked_in_vendored_copy_matches_upstream() {
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    let dir = egm_dir();
    let json = parse_json(&std::fs::read_to_string(dir.join("UPSTREAM.json")).expect("UPSTREAM.json"));
    let pin = pin_from_upstream(&json);
    let mut present = Vec::new();
    walk(&dir, &dir, &mut present, false);
    present.retain(|p| p != "UPSTREAM.json");
    let read = |p: &str| std::fs::read(dir.join(p)).ok();
    assert_eq!(design::verify_pin(&pin, &present, &read, &sha), Ok(()));
    assert_eq!(pin.commit, COMMIT);
    let rec: Vec<&str> = pin.files.iter().map(|(p, _)| p.as_str()).collect();
    assert!(rec.contains(&"LICENSE"));
    assert!(rec.contains(&"resources/execution_instructions.md"));
    assert!(rec.iter().any(|p| p.starts_with("agents/") && p.ends_with(".md")));
    assert!(rec.iter().any(|p| p.starts_with("resources/document_standards/") && p.ends_with(".md")));
    for skill in ["egm-brainstorm", "egm-draft", "egm-review", "egm-resolve", "egm-drift-report"] {
        assert!(rec.iter().any(|p| p.contains(skill)), "skill {skill} not recorded");
    }
}

// ================================================================ §2

/// S07: Attribution document and help text name EGM
#[test]
fn s07_attribution_and_help_name_egm() {
    let attributions = std::fs::read_to_string(repo_root().join("docs/ATTRIBUTIONS.md")).expect("docs/ATTRIBUTIONS.md");
    for needle in ["Elephant-Goldfish Model", "Google LLC", "Apache-2.0", REPO, COMMIT, "Rensin"] {
        assert!(attributions.contains(needle), "ATTRIBUTIONS.md lacks {needle}");
    }
    let readme = std::fs::read_to_string(repo_root().join("README.md")).expect("README.md");
    assert!(readme.contains("docs/ATTRIBUTIONS.md") || readme.contains(REPO));
    for cmd in [["design", "--help"], ["drift", "--help"]] {
        let out = Command::new(env!("CARGO_BIN_EXE_hexa")).args(cmd).output().unwrap();
        assert!(out.status.success(), "hexa {:?} failed", cmd);
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("EGM") || text.contains("Elephant-Goldfish"), "hexa {:?}: {text}", cmd);
    }
}

/// S08: A report from an EGM prompt names its persona and pinned commit
#[test]
fn s08_review_report_names_persona_and_commit() {
    let p = Project::new();
    with_draft(&p);
    let before = p.snapshot();
    p.hexa_ok(&["design", "review", DRAFT_REL, "--stage", "6"]);
    let report = new_reports(&before, &p.snapshot(), DRAFT_REL);
    assert!(report.contains(COMMIT), "report lacks pinned commit");
    let personas = design::stage_reviewers(Stage::Comprehension);
    assert!(personas.iter().any(|s| report.contains(s)), "report names none of {personas:?}");
}

/// S09: No EGM-derived report is written without attribution
#[test]
fn s09_drift_report_has_persona_and_commit() {
    let p = drift_project();
    p.hexa_ok(&["drift"]);
    let report = drift_report(&p);
    assert!(report.contains(COMMIT), "drift report lacks pinned commit");
    assert!(agent_stems().iter().any(|s| report.contains(s.as_str())), "drift report names no persona");
}

// ================================================================ §3

/// S10: The goldfish prompt contains the persona, standard, document and sources
#[test]
fn s10_goldfish_prompt_contains_all_inputs() {
    let sources = vec![("src/a.rs".to_string(), "fn a() {}".to_string())];
    let out = design::goldfish_prompt("PERSONA-TEXT", Some("STANDARD-TEXT"), "DOC-TEXT", &sources, 1000);
    for needle in ["PERSONA-TEXT", "STANDARD-TEXT", "DOC-TEXT", "src/a.rs", "fn a() {}"] {
        assert!(out.contains(needle), "prompt lacks {needle}");
    }
}

/// S11: A reviewer's prompt has no conversation, earlier review or other reviewer's findings
#[test]
fn s11_reviewer_prompt_has_no_earlier_or_peer_findings() {
    let p = Project::new();
    with_draft(&p);
    let stage5: Vec<String> = exclusive_stems(Stage::ZoomOut).iter().map(|s| persona_fingerprint(s)).collect();
    let fp = p.write_base("stage5.txt", &(stage5.join("\n") + "\n"));
    p.set_reply(&format!(
        "if grep -qFf '{}' \"$PROMPT\"; then echo '- EARLIER-FINDING-XYZ'; else echo '- OTHER-REVIEWER-XYZ'; fi",
        fp.display()
    ));
    p.hexa_ok(&["design", "review", DRAFT_REL]);
    let later: Vec<Call> = p
        .calls()
        .into_iter()
        .filter(|c| {
            sole_persona(&c.prompt)
                .map(|s| stages_of_stem(&s).iter().any(|st| *st > Stage::ZoomOut))
                .unwrap_or(false)
        })
        .collect();
    assert!(!later.is_empty(), "no later-stage reviewer calls recorded");
    for c in later {
        assert!(!c.prompt.contains("EARLIER-FINDING-XYZ"));
        assert!(!c.prompt.contains("OTHER-REVIEWER-XYZ"));
    }
}

/// S12: An agent route that could read the tree is refused
#[test]
fn s12_agent_route_is_refused() {
    let p = Project::new();
    with_draft(&p);
    p.write(".hexa/config.toml", "[inference.tier_models]\n\"t2.5\" = \"claude-agent:sonnet\"\n");
    let out = p.hexa(&["design", "review", DRAFT_REL]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr).to_lowercase();
    assert!(err.contains("agent") && err.contains("refus"), "stderr: {err}");
    assert!(p.calls().is_empty(), "a completion was made");
    let out2 = p.hexa(&["design", "review", DRAFT_REL, "--model", "claude-agent:sonnet"]);
    assert!(!out2.status.success());
    assert!(p.calls().is_empty(), "a completion was made");
}

/// S13: The goldfish uses --model, else tier t2.5
#[test]
fn s13_goldfish_uses_model_flag_else_tier() {
    let p = Project::new();
    with_draft(&p);
    p.write(".hexa/config.toml", "[inference.tier_models]\n\"t2.5\" = \"claude-text:model-one\"\n");
    p.hexa_ok(&["design", "review", DRAFT_REL, "--stage", "5"]);
    let first = p.calls();
    assert!(!first.is_empty());
    assert!(first.iter().all(|c| c.meta.contains("model-one")), "first run not on M1");
    p.clear_log();
    p.hexa_ok(&["design", "review", DRAFT_REL, "--stage", "5", "--model", "claude-text:model-two"]);
    let second = p.calls();
    assert!(!second.is_empty());
    assert!(second.iter().all(|c| c.meta.contains("model-two") && !c.meta.contains("model-one")));
}

/// S14: goldfish_prompt is a function of its inputs only
#[test]
fn s14_goldfish_prompt_is_pure() {
    let mut rng = Rng(0x1234_5678_9abc_def1);
    let scratch = std::env::temp_dir().join(format!("hexa-s14-{}", std::process::id()));
    let mutate = |s: &str, i: usize| -> String {
        let mut b = s.as_bytes().to_vec();
        b[i] = if b[i] == b'Z' { b'Y' } else { b'Z' };
        String::from_utf8(b).unwrap()
    };
    for round in 0..100 {
        let persona = format!("P{}", rng.ascii(40));
        let standard = if rng.coin() { Some(format!("S{}", rng.ascii(40))) } else { None };
        let document = format!("D{}", rng.ascii(80));
        let cap = 1 + rng.below(60);
        let sources: Vec<(String, String)> =
            (0..rng.below(3)).map(|i| (format!("src/f{i}.rs"), rng.ascii(80))).collect();
        let a = design::goldfish_prompt(&persona, standard.as_deref(), &document, &sources, cap);
        std::fs::write(&scratch, format!("round {round}")).unwrap();
        let b = design::goldfish_prompt(&persona, standard.as_deref(), &document, &sources, cap);
        assert_eq!(a, b);

        let i = rng.below(persona.len());
        assert_ne!(a, design::goldfish_prompt(&mutate(&persona, i), standard.as_deref(), &document, &sources, cap));
        let i = rng.below(document.len());
        assert_ne!(a, design::goldfish_prompt(&persona, standard.as_deref(), &mutate(&document, i), &sources, cap));
        if let Some(s) = &standard {
            let m = mutate(s, rng.below(s.len()));
            assert_ne!(a, design::goldfish_prompt(&persona, Some(&m), &document, &sources, cap));
        }
        for (k, (_, c)) in sources.iter().enumerate() {
            if !c.is_empty() && c.len() <= cap {
                let mut s2 = sources.clone();
                s2[k].1 = mutate(c, rng.below(c.len()));
                assert_ne!(a, design::goldfish_prompt(&persona, standard.as_deref(), &document, &s2, cap));
            }
        }
    }
    let _ = std::fs::remove_file(&scratch);
}

/// S15: A source longer than the cap is cut and the cut is stated
#[test]
fn s15_oversized_source_is_cut_and_stated() {
    let sources = vec![("src/big.rs".to_string(), "x".repeat(5000))];
    let out = design::goldfish_prompt("persona", None, "document", &sources, 100);
    let longest = out.split(|c| c != 'x').map(str::len).max().unwrap_or(0);
    assert!(longest <= 100, "run of {longest} 'x' bytes");
    assert!(
        out.lines().any(|l| {
            let l2 = l.to_lowercase();
            l.contains("src/big.rs") && l.contains("100") && (l2.contains("cut") || l2.contains("truncat"))
        }),
        "no statement that src/big.rs was cut at 100 bytes:\n{out}"
    );
}

// ================================================================ §4

/// S16: Backquoted existing paths are returned in first-mention order, once each
#[test]
fn s16_backquoted_paths_in_first_mention_order() {
    let doc = "See `src/b.rs`, then `src/a.rs`, and again `src/b.rs`.";
    let exists = |p: &str| p == "src/a.rs" || p == "src/b.rs";
    assert_eq!(design::referenced_paths(doc, &exists), vec!["src/b.rs".to_string(), "src/a.rs".to_string()]);
}

/// S17: Paths that do not exist, or are outside the project, are not returned
#[test]
fn s17_missing_and_outside_paths_are_not_returned() {
    let doc = "`src/a.rs` `src/missing.rs` `../outside.rs` `/etc/passwd`";
    let exists = |p: &str| p == "src/a.rs";
    assert_eq!(design::referenced_paths(doc, &exists), vec!["src/a.rs".to_string()]);
}

/// S18: Unresolved paths are not read and are listed in the report
#[test]
fn s18_unresolved_paths_are_not_read_and_are_listed() {
    let p = Project::new();
    p.write("src/real.rs", "pub fn real() {} // REAL-MARKER\n");
    p.write_base("secret.txt", "SECRET-XYZ\n");
    p.write(DRAFT_REL, "# Draft\n\nUses `src/real.rs`, `src/ghost.rs` and `../secret.txt`.\n");
    let before = p.snapshot();
    p.hexa_ok(&["design", "review", DRAFT_REL, "--stage", "6"]);
    for c in p.calls() {
        assert!(!c.prompt.contains("SECRET-XYZ"), "a prompt contains the outside file");
    }
    let report = new_reports(&before, &p.snapshot(), DRAFT_REL);
    assert!(report.to_lowercase().contains("unresolved"));
    assert!(report.contains("src/ghost.rs"));
    assert!(report.contains("../secret.txt"));
}

/// S19: Paths mentioned outside backquotes are not returned
#[test]
fn s19_unquoted_paths_are_not_returned() {
    let exists = |p: &str| p == "src/a.rs";
    assert!(design::referenced_paths("see src/a.rs for details", &exists).is_empty());
}

/// S20: referenced_paths returns a duplicate-free, existing, order-preserving subset of backquoted spans
#[test]
fn s20_referenced_paths_property() {
    let pool = ["src/a.rs", "src/b.rs", "lib/c.md", "docs/d.txt", "x.toml", "not a path"];
    let mut rng = Rng(0xdead_beef_cafe_f00d);
    for _ in 0..500 {
        let mut doc = String::new();
        for _ in 0..rng.below(12) {
            let t = pool[rng.below(pool.len())];
            match rng.below(3) {
                0 => doc.push_str(&format!("`{t}` ")),
                1 => doc.push_str(&format!("{t} ")),
                _ => doc.push_str("and then "),
            }
        }
        let mask = rng.next();
        let exists = |p: &str| pool.iter().position(|q| *q == p).map(|i| mask >> i & 1 == 1).unwrap_or(false);
        let mut expected: Vec<String> = Vec::new();
        for (i, span) in doc.split('`').enumerate() {
            if i % 2 == 1 && exists(span) && !expected.iter().any(|e| e == span) {
                expected.push(span.to_string());
            }
        }
        assert_eq!(design::referenced_paths(&doc, &exists), expected, "doc: {doc:?}");
    }
}

// ================================================================ §5

/// S21: Presets give EGM's stages in order
#[test]
fn s21_presets_give_egm_stages_in_order() {
    let nums = |p| design::stages_for(p).into_iter().map(design::stage_number).collect::<Vec<u8>>();
    assert_eq!(nums(Preset::Full), vec![5, 6, 7, 8, 9]);
    assert_eq!(nums(Preset::FeatureSpecOnly), vec![5, 6, 7, 8]);
    assert_eq!(nums(Preset::ImplementationPlanOnly), vec![9]);
}

/// S22: Each stage has its reviewers and findings are numbered per stage
#[test]
fn s22_stage_reviewers_and_per_stage_numbering() {
    for s in ALL_STAGES {
        assert!(!design::stage_reviewers(s).is_empty(), "{s:?} has no reviewers");
    }
    assert_eq!(design::stage_reviewers(Stage::Critic).len(), 3);
    assert_eq!(design::stage_reviewers(Stage::ImplementationReadiness).len(), 3);

    let p = Project::new();
    with_draft(&p);
    p.set_reply(
        "if grep -q 'FINDING-ID-' \"$PROMPT\"; then grep -o 'FINDING-ID-[0-9-]*' \"$PROMPT\" | sort -u | sed 's/^/- /'; else echo \"- FINDING-ID-$id\"; fi",
    );
    let before = p.snapshot();
    p.hexa_ok(&["design", "review", DRAFT_REL]);
    let report = new_reports(&before, &p.snapshot(), DRAFT_REL);
    let pos: Vec<usize> = (5..=9)
        .map(|n| stage_heading_pos(&report, n).unwrap_or_else(|| panic!("no stage {n} section")))
        .collect();
    assert!(pos.windows(2).all(|w| w[0] < w[1]), "stage sections out of order: {pos:?}");
    assert!(report.contains("S7-1") && report.contains("S7-2"), "stage 7 findings not numbered S7-n");
}

/// S23: The review never edits the draft
#[test]
fn s23_review_never_edits_draft() {
    let p = Project::new();
    let draft = with_draft(&p);
    p.hexa_ok(&["design", "review", DRAFT_REL]);
    assert_eq!(p.read(DRAFT_REL), draft);
}

/// S24: A stage does not start before the previous stage's reviewers have all returned
#[test]
fn s24_stage_waits_for_previous_stage() {
    let p = Project::new();
    with_draft(&p);
    let s5 = exclusive_stems(Stage::ZoomOut);
    let s6 = exclusive_stems(Stage::Comprehension);
    assert!(!s5.is_empty() && !s6.is_empty());
    let fp = p.write_base("slow.txt", &(persona_fingerprint(&s5[0]) + "\n"));
    p.set_reply(&format!(
        "if grep -qFf '{}' \"$PROMPT\"; then sleep 2; fi; echo '- A finding.'",
        fp.display()
    ));
    p.hexa_ok(&["design", "review", DRAFT_REL]);
    let calls = p.calls();
    let in_stage = |c: &Call, stems: &[String]| sole_persona(&c.prompt).map(|s| stems.contains(&s)).unwrap_or(false);
    let last5 = calls.iter().filter(|c| in_stage(c, &s5)).map(|c| c.end).max().expect("stage-5 calls");
    let first6 = calls.iter().filter(|c| in_stage(c, &s6)).map(|c| c.start).min().expect("stage-6 calls");
    assert!(first6 >= last5, "a stage-6 call started before stage 5 returned");
}

/// S25: --stage N runs one stage only
#[test]
fn s25_single_stage_runs_only_that_stage() {
    let p = Project::new();
    with_draft(&p);
    let before = p.snapshot();
    p.hexa_ok(&["design", "review", DRAFT_REL, "--stage", "7"]);
    let critic = design::stage_reviewers(Stage::Critic);
    let mut seen = BTreeSet::new();
    for c in p.calls() {
        if let Some(s) = sole_persona(&c.prompt) {
            seen.insert(s.clone());
            let in_critic = critic.contains(&s.as_str());
            let other = stages_of_stem(&s).iter().any(|st| *st != Stage::Critic);
            assert!(in_critic || !other, "persona {s} of another stage was called");
        }
    }
    assert!(critic.iter().all(|s| seen.contains(*s)), "not every stage-7 reviewer was called");
    let report = new_reports(&before, &p.snapshot(), DRAFT_REL);
    assert!(stage_heading_pos(&report, 7).is_some());
    for n in [5, 6, 8, 9] {
        assert!(stage_heading_pos(&report, n).is_none(), "report has a stage {n} section");
    }
}

/// S26: Every preset runs stages in strictly ascending stage number
#[test]
fn s26_presets_run_stages_ascending() {
    let numbers: Vec<u8> = ALL_STAGES.iter().map(|s| design::stage_number(*s)).collect();
    assert_eq!(numbers.iter().collect::<BTreeSet<_>>().len(), ALL_STAGES.len());
    for preset in [Preset::Full, Preset::FeatureSpecOnly, Preset::ImplementationPlanOnly] {
        let n: Vec<u8> = design::stages_for(preset).into_iter().map(design::stage_number).collect();
        assert!(!n.is_empty());
        assert!(n.iter().all(|x| (5..=9).contains(x)), "{preset:?}: {n:?}");
        assert!(n.windows(2).all(|w| w[0] < w[1]), "{preset:?}: {n:?}");
    }
}

// ================================================================ §6

/// S27: Brainstorm and draft write their artifacts to the artifact directory
#[test]
fn s27_brainstorm_and_draft_use_artifact_directory() {
    let p = Project::new();
    p.write("CLAUDE.md", "# Project\n\nEGM artifact directory: design/egm\n");
    p.set_reply("if grep -q 'an idea' \"$PROMPT\"; then echo 'BRIEF-MARKER-123 concept brief'; fi; echo '- A note.'");
    p.hexa_ok(&["design", "brainstorm", "an idea"]);
    let mut critic_stems: BTreeSet<String> =
        design::stage_reviewers(Stage::Critic).into_iter().map(String::from).collect();
    critic_stems.extend(agent_stems().into_iter().filter(|s| s.contains("critic")));
    let fps: Vec<String> = critic_stems.iter().map(|s| persona_fingerprint(s)).collect();
    assert!(
        p.calls()
            .iter()
            .any(|c| c.prompt.contains("BRIEF-MARKER-123") && fps.iter().any(|f| c.prompt.contains(f.as_str()))),
        "the critic goldfish was not called on the brief"
    );
    p.hexa_ok(&["design", "draft"]);
    assert!(p.root.join("design/egm/concepts_brief.md").is_file());
    assert!(p.root.join("design/egm/design_draft.md").is_file());
}

/// S28: Without the CLAUDE.md line, artifacts go to docs/designs/
#[test]
fn s28_default_artifact_directory() {
    let p = Project::new();
    p.hexa_ok(&["design", "brainstorm", "an idea"]);
    assert!(p.root.join("docs/designs/concepts_brief.md").is_file());
}

/// S29: Resolve hands the elephant only the accepted findings
#[test]
fn s29_resolve_passes_only_accepted_findings() {
    let p = Project::new();
    with_draft(&p);
    handwritten_report(&p, "docs/designs/review_report.md", DRAFT_REL);
    p.hexa_ok(&["design", "resolve", "docs/designs/review_report.md", "--accept", "S6-1,S7-1"]);
    let calls = p.calls();
    assert!(calls.iter().any(|c| c.prompt.contains("DRAFT-MARKER-QQQ") && c.prompt.contains("AAA") && c.prompt.contains("CCC")));
    assert!(calls.iter().all(|c| !c.prompt.contains("BBB")), "a rejected finding reached the elephant");
}

/// S30: No command marks a design approved
#[test]
fn s30_no_command_marks_design_approved() {
    let p = Project::new();
    p.git(&["init", "-q"]);
    p.git(&["config", "user.email", "test@example.invalid"]);
    p.git(&["config", "user.name", "Test"]);
    p.git(&["add", "-A"]);
    p.git(&["commit", "-q", "-m", "initial"]);
    let commits = p.git(&["rev-list", "--count", "HEAD"]);
    p.hexa_ok(&["design", "brainstorm", "an idea"]);
    p.hexa_ok(&["design", "draft"]);
    p.hexa_ok(&["design", "review", DRAFT_REL]);
    handwritten_report(&p, "docs/designs/review_report.md", DRAFT_REL);
    p.hexa_ok(&["design", "resolve", "docs/designs/review_report.md", "--accept", "S6-1"]);
    assert_eq!(p.git(&["rev-list", "--count", "HEAD"]), commits, "a commit was created");
    let staged = p.git(&["diff", "--cached", "--name-only"]);
    assert!(!staged.lines().any(|l| l.starts_with("docs/designs/")), "staged: {staged}");
    for (path, bytes) in p.snapshot() {
        if path.starts_with("docs/designs/") {
            let text = String::from_utf8_lossy(&bytes).to_lowercase();
            assert!(!text.contains("approved"), "{path} carries an approval");
            assert!(!path.to_lowercase().contains("approved"));
        }
    }
}

// ================================================================ §7

/// S31: Drift report has one section per document, with No drift detected where none was found
#[test]
fn s31_drift_report_sections() {
    let p = drift_project();
    p.set_reply(DRIFT_README_ONLY);
    let out = p.hexa(&["drift"]);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let report = p.read("docs/analysis/2026-10-27_DRIFT_REPORT.md");
    let mut sections: Vec<String> = Vec::new();
    for line in report.lines() {
        if line.trim_start().starts_with('#') {
            sections.push(String::new());
        }
        if let Some(s) = sections.last_mut() {
            s.push_str(line);
            s.push('\n');
        }
    }
    let section = |name: &str| -> String {
        sections
            .iter()
            .find(|s| s.lines().next().map(|h| h.contains(name)).unwrap_or(false))
            .unwrap_or_else(|| panic!("no section for {name}"))
            .clone()
    };
    assert!(section("README.md").contains("DIVERGENCE-README"));
    assert!(section("ARCHITECTURE.md").contains("No drift detected"));
    assert!(section("docs/guide.md").contains("No drift detected"));
}

/// S32: ADRs are not checked
#[test]
fn s32_adrs_are_not_checked() {
    let p = drift_project();
    p.write("docs/adr/ADR-0001.md", "# ADR-0001\n\nADR-CONTENT-MARK decision.\n");
    p.write("docs/adr/ADR-0002.md", "# ADR-0002\n\nADR-CONTENT-MARK decision two.\n");
    p.hexa_ok(&["drift"]);
    assert!(p.calls().iter().all(|c| !c.prompt.contains("ADR-CONTENT-MARK")));
    let report = drift_report(&p);
    assert!(!report.contains("ADR-0001") && !report.contains("ADR-0002"));
}

/// S33: Drift found does not fail the run unless asked
#[test]
fn s33_drift_fails_only_when_asked() {
    let p = drift_project();
    p.set_reply(DRIFT_README_ONLY);
    assert_eq!(p.hexa(&["drift"]).status.code(), Some(0));
    assert!(!p.hexa(&["drift", "--fail-on-drift"]).status.success());
}

/// S34: No drift with --fail-on-drift exits 0
#[test]
fn s34_no_drift_with_fail_on_drift_exits_zero() {
    let p = drift_project();
    p.set_reply("echo 'No divergences.'");
    assert_eq!(p.hexa(&["drift", "--fail-on-drift"]).status.code(), Some(0));
}

// ================================================================ §8

/// S35: Design and drift commands write only to their artifact directory and docs/analysis/
#[test]
fn s35_commands_write_only_allowed_directories() {
    let p = drift_project();
    handwritten_report(&p, "docs/designs/review_report.md", DRAFT_REL);
    let runs: Vec<Vec<&str>> = vec![
        vec!["design", "brainstorm", "an idea"],
        vec!["design", "draft"],
        vec!["design", "review", DRAFT_REL, "--stage", "6"],
        vec!["design", "resolve", "docs/designs/review_report.md", "--accept", "S6-1"],
        vec!["drift"],
    ];
    for args in runs {
        let before = p.snapshot();
        p.hexa_ok(&args);
        assert_writes_allowed(&changed(&before, &p.snapshot()), "docs/designs");
    }
}

/// S36: A model reply proposing code changes does not write code
#[test]
fn s36_code_in_reply_is_not_written() {
    let p = Project::new();
    with_draft(&p);
    handwritten_report(&p, "docs/designs/review_report.md", DRAFT_REL);
    p.set_reply("printf '%s\\n' 'Replace src/lib.rs with:' '```rust' '// src/lib.rs' 'pub fn replaced() {}' '```'");
    let before = p.snapshot();
    p.hexa_ok(&["design", "resolve", "docs/designs/review_report.md", "--accept", "S6-1"]);
    assert_eq!(p.read("src/lib.rs"), "pub fn original() {}\n");
    assert_writes_allowed(&changed(&before, &p.snapshot()), "docs/designs");
}

/// S37: No hexa design or hexa drift run writes outside the allowed directories
#[test]
fn s37_no_run_writes_outside_allowed_directories() {
    let settings: [Option<&str>; 3] = [None, Some("design/egm"), Some("notes/egm-artifacts")];
    let replies = [
        "echo '- Canned finding.'",
        "printf '%s\\n' '```rust' '// src/lib.rs' 'pub fn replaced() {}' '```' '- S1: write ../escape.txt'",
        "echo 'No divergences. Status: done'",
    ];
    for (i, setting) in settings.iter().enumerate() {
        let p = drift_project();
        p.set_reply(replies[i % replies.len()]);
        let dir = setting.unwrap_or("docs/designs");
        if let Some(d) = setting {
            p.write("CLAUDE.md", &format!("# Project\n\nEGM artifact directory: {d}\n"));
        }
        let draft = format!("{dir}/design_draft.md");
        let report = format!("{dir}/review_report.md");
        handwritten_report(&p, &report, &draft);
        let runs: Vec<Vec<&str>> = vec![
            vec!["design", "brainstorm", "an idea"],
            vec!["design", "draft"],
            vec!["design", "review", &draft, "--stage", "5"],
            vec!["design", "resolve", &report, "--accept", "S6-1"],
            vec!["drift"],
        ];
        for args in runs {
            let before = p.snapshot();
            let out = p.hexa(&args);
            assert_eq!(out.status.code(), Some(0), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
            let ch = changed(&before, &p.snapshot());
            assert!(!ch.is_empty(), "{args:?} wrote nothing");
            assert_writes_allowed(&ch, dir);
        }
        assert!(!p.base.join("escape.txt").exists());
    }
}
