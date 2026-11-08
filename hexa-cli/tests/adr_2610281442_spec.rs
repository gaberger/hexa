//! Tests for ADR-2610221540 follow-up: public signatures for the test writer,
//! unpassable sites in generated tests, red proved at an ancestor commit, and
//! replacing a sealed test file.
//!
//! Assumptions about the CLI surface. The signatures given show only the
//! domain, so these are inferred and kept in one place each, so they are easy
//! to adjust:
//! - The agent that writes specs and tests is an external command named by
//!   the `HEXA_AGENT` environment variable (`AGENT_ENV`). It receives its
//!   inputs on stdin and/or as arguments, and replies on stdout. The fake
//!   agent here records its inputs to a file and replies with a prepared text.
//! - ADRs live at `docs/adr/<ADR-id>.md` and specs at `docs/specs/<ADR-id>.json`.
//! - `hexa spec accept <ADR>` records acceptance. Its outcome is not asserted.
//! - A generated test file's location is read from the spec's seal or test refs.
#![allow(unused, clippy::all)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use hexa_core::domain::spec::{public_signatures, unpassable_sites, Spec};

const ADR: &str = "ADR-2610010000";
const AGENT_ENV: &str = "HEXA_AGENT";
const GIT_NAME: &str = "Hexa Test";
const GIT_EMAIL: &str = "hexa-test@example.invalid";

static COUNTER: AtomicUsize = AtomicUsize::new(0);

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn fresh_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("hexa-cli-{tag}-{}-{nanos}-{n}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn text(o: &Output) -> String {
    format!("{}\n{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

fn rust_block(code: &str) -> String {
    format!("```rust\n{code}```\n")
}

struct Repo {
    root: PathBuf,
    capture: PathBuf,
    reply: PathBuf,
    agent: PathBuf,
}

impl Repo {
    fn new(tag: &str) -> Repo {
        let base = fresh_dir(tag);
        let root = base.join("repo");
        let scripts = base.join("scripts");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&scripts).unwrap();
        let capture = scripts.join("capture.txt");
        let reply = scripts.join("reply.txt");
        fs::write(&capture, "").unwrap();
        fs::write(&reply, "").unwrap();
        let agent = scripts.join("agent.sh");
        fs::write(
            &agent,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" >> '{c}'\nif [ ! -t 0 ]; then cat >> '{c}'; fi\ncat '{r}'\n",
                c = capture.display(),
                r = reply.display()
            ),
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&agent, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let repo = Repo { root, capture, reply, agent };
        repo.git(&["init", "-q"]);
        repo.git(&["config", "user.name", GIT_NAME]);
        repo.git(&["config", "user.email", GIT_EMAIL]);
        repo.git(&["config", "commit.gpgsign", "false"]);
        repo
    }

    fn git_out(&self, args: &[&str]) -> Output {
        Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .env("GIT_AUTHOR_NAME", GIT_NAME)
            .env("GIT_AUTHOR_EMAIL", GIT_EMAIL)
            .env("GIT_COMMITTER_NAME", GIT_NAME)
            .env("GIT_COMMITTER_EMAIL", GIT_EMAIL)
            .stdin(Stdio::null())
            .output()
            .expect("git runs")
    }

    fn git(&self, args: &[&str]) -> String {
        let o = self.git_out(args);
        assert!(o.status.success(), "git {args:?} failed: {}", text(&o));
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    }

    fn write(&self, rel: &str, content: &str) {
        let p = self.root.join(rel);
        if let Some(d) = p.parent() {
            fs::create_dir_all(d).unwrap();
        }
        fs::write(p, content).unwrap();
    }

    fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.root.join(rel)).unwrap_or_default()
    }

    fn commit_all(&self, msg: &str) -> String {
        self.git(&["add", "-A"]);
        if !self.git(&["status", "--porcelain"]).is_empty() {
            self.git(&["commit", "-q", "-m", msg]);
        }
        self.head()
    }

    fn head(&self) -> String {
        self.git(&["rev-parse", "HEAD"])
    }

    fn red_ref(&self) -> Option<String> {
        let o = self.git_out(&["rev-parse", "--verify", "--quiet", &format!("refs/hexa/red/{ADR}")]);
        o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
    }

    fn set_reply(&self, reply: &str) {
        fs::write(&self.reply, reply).unwrap();
        fs::write(&self.capture, "").unwrap();
    }

    fn captured(&self) -> String {
        fs::read_to_string(&self.capture).unwrap_or_default()
    }

    fn hexa(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_hexa"))
            .args(args)
            .current_dir(&self.root)
            .env(AGENT_ENV, &self.agent)
            .env("GIT_AUTHOR_NAME", GIT_NAME)
            .env("GIT_AUTHOR_EMAIL", GIT_EMAIL)
            .env("GIT_COMMITTER_NAME", GIT_NAME)
            .env("GIT_COMMITTER_EMAIL", GIT_EMAIL)
            .stdin(Stdio::null())
            .output()
            .expect("hexa runs")
    }

    fn spec_path(&self) -> String {
        format!("docs/specs/{ADR}.json")
    }

    fn spec(&self) -> Spec {
        let raw = self.read(&self.spec_path());
        serde_json::from_str(&raw).unwrap_or_else(|e| panic!("spec does not parse: {e}\n{raw}"))
    }

    fn test_file(&self) -> String {
        let spec = self.spec();
        spec.sealed
            .keys()
            .next()
            .cloned()
            .or_else(|| spec.test_files().into_iter().next())
            .expect("the spec names a generated test file")
    }

    fn sha256(&self, rel: &str) -> String {
        let o = Command::new("sha256sum").arg(self.root.join(rel)).output().expect("sha256sum runs");
        String::from_utf8_lossy(&o.stdout).split_whitespace().next().unwrap_or_default().to_string()
    }
}

