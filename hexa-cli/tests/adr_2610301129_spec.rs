//! Integration tests for installing hexa's Claude Code mods (ADR-2610301129).

use hexa_core::domain::claude_mods::{install_steps, Step};
use hexa_core::ports::claude_mods::{apply, ClaudePlugins};

use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// Minimal JSON parser (avoids depending on dev-dependencies we cannot see).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
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
            Json::Obj(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
    fn as_arr(&self) -> Option<&Vec<Json>> {
        match self {
            Json::Arr(a) => Some(a),
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
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }
    fn expect(&mut self, c: u8) -> Result<(), String> {
        self.ws();
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("expected '{}' at {}", c as char, self.i))
        }
    }
    fn lit(&mut self, word: &str, v: Json) -> Result<Json, String> {
        if self.s[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(v)
        } else {
            Err(format!("bad literal at {}", self.i))
        }
    }
    fn value(&mut self) -> Result<Json, String> {
        self.ws();
        match self.peek() {
            Some(b'{') => {
                self.i += 1;
                let mut fields = Vec::new();
                self.ws();
                if self.peek() == Some(b'}') {
                    self.i += 1;
                    return Ok(Json::Obj(fields));
                }
                loop {
                    self.ws();
                    let k = self.string()?;
                    self.expect(b':')?;
                    let v = self.value()?;
                    fields.push((k, v));
                    self.ws();
                    match self.peek() {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Json::Obj(fields));
                        }
                        _ => return Err(format!("bad object at {}", self.i)),
                    }
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.peek() == Some(b']') {
                    self.i += 1;
                    return Ok(Json::Arr(items));
                }
                loop {
                    items.push(self.value()?);
                    self.ws();
                    match self.peek() {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(Json::Arr(items));
                        }
                        _ => return Err(format!("bad array at {}", self.i)),
                    }
                }
            }
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b't') => self.lit("true", Json::Bool(true)),
            Some(b'f') => self.lit("false", Json::Bool(false)),
            Some(b'n') => self.lit("null", Json::Null),
            Some(_) => {
                let start = self.i;
                while let Some(c) = self.peek() {
                    if c == b'-' || c == b'+' || c == b'.' || c == b'e' || c == b'E' || c.is_ascii_digit() {
                        self.i += 1;
                    } else {
                        break;
                    }
                }
                let txt = std::str::from_utf8(&self.s[start..self.i]).map_err(|e| e.to_string())?;
                txt.parse::<f64>()
                    .map(Json::Num)
                    .map_err(|_| format!("bad number at {}", start))
            }
            None => Err("unexpected end".into()),
        }
    }
    fn string(&mut self) -> Result<String, String> {
        if self.peek() != Some(b'"') {
            return Err(format!("expected string at {}", self.i));
        }
        self.i += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let c = self.peek().ok_or("unterminated string")?;
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let e = self.peek().ok_or("bad escape")?;
                    self.i += 1;
                    match e {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'u' => {
                            let hex = std::str::from_utf8(&self.s[self.i..self.i + 4])
                                .map_err(|e| e.to_string())?;
                            self.i += 4;
                            let code = u32::from_str_radix(hex, 16).map_err(|e| e.to_string())?;
                            let ch = char::from_u32(code).unwrap_or('\u{FFFD}');
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                        _ => return Err("bad escape".into()),
                    }
                }
                _ => out.push(c),
            }
        }
        String::from_utf8(out).map_err(|e| e.to_string())
    }
}

fn parse_json(text: &str) -> Json {
    let mut p = JsonParser { s: text.as_bytes(), i: 0 };
    let v = p.value().unwrap_or_else(|e| panic!("invalid JSON: {e}\n{text}"));
    p.ws();
    assert_eq!(p.i, p.s.len(), "trailing content after JSON value");
    v
}

fn assets_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("assets").join("claude-mods")
}

fn read_json(path: &Path) -> Json {
    let text = fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    parse_json(&text)
}

// ---------------------------------------------------------------------------
// Deterministic pseudo-random generation for property tests.
// ---------------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn bool(&mut self) -> bool {
        self.next() % 2 == 0
    }
}

