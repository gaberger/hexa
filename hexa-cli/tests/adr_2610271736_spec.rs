//! Integration tests for ADR-2610271736: hexa designs with an elephant and reviews with goldfish.

use hexa_core::domain::design::{
    goldfish_prompt, referenced_paths, stage_number, stage_reviewers, stages_for, verify_pin, Pin,
    Preset, Stage,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const REPO: &str = "https://github.com/google/elephant-goldfish-model";
const COMMIT: &str = "a8380ab5f0aba392c97e31c79f9a6666b3f09f14";
const TEXT_M1: &str = "claude-text:sonnet";
const TEXT_M2: &str = "claude-text:opus";
const AGENT_MODEL: &str = "claude-code:sonnet";
const DEFAULT_ARTIFACT: &str = "docs/designs";
const DEFAULT_REPLY: &str = "- Finding A-{N}: the design leaves cache eviction unspecified.\n- Finding B-{N}: the draft relies on an undefined term.\n";

// ---------------------------------------------------------------- utilities

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn workspace() -> PathBuf {
    manifest().parent().expect("workspace root").to_path_buf()
}

fn all_stages() -> Vec<Stage> {
    vec![
        Stage::ZoomOut,
        Stage::Comprehension,
        Stage::Critic,
        Stage::IntermediateZoomOut,
        Stage::ImplementationReadiness,
    ]
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn coin(&mut self) -> bool {
        self.below(2) == 0
    }
    fn text(&mut self, max: u64) -> String {
        let alphabet = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJ0123456789 .,";
        let len = 1 + self.below(max);
        (0..len)
            .map(|_| alphabet[self.below(alphabet.len() as u64) as usize] as char)
            .collect()
    }
}

fn fake_hash(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[4 * i],
                chunk[4 * i + 1],
                chunk[4 * i + 2],
                chunk[4 * i + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut a = h;
        for i in 0..64 {
            let s1 = a[4].rotate_right(6) ^ a[4].rotate_right(11) ^ a[4].rotate_right(25);
            let ch = (a[4] & a[5]) ^ (!a[4] & a[6]);
            let t1 = a[7]
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a[0].rotate_right(2) ^ a[0].rotate_right(13) ^ a[0].rotate_right(22);
            let maj = (a[0] & a[1]) ^ (a[0] & a[2]) ^ (a[1] & a[2]);
            let t2 = s0.wrapping_add(maj);
            a = [
                t1.wrapping_add(t2),
                a[0],
                a[1],
                a[2],
                a[3].wrapping_add(t1),
                a[4],
                a[5],
                a[6],
            ];
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(a[i]);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}

// ---------------------------------------------------------------- minimal JSON

#[derive(Debug, Clone)]
enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
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
        while self.i < self.s.len() && (self.s[self.i] as char).is_whitespace() {
            self.i += 1;
        }
    }
    fn value(&mut self) -> Json {
        self.ws();
        match self.s[self.i] {
            b'{' => {
                self.i += 1;
                let mut pairs = Vec::new();
                loop {
                    self.ws();
                    if self.s[self.i] == b'}' {
                        self.i += 1;
                        break;
                    }
                    let k = self.string();
                    self.ws();
                    assert_eq!(self.s[self.i], b':', "JSON: expected ':'");
                    self.i += 1;
                    let v = self.value();
                    pairs.push((k, v));
                    self.ws();
                    if self.s[self.i] == b',' {
                        self.i += 1;
                    }
                }
                Json::Obj(pairs)
            }
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    self.ws();
                    if self.s[self.i] == b']' {
                        self.i += 1;
                        break;
                    }
                    items.push(self.value());
                    self.ws();
                    if self.s[self.i] == b',' {
                        self.i += 1;
                    }
                }
                Json::Arr(items)
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
                let start = self.i;
                while self.i < self.s.len() && b"+-0123456789.eE".contains(&self.s[self.i]) {
                    self.i += 1;
                }
                let t = std::str::from_utf8(&self.s[start..self.i]).unwrap();
                Json::Num(t.parse().unwrap_or(0.0))
            }
        }
    }
    fn string(&mut self) -> String {
        self.ws();
        assert_eq!(self.s[self.i], b'"', "JSON: expected string");
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
                            let ch = u32::from_str_radix(hex, 16)
                                .ok()
                                .and_then(char::from_u32)
                                .unwrap_or('?');
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
    let mut p = JsonParser {
        s: text.as_bytes(),
        i: 0,
    };
    p.value()
}

fn pin_from_upstream_json(json: &Json) -> Pin {
    let repository = json
        .get("repository")
        .or_else(|| json.get("repo"))
        .and_then(Json::as_str)
        .expect("UPSTREAM.json records the repository")
        .to_string();
    let commit = json
        .get("commit")
        .and_then(Json::as_str)
        .expect("UPSTREAM.json records the commit")
        .to_string();
    let files_json = json.get("files").expect("UPSTREAM.json records files");
    let mut files = Vec::new();
    match files_json {
        Json::Obj(pairs) => {
            for (path, v) in pairs {
                let hash = match v {
                    Json::Str(s) => s.clone(),
                    other => other
                        .get("sha256")
                        .and_then(Json::as_str)
                        .expect("file hash")
                        .to_string(),
                };
                files.push((path.clone(), hash));
            }
        }
        Json::Arr(items) => {
            for item in items {
                let path = item
                    .get("path")
                    .and_then(Json::as_str)
                    .expect("file path")
                    .to_string();
                let hash = item
                    .get("sha256")
                    .or_else(|| item.get("hash"))
                    .and_then(Json::as_str)
                    .expect("file hash")
                    .to_string();
                files.push((path, hash));
            }
        }
        _ => panic!("UPSTREAM.json files is neither an object nor an array"),
    }
    Pin {
        repository,
        commit,
        files,
    }
}

// ---------------------------------------------------------------- file walking

fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.file_name().map(|n| n == ".git").unwrap_or(false) {
            continue;
        }
        if path.is_dir() {
            walk(&path, base, out);
        } else {
            let rel = path
                .strip_prefix(base)
                .unwrap()
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            out.push(rel);
        }
    }
}

// ---------------------------------------------------------------- project harness

static COUNTER: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Clone)]
struct Call {
    args: String,
    text: String,
    start: u128,
    end: u128,
}

struct Project {
    base: PathBuf,
    root: PathBuf,
    log: PathBuf,
    bin: PathBuf,
    home: PathBuf,
    artifact: String,
}