const STUB_LIB: &str = "pub fn add(a: u32, b: u32) -> u32 {\n    let _ = (a, b);\n    0\n}\n";
const REAL_LIB: &str = "pub fn add(a: u32, b: u32) -> u32 {\n    a + b\n}\n";

const ADR_TEXT: &str = "# ADR-2610010000: Add two numbers\n\nADR_TEXT_MARKER_ADD\n\n## Context\n\nThe adder is `hexa-core/src/ports/adder.rs`, `hexa-core/src/domain/sum.rs` and `hexa-exec/src/usecase.rs`.\n\n## Decision\n\n1. **Add.** `add(a, b)` returns the sum of a and b.\n\n## Consequences\n\nNone.\n";

const PORT_FILE: &str = "//! PORT_FILE_MARKER\npub trait Adder {\n    fn add(&self, a: u32, b: u32) -> u32;\n}\n";

const DOMAIN_FILE: &str = "//! DOMAIN_FILE_MARKER\npub fn sum(a: u32, b: u32) -> u32 {\n    let domain_body_kept = a + b;\n    domain_body_kept\n}\n";

const USECASE_FILE: &str = "pub struct RunUsecase<S> {\n    store: S,\n}\n\nimpl<S> RunUsecase<S> {\n    pub fn new(store: S) -> Self {\n        let _marker = \"BODY_MARKER_7\";\n        RunUsecase { store }\n    }\n}\n";

const SPEC_JSON: &str = r#"{
  "adr": "ADR-2610010000",
  "feature": "add",
  "specs": [
    {"id": "S1", "decision": ["§1"], "kind": "example", "description": "two and three make five",
     "given": "add", "when": "add(2, 3)", "then": "it returns 5"},
    {"id": "S2", "decision": ["§1"], "kind": "negative", "description": "one and one is not zero",
     "given": "add", "when": "add(1, 1)", "then": "it does not return 0"}
  ]
}
"#;

const GOOD_TESTS: &str = "use demo::add;\n\n/// S1: two and three make five\n#[test]\nfn two_and_three_make_five() {\n    assert_eq!(add(2, 3), 5);\n}\n\n/// S2: one and one is not zero\n#[test]\nfn one_and_one_is_not_zero() {\n    assert_ne!(add(1, 1), 0);\n}\n";

const REPLACEMENT_TESTS: &str = "use demo::add;\n\n/// S1: two and three make five\n#[test]\nfn sum_of_three_and_two() {\n    assert_eq!(add(3, 2), 5);\n}\n\n/// S2: one and one is not zero\n#[test]\nfn sum_of_ones_is_two() {\n    assert_eq!(add(1, 1), 2);\n}\n";

const ONE_PASSES_AT_RED: &str = "use demo::add;\n\n/// S1: two and three make five\n#[test]\nfn two_and_three_make_five() {\n    assert_eq!(add(2, 3), 5);\n}\n\n/// S2: one and one is not zero\n#[test]\nfn zero_plus_zero_is_zero() {\n    assert_eq!(add(0, 0), 0);\n}\n";