const MOD_POOL: &[&str] = &["a", "b", "c", "question-alert", "x-y", "z9"];
const SUFFIXES: &[&str] = &["@hexa", "@other", "", "@hexa2", "@HEXA"];
const MARKETPLACES: &[&str] = &["/m", "/home/u/.hexa/claude-mods", "rel/path", "", "/a b/c"];

fn gen_mods(rng: &mut Rng) -> Vec<&'static str> {
    let mut pool: Vec<&'static str> = MOD_POOL.to_vec();
    for i in (1..pool.len()).rev() {
        let j = rng.below(i + 1);
        pool.swap(i, j);
    }
    let n = rng.below(pool.len() + 1);
    pool.truncate(n);
    pool
}

fn gen_installed(rng: &mut Rng) -> Vec<String> {
    let n = rng.below(7);
    (0..n)
        .map(|_| {
            let name = MOD_POOL[rng.below(MOD_POOL.len())];
            let suffix = SUFFIXES[rng.below(SUFFIXES.len())];
            format!("{name}{suffix}")
        })
        .collect()
}

fn gen_marketplace(rng: &mut Rng) -> PathBuf {
    PathBuf::from(MARKETPLACES[rng.below(MARKETPLACES.len())])
}

fn expected_steps(available: bool, installed: &[String], marketplace: &Path, mods: &[&str]) -> Option<Vec<Step>> {
    // None means "exactly one Skip containing claude".
    if !available {
        return None;
    }
    let missing: Vec<String> = mods
        .iter()
        .map(|m| format!("{m}@hexa"))
        .filter(|id| !installed.contains(id))
        .collect();
    if missing.is_empty() {
        return Some(Vec::new());
    }
    let mut steps = vec![Step::AddMarketplace(marketplace.to_path_buf())];
    steps.extend(missing.into_iter().map(Step::Install));
    Some(steps)
}

fn assert_single_claude_skip(steps: &[Step]) {
    assert_eq!(steps.len(), 1, "expected exactly one step, got {steps:?}");
    match &steps[0] {
        Step::Skip(reason) => assert!(
            reason.contains("claude"),
            "skip reason should name claude: {reason:?}"
        ),
        other => panic!("expected Skip, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Fake ClaudePlugins port.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Call {
    Add(PathBuf),
    Install(String),
}

struct FakePlugins {
    available: bool,
    installed: Result<Vec<String>, String>,
    add_result: Result<(), String>,
    install_results: HashMap<String, Result<(), String>>,
    calls: Mutex<Vec<Call>>,
}

impl FakePlugins {
    fn new(available: bool, installed: Result<Vec<String>, String>) -> Self {
        FakePlugins {
            available,
            installed,
            add_result: Ok(()),
            install_results: HashMap::new(),
            calls: Mutex::new(Vec::new()),
        }
    }
    fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }
    fn install_result(&self, id: &str) -> Result<(), String> {
        self.install_results.get(id).cloned().unwrap_or(Ok(()))
    }
}

impl ClaudePlugins for FakePlugins {
    fn available(&self) -> bool {
        self.available
    }
    fn installed(&self) -> Result<Vec<String>, String> {
        self.installed.clone()
    }
    fn add_marketplace(&self, folder: &Path) -> Result<(), String> {
        self.calls.lock().unwrap().push(Call::Add(folder.to_path_buf()));
        self.add_result.clone()
    }
    fn install(&self, id: &str) -> Result<(), String> {
        self.calls.lock().unwrap().push(Call::Install(id.to_string()));
        self.install_result(id)
    }
}

// ---------------------------------------------------------------------------
// Helpers for running the `hexa` binary in a sandboxed HOME.
// ---------------------------------------------------------------------------

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn temp_root(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "hexa-cm-{tag}-{}-{n}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

/// PATH entries from the environment that do not contain a `claude` binary.
fn path_without_claude() -> Vec<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .filter(|d| !d.join("claude").exists())
        .collect()
}

/// Writes a fake `claude` into `root/bin`; returns (bin dir, log file).
fn fake_claude(root: &Path, fail_install: bool) -> (PathBuf, PathBuf) {
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let log = root.join("claude-calls.log");
    let install_branch = if fail_install {
        "echo 'install exploded xyzzy' >&2\n    exit 1"
    } else {
        "exit 0"
    };
    let script = format!(
        r#"#!/bin/sh
echo "$*" >> "{log}"
case "$1" in
  --version|-v|version) echo "1.0.0 (Claude Code)"; exit 0;;
esac
if [ "$1" = "plugin" ] && [ "$2" = "install" ]; then
    {install_branch}
fi
case "$*" in
  *list*) echo "[]";;
esac
exit 0
"#,
        log = log.display(),
        install_branch = install_branch
    );
    let path = bin.join("claude");
    fs::write(&path, script).unwrap();
    let mut perm = fs::metadata(&path).unwrap().permissions();
    perm.set_mode(0o755);
    fs::set_permissions(&path, perm).unwrap();
    (bin, log)
}