const FAKE_CLAUDE: &str = r#"#!/bin/sh
LOG='@LOG@'
start=$(date +%s%N)
rec="$LOG/call-$start-$$"
: > "$rec.args"
for a in "$@"; do printf '%s\n' "$a" >> "$rec.args"; done
if [ -t 0 ]; then : > "$rec.stdin"; else cat > "$rec.stdin"; fi
@DELAY@
reply_file='@REPLY@'
@MATCH@
reply=$(sed "s/{N}/$$/g" "$reply_file")
end=$(date +%s%N)
if grep -qE '^(json|--output-format=json)$' "$rec.args"; then
  esc=$(printf '%s' "$reply" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' | awk '{printf "%s\\n", $0}')
  printf '%s' '{"type":"result","subtype":"success","is_error":false,"result":"'
  printf '%s' "$esc"
  printf '%s\n' '"}'
else
  printf '%s\n' "$reply"
fi
printf '%s %s\n' "$start" "$end" > "$rec.time"
"#;

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(["-c", "user.name=test", "-c", "user.email=test@example.invalid"])
        .args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        None
    }
}

fn new_project(name: &str) -> Project {
    new_project_with(name, None)
}

fn new_project_with(name: &str, artifact: Option<&str>) -> Project {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let base = std::env::temp_dir().join(format!(
        "hexa-egm-{name}-{}-{nanos}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    let root = base.join("proj");
    let log = base.join("log");
    let bin = base.join("bin");
    let home = base.join("home");
    for d in [&root, &log, &bin, &home] {
        fs::create_dir_all(d).unwrap();
    }
    let artifact_dir = artifact.unwrap_or(DEFAULT_ARTIFACT).to_string();
    let p = Project {
        base,
        root,
        log,
        bin,
        home,
        artifact: artifact_dir.clone(),
    };
    let claude_md = match artifact {
        Some(a) => format!("# Project notes\n\nEGM artifact directory: {a}\n"),
        None => "# Project notes\n\nA small widget cache.\n".to_string(),
    };
    p.write("CLAUDE.md", &claude_md);
    p.write("README.md", "# Widget cache\n\nThe cache lives in `src/lib.rs`.\n");
    p.write("src/lib.rs", "pub fn cache() -> u32 { 42 }\n");
    p.write(
        &format!("{artifact_dir}/design_draft.md"),
        "# Design: widget cache\n\nDRAFT-MARKER-QQQ\n\nThe cache lives in `src/lib.rs`.\n",
    );
    p.write(
        &format!("{artifact_dir}/review.md"),
        &format!(
            "# Review report: `{a}/design_draft.md`\n\nDraft: `{a}/design_draft.md`\n\nProduced by EGM at commit {COMMIT}.\n\n## Stage 6\n\n- S6-1: AAA-MARKER the draft does not say who owns the cache.\n- S6-2: BBB-MARKER the term warm path is undefined.\n\n## Stage 7\n\n- S7-1: CCC-MARKER the eviction policy is unstated.\n",
            a = artifact_dir
        ),
    );
    p.write_config(TEXT_M1);
    p.set_reply(DEFAULT_REPLY);
    p.install_fake(None, false);
    if git(&p.root, &["init", "-q"]).is_some() {
        git(&p.root, &["add", "-A"]);
        git(&p.root, &["commit", "-qm", "init"]);
    }
    p
}

impl Project {
    fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    fn write(&self, rel: &str, content: &str) {
        let path = self.path(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn write_config(&self, t25_model: &str) {
        self.write(
            ".hexa/config.toml",
            &format!("[inference.tier_models]\n\"t2.5\" = \"{t25_model}\"\n"),
        );
    }

    fn draft_rel(&self) -> String {
        format!("{}/design_draft.md", self.artifact)
    }

    fn report_rel(&self) -> String {
        format!("{}/review.md", self.artifact)
    }

    fn set_reply(&self, reply: &str) {
        fs::write(self.base.join("reply.txt"), reply).unwrap();
    }

    fn install_fake(&self, matcher: Option<(&str, &str)>, delay_first: bool) {
        let delay = if delay_first {
            "if mkdir \"$LOG/.first\" 2>/dev/null; then sleep 1; fi".to_string()
        } else {
            String::new()
        };
        let matched = match matcher {
            Some((needle, reply)) => {
                let file = self.base.join("match_reply.txt");
                fs::write(&file, reply).unwrap();
                format!(
                    "if grep -qF -- '{needle}' \"$rec.args\" \"$rec.stdin\"; then reply_file='{}'; fi",
                    file.display()
                )
            }
            None => String::new(),
        };
        let script = FAKE_CLAUDE
            .replace("@LOG@", &self.log.display().to_string())
            .replace("@DELAY@", &delay)
            .replace("@REPLY@", &self.base.join("reply.txt").display().to_string())
            .replace("@MATCH@", &matched);
        let path = self.bin.join("claude");
        fs::write(&path, script).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn hexa(&self, args: &[&str]) -> Output {
        let path = format!(
            "{}:{}",
            self.bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        Command::new(env!("CARGO_BIN_EXE_hexa"))
            .args(args)
            .current_dir(&self.root)
            .env("PATH", path)
            .env("HOME", &self.home)
            .stdin(Stdio::null())
            .output()
            .expect("run hexa")
    }

    fn calls(&self) -> Vec<Call> {
        let mut stems = BTreeSet::new();
        for entry in fs::read_dir(&self.log).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(stem) = name.strip_suffix(".args") {
                stems.insert(stem.to_string());
            }
        }
        let mut calls = Vec::new();
        for stem in stems {
            let args = fs::read_to_string(self.log.join(format!("{stem}.args"))).unwrap_or_default();
            let stdin =
                fs::read_to_string(self.log.join(format!("{stem}.stdin"))).unwrap_or_default();
            let time = fs::read_to_string(self.log.join(format!("{stem}.time"))).unwrap_or_default();
            let mut parts = time.split_whitespace();
            let start = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            let end = parts.next().and_then(|s| s.parse().ok()).unwrap_or(u128::MAX);
            calls.push(Call {
                text: format!("{args}\n{stdin}"),
                args,
                start,
                end,
            });
        }
        calls
    }

    fn clear_calls(&self) {
        fs::remove_dir_all(&self.log).unwrap();
        fs::create_dir_all(&self.log).unwrap();
    }

    fn snapshot(&self) -> BTreeMap<String, Vec<u8>> {
        let mut files = Vec::new();
        walk(&self.root, &self.root, &mut files);
        files
            .into_iter()
            .map(|f| {
                let bytes = fs::read(self.root.join(&f)).unwrap_or_default();
                (f, bytes)
            })
            .collect()
    }
}

fn changed(before: &BTreeMap<String, Vec<u8>>, after: &BTreeMap<String, Vec<u8>>) -> Vec<String> {
    let mut out = BTreeSet::new();
    for (k, v) in after {
        if before.get(k) != Some(v) {
            out.insert(k.clone());
        }
    }
    for k in before.keys() {
        if !after.contains_key(k) {
            out.insert(k.clone());
        }
    }
    out.into_iter().collect()
}

fn written_text(
    before: &BTreeMap<String, Vec<u8>>,
    after: &BTreeMap<String, Vec<u8>>,
    exclude: &[&str],
) -> String {
    changed(before, after)
        .into_iter()
        .filter(|f| !exclude.contains(&f.as_str()))
        .filter_map(|f| after.get(&f).map(|b| String::from_utf8_lossy(b).into_owned()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn drift_report(p: &Project) -> (String, String) {
    let dir = p.path("docs/analysis");
    let entries = fs::read_dir(&dir).expect("docs/analysis exists after hexa drift");
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with("_DRIFT_REPORT.md") {
            let date = &name[..name.len() - "_DRIFT_REPORT.md".len()];
            assert_eq!(date.len(), 10, "report name carries a YYYY-MM-DD date: {name}");
            assert!(
                date.chars().enumerate().all(|(i, c)| if i == 4 || i == 7 {
                    c == '-'
                } else {
                    c.is_ascii_digit()
                }),
                "report name carries a YYYY-MM-DD date: {name}"
            );
            return (name, fs::read_to_string(entry.path()).unwrap());
        }
    }
    panic!("no docs/analysis/<YYYY-MM-DD>_DRIFT_REPORT.md written");
}

fn persona_marker(stem: &str) -> String {
    let path = manifest()
        .join("assets/egm/agents")
        .join(format!("{stem}.md"));
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("vendored persona {} missing: {e}", path.display()));
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .max_by_key(|l| l.len())
        .expect("persona has text")
        .to_string()
}

fn markers_for(stems: &[&str]) -> Vec<(String, String)> {
    stems
        .iter()
        .map(|s| (s.to_string(), persona_marker(s)))
        .collect()
}

fn all_reviewer_markers() -> Vec<(String, String)> {
    let mut stems = BTreeSet::new();
    for s in all_stages() {
        for r in stage_reviewers(s) {
            stems.insert(r);
        }
    }
    markers_for(&stems.into_iter().collect::<Vec<_>>())
}

fn assert_ok(out: &Output, what: &str) {
    assert!(
        out.status.success(),
        "{what} failed: status {:?}\nstdout: {}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn allowed(path: &str, artifact: &str) -> bool {
    path.starts_with(&format!("{artifact}/")) || path.starts_with("docs/analysis/")
}

// ---------------------------------------------------------------- §1

/// S01: A vendored copy that matches its pin verifies
#[test]
fn s01_matching_vendored_copy_verifies() {
    let critic = b"critic persona".to_vec();
    let license = b"Apache License 2.0".to_vec();
    let pin = Pin {
        repository: REPO.to_string(),
        commit: COMMIT.to_string(),
        files: vec![
            ("agents/critic.md".to_string(), fake_hash(&critic)),
            ("LICENSE".to_string(), fake_hash(&license)),
        ],
    };
    let present = vec!["agents/critic.md".to_string(), "LICENSE".to_string()];
    let read = |p: &str| match p {
        "agents/critic.md" => Some(critic.clone()),
        "LICENSE" => Some(license.clone()),
        _ => None,
    };
    assert_eq!(verify_pin(&pin, &present, &read, &|b: &[u8]| fake_hash(b)), Ok(()));
}

/// S02: A file whose hash differs from its pin is refused and named
#[test]
fn s02_hash_mismatch_is_named() {
    let license = b"Apache License 2.0".to_vec();
    let pin = Pin {
        repository: REPO.to_string(),
        commit: COMMIT.to_string(),
        files: vec![
            ("agents/critic.md".to_string(), fake_hash(b"original critic")),
            ("LICENSE".to_string(), fake_hash(&license)),
        ],
    };
    let present = vec!["agents/critic.md".to_string(), "LICENSE".to_string()];
    let read = |p: &str| match p {
        "agents/critic.md" => Some(b"edited critic".to_vec()),
        "LICENSE" => Some(license.clone()),
        _ => None,
    };
    let err = verify_pin(&pin, &present, &read, &|b: &[u8]| fake_hash(b))
        .expect_err("a changed file must be refused");
    assert!(err.iter().any(|e| e.contains("agents/critic.md")), "{err:?}");
    assert!(!err.iter().any(|e| e.contains("LICENSE")), "{err:?}");
}

/// S03: A recorded file that is missing is refused and named
#[test]
fn s03_missing_file_is_named() {
    let license = b"Apache License 2.0".to_vec();
    let pin = Pin {
        repository: REPO.to_string(),
        commit: COMMIT.to_string(),
        files: vec![
            ("agents/critic.md".to_string(), fake_hash(b"critic")),
            ("LICENSE".to_string(), fake_hash(&license)),
        ],
    };
    let present = vec!["LICENSE".to_string()];
    let read = |p: &str| match p {
        "LICENSE" => Some(license.clone()),
        _ => None,
    };
    let err = verify_pin(&pin, &present, &read, &|b: &[u8]| fake_hash(b))
        .expect_err("a missing file must be refused");
    assert!(err.iter().any(|e| e.contains("agents/critic.md")), "{err:?}");
}

/// S04: A file present but not recorded in the pin is refused and named
#[test]
fn s04_unrecorded_file_is_named() {
    let license = b"Apache License 2.0".to_vec();
    let pin = Pin {
        repository: REPO.to_string(),
        commit: COMMIT.to_string(),
        files: vec![("LICENSE".to_string(), fake_hash(&license))],
    };
    let present = vec!["LICENSE".to_string(), "agents/extra.md".to_string()];
    let read = |p: &str| match p {
        "LICENSE" => Some(license.clone()),
        "agents/extra.md" => Some(b"extra".to_vec()),
        _ => None,
    };
    let err = verify_pin(&pin, &present, &read, &|b: &[u8]| fake_hash(b))
        .expect_err("an unrecorded file must be refused");
    assert!(err.iter().any(|e| e.contains("agents/extra.md")), "{err:?}");
}

/// S05: verify_pin names exactly the set of mismatched, missing and unrecorded files
#[test]
fn s05_verify_pin_names_exactly_the_bad_files() {
    let universe = [
        "LICENSE",
        "agents/critic.md",
        "agents/style.md",
        "resources/execution_instructions.md",
        "skills/egm-review/SKILL.md",
        "x.md",
    ];
    let mut rng = Rng(0x5eed_0005);
    for _ in 0..300 {
        let mut files = Vec::new();
        let mut present = Vec::new();
        let mut contents: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        let mut expected = BTreeSet::new();
        for name in universe {
            let recorded = rng.coin();
            let is_present = rng.below(4) != 0;
            let body = rng.text(20).into_bytes();
            if is_present {
                present.push(name.to_string());
                contents.insert(name.to_string(), body.clone());
            }
            if recorded {
                let correct = rng.below(3) != 0;
                let hash = if correct {
                    fake_hash(&body)
                } else {
                    fake_hash(format!("{}-other", String::from_utf8_lossy(&body)).as_bytes())
                };
                files.push((name.to_string(), hash));
                if !is_present || !correct {
                    expected.insert(name.to_string());
                }
            } else if is_present {
                expected.insert(name.to_string());
            }
        }
        let pin = Pin {
            repository: REPO.to_string(),
            commit: COMMIT.to_string(),
            files,
        };
        let read = |p: &str| contents.get(p).cloned();
        let result = verify_pin(&pin, &present, &read, &|b: &[u8]| fake_hash(b));
        if expected.is_empty() {
            assert_eq!(result, Ok(()), "pin {pin:?} present {present:?}");
        } else {
            let err = result.expect_err("bad files must be refused");
            let named: BTreeSet<String> = universe
                .iter()
                .filter(|u| err.iter().any(|e| e == *u || e.contains(&format!("{u}"))))
                .map(|u| u.to_string())
                .collect();
            // `agents/critic.md` contains no other universe name as a substring, so
            // substring matching identifies paths exactly here.
            assert_eq!(named, expected, "pin {pin:?} present {present:?} err {err:?}");
        }
    }
}

/// S06: The checked-in vendored copy matches UPSTREAM.json
#[test]
fn s06_checked_in_vendored_copy_matches_upstream_json() {
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    let root = manifest().join("assets/egm");
    let upstream = fs::read_to_string(root.join("UPSTREAM.json"))
        .expect("hexa-cli/assets/egm/UPSTREAM.json exists");
    let pin = pin_from_upstream_json(&parse_json(&upstream));
    assert_eq!(pin.commit, COMMIT);
    assert!(pin.repository.contains("google/elephant-goldfish-model"));

    let mut present = Vec::new();
    walk(&root, &root, &mut present);
    present.retain(|p| p != "UPSTREAM.json");
    let read = |p: &str| fs::read(root.join(p)).ok();
    assert_eq!(verify_pin(&pin, &present, &read, &|b: &[u8]| sha256_hex(b)), Ok(()));

    let recorded: Vec<&str> = pin.files.iter().map(|(p, _)| p.as_str()).collect();
    assert!(recorded.contains(&"LICENSE"), "{recorded:?}");
    assert!(recorded.contains(&"resources/execution_instructions.md"), "{recorded:?}");
    assert!(
        recorded.iter().any(|p| p.starts_with("agents/") && p.ends_with(".md")),
        "{recorded:?}"
    );
    assert!(
        recorded
            .iter()
            .any(|p| p.starts_with("resources/document_standards/") && p.ends_with(".md")),
        "{recorded:?}"
    );
    for skill in [
        "egm-brainstorm",
        "egm-draft",
        "egm-review",
        "egm-resolve",
        "egm-drift-report",
    ] {
        assert!(
            recorded.iter().any(|p| p.contains(skill)),
            "skill {skill} not recorded: {recorded:?}"
        );
    }
}

// ---------------------------------------------------------------- §2

/// S07: Attribution document and help text name EGM
#[test]
fn s07_attribution_and_help_name_egm() {
    let attributions = fs::read_to_string(workspace().join("docs/ATTRIBUTIONS.md"))
        .expect("docs/ATTRIBUTIONS.md exists");
    for needle in [
        "Elephant-Goldfish Model",
        "Google LLC",
        "Apache-2.0",
        REPO,
        COMMIT,
        "Dave Rensin",
    ] {
        assert!(attributions.contains(needle), "ATTRIBUTIONS.md lacks {needle}");
    }
    let readme = fs::read_to_string(workspace().join("README.md")).expect("README.md");
    assert!(
        readme.contains("ATTRIBUTIONS.md") || readme.contains(REPO),
        "README links neither ATTRIBUTIONS.md nor EGM"
    );
    for cmd in [["design", "--help"], ["drift", "--help"]] {
        let out = Command::new(env!("CARGO_BIN_EXE_hexa"))
            .args(cmd)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(out.status.success(), "hexa {cmd:?} failed");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            text.contains("EGM") || text.contains("Elephant-Goldfish"),
            "hexa {cmd:?} does not name EGM: {text}"
        );
    }
}

/// S08: A report from an EGM prompt names its persona and pinned commit
#[test]
fn s08_review_report_names_persona_and_commit() {
    let p = new_project("s08");
    let before = p.snapshot();
    let draft = p.draft_rel();
    let out = p.hexa(&["design", "review", &draft, "--stage", "6"]);
    assert_ok(&out, "hexa design review --stage 6");
    let after = p.snapshot();
    let report = written_text(&before, &after, &[draft.as_str()]);
    assert!(!report.is_empty(), "no review report written");
    assert!(report.contains(COMMIT), "report lacks pinned commit");
    for stem in stage_reviewers(Stage::Comprehension) {
        assert!(report.contains(stem), "report lacks persona {stem}");
    }
}

/// S09: No EGM-derived report is written without attribution
#[test]
fn s09_drift_report_is_attributed() {
    let p = new_project("s09");
    p.write("ARCHITECTURE.md", "# Architecture\n\nSee `src/lib.rs`.\n");
    let out = p.hexa(&["drift"]);
    assert_ok(&out, "hexa drift");
    let (_, report) = drift_report(&p);
    assert!(report.contains(COMMIT), "drift report lacks pinned commit");
    assert!(
        report.contains("egm-drift-report") || report.contains("drift-report"),
        "drift report lacks the persona/prompt that produced it"
    );
}

// ---------------------------------------------------------------- §3

/// S10: The goldfish prompt contains the persona, standard, document and sources
#[test]
fn s10_goldfish_prompt_contains_all_inputs() {
    let prompt = goldfish_prompt(
        "PERSONA-TEXT",
        Some("STANDARD-TEXT"),
        "DOC-TEXT",
        &[("src/a.rs".to_string(), "fn a() {}".to_string())],
        1000,
    );
    for needle in ["PERSONA-TEXT", "STANDARD-TEXT", "DOC-TEXT", "src/a.rs", "fn a() {}"] {
        assert!(prompt.contains(needle), "prompt lacks {needle}");
    }
}

/// S11: A reviewer's prompt has no conversation, earlier review or other reviewer's findings
#[test]
fn s11_reviewer_prompt_has_no_other_findings() {
    let p = new_project("s11");
    p.set_reply("- Finding A-{N}: EARLIER-FINDING-XYZ OTHER-REVIEWER-XYZ the design is unclear.\n");
    let draft = p.draft_rel();
    let out = p.hexa(&["design", "review", &draft]);
    assert_ok(&out, "hexa design review");
    let markers = all_reviewer_markers();
    let later: Vec<String> = all_stages()
        .into_iter()
        .filter(|s| *s != Stage::ZoomOut)
        .flat_map(stage_reviewers)
        .map(|s| s.to_string())
        .collect();
    let calls = p.calls();
    let mut later_seen = 0;
    for call in &calls {
        let is_reviewer = markers.iter().any(|(_, m)| call.text.contains(m));
        if !is_reviewer {
            continue;
        }
        if markers
            .iter()
            .any(|(stem, m)| later.contains(stem) && call.text.contains(m))
        {
            later_seen += 1;
        }
        assert!(
            !call.text.contains("EARLIER-FINDING-XYZ"),
            "a reviewer prompt contains an earlier finding"
        );
        assert!(
            !call.text.contains("OTHER-REVIEWER-XYZ"),
            "a reviewer prompt contains another reviewer's output"
        );
    }
    assert!(later_seen > 0, "no later-stage reviewer was run");
}

/// S12: An agent route that could read the tree is refused
#[test]
fn s12_agent_route_is_refused() {
    let p = new_project("s12");
    p.write_config(AGENT_MODEL);
    let draft = p.draft_rel();
    let out = p.hexa(&["design", "review", &draft]);
    assert!(!out.status.success(), "an agent route must be refused");
    let err = String::from_utf8_lossy(&out.stderr).to_lowercase();
    assert!(
        err.contains("agent") && err.contains("refus"),
        "error does not say the agent route is refused: {err}"
    );
    assert!(p.calls().is_empty(), "a completion was made");
}

/// S13: The goldfish uses --model, else tier t2.5
#[test]
fn s13_goldfish_uses_model_flag_else_t25() {
    let p = new_project("s13");
    let draft = p.draft_rel();
    let out = p.hexa(&["design", "review", &draft, "--stage", "5"]);
    assert_ok(&out, "review with t2.5");
    let first = p.calls();
    assert!(!first.is_empty(), "no calls made");
    for c in &first {
        assert!(c.args.contains("sonnet") && !c.args.contains("opus"), "{}", c.args);
    }
    p.clear_calls();
    let out = p.hexa(&["design", "review", &draft, "--stage", "5", "--model", TEXT_M2]);
    assert_ok(&out, "review with --model");
    let second = p.calls();
    assert!(!second.is_empty(), "no calls made");
    for c in &second {
        assert!(c.args.contains("opus") && !c.args.contains("sonnet"), "{}", c.args);
    }
}

/// S14: goldfish_prompt is a function of its inputs only
#[test]
fn s14_goldfish_prompt_is_pure() {
    let mut rng = Rng(0x5eed_0014);
    let scratch = std::env::temp_dir().join(format!("hexa-egm-s14-{}", std::process::id()));
    for round in 0..100 {
        let persona = rng.text(30);
        let standard = if rng.coin() { Some(rng.text(30)) } else { None };
        let document = rng.text(40);
        let sources: Vec<(String, String)> = (0..rng.below(3))
            .map(|i| (format!("src/f{i}.rs"), rng.text(30)))
            .collect();
        let cap = 200;
        let a = goldfish_prompt(&persona, standard.as_deref(), &document, &sources, cap);
        fs::write(&scratch, format!("round {round}")).unwrap();
        let b = goldfish_prompt(&persona, standard.as_deref(), &document, &sources, cap);
        assert_eq!(a, b, "same inputs gave different prompts");

        let mutate = |s: &str, rng: &mut Rng| -> String {
            let mut chars: Vec<char> = s.chars().collect();
            let i = rng.below(chars.len() as u64) as usize;
            chars[i] = if chars[i] == 'Z' { 'Y' } else { 'Z' };
            chars.into_iter().collect()
        };
        let p2 = mutate(&persona, &mut rng);
        assert_ne!(a, goldfish_prompt(&p2, standard.as_deref(), &document, &sources, cap));
        if let Some(st) = &standard {
            let st2 = mutate(st, &mut rng);
            assert_ne!(a, goldfish_prompt(&persona, Some(&st2), &document, &sources, cap));
        }
        let d2 = mutate(&document, &mut rng);
        assert_ne!(a, goldfish_prompt(&persona, standard.as_deref(), &d2, &sources, cap));
        if !sources.is_empty() {
            let mut s2 = sources.clone();
            s2[0].1 = mutate(&s2[0].1, &mut rng);
            assert_ne!(a, goldfish_prompt(&persona, standard.as_deref(), &document, &s2, cap));
            let mut s3 = sources.clone();
            s3[0].0 = format!("{}x", s3[0].0);
            assert_ne!(a, goldfish_prompt(&persona, standard.as_deref(), &document, &s3, cap));
        }
    }
    let _ = fs::remove_file(&scratch);
}

/// S15: A source longer than the cap is cut and the cut is stated
#[test]
fn s15_long_source_is_cut_and_cut_stated() {
    let prompt = goldfish_prompt(
        "PERSONA",
        None,
        "DOCUMENT",
        &[("src/big.rs".to_string(), "x".repeat(5000))],
        100,
    );
    let longest = prompt
        .split(|c| c != 'x')
        .map(str::len)
        .max()
        .unwrap_or(0);
    assert!(longest <= 100, "a run of {longest} 'x' bytes survived the cap");
    assert!(prompt.contains("src/big.rs"));
    let lower = prompt.to_lowercase();
    assert!(
        lower.contains("100") && (lower.contains("cut") || lower.contains("truncat")),
        "prompt does not state that src/big.rs was cut at 100 bytes"
    );
}

// ---------------------------------------------------------------- §4

/// S16: Backquoted existing paths are returned in first-mention order, once each
#[test]
fn s16_backquoted_paths_in_order_once() {
    let doc = "See `src/b.rs`, then `src/a.rs`, and again `src/b.rs`.";
    let exists = |p: &str| p == "src/a.rs" || p == "src/b.rs";
    assert_eq!(referenced_paths(doc, &exists), vec!["src/b.rs", "src/a.rs"]);
}

/// S17: Paths that do not exist, or are outside the project, are not returned
#[test]
fn s17_missing_and_outside_paths_not_returned() {
    let doc = "`src/a.rs` `src/missing.rs` `../outside.rs` `/etc/passwd`";
    let exists = |p: &str| p == "src/a.rs";
    assert_eq!(referenced_paths(doc, &exists), vec!["src/a.rs"]);
}

/// S18: Unresolved paths are not read and are listed in the report
#[test]
fn s18_unresolved_paths_not_read_and_listed() {
    let p = new_project("s18");
    p.write("src/real.rs", "pub fn real() {}\n");
    fs::write(p.base.join("secret.txt"), "SECRET-XYZ\n").unwrap();
    let draft = p.draft_rel();
    p.write(
        &draft,
        "# Design\n\nUses `src/real.rs`, `src/ghost.rs` and `../secret.txt`.\n",
    );
    let before = p.snapshot();
    let out = p.hexa(&["design", "review", &draft, "--stage", "6"]);
    assert_ok(&out, "hexa design review --stage 6");
    for c in p.calls() {
        assert!(!c.text.contains("SECRET-XYZ"), "a prompt contains the outside file");
    }
    let after = p.snapshot();
    let report = written_text(&before, &after, &[draft.as_str()]);
    assert!(report.to_lowercase().contains("unresolved"), "report lists no unresolved paths");
    assert!(report.contains("src/ghost.rs"), "report does not list src/ghost.rs");
    assert!(report.contains("../secret.txt"), "report does not list ../secret.txt");
}

/// S19: Paths mentioned outside backquotes are not returned
#[test]
fn s19_unquoted_paths_not_returned() {
    let exists = |p: &str| p == "src/a.rs";
    assert!(referenced_paths("see src/a.rs for details", &exists).is_empty());
}

/// S20: referenced_paths returns a duplicate-free, existing, order-preserving subset of backquoted spans
#[test]
fn s20_referenced_paths_property() {
    let pool = [
        "src/a.rs",
        "src/b.rs",
        "src/c/d.rs",
        "docs/x.md",
        "Cargo.toml",
        "hexa-core/src/lib.rs",
    ];
    let mut rng = Rng(0x5eed_0020);
    for _ in 0..300 {
        let existing: BTreeSet<&str> = pool.iter().copied().filter(|_| rng.coin()).collect();
        let mut doc = String::new();
        let mut expected: Vec<String> = Vec::new();
        for _ in 0..rng.below(12) {
            let path = pool[rng.below(pool.len() as u64) as usize];
            match rng.below(3) {
                0 => {
                    doc.push_str(&format!("`{path}` "));
                    if existing.contains(path) && !expected.iter().any(|e| e == path) {
                        expected.push(path.to_string());
                    }
                }
                1 => doc.push_str(&format!("{path} ")),
                _ => doc.push_str("some words here "),
            }
        }
        let exists = |p: &str| existing.contains(p);
        assert_eq!(referenced_paths(&doc, &exists), expected, "document: {doc}");
    }
}

// ---------------------------------------------------------------- §5

/// S21: Presets give EGM's stages in order
#[test]
fn s21_presets_give_stages_in_order() {
    let nums = |p| stages_for(p).into_iter().map(stage_number).collect::<Vec<_>>();
    assert_eq!(nums(Preset::Full), vec![5, 6, 7, 8, 9]);
    assert_eq!(nums(Preset::FeatureSpecOnly), vec![5, 6, 7, 8]);
    assert_eq!(nums(Preset::ImplementationPlanOnly), vec![9]);
}

/// S22: Each stage has its reviewers and findings are numbered per stage
#[test]
fn s22_stage_reviewers_and_numbered_findings() {
    for s in all_stages() {
        assert!(!stage_reviewers(s).is_empty(), "{s:?} has no reviewers");
    }
    assert_eq!(stage_reviewers(Stage::Critic).len(), 3);
    assert_eq!(stage_reviewers(Stage::ImplementationReadiness).len(), 3);

    let p = new_project("s22");
    let draft = p.draft_rel();
    let before = p.snapshot();
    let out = p.hexa(&["design", "review", &draft]);
    assert_ok(&out, "hexa design review");
    let after = p.snapshot();
    let report = written_text(&before, &after, &[draft.as_str()]);
    let mut last = 0;
    for id in ["S5-1", "S6-1", "S7-1", "S8-1", "S9-1"] {
        let pos = report.find(id).unwrap_or_else(|| panic!("report lacks {id}"));
        assert!(pos >= last, "{id} is out of stage order");
        last = pos;
    }
    assert!(report.contains("S7-2"), "stage 7 findings are not numbered S7-1, S7-2, …");
}

/// S23: The review never edits the draft
#[test]
fn s23_review_does_not_edit_draft() {
    let p = new_project("s23");
    let draft = p.draft_rel();
    let bytes = fs::read(p.path(&draft)).unwrap();
    let out = p.hexa(&["design", "review", &draft]);
    assert_ok(&out, "hexa design review");
    assert_eq!(fs::read(p.path(&draft)).unwrap(), bytes, "the draft was edited");
}

/// S24: A stage does not start before the previous stage's reviewers have all returned
#[test]
fn s24_stage_waits_for_previous_stage() {
    let p = new_project("s24");
    p.install_fake(None, true);
    let draft = p.draft_rel();
    let out = p.hexa(&["design", "review", &draft]);
    assert_ok(&out, "hexa design review");
    let calls = p.calls();
    let first_of = |marker: &str| -> Call {
        calls
            .iter()
            .filter(|c| c.text.contains(marker))
            .min_by_key(|c| c.start)
            .cloned()
            .unwrap_or_else(|| panic!("persona never called: {marker}"))
    };
    let s5_end = markers_for(&stage_reviewers(Stage::ZoomOut))
        .iter()
        .map(|(_, m)| first_of(m).end)
        .max()
        .unwrap();
    let s6_start = markers_for(&stage_reviewers(Stage::Comprehension))
        .iter()
        .map(|(_, m)| first_of(m).start)
        .min()
        .unwrap();
    assert!(
        s6_start >= s5_end,
        "a stage-6 call started before the last stage-5 reviewer returned"
    );
}

/// S25: --stage N runs one stage only
#[test]
fn s25_stage_flag_runs_one_stage() {
    let p = new_project("s25");
    let draft = p.draft_rel();
    let before = p.snapshot();
    let out = p.hexa(&["design", "review", &draft, "--stage", "7"]);
    assert_ok(&out, "hexa design review --stage 7");
    let stage7: Vec<&str> = stage_reviewers(Stage::Critic);
    let stage7_markers = markers_for(&stage7);
    let others: Vec<&str> = all_stages()
        .into_iter()
        .filter(|s| *s != Stage::Critic)
        .flat_map(stage_reviewers)
        .filter(|s| !stage7.contains(s))
        .collect();
    let other_markers = markers_for(&others);
    let calls = p.calls();
    for c in &calls {
        for (stem, m) in &other_markers {
            assert!(!c.text.contains(m.as_str()), "persona {stem} of another stage was called");
        }
    }
    for (stem, m) in &stage7_markers {
        assert!(
            calls.iter().any(|c| c.text.contains(m.as_str())),
            "stage-7 persona {stem} was not called"
        );
    }
    let non_reviewer = calls
        .iter()
        .filter(|c| !stage7_markers.iter().any(|(_, m)| c.text.contains(m.as_str())))
        .count();
    assert_eq!(non_reviewer, 1, "expected exactly one synthesizer call");
    let after = p.snapshot();
    let report = written_text(&before, &after, &[draft.as_str()]);
    assert!(report.contains("S7-1"), "report lacks stage 7 section");
    for other in ["S5-", "S6-", "S8-", "S9-"] {
        assert!(!report.contains(other), "report has findings {other}*");
    }
}

/// S26: Every preset runs stages in strictly ascending stage number
#[test]
fn s26_presets_strictly_ascending() {
    for preset in [Preset::Full, Preset::FeatureSpecOnly, Preset::ImplementationPlanOnly] {
        let nums: Vec<u8> = stages_for(preset).into_iter().map(stage_number).collect();
        assert!(!nums.is_empty());
        for n in &nums {
            assert!((5..=9).contains(n), "{preset:?}: {n} out of range");
        }
        for w in nums.windows(2) {
            assert!(w[0] < w[1], "{preset:?}: {nums:?} not strictly ascending");
        }
    }
    let all: BTreeSet<u8> = all_stages().into_iter().map(stage_number).collect();
    assert_eq!(all.len(), 5, "stage numbers are not distinct");
}

// ---------------------------------------------------------------- §6

/// S27: Brainstorm and draft write their artifacts to the artifact directory
#[test]
fn s27_brainstorm_and_draft_write_to_artifact_dir() {
    let p = new_project_with("s27", Some("design/egm"));
    let out = p.hexa(&["design", "brainstorm", "an idea"]);
    assert_ok(&out, "hexa design brainstorm");
    let brainstorm_calls = p.calls();
    let out = p.hexa(&["design", "draft"]);
    assert_ok(&out, "hexa design draft");
    assert!(p.path("design/egm/concepts_brief.md").is_file());
    assert!(p.path("design/egm/design_draft.md").is_file());
    let critic = markers_for(&stage_reviewers(Stage::Critic));
    assert!(
        brainstorm_calls
            .iter()
            .any(|c| critic.iter().any(|(_, m)| c.text.contains(m.as_str()))),
        "the critic goldfish was not called after brainstorm"
    );
}

/// S28: Without the CLAUDE.md line, artifacts go to docs/designs/
#[test]
fn s28_default_artifact_dir() {
    let p = new_project("s28");
    let out = p.hexa(&["design", "brainstorm", "an idea"]);
    assert_ok(&out, "hexa design brainstorm");
    assert!(p.path("docs/designs/concepts_brief.md").is_file());
}

/// S29: Resolve hands the elephant only the accepted findings
#[test]
fn s29_resolve_hands_only_accepted_findings() {
    let p = new_project("s29");
    let report = p.report_rel();
    let out = p.hexa(&["design", "resolve", &report, "--accept", "S6-1,S7-1"]);
    assert_ok(&out, "hexa design resolve");
    let calls = p.calls();
    assert!(
        calls.iter().any(|c| c.text.contains("DRAFT-MARKER-QQQ")
            && c.text.contains("AAA-MARKER")
            && c.text.contains("CCC-MARKER")),
        "the elephant did not receive the draft and accepted findings"
    );
    for c in &calls {
        assert!(!c.text.contains("BBB-MARKER"), "an unaccepted finding was handed over");
    }
}

/// S30: No command marks a design approved
#[test]
fn s30_no_command_marks_approved() {
    let p = new_project("s30");
    let commits_before = git(&p.root, &["rev-list", "--count", "HEAD"]);
    let before = p.snapshot();
    let draft = p.draft_rel();
    let report = p.report_rel();
    for args in [
        vec!["design", "brainstorm", "an idea"],
        vec!["design", "draft"],
        vec!["design", "review", draft.as_str(), "--stage", "6"],
        vec!["design", "resolve", report.as_str(), "--accept", "S6-1"],
    ] {
        let out = p.hexa(&args);
        assert_ok(&out, &format!("hexa {args:?}"));
    }
    assert!(commits_before.is_some(), "test project is not a git repository");
    assert_eq!(
        git(&p.root, &["rev-list", "--count", "HEAD"]),
        commits_before,
        "a git commit was created"
    );
    let staged = git(&p.root, &["diff", "--cached", "--name-only"]).unwrap_or_default();
    assert!(staged.trim().is_empty(), "files were added to the index: {staged}");
    let after = p.snapshot();
    let text = written_text(&before, &after, &[]).to_lowercase();
    for marker in ["status: approved", "status:** approved", "approved: true"] {
        assert!(!text.contains(marker), "an approval marker was written: {marker}");
    }
}

// ---------------------------------------------------------------- §7

fn section<'a>(report: &'a str, name: &str, all: &[&str]) -> String {
    let lines: Vec<&str> = report.lines().collect();
    let heading = |n: &str| {
        lines
            .iter()
            .position(|l| l.trim_start().starts_with('#') && l.contains(n))
    };
    let start = heading(name).unwrap_or_else(|| panic!("no section for {name}"));
    let end = all
        .iter()
        .filter_map(|n| heading(n))
        .filter(|i| *i > start)
        .min()
        .unwrap_or(lines.len());
    lines[start..end].join("\n")
}

/// S31: Drift report has one section per document, with No drift detected where none was found
#[test]
fn s31_drift_report_sections() {
    let p = new_project("s31");
    p.write("README.md", "# Widget cache\n\nREADME-DRIFT-MARK uses `src/lib.rs`.\n");
    p.write("ARCHITECTURE.md", "# Architecture\n\nOne module.\n");
    p.write("docs/guide.md", "# Guide\n\nCall cache().\n");
    p.set_reply("No drift detected.\n");
    p.install_fake(
        Some((
            "README-DRIFT-MARK",
            "- Divergence: DIVERGENCE-XYZ README says the cache returns 7, the code returns 42.\n",
        )),
        false,
    );
    let out = p.hexa(&["drift"]);
    assert_ok(&out, "hexa drift");
    assert_eq!(out.status.code(), Some(0));
    let (_, report) = drift_report(&p);
    let names = ["README.md", "ARCHITECTURE.md", "guide.md"];
    let readme = section(&report, "README.md", &names);
    assert!(readme.contains("DIVERGENCE-XYZ"), "README section lacks the divergence");
    for other in ["ARCHITECTURE.md", "guide.md"] {
        let s = section(&report, other, &names);
        assert!(s.contains("No drift detected"), "{other} section: {s}");
        assert!(!s.contains("DIVERGENCE-XYZ"));
    }
}

/// S32: ADRs are not checked
#[test]
fn s32_adrs_not_checked() {
    let p = new_project("s32");
    p.write("docs/guide.md", "# Guide\n\nCall cache().\n");
    p.write("docs/adr/ADR-0001.md", "# ADR-0001\n\nADR-CONTENT-XYZ\n");
    p.set_reply("No drift detected.\n");
    let out = p.hexa(&["drift", "docs"]);
    assert_ok(&out, "hexa drift docs");
    for c in p.calls() {
        assert!(!c.text.contains("ADR-CONTENT-XYZ"), "an ADR was sent to a goldfish");
    }
    let (_, report) = drift_report(&p);
    assert!(!report.contains("ADR-0001"), "the report has a section for an ADR");
}

/// S33: Drift found does not fail the run unless asked
#[test]
fn s33_drift_fails_only_with_flag() {
    let p = new_project("s33");
    p.write("README.md", "# Widget cache\n\nREADME-DRIFT-MARK\n");
    p.set_reply("No drift detected.\n");
    p.install_fake(
        Some(("README-DRIFT-MARK", "- Divergence: README disagrees with src/lib.rs.\n")),
        false,
    );
    let out = p.hexa(&["drift"]);
    assert_eq!(out.status.code(), Some(0), "drift without the flag must exit 0");
    let out = p.hexa(&["drift", "--fail-on-drift"]);
    assert!(!out.status.success(), "--fail-on-drift with drift must exit non-zero");
}

/// S34: No drift with --fail-on-drift exits 0
#[test]
fn s34_no_drift_with_flag_exits_zero() {
    let p = new_project("s34");
    p.set_reply("No drift detected.\n");
    let out = p.hexa(&["drift", "--fail-on-drift"]);
    assert_eq!(out.status.code(), Some(0));
}

// ---------------------------------------------------------------- §8

fn run_and_check_writes(p: &Project, args: &[&str]) -> Vec<String> {
    let before = p.snapshot();
    let out = p.hexa(args);
    assert_ok(&out, &format!("hexa {args:?}"));
    let after = p.snapshot();
    let changes = changed(&before, &after);
    for c in &changes {
        assert!(
            allowed(c, &p.artifact),
            "hexa {args:?} wrote {c} outside {}/ and docs/analysis/",
            p.artifact
        );
    }
    changes
}

/// S35: Design and drift commands write only to their artifact directory and docs/analysis/
#[test]
fn s35_writes_only_to_allowed_dirs() {
    let p = new_project("s35");
    let draft = p.draft_rel();
    let report = p.report_rel();
    run_and_check_writes(&p, &["design", "brainstorm", "an idea"]);
    run_and_check_writes(&p, &["design", "draft"]);
    run_and_check_writes(&p, &["design", "review", &draft, "--stage", "6"]);
    run_and_check_writes(&p, &["design", "resolve", &report, "--accept", "S6-1"]);
    run_and_check_writes(&p, &["drift"]);
}

/// S36: A model reply proposing code changes does not write code
#[test]
fn s36_code_in_reply_is_not_written() {
    let p = new_project("s36");
    p.set_reply(
        "Revised draft below.\n\n```rust\n// src/lib.rs\npub fn replaced_by_model() {}\n```\n\nReplace `src/lib.rs` with the code above.\n",
    );
    let lib = fs::read(p.path("src/lib.rs")).unwrap();
    let report = p.report_rel();
    run_and_check_writes(&p, &["design", "resolve", &report, "--accept", "S6-1"]);
    assert_eq!(fs::read(p.path("src/lib.rs")).unwrap(), lib, "src/lib.rs was changed");
}

/// S37: No hexa design or hexa drift run writes outside the allowed directories
#[test]
fn s37_no_run_writes_outside_allowed_dirs() {
    let replies = [
        DEFAULT_REPLY,
        "No drift detected.\n",
        "```rust\n// src/lib.rs\npub fn x() {}\n```\nWrite this to `src/lib.rs` and `Cargo.toml`.\n",
    ];
    for (i, artifact) in [None, Some("design/egm"), Some("notes/egm-out")]
        .into_iter()
        .enumerate()
    {
        let p = new_project_with(&format!("s37-{i}"), artifact);
        p.set_reply(replies[i % replies.len()]);
        let draft = p.draft_rel();
        let report = p.report_rel();
        let runs: Vec<Vec<&str>> = vec![
            vec!["design", "brainstorm", "an idea"],
            vec!["design", "draft"],
            vec!["design", "review", draft.as_str(), "--stage", "7"],
            vec!["design", "resolve", report.as_str(), "--accept", "S6-1,S7-1"],
            vec!["drift"],
        ];
        for args in runs {
            let changes = run_and_check_writes(&p, &args);
            assert!(!changes.is_empty(), "hexa {args:?} wrote nothing");
        }
    }
}