/// Line 12 holds `todo!()` inside a helper.
const TODO_AT_LINE_12: &str = "use demo::add;\n\n/// S1: two and three make five\n#[test]\nfn two_and_three_make_five() {\n    assert_eq!(add(2, 3), helper());\n}\n\nfn helper() -> u32 {\n    let base = 5;\n    let _ = base;\n    todo!()\n}\n\n/// S2: one and one is not zero\n#[test]\nfn one_and_one_is_not_zero() {\n    assert_ne!(add(1, 1), 0);\n}\n";

const WITH_UNIMPLEMENTED: &str = "use demo::add;\n\n/// S1: two and three make five\n#[test]\nfn sum_of_three_and_two() {\n    assert_eq!(add(3, 2), expected());\n}\n\nfn expected() -> u32 {\n    let _ = 1;\n    unimplemented!(\"later\")\n}\n\n/// S2: one and one is not zero\n#[test]\nfn sum_of_ones_is_two() {\n    assert_eq!(add(1, 1), 2);\n}\n";

/// A cargo project with a stub `add` at C and the real one at HEAD, the ADR,
/// the files it names, and (optionally) an accepted spec. Returns C.
fn project(tag: &str, with_spec: bool) -> (Repo, String) {
    let repo = Repo::new(tag);
    repo.write("Cargo.toml", "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n");
    repo.write(".gitignore", "target/\n");
    repo.write("src/lib.rs", STUB_LIB);
    let c = repo.commit_all("stub add");
    repo.write("src/lib.rs", REAL_LIB);
    repo.write(&format!("docs/adr/{ADR}.md"), ADR_TEXT);
    repo.write("hexa-core/src/ports/adder.rs", PORT_FILE);
    repo.write("hexa-core/src/domain/sum.rs", DOMAIN_FILE);
    repo.write("hexa-exec/src/usecase.rs", USECASE_FILE);
    if with_spec {
        repo.write(&repo.spec_path(), SPEC_JSON);
    }
    repo.commit_all("implement add");
    if with_spec {
        let _ = repo.hexa(&["spec", "accept", ADR]);
        repo.commit_all("accept spec");
    }
    (repo, c)
}

/// A project whose GOOD_TESTS have been generated, proved red at C and sealed.
fn sealed_project(tag: &str) -> (Repo, String) {
    let (repo, c) = project(tag, true);
    repo.set_reply(&rust_block(GOOD_TESTS));
    let o = repo.hexa(&["spec", "tests", ADR, "--red-at", &c]);
    assert!(o.status.success(), "generation at red commit {c} should succeed: {}", text(&o));
    assert!(!repo.spec().sealed.is_empty(), "the generated file is sealed");
    (repo, c)
}

// ---------------------------------------------------------------------------
// Pseudo-random generation for the properties
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
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

// ---------------------------------------------------------------------------
// §1 public signatures
// ---------------------------------------------------------------------------

/// S01: A pub function keeps its signature and loses its body
#[test]
fn s01_pub_function_keeps_signature_and_loses_body() {
    let src = "pub fn add(a: u32, b: u32) -> u32 { let s = a + b; s }\n";
    let out = public_signatures(src);
    assert!(out.contains("pub fn add(a: u32, b: u32) -> u32 { … }"), "signature kept with elided body:\n{out}");
    assert!(!out.contains("let s = a + b"), "body removed:\n{out}");
}

/// S02: Pub structs, enums, traits and impl blocks are kept, and method bodies are elided
#[test]
fn s02_pub_types_traits_and_impls_kept_with_method_bodies_elided() {
    let src = "pub struct Store { pub path: String }\n\npub enum Mode { A, B }\n\npub trait Port { fn get(&self) -> u8; }\n\nimpl Store { pub fn new(path: String) -> Self { Store { path } } }\n";
    let out = public_signatures(src);
    for want in ["pub struct Store", "pub enum Mode", "pub trait Port", "impl Store", "pub fn new(path: String) -> Self { … }"] {
        assert!(out.contains(want), "expected `{want}` in:\n{out}");
    }
    assert!(!out.contains("Store { path }"), "method body removed:\n{out}");
}