fn run_hexa(args: &[&str], home: &Path, extra_bin: Option<&Path>, cwd: &Path) -> Output {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(b) = extra_bin {
        dirs.push(b.to_path_buf());
    }
    dirs.extend(path_without_claude());
    let path = std::env::join_paths(dirs).unwrap();
    Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(args)
        .env("HOME", home)
        .env("PATH", path)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .output()
        .expect("failed to run hexa")
}

fn combined_output(out: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn read_log(log: &Path) -> String {
    fs::read_to_string(log).unwrap_or_default()
}

fn assert_extracted(home: &Path) -> PathBuf {
    let mods = home.join(".hexa").join("claude-mods");
    assert!(
        mods.join(".claude-plugin").join("marketplace.json").is_file(),
        "marketplace.json not extracted under {}",
        mods.display()
    );
    assert!(
        mods.join("question-alert").is_dir(),
        "question-alert/ not extracted under {}",
        mods.display()
    );
    mods
}

fn assert_install_calls(log_text: &str, mods_dir: &Path) {
    let mods_str = mods_dir.display().to_string();
    assert!(
        log_text
            .lines()
            .any(|l| l.contains("marketplace") && l.contains(&mods_str)),
        "claude was not asked to add marketplace {mods_str}; calls:\n{log_text}"
    );
    assert!(
        log_text
            .lines()
            .any(|l| l.contains("install") && l.contains("question-alert@hexa")),
        "claude was not asked to install question-alert@hexa; calls:\n{log_text}"
    );
}

fn assert_step_lines(output: &str) {
    assert!(
        output
            .lines()
            .any(|l| l.to_lowercase().contains("marketplace")),
        "no line printed for the add-marketplace step:\n{output}"
    );
    assert!(
        output.lines().any(|l| l.contains("question-alert@hexa")),
        "no line printed for installing question-alert@hexa:\n{output}"
    );
}

// ---------------------------------------------------------------------------
// §1 — embedded assets
// ---------------------------------------------------------------------------

/// S01: The embedded marketplace is named hexa and lists question-alert by relative source
#[test]
fn s01_marketplace_named_hexa_lists_question_alert() {
    let mp = read_json(&assets_root().join(".claude-plugin").join("marketplace.json"));
    assert_eq!(mp.get("name").and_then(Json::as_str), Some("hexa"));
    let plugins = mp.get("plugins").and_then(Json::as_arr).expect("plugins array");
    let entry = plugins
        .iter()
        .find(|p| p.get("name").and_then(Json::as_str) == Some("question-alert"))
        .expect("question-alert entry");
    assert_eq!(
        entry.get("source").and_then(Json::as_str),
        Some("./question-alert")
    );
}

/// S02: The question-alert plugin folder is a complete Claude Code plugin
#[test]
fn s02_question_alert_is_complete_plugin() {
    let folder = assets_root().join("question-alert");
    let plugin_json = folder.join(".claude-plugin").join("plugin.json");
    let hooks_json = folder.join("hooks").join("hooks.json");
    assert!(plugin_json.is_file(), "missing {}", plugin_json.display());
    assert!(hooks_json.is_file(), "missing {}", hooks_json.display());
    let plugin = read_json(&plugin_json);
    assert_eq!(
        plugin.get("name").and_then(Json::as_str),
        Some("question-alert")
    );
    // hooks.json must at least be valid JSON.
    let _ = read_json(&hooks_json);
}

/// S03: No marketplace entry points outside the marketplace folder or at a missing folder
#[test]
fn s03_marketplace_sources_are_local_existing_folders() {
    let root = assets_root();
    let mp = read_json(&root.join(".claude-plugin").join("marketplace.json"));
    let plugins = mp.get("plugins").and_then(Json::as_arr).expect("plugins array");
    assert!(!plugins.is_empty(), "marketplace lists no plugins");
    for p in plugins {
        let name = p
            .get("name")
            .and_then(Json::as_str)
            .unwrap_or_else(|| panic!("entry without name: {p:?}"));
        let source = p
            .get("source")
            .and_then(Json::as_str)
            .unwrap_or_else(|| panic!("entry {name} has non-string source: {p:?}"));
        assert_eq!(source, format!("./{name}"), "source of {name}");
        assert!(!name.is_empty() && !name.contains('/') && !name.contains(".."));
        assert!(
            root.join(name).is_dir(),
            "folder for {name} does not exist beside .claude-plugin/"
        );
    }
}

// ---------------------------------------------------------------------------
// §2 — install_steps
// ---------------------------------------------------------------------------

/// S04: Claude unavailable yields exactly one Skip naming claude
#[test]
fn s04_unavailable_yields_single_skip() {
    let steps = install_steps(
        false,
        &[],
        Path::new("/home/u/.hexa/claude-mods"),
        &["question-alert"],
    );
    assert_single_claude_skip(&steps);
}

/// S05: Nothing installed yields AddMarketplace then Install
#[test]
fn s05_nothing_installed_yields_add_then_install() {
    let steps = install_steps(true, &[], Path::new("/m"), &["question-alert"]);
    assert_eq!(
        steps,
        vec![
            Step::AddMarketplace(PathBuf::from("/m")),
            Step::Install("question-alert@hexa".to_string()),
        ]
    );
}

/// S06: Everything already installed yields no steps
#[test]
fn s06_everything_installed_yields_nothing() {
    let installed = vec!["question-alert@hexa".to_string(), "other@elsewhere".to_string()];
    let steps = install_steps(true, &installed, Path::new("/m"), &["question-alert"]);
    assert!(steps.is_empty(), "expected no steps, got {steps:?}");
}

/// S07: Only missing mods are installed, in the order of mods
#[test]
fn s07_only_missing_in_mods_order() {
    let installed = vec!["b@hexa".to_string()];
    let steps = install_steps(true, &installed, Path::new("/m"), &["c", "b", "a"]);
    assert_eq!(
        steps,
        vec![
            Step::AddMarketplace(PathBuf::from("/m")),
            Step::Install("c@hexa".to_string()),
            Step::Install("a@hexa".to_string()),
        ]
    );
}

/// S08: A mod installed from another marketplace does not count as installed
#[test]
fn s08_other_marketplace_does_not_count() {
    let installed = vec!["question-alert@other".to_string(), "question-alert".to_string()];
    let steps = install_steps(true, &installed, Path::new("/m"), &["question-alert"]);
    assert!(!steps.is_empty());
    assert_eq!(
        steps,
        vec![
            Step::AddMarketplace(PathBuf::from("/m")),
            Step::Install("question-alert@hexa".to_string()),
        ]
    );
}

/// S09: Unavailable claude never yields AddMarketplace or Install, even when mods are missing or already installed
#[test]
fn s09_unavailable_never_adds_or_installs() {
    for installed in [vec![], vec!["question-alert@hexa".to_string()]] {
        let steps = install_steps(false, &installed, Path::new("/m"), &["question-alert"]);
        assert_eq!(steps.len(), 1, "installed={installed:?}: {steps:?}");
        assert!(matches!(steps[0], Step::Skip(_)), "{steps:?}");
        assert!(!steps
            .iter()
            .any(|s| matches!(s, Step::AddMarketplace(_) | Step::Install(_))));
    }
}

/// S10: Shape of the steps for every input
#[test]
fn s10_shape_of_steps_property() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for _ in 0..2000 {
        let available = rng.bool();
        let installed = gen_installed(&mut rng);
        let marketplace = gen_marketplace(&mut rng);
        let mods = gen_mods(&mut rng);
        let steps = install_steps(available, &installed, &marketplace, &mods);
        match expected_steps(available, &installed, &marketplace, &mods) {
            None => assert_single_claude_skip(&steps),
            Some(expected) => assert_eq!(
                steps, expected,
                "available={available} installed={installed:?} marketplace={marketplace:?} mods={mods:?}"
            ),
        }
    }
}