/// S03: Private items and mod tests are removed
#[test]
fn s03_private_items_and_mod_tests_are_removed() {
    let src = "pub fn visible() {}\n\nfn hidden_helper() {}\n\nstruct HiddenType;\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn secret_test() {}\n}\n";
    let out = public_signatures(src);
    assert!(out.contains("visible"), "pub item kept:\n{out}");
    for gone in ["hidden_helper", "HiddenType", "mod tests", "secret_test"] {
        assert!(!out.contains(gone), "`{gone}` should be removed:\n{out}");
    }
}

/// S04: hexa spec tests shows the writer the public signatures of each other named .rs file
#[test]
fn s04_spec_tests_shows_writer_public_signatures_of_other_files() {
    let (repo, c) = project("s04", true);
    repo.set_reply(&rust_block(GOOD_TESTS));
    let _ = repo.hexa(&["spec", "tests", ADR, "--red-at", &c]);
    let inputs = repo.captured();
    assert!(!inputs.trim().is_empty(), "the writer was invoked with inputs");
    assert!(inputs.contains("ADR_TEXT_MARKER_ADD"), "inputs hold the ADR text");
    assert!(inputs.contains("PORT_FILE_MARKER") && inputs.contains("fn add(&self, a: u32, b: u32) -> u32;"), "port file whole");
    assert!(inputs.contains("DOMAIN_FILE_MARKER") && inputs.contains("let domain_body_kept = a + b;"), "domain file whole");
    assert!(inputs.contains("pub fn new(store: S) -> Self { … }"), "usecase.rs signatures shown:\n{inputs}");
}

/// S05: The writer never sees the body of a non-port, non-domain file
#[test]
fn s05_writer_never_sees_non_port_non_domain_bodies() {
    let (repo, c) = project("s05", true);
    repo.set_reply(&rust_block(GOOD_TESTS));
    let _ = repo.hexa(&["spec", "tests", ADR, "--red-at", &c]);
    let inputs = repo.captured();
    assert!(!inputs.trim().is_empty(), "the writer was invoked with inputs");
    assert!(inputs.contains("pub fn new(store: S) -> Self"), "usecase.rs is shown as signatures");
    assert!(!inputs.contains("BODY_MARKER_7"), "a body of usecase.rs leaked into the writer's inputs");
}

/// S06: hexa spec draft does not gain the extra files
#[test]
fn s06_spec_draft_does_not_gain_extra_files() {
    let (repo, _c) = project("s06", false);
    repo.set_reply(&format!("```json\n{SPEC_JSON}```\n"));
    let _ = repo.hexa(&["spec", "draft", ADR]);
    let inputs = repo.captured();
    assert!(!inputs.trim().is_empty(), "the spec author was invoked with inputs");
    assert!(inputs.contains("ADR_TEXT_MARKER_ADD"), "inputs hold the ADR");
    assert!(inputs.contains("PORT_FILE_MARKER"), "inputs hold the port file");
    assert!(inputs.contains("DOMAIN_FILE_MARKER") && inputs.contains("let domain_body_kept"), "inputs hold the domain file");
    for absent in ["RunUsecase", "pub fn new(store: S)", "BODY_MARKER_7"] {
        assert!(!inputs.contains(absent), "draft inputs should not hold `{absent}` from usecase.rs");
    }
}

/// S07: No function body and no private item survives, and the operation is idempotent
#[test]
fn s07_no_body_or_private_item_survives_and_idempotent() {
    for seed in 1..=64u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut src = String::new();
        let mut markers = Vec::new();
        let mut privates = Vec::new();
        let items = 1 + rng.below(8);
        for i in 0..items {
            let marker = format!("BODYMARK_{seed}_{i}_END");
            match rng.below(7) {
                0 => {
                    src.push_str(&format!("pub fn pub_fn_{i}(x: u32) -> u32 {{\n    let m = \"{marker}\";\n    let _ = m;\n    x\n}}\n\n"));
                    markers.push(marker);
                }
                1 => {
                    let name = format!("hidden_{i}_x");
                    src.push_str(&format!("fn {name}() -> u32 {{\n    1\n}}\n\n"));
                    privates.push(name);
                }
                2 => {
                    let hidden = format!("hidden_{i}_m");
                    src.push_str(&format!(
                        "pub struct Pub{i} {{\n    pub v: u32,\n}}\n\nimpl Pub{i} {{\n    pub fn get(&self) -> u32 {{\n        let m = \"{marker}\";\n        let _ = m;\n        self.v\n    }}\n\n    fn {hidden}(&self) -> u32 {{\n        2\n    }}\n}}\n\n"
                    ));
                    markers.push(marker);
                    privates.push(hidden);
                }
                3 => {
                    let name = format!("Hidden{i}X");
                    src.push_str(&format!("struct {name};\n\n"));
                    privates.push(name);
                }
                4 => {
                    src.push_str(&format!(
                        "pub trait Tr{i} {{\n    fn d(&self) -> u32 {{\n        let m = \"{marker}\";\n        let _ = m;\n        0\n    }}\n}}\n\n"
                    ));
                    markers.push(marker);
                }
                5 => {
                    let name = format!("HiddenEnum{i}X");
                    src.push_str(&format!("enum {name} {{ A, B }}\n\n"));
                    privates.push(name);
                }
                _ => {
                    let name = format!("secret_{i}_test");
                    src.push_str(&format!(
                        "#[cfg(test)]\nmod tests {{\n    #[test]\n    fn {name}() {{\n        let m = \"{marker}\";\n        assert!(!m.is_empty());\n    }}\n}}\n\n"
                    ));
                    markers.push(marker);
                    privates.push(name);
                }
            }
        }
        let once = public_signatures(&src);
        for m in &markers {
            assert!(!once.contains(m.as_str()), "seed {seed}: body marker {m} survived:\n{src}\n---\n{once}");
        }
        for p in &privates {
            assert!(!once.contains(p.as_str()), "seed {seed}: private item {p} survived:\n{src}\n---\n{once}");
        }
        let twice = public_signatures(&once);
        assert_eq!(twice, once, "seed {seed}: not idempotent");
    }
}

// ---------------------------------------------------------------------------
// §2 unpassable sites
// ---------------------------------------------------------------------------

/// S08: todo! and unimplemented! are reported with their 1-based lines
#[test]
fn s08_todo_and_unimplemented_reported_with_lines() {
    let src = "#[test]\nfn a() {\n    todo!()\n}\n#[test]\nfn b() {\n    unimplemented!(\"x\")\n}\n";
    let sites = unpassable_sites(src);
    assert!(sites.iter().any(|(l, t)| *l == 3 && t.contains("todo!(")), "todo! at line 3: {sites:?}");
    assert!(sites.iter().any(|(l, t)| *l == 7 && t.contains("unimplemented!(")), "unimplemented! at line 7: {sites:?}");
    assert_eq!(sites.len(), 2, "exactly two sites: {sites:?}");
}

/// S09: A panic! that is the first statement of a function or closure body is a site
#[test]
fn s09_leading_panic_in_function_or_closure_is_a_site() {
    let src = "fn make() -> Svc {\n    panic!(\"stub\")\n}\n\nfn g() {\n    let f = || { panic!(\"stub\") };\n    f();\n}\n";
    // Line 5 is the closure in the scenario; place it there exactly.
    let src = src.replacen("\n\nfn g() {\n    let f", "\nfn g() {\n    let f", 1);
    let src = src.replacen("fn g() {\n", "fn g() {\n", 1);
    let lines: Vec<&str> = src.lines().collect();
    let closure_line = lines.iter().position(|l| l.contains("let f = ||")).unwrap() + 1;
    let src = if closure_line == 5 {
        src
    } else {
        "fn make() -> Svc {\n    panic!(\"stub\")\n}\nfn g() {\n    let f = || { panic!(\"stub\") };\n    f();\n}\n".to_string()
    };
    assert_eq!(src.lines().nth(4).unwrap().trim(), "let f = || { panic!(\"stub\") };");
    let sites = unpassable_sites(&src);
    assert!(sites.iter().any(|(l, t)| *l == 2 && t.contains("panic!(")), "leading panic at line 2: {sites:?}");
    assert!(sites.iter().any(|(l, t)| *l == 5 && t.contains("panic!(")), "closure panic at line 5: {sites:?}");
}