/// S11: Idempotence: running the steps once leaves nothing to do
#[test]
fn s11_idempotence_property() {
    let mut rng = Rng(0xDEAD_BEEF_CAFE_F00D);
    for _ in 0..2000 {
        let mut installed = gen_installed(&mut rng);
        let marketplace = gen_marketplace(&mut rng);
        let mods = gen_mods(&mut rng);
        let first = install_steps(true, &installed, &marketplace, &mods);
        for s in &first {
            if let Step::Install(id) = s {
                installed.push(id.clone());
            }
        }
        let second = install_steps(true, &installed, &marketplace, &mods);
        assert!(
            second.is_empty(),
            "second run not empty: {second:?} (installed={installed:?} mods={mods:?})"
        );
    }
}

// ---------------------------------------------------------------------------
// §3 — apply
// ---------------------------------------------------------------------------

/// S12: apply runs the steps through the port and reports each outcome
#[test]
fn s12_apply_runs_steps_and_reports() {
    let fake = FakePlugins::new(true, Ok(vec![]));
    let result = apply(&fake, Path::new("/m"), &["question-alert"]);
    assert_eq!(
        fake.calls(),
        vec![
            Call::Add(PathBuf::from("/m")),
            Call::Install("question-alert@hexa".to_string()),
        ]
    );
    assert_eq!(
        result,
        vec![
            (Step::AddMarketplace(PathBuf::from("/m")), Ok(())),
            (Step::Install("question-alert@hexa".to_string()), Ok(())),
        ]
    );
}