/// S10: A panic! inside unwrap_or_else, a match arm or an assertion message is not a site
#[test]
fn s10_panics_after_real_code_are_not_sites() {
    let src = "#[test]\nfn a() {\n    let x: Result<u8, String> = Ok(1);\n    let _ = x.unwrap_or_else(|e| panic!(\"{e}\"));\n}\n\n#[test]\nfn b() {\n    let r: Result<u8, String> = Ok(1);\n    match r {\n        Ok(v) => drop(v),\n        Err(e) => panic!(\"{e}\"),\n    }\n}\n\n#[test]\nfn c() {\n    let ok = true;\n    let panic_msg = \"m\";\n    assert!(ok, \"{}\", panic_msg);\n}\n";
    assert_eq!(unpassable_sites(src), Vec::<(usize, String)>::new());
}

/// S11: hexa spec tests refuses a generation with an unpassable site and names each site
#[test]
fn s11_spec_tests_refuses_unpassable_site_and_names_it() {
    let (repo, c) = project("s11", true);
    assert_eq!(TODO_AT_LINE_12.lines().nth(11).unwrap().trim(), "todo!()");
    let head = repo.head();
    repo.set_reply(&rust_block(TODO_AT_LINE_12));
    let o = repo.hexa(&["spec", "tests", ADR, "--red-at", &c]);
    let out = text(&o);
    assert!(!o.status.success(), "generation with todo! should be refused: {out}");
    assert!(out.contains("12"), "message names line 12: {out}");
    assert!(out.contains("todo!("), "message names `todo!(`: {out}");
    let spec = repo.spec();
    assert!(spec.sealed.is_empty(), "no seal recorded");
    assert!(spec.red.is_none(), "no red recorded");
    assert!(repo.red_ref().is_none(), "no red ref written");
    assert_eq!(repo.head(), head, "HEAD unchanged");
}

/// S12: A clean generated file has no sites and is not refused for this reason
#[test]
fn s12_clean_file_has_no_sites() {
    assert_eq!(unpassable_sites(GOOD_TESTS), Vec::<(usize, String)>::new());
    assert_eq!(unpassable_sites(REPLACEMENT_TESTS), Vec::<(usize, String)>::new());
}

/// S13: Every todo! and unimplemented! is reported at its own line, and lines shift with inserted text
#[test]
fn s13_sites_reported_at_their_lines_and_shift_with_prepended_lines() {
    for seed in 1..=64u64 {
        let mut rng = Rng(seed.wrapping_mul(0xD1B5_4A32_D192_ED03) | 1);
        let mut lines: Vec<String> = vec!["use std::fmt;".into(), String::new()];
        let mut expected: Vec<(usize, &str)> = Vec::new();
        let fns = 1 + rng.below(5);
        for f in 0..fns {
            lines.push("#[test]".into());
            lines.push(format!("fn case_{f}() {{"));
            let body = 1 + rng.below(6);
            for b in 0..body {
                match rng.below(3) {
                    0 => {
                        lines.push("    todo!();".into());
                        expected.push((lines.len(), "todo!("));
                    }
                    1 => {
                        lines.push(format!("    unimplemented!(\"m{b}\");"));
                        expected.push((lines.len(), "unimplemented!("));
                    }
                    _ => lines.push(format!("    let v{b} = {b};")),
                }
            }
            lines.push("}".into());
            lines.push(String::new());
        }
        let src = lines.join("\n") + "\n";
        let line_count = src.lines().count();

        let mut first = unpassable_sites(&src);
        first.sort();
        assert_eq!(
            first.iter().map(|(l, _)| *l).collect::<Vec<_>>(),
            expected.iter().map(|(l, _)| *l).collect::<Vec<_>>(),
            "seed {seed}: one site per call at its line\n{src}"
        );
        for ((l, t), (_, m)) in first.iter().zip(&expected) {
            assert!(t.contains(m), "seed {seed}: site at {l} has text {t:?}, expected {m}");
            assert!(*l >= 1 && *l <= line_count, "seed {seed}: line {l} out of 1..={line_count}");
        }

        let k = 1 + rng.below(5) as usize;
        let shifted_src = "\n".repeat(k) + &src;
        let mut second = unpassable_sites(&shifted_src);
        second.sort();
        let want: Vec<(usize, String)> = first.iter().map(|(l, t)| (l + k, t.clone())).collect();
        assert_eq!(second, want, "seed {seed}: sites shift by {k}");
    }
}

// ---------------------------------------------------------------------------
// §3 red at an ancestor commit
// ---------------------------------------------------------------------------

/// S14: Red is proved at an ancestor commit and recorded
#[test]
fn s14_red_proved_at_ancestor_and_recorded() {
    let (repo, c) = project("s14", true);
    let old_head = repo.head();
    repo.set_reply(&rust_block(GOOD_TESTS));
    let o = repo.hexa(&["spec", "tests", ADR, "--red-at", &c]);
    assert!(o.status.success(), "generation should succeed: {}", text(&o));

    let path = repo.test_file();
    let red = repo.red_ref().expect("refs/hexa/red/<ADR> is written");
    let diff = repo.git(&["diff", "--name-status", &c, &red]);
    assert_eq!(diff, format!("A\t{path}"), "red commit's tree is C's tree plus the test file");

    let recorded = repo.spec().red.expect("the spec records red");
    assert!(recorded.len() >= 7 && red.starts_with(recorded.trim()), "spec red {recorded} names {red}");

    assert!(repo.git_out(&["cat-file", "-e", &format!("HEAD:{path}")]).status.success(), "test file committed at HEAD");
    assert!(repo.git_out(&["merge-base", "--is-ancestor", &old_head, "HEAD"]).status.success(), "committed onto HEAD");
    assert_ne!(repo.head(), old_head, "a commit was made on HEAD");
    assert_eq!(repo.git(&["status", "--porcelain", "--", &path]), "", "the test file is committed");
}

/// S15: verify --prove checks red at the recorded commit
#[test]
fn s15_verify_prove_checks_red_at_recorded_commit() {
    let (repo, _c) = sealed_project("s15");
    let red = repo.red_ref().expect("red ref recorded");
    let o = repo.hexa(&["spec", "verify", ADR, "--prove"]);
    let out = text(&o);
    assert!(o.status.success(), "verify --prove passes when the tests fail at the red commit: {out}");
    assert!(out.contains(&red[..7]), "the red check names the commit under refs/hexa/red/{ADR} ({red}): {out}");
}

/// S16: A commit that is not an ancestor of HEAD is refused
#[test]
fn s16_non_ancestor_red_commit_is_refused() {
    let (repo, c) = project("s16", true);
    let head = repo.head();
    let branch = repo.git(&["rev-parse", "--abbrev-ref", "HEAD"]);
    repo.git(&["checkout", "-q", "-b", "side", &c]);
    repo.write("side.txt", "off the main line\n");
    let d = repo.commit_all("side commit");
    repo.git(&["checkout", "-q", &branch]);
    assert_eq!(repo.head(), head);

    repo.set_reply(&rust_block(GOOD_TESTS));
    let spec_before = repo.read(&repo.spec_path());
    let o = repo.hexa(&["spec", "tests", ADR, "--red-at", &d]);
    let out = text(&o);
    assert!(!o.status.success(), "a non-ancestor red commit should be refused: {out}");
    assert!(out.to_lowercase().contains("ancestor"), "refused for not being an ancestor: {out}");
    assert!(repo.red_ref().is_none(), "no red ref written");
    assert_eq!(repo.read(&repo.spec_path()), spec_before, "no red value written");
    assert_eq!(repo.head(), head, "no commit on HEAD");
}

/// S17: A test that passes at the red commit is refused
#[test]
fn s17_test_passing_at_red_commit_is_refused() {
    let (repo, c) = project("s17", true);
    let head = repo.head();
    let spec_before = repo.read(&repo.spec_path());
    repo.set_reply(&rust_block(ONE_PASSES_AT_RED));
    let o = repo.hexa(&["spec", "tests", ADR, "--red-at", &c]);
    let out = text(&o);
    assert!(!o.status.success(), "a test passing at the red commit should be refused: {out}");
    assert!(out.contains("zero_plus_zero_is_zero"), "names the test that passed: {out}");
    assert!(repo.red_ref().is_none(), "red ref unchanged");
    assert_eq!(repo.read(&repo.spec_path()), spec_before, "spec's red unchanged");
    assert!(repo.spec().red.is_none());
    assert_eq!(repo.head(), head, "HEAD unchanged");
}

// ---------------------------------------------------------------------------
// §4 replacing a sealed file
// ---------------------------------------------------------------------------