/// S13: Nothing to do means no port mutations
#[test]
fn s13_nothing_to_do_no_mutations() {
    let fake = FakePlugins::new(true, Ok(vec!["question-alert@hexa".to_string()]));
    let result = apply(&fake, Path::new("/m"), &["question-alert"]);
    assert!(result.is_empty(), "expected empty result, got {result:?}");
    assert!(fake.calls().is_empty(), "unexpected calls: {:?}", fake.calls());
}

/// S14: A failed step does not stop the later steps
#[test]
fn s14_failed_step_does_not_stop_later() {
    let mut fake = FakePlugins::new(true, Ok(vec![]));
    fake.add_result = Err("boom".to_string());
    fake.install_results
        .insert("a@hexa".to_string(), Err("nope".to_string()));
    let result = apply(&fake, Path::new("/m"), &["a", "b"]);
    let calls = fake.calls();
    assert!(calls.contains(&Call::Install("a@hexa".to_string())), "{calls:?}");
    assert!(calls.contains(&Call::Install("b@hexa".to_string())), "{calls:?}");
    assert_eq!(
        result,
        vec![
            (Step::AddMarketplace(PathBuf::from("/m")), Err("boom".to_string())),
            (Step::Install("a@hexa".to_string()), Err("nope".to_string())),
            (Step::Install("b@hexa".to_string()), Ok(())),
        ]
    );
}

/// S15: A failing installed() is treated as nothing installed, not as a fatal error
#[test]
fn s15_failing_installed_treated_as_empty() {
    let fake = FakePlugins::new(true, Err("cli broke".to_string()));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        apply(&fake, Path::new("/m"), &["question-alert"])
    }))
    .expect("apply panicked");
    assert_eq!(
        result,
        vec![
            (Step::AddMarketplace(PathBuf::from("/m")), Ok(())),
            (Step::Install("question-alert@hexa".to_string()), Ok(())),
        ]
    );
}