/// S18: A sealed file is replaced in one commit that names the file and the reason
#[test]
fn s18_sealed_file_replaced_in_one_commit_naming_file_and_reason() {
    let (repo, c) = sealed_project("s18");
    let before = repo.spec();
    let old_path = repo.test_file();
    let old_head = repo.head();

    repo.set_reply(&rust_block(REPLACEMENT_TESTS));
    let o = repo.hexa(&["spec", "tests", ADR, "--replace", "--red-at", &c, "--because", "helper stubbed constructor"]);
    assert!(o.status.success(), "replacement should succeed: {}", text(&o));

    assert_eq!(repo.git(&["rev-parse", "HEAD~1"]), old_head, "exactly one new commit");
    let msg = repo.git(&["log", "-1", "--format=%B"]);
    assert!(msg.contains(&old_path), "commit message names {old_path}: {msg}");
    assert!(msg.contains("helper stubbed constructor"), "commit message holds the reason: {msg}");

    let after = repo.spec();
    assert_eq!(after.sealed.len(), 1, "one sealed file: {:?}", after.sealed);
    let (new_path, seal) = after.sealed.iter().next().unwrap();
    let content = repo.read(new_path);
    assert!(content.contains("sum_of_three_and_two"), "the new file is in place");
    assert!(!content.contains("two_and_three_make_five"), "the old file is gone");
    if new_path != &old_path {
        assert!(!repo.root.join(&old_path).exists(), "old file removed");
    }
    let changed = repo.git(&["show", "--name-only", "--format=", "HEAD"]);
    assert!(changed.lines().any(|l| l == new_path), "the commit carries the new file: {changed}");
    assert!(seal.to_lowercase().contains(&repo.sha256(new_path)), "seal holds the new file's hash");

    assert_eq!(after.accepted, before.accepted, "accepted record unchanged");
    assert_eq!(after.accepted_content(), before.accepted_content(), "accepted content unchanged");
}

/// S19: --replace without --because is refused
#[test]
fn s19_replace_without_because_is_refused() {
    let (repo, c) = sealed_project("s19");
    let path = repo.test_file();
    let file_before = repo.read(&path);
    let seal_before = repo.spec().sealed;
    let head = repo.head();

    repo.set_reply(&rust_block(REPLACEMENT_TESTS));
    let o = repo.hexa(&["spec", "tests", ADR, "--replace", "--red-at", &c]);
    let out = text(&o);
    assert!(!o.status.success(), "--replace without --because should be refused: {out}");
    let lower = out.to_lowercase();
    assert!(lower.contains("because") || lower.contains("reason"), "refused for the missing reason: {out}");
    assert_eq!(repo.read(&path), file_before, "sealed file unchanged");
    assert_eq!(repo.spec().sealed, seal_before, "seal unchanged");
    assert_eq!(repo.head(), head, "HEAD unchanged");
}

/// S20: A replacement that fails §2 or §3 leaves the sealed file in place
#[test]
fn s20_failing_replacement_leaves_sealed_file_in_place() {
    let (repo, c) = sealed_project("s20");
    let path = repo.test_file();
    let file_before = repo.read(&path);
    let seal_before = repo.spec().sealed;
    let head = repo.head();

    repo.set_reply(&rust_block(WITH_UNIMPLEMENTED));
    let o = repo.hexa(&["spec", "tests", ADR, "--replace", "--red-at", &c, "--because", "r"]);
    let out = text(&o);
    assert!(!o.status.success(), "a replacement with an unpassable site should be refused: {out}");
    assert!(out.contains("unimplemented!("), "names the site: {out}");
    assert!(out.contains("11"), "names the site's line: {out}");
    assert_eq!(repo.read(&path), file_before, "original sealed file unchanged");
    assert_eq!(repo.spec().sealed, seal_before, "seal hash unchanged");
    assert_eq!(repo.head(), head, "no commit made");
}

/// S21: Editing a sealed file by hand does not count as a replacement
#[test]
fn s21_hand_edited_sealed_file_fails_verify() {
    let (repo, _c) = sealed_project("s21");
    let path = repo.test_file();
    let edited = repo.read(&path) + "\n// edited by hand\n";
    repo.write(&path, &edited);
    repo.commit_all("hand edit of a sealed test file");

    let o = repo.hexa(&["spec", "verify", ADR]);
    let out = text(&o);
    assert!(!o.status.success(), "verify should fail for a hand-edited sealed file: {out}");
    assert!(out.contains(&path), "names the file: {out}");
    assert!(out.to_lowercase().contains("seal"), "reports the seal mismatch: {out}");
}

#[allow(dead_code)]
fn _unused(_: &Path) {}