/// S16: Unavailable claude: the Skip is reported and nothing is attempted
#[test]
fn s16_unavailable_reports_skip_only() {
    let fake = FakePlugins::new(false, Ok(vec![]));
    let result = apply(&fake, Path::new("/m"), &["question-alert"]);
    assert_eq!(result.len(), 1, "{result:?}");
    match &result[0].0 {
        Step::Skip(reason) => assert!(reason.contains("claude"), "{reason:?}"),
        other => panic!("expected Skip, got {other:?}"),
    }
    assert!(fake.calls().is_empty(), "unexpected calls: {:?}", fake.calls());
}

/// S17: apply reports exactly install_steps' steps, each run once, and never panics
#[test]
fn s17_apply_matches_install_steps_property() {
    let mut rng = Rng(0x0123_4567_89AB_CDEF);
    for _ in 0..1000 {
        let available = rng.bool();
        let installed_list = gen_installed(&mut rng);
        let installed = if rng.below(4) == 0 {
            Err("listing failed".to_string())
        } else {
            Ok(installed_list)
        };
        let marketplace = gen_marketplace(&mut rng);
        let mods = gen_mods(&mut rng);

        let mut fake = FakePlugins::new(available, installed.clone());
        if rng.bool() {
            fake.add_result = Err(format!("add-err-{}", rng.below(100)));
        }
        for m in MOD_POOL {
            if rng.bool() {
                let id = format!("{m}@hexa");
                fake.install_results
                    .insert(id.clone(), Err(format!("install-err-{id}")));
            }
        }

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            apply(&fake, &marketplace, &mods)
        }))
        .expect("apply panicked");

        let effective = installed.clone().unwrap_or_default();
        let expected = install_steps(available, &effective, &marketplace, &mods);
        let steps: Vec<Step> = result.iter().map(|(s, _)| s.clone()).collect();
        assert_eq!(steps, expected, "mods={mods:?} installed={installed:?}");

        let calls = fake.calls();
        let add_steps = steps
            .iter()
            .filter(|s| matches!(s, Step::AddMarketplace(_)))
            .count();
        let add_calls = calls.iter().filter(|c| matches!(c, Call::Add(_))).count();
        assert_eq!(add_steps, add_calls, "calls={calls:?}");
        for s in &steps {
            if let Step::AddMarketplace(p) = s {
                assert!(calls.contains(&Call::Add(p.clone())));
            }
        }
        let install_calls: Vec<&String> = calls
            .iter()
            .filter_map(|c| match c {
                Call::Install(id) => Some(id),
                _ => None,
            })
            .collect();
        let install_steps_ids: Vec<&String> = steps
            .iter()
            .filter_map(|s| match s {
                Step::Install(id) => Some(id),
                _ => None,
            })
            .collect();
        assert_eq!(install_calls.len(), install_steps_ids.len(), "calls={calls:?}");
        for id in &install_steps_ids {
            let n_calls = install_calls.iter().filter(|c| **c == *id).count();
            let n_steps = install_steps_ids.iter().filter(|s| **s == *id).count();
            assert_eq!(n_calls, n_steps, "id={id}");
        }

        for (step, outcome) in &result {
            match step {
                Step::AddMarketplace(_) => assert_eq!(outcome, &fake.add_result),
                Step::Install(id) => assert_eq!(outcome, &fake.install_result(id)),
                Step::Skip(_) => {}
            }
        }
    }
}

// ---------------------------------------------------------------------------
// §4 — wiring into hexa bootstrap / hexa dev init
// ---------------------------------------------------------------------------

/// S18: hexa bootstrap extracts the mods and installs question-alert
#[test]
fn s18_bootstrap_extracts_and_installs() {
    let root = temp_root("s18");
    let home = root.join("home");
    fs::create_dir_all(&home).unwrap();
    let (bin, log) = fake_claude(&root, false);

    let out = run_hexa(&["bootstrap"], &home, Some(&bin), &home);
    let text = combined_output(&out);

    let mods_dir = assert_extracted(&home);
    assert_install_calls(&read_log(&log), &mods_dir);
    assert_step_lines(&text);
    assert_eq!(out.status.code(), Some(0), "bootstrap failed:\n{text}");
}

/// S19: hexa dev init does the same extraction and install
#[test]
fn s19_dev_init_extracts_and_installs() {
    let root = temp_root("s19");
    let home = root.join("home");
    let project = root.join("project");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&project).unwrap();
    let (bin, log) = fake_claude(&root, false);

    let out = run_hexa(&["dev", "init"], &home, Some(&bin), &project);
    let text = combined_output(&out);

    let mods_dir = assert_extracted(&home);
    assert_install_calls(&read_log(&log), &mods_dir);
    assert_step_lines(&text);
}

/// S20: Extraction overwrites stale files hexa owns
#[test]
fn s20_extraction_overwrites_stale_files() {
    let root = temp_root("s20");
    let home = root.join("home");
    let stale = home
        .join(".hexa")
        .join("claude-mods")
        .join("question-alert")
        .join("hooks")
        .join("hooks.json");
    fs::create_dir_all(stale.parent().unwrap()).unwrap();
    fs::write(&stale, "{\"stale\": true}").unwrap();
    let (bin, _log) = fake_claude(&root, false);

    let _ = run_hexa(&["bootstrap"], &home, Some(&bin), &home);

    let embedded = fs::read(assets_root().join("question-alert").join("hooks").join("hooks.json"))
        .expect("embedded hooks.json");
    let actual = fs::read(&stale).expect("extracted hooks.json");
    assert_eq!(actual, embedded, "stale hooks.json was not overwritten");
}

/// S21: Missing claude produces a warning and does not fail setup
///
/// Runs `hexa dev init` rather than `hexa bootstrap`: bootstrap also needs
/// claude or Ollama as its model path, so it cannot isolate the mods.
#[test]
fn s21_missing_claude_warns_without_failing() {
    // Reference run with claude present.
    let root_ok = temp_root("s21-ok");
    let home_ok = root_ok.join("home");
    let project_ok = root_ok.join("project");
    fs::create_dir_all(&home_ok).unwrap();
    fs::create_dir_all(&project_ok).unwrap();
    let (bin, _log) = fake_claude(&root_ok, false);
    let with_claude = run_hexa(&["dev", "init"], &home_ok, Some(&bin), &project_ok);

    // Run with no claude on PATH.
    let root = temp_root("s21-missing");
    let home = root.join("home");
    let project = root.join("project");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&project).unwrap();
    let without = run_hexa(&["dev", "init"], &home, None, &project);
    let text = combined_output(&without);

    assert!(
        text.lines().any(|l| l.contains("claude")),
        "no warning naming claude was printed:\n{text}"
    );
    assert_eq!(
        without.status.code(),
        with_claude.status.code(),
        "exit status differs without claude:\n{text}"
    );
}

/// S22: A failed install produces a warning and does not fail setup
#[test]
fn s22_failed_install_warns_without_failing() {
    let root_ok = temp_root("s22-ok");
    let home_ok = root_ok.join("home");
    fs::create_dir_all(&home_ok).unwrap();
    let (bin_ok, _) = fake_claude(&root_ok, false);
    let ok = run_hexa(&["bootstrap"], &home_ok, Some(&bin_ok), &home_ok);

    let root = temp_root("s22-fail");
    let home = root.join("home");
    fs::create_dir_all(&home).unwrap();
    let (bin, log) = fake_claude(&root, true);
    let out = run_hexa(&["bootstrap"], &home, Some(&bin), &home);
    let text = combined_output(&out);

    assert!(
        read_log(&log)
            .lines()
            .any(|l| l.contains("install") && l.contains("question-alert@hexa")),
        "install was not attempted"
    );
    assert!(
        text.lines()
            .any(|l| l.contains("install exploded xyzzy")),
        "failed step not printed with its error:\n{text}"
    );
    assert_eq!(
        out.status.code(),
        ok.status.code(),
        "exit status changed because of the failed install:\n{text}"
    );
}
