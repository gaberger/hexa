//! ADR-2610281628: an amended decision is re-proved where it changed.
#![allow(unused, clippy::all)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use hexa_core::domain::spec::{rebind, without_tests, Acceptance, Kind, Scenario, Spec};
use hexa_exec::spec_author::{accept as accept_spec, sha256_hex};

const ADR: &str = "ADR-2610281700";

// ---------------------------------------------------------------- values

fn scen(id: &str, kind: Kind, test: Option<&str>, red: Option<&str>) -> Scenario {
    Scenario {
        id: id.to_string(),
        decision: vec!["§1".to_string()],
        kind,
        description: format!("{id} does its thing"),
        given: format!("given for {id}"),
        when: format!("when for {id}"),
        then: format!("then for {id}"),
        test: test.map(str::to_string),
        red: red.map(str::to_string),
    }
}

fn spec_of(specs: Vec<Scenario>, red: Option<&str>) -> Spec {
    Spec {
        adr: ADR.to_string(),
        feature: "fixture".to_string(),
        description: String::new(),
        universal: vec![],
        exempt: vec![],
        accepted: None,
        sealed: BTreeMap::new(),
        red: red.map(str::to_string),
        specs,
    }
}

fn find<'a>(spec: &'a Spec, id: &str) -> &'a Scenario {
    let found = spec.specs.iter().find(|s| s.id == id);
    assert!(found.is_some(), "no scenario {id}");
    found.unwrap()
}

// ---------------------------------------------------------------- a small rng

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
    fn coin(&mut self) -> bool {
        self.below(2) == 0
    }
}

// ---------------------------------------------------------------- repos

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn fresh_dir(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("hexa-2610281628-{tag}-{}-{n}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

fn read(dir: &Path, rel: &str) -> String {
    let r = std::fs::read_to_string(dir.join(rel));
    assert!(r.is_ok(), "reading {rel}: {:?}", r.as_ref().err());
    r.unwrap()
}

fn git(dir: &Path, args: &[&str]) -> String {
    let o = Command::new("git").args(args).current_dir(dir).output().expect("git runs");
    assert!(o.status.success(), "git {args:?} failed: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

fn git_ok(dir: &Path, args: &[&str]) -> bool {
    Command::new("git").args(args).current_dir(dir).output().map(|o| o.status.success()).unwrap_or(false)
}

fn commit_all(dir: &Path, msg: &str) -> String {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", msg]);
    git(dir, &["rev-parse", "HEAD"])
}

fn commit_count(dir: &Path) -> u64 {
    git(dir, &["rev-list", "--count", "HEAD"]).parse().unwrap()
}

const ADR_MD: &str = "---\nid: ADR-2610281700\nstatus: proposed\ndate: 2026-10-28\n---\n# ADR-2610281700: fixture\n\n## Context\n\nA fixture.\n\n## Decision\n\n1. **One.** The fixture answers.\n\n## Implementation\n\n`src/lib.rs`.\n";

fn project(tag: &str, lib: &str) -> PathBuf {
    let dir = fresh_dir(tag);
    write(
        &dir,
        "Cargo.toml",
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n",
    );
    write(&dir, "src/lib.rs", lib);
    write(&dir, ".gitignore", "target/\n");
    write(&dir, &format!("docs/adrs/{ADR}-fixture.md"), ADR_MD);
    git(&dir, &["init", "-q"]);
    git(&dir, &["config", "user.name", "tester"]);
    git(&dir, &["config", "user.email", "tester@example.invalid"]);
    git(&dir, &["config", "commit.gpgsign", "false"]);
    dir
}

fn spec_path() -> String {
    format!("docs/specs/{ADR}.json")
}

fn write_spec(dir: &Path, spec: &Spec) {
    write(dir, &spec_path(), &serde_json::to_string_pretty(spec).unwrap());
}

fn read_spec(dir: &Path) -> Spec {
    serde_json::from_str(&read(dir, &spec_path())).expect("the spec parses")
}

/// A `claude` on PATH that reads its prompt and answers with `rust`, as JSON.
fn fake_model(tag: &str, rust: &str) -> PathBuf {
    let bin = fresh_dir(&format!("{tag}-bin"));
    let text = format!("```rust\n{rust}\n```\n");
    let reply = serde_json::json!({
        "type": "result",
        "subtype": "success",
        "is_error": false,
        "result": text,
    });
    let reply_path = bin.join("reply.json");
    std::fs::write(&reply_path, serde_json::to_string(&reply).unwrap()).unwrap();
    let script = format!("#!/bin/sh\ncat >/dev/null\ncat '{}'\n", reply_path.display());
    let claude = bin.join("claude");
    std::fs::write(&claude, script).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

fn hexa(dir: &Path, args: &[&str], bin: Option<&Path>) -> Output {
    let base = std::env::var_os("PATH").unwrap_or_default();
    let path = match bin {
        Some(b) => {
            let mut p = OsString::from(b);
            p.push(":");
            p.push(&base);
            p
        }
        None => base,
    };
    Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(args)
        .current_dir(dir)
        .env("PATH", path)
        .env("CARGO_TARGET_DIR", dir.join("target"))
        .env("GIT_AUTHOR_NAME", "tester")
        .env("GIT_AUTHOR_EMAIL", "tester@example.invalid")
        .env("GIT_COMMITTER_NAME", "tester")
        .env("GIT_COMMITTER_EMAIL", "tester@example.invalid")
        .output()
        .expect("hexa runs")
}

fn text(o: &Output) -> String {
    format!("{}\n{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

fn red_matches(stored: Option<&str>, full: &str) -> bool {
    match stored {
        Some(r) if r.len() >= 7 => full.starts_with(r) || r.starts_with(full),
        _ => false,
    }
}

// ---------------------------------------------------------------- §1 rebind

/// S01: An unchanged scenario keeps its test and its own red commit
#[test]
fn s01_unchanged_scenario_keeps_test_and_own_red() {
    let previous = spec_of(vec![scen("S01", Kind::Example, Some("tests/a_spec.rs::s01"), Some("abc1234"))], None);
    let mut accepted = spec_of(vec![scen("S01", Kind::Example, None, None)], None);
    accepted.feature = "fixture, amended".to_string();
    accepted.accepted = Some(Acceptance { by: "p".into(), sha256: "h".into() });
    accepted.sealed.insert("tests/a_spec.rs".into(), "hash".into());

    let out = rebind(&previous, &accepted);

    let mut expected = accepted.clone();
    expected.specs[0].test = Some("tests/a_spec.rs::s01".to_string());
    expected.specs[0].red = Some("abc1234".to_string());
    assert_eq!(out, expected);
}

/// S02: An unchanged scenario proved at the spec-level red gets that red written onto it
#[test]
fn s02_spec_level_red_is_written_onto_the_scenario() {
    let previous = spec_of(vec![scen("S02", Kind::Example, Some("tests/a_spec.rs::s02"), None)], Some("def5678"));
    let accepted = spec_of(vec![scen("S02", Kind::Example, None, None)], None);

    let out = rebind(&previous, &accepted);

    let s = find(&out, "S02");
    assert_eq!(s.test.as_deref(), Some("tests/a_spec.rs::s02"));
    assert_eq!(s.red.as_deref(), Some("def5678"));
}

/// S03: A scenario whose behaviour changed loses its test and red
#[test]
fn s03_changed_then_drops_test_and_red() {
    let previous = spec_of(vec![scen("S03", Kind::Example, Some("tests/a_spec.rs::s03"), Some("abc1234"))], None);
    let mut accepted = spec_of(vec![scen("S03", Kind::Example, None, None)], None);
    accepted.specs[0].then = "something new happens".to_string();

    let out = rebind(&previous, &accepted);

    let s = find(&out, "S03");
    assert_eq!(s.test, None);
    assert_eq!(s.red, None);
    assert_eq!(s.then, "something new happens");
}

/// S04: A change to any one compared field drops the binding
#[test]
fn s04_any_compared_field_change_drops_binding() {
    let changes: Vec<(&str, Box<dyn Fn(&mut Scenario)>)> = vec![
        ("decision", Box::new(|s: &mut Scenario| s.decision = vec!["§2".to_string()])),
        ("kind", Box::new(|s: &mut Scenario| s.kind = Kind::Negative)),
        ("description", Box::new(|s: &mut Scenario| s.description = "other".to_string())),
        ("given", Box::new(|s: &mut Scenario| s.given = "other".to_string())),
        ("when", Box::new(|s: &mut Scenario| s.when = "other".to_string())),
    ];
    for (field, change) in changes {
        let previous =
            spec_of(vec![scen("S04", Kind::Example, Some("tests/a_spec.rs::s04"), Some("abc1234"))], Some("def5678"));
        let mut changed = scen("S04", Kind::Example, None, None);
        change(&mut changed);
        let accepted = spec_of(vec![changed], None);

        let out = rebind(&previous, &accepted);

        let s = find(&out, "S04");
        assert_eq!(s.test, None, "a change to {field} must drop the test");
        assert_eq!(s.red, None, "a change to {field} must drop the red");
    }
}

/// S05: A new scenario gains no test, even when the previous spec has a red
#[test]
fn s05_new_scenario_gains_no_test() {
    let previous = spec_of(vec![scen("S01", Kind::Example, Some("tests/a_spec.rs::s01"), None)], Some("def5678"));
    let accepted =
        spec_of(vec![scen("S01", Kind::Example, None, None), scen("S09", Kind::Negative, None, None)], None);

    let out = rebind(&previous, &accepted);

    let s = find(&out, "S09");
    assert_eq!(s.test, None);
    assert_eq!(s.red, None);
}

// ---------------------------------------------------------------- §1 accept

fn sealed_accept_fixture(tag: &str, files: &[(&str, &str)], specs: Vec<Scenario>) -> (PathBuf, Spec, String) {
    let dir = project(tag, "pub fn answer() -> u32 { 1 }\n");
    for (rel, content) in files {
        write(&dir, rel, content);
    }
    let red = commit_all(&dir, "tests");
    let mut spec = accept_spec(spec_of(specs, Some(&red)), "tester");
    for (rel, content) in files {
        spec.sealed.insert(rel.to_string(), sha256_hex(content.as_bytes()));
    }
    write_spec(&dir, &spec);
    commit_all(&dir, "spec accepted and sealed");
    (dir, spec, red)
}

/// S06: Re-accepting keeps the seal of a file whose tests all belong to unchanged scenarios
#[test]
fn s06_reaccept_keeps_seal_of_untouched_file() {
    let a = "#[test]\nfn s01() { assert_eq!(fixture::answer(), 1); }\n\n#[test]\nfn s02() { assert_ne!(fixture::answer(), 0); }\n";
    let a2 = "#[test]\nfn s03() { assert_eq!(fixture::answer(), 1); }\n\n#[test]\nfn s04() { assert_ne!(fixture::answer(), 2); }\n";
    let (dir, before, _red) = sealed_accept_fixture(
        "s06",
        &[("tests/a_spec.rs", a), ("tests/a_spec_2.rs", a2)],
        vec![
            scen("S01", Kind::Example, Some("tests/a_spec.rs::s01"), None),
            scen("S02", Kind::Negative, Some("tests/a_spec.rs::s02"), None),
            scen("S03", Kind::Example, Some("tests/a_spec_2.rs::s03"), None),
            scen("S04", Kind::Negative, Some("tests/a_spec_2.rs::s04"), None),
        ],
    );
    let mut changed = before.clone();
    changed.specs.iter_mut().find(|s| s.id == "S04").unwrap().then = "a changed outcome".to_string();
    write_spec(&dir, &changed);

    let o = hexa(&dir, &["spec", "accept", ADR, "--by", "tester"], None);
    assert!(o.status.success(), "accept failed: {}", text(&o));

    let after = read_spec(&dir);
    assert_eq!(after.sealed.get("tests/a_spec.rs"), before.sealed.get("tests/a_spec.rs"));
    assert!(after.sealed.get("tests/a_spec.rs").is_some());
    assert!(!after.sealed.contains_key("tests/a_spec_2.rs"), "the changed scenario's file must lose its seal");
    for id in ["S01", "S02", "S03"] {
        let b = find(&before, id);
        let a = find(&after, id);
        assert_eq!(a.test, b.test, "{id} keeps its test");
        assert_eq!(a.red_commit(&after), b.red_commit(&before), "{id} keeps its red commit");
    }
    assert_eq!(find(&after, "S04").test, None);
}

/// S07: Removing a scenario drops the seal of the file that held its test
#[test]
fn s07_removed_scenario_drops_seal_of_its_file() {
    let a = "#[test]\nfn s01() { assert_eq!(fixture::answer(), 1); }\n\n#[test]\nfn s05() { assert!(fixture::answer() > 0); }\n";
    let a2 = "#[test]\nfn s02() { assert_ne!(fixture::answer(), 2); }\n";
    let (dir, before, _red) = sealed_accept_fixture(
        "s07",
        &[("tests/a_spec.rs", a), ("tests/a_spec_2.rs", a2)],
        vec![
            scen("S01", Kind::Example, Some("tests/a_spec.rs::s01"), None),
            scen("S05", Kind::Example, Some("tests/a_spec.rs::s05"), None),
            scen("S02", Kind::Negative, Some("tests/a_spec_2.rs::s02"), None),
        ],
    );
    let mut changed = before.clone();
    changed.specs.retain(|s| s.id != "S05");
    write_spec(&dir, &changed);

    let o = hexa(&dir, &["spec", "accept", ADR, "--by", "tester"], None);
    assert!(o.status.success(), "accept failed: {}", text(&o));

    let after = read_spec(&dir);
    assert!(!after.sealed.contains_key("tests/a_spec.rs"), "the removed scenario's file must lose its seal");
    assert!(after.specs.iter().all(|s| s.test.as_deref() != Some("tests/a_spec.rs::s05")));
    assert!(after.sealed.get("tests/a_spec_2.rs").is_some());
    assert_eq!(after.sealed.get("tests/a_spec_2.rs"), before.sealed.get("tests/a_spec_2.rs"));
}

/// S08: Rebinding never alters behaviour and carries a binding only across an identical scenario
#[test]
fn s08_rebind_property() {
    let mut rng = Rng(0x2610_2816_28ab_cdef);
    let ids = ["S1", "S2", "S3", "S4", "S5", "S6"];
    let decisions = [vec!["§1".to_string()], vec!["§2".to_string()], vec!["§1".to_string(), "§2".to_string()]];
    let kinds = [Kind::Example, Kind::Negative, Kind::Property];
    let words = ["a", "b"];
    let reds = ["r1111111", "r2222222"];

    let random_scenario = |rng: &mut Rng, id: &str| Scenario {
        id: id.to_string(),
        decision: decisions[rng.below(3) as usize].clone(),
        kind: kinds[rng.below(3) as usize],
        description: words[rng.below(2) as usize].to_string(),
        given: words[rng.below(2) as usize].to_string(),
        when: words[rng.below(2) as usize].to_string(),
        then: words[rng.below(2) as usize].to_string(),
        test: rng.coin().then(|| format!("tests/x_spec.rs::{}", id.to_lowercase())),
        red: rng.coin().then(|| reds[rng.below(2) as usize].to_string()),
    };

    for _case in 0..400 {
        let mut prev_specs = Vec::new();
        for id in ids {
            if rng.coin() {
                prev_specs.push(random_scenario(&mut rng, id));
            }
        }
        let prev_red = rng.coin().then_some("sprev00");
        let previous = spec_of(prev_specs.clone(), prev_red);

        let mut acc_specs = Vec::new();
        for id in ids {
            if !rng.coin() {
                continue;
            }
            let mut s = match prev_specs.iter().find(|p| p.id == id) {
                Some(p) if rng.coin() => p.clone(),
                Some(p) => {
                    let mut c = p.clone();
                    match rng.below(6) {
                        0 => c.decision = vec!["§9".to_string()],
                        1 => c.kind = if c.kind == Kind::Example { Kind::Negative } else { Kind::Example },
                        2 => c.description.push('!'),
                        3 => c.given.push('!'),
                        4 => c.when.push('!'),
                        _ => c.then.push('!'),
                    }
                    c
                }
                None => random_scenario(&mut rng, id),
            };
            s.test = rng.coin().then(|| "tests/stale.rs::stale".to_string());
            s.red = rng.coin().then(|| "stale00".to_string());
            acc_specs.push(s);
        }
        let mut accepted = spec_of(acc_specs, rng.coin().then_some("sacc000"));
        if rng.coin() {
            accepted.accepted = Some(Acceptance { by: "p".into(), sha256: "h".into() });
        }

        let out = rebind(&previous, &accepted);
        assert_eq!(out.accepted_content(), accepted.accepted_content());
        assert_eq!(out.specs.len(), accepted.specs.len());

        for r in &out.specs {
            let same = previous.specs.iter().find(|p| {
                p.id == r.id
                    && p.decision == r.decision
                    && p.kind == r.kind
                    && p.description == r.description
                    && p.given == r.given
                    && p.when == r.when
                    && p.then == r.then
            });
            match same.filter(|p| p.test.is_some()) {
                Some(p) => {
                    assert_eq!(r.test, p.test, "{} carries its test", r.id);
                    let expected_red = p.red.clone().or_else(|| previous.red.clone());
                    assert_eq!(r.red, expected_red, "{} carries its red", r.id);
                }
                None => {
                    assert_eq!(r.test, None, "{} must carry no test", r.id);
                    assert_eq!(r.red, None, "{} must carry no red", r.id);
                }
            }
        }
    }
}

// ---------------------------------------------------------------- §2 without_tests

/// S09: without_tests removes a named test with its docs and attributes and keeps the rest
#[test]
fn s09_without_tests_removes_named_test_with_docs_and_attributes() {
    let s04 = "#[test]\nfn s04() {\n    assert_eq!(2, 2);\n}\n";
    let source = format!("/// doc\n#[test]\n#[ignore]\nfn s03() {{\n    assert_eq!(1, 1);\n}}\n\n{s04}");

    let out = without_tests(&source, &["s03".to_string()]);

    assert!(!out.contains("fn s03"), "{out}");
    assert!(!out.contains("/// doc"), "{out}");
    assert!(!out.contains("#[ignore]"), "{out}");
    assert_eq!(out.matches("#[test]").count(), 1, "{out}");
    assert!(out.contains(s04), "s04 must remain byte for byte: {out}");
}

/// S10: without_tests leaves alone what only looks like the named test
#[test]
fn s10_without_tests_leaves_lookalikes_alone() {
    let source = "\
// s03 is mentioned here, but only in a comment
fn s03_helper() -> u32 {
    3
}

const TEXT: &str = \"#[test] fn s03() {}\";

/// s04 checks the helper
#[test]
fn s04() {
    assert_eq!(s03_helper(), 3);
    assert!(TEXT.contains(\"s03\"));
}
";
    let out = without_tests(source, &["s03".to_string()]);
    assert_eq!(out, source);
}

// ---------------------------------------------------------------- §2 --only

const OLD_TESTS: &str = "\
/// S15a: one reply is served
#[test]
fn s15a() {
    assert_eq!(fixture::replies(), 1);
}

/// S15b: one reply is enough
#[test]
fn s15b() {
    assert!(fixture::replies() >= 1);
}
";

const OLD_FILE: &str = "tests/adr_2610281700_spec.rs";

fn reprove_fixture(tag: &str) -> (PathBuf, Spec) {
    let dir = project(tag, "pub fn replies() -> usize { 1 }\n");
    write(&dir, OLD_FILE, OLD_TESTS);
    let red = commit_all(&dir, "tests");
    let mut spec = accept_spec(
        spec_of(
            vec![
                scen("S15a", Kind::Example, Some(&format!("{OLD_FILE}::s15a")), None),
                scen("S15b", Kind::Negative, Some(&format!("{OLD_FILE}::s15b")), None),
            ],
            Some(&red),
        ),
        "tester",
    );
    spec.sealed.insert(OLD_FILE.to_string(), sha256_hex(OLD_TESTS.as_bytes()));
    write_spec(&dir, &spec);
    commit_all(&dir, "spec");
    (dir, spec)
}

fn ref_path(r: &str) -> String {
    hexa_core::domain::spec::test_ref(r).expect("a test ref").0.to_string()
}

/// S11: Re-proving named scenarios writes a new file, binds and seals it, removes the old tests, in one commit
#[test]
fn s11_reprove_writes_new_file_binds_seals_and_commits_once() {
    let (dir, before) = reprove_fixture("s11");
    let bin = fake_model(
        "s11",
        "/// S15b: two replies are needed\n#[test]\nfn s15b() {\n    assert_eq!(fixture::replies(), 2);\n}\n",
    );
    let commits = commit_count(&dir);
    let reason = "one reply is no longer enough";

    let o = hexa(&dir, &["spec", "tests", ADR, "--crate", ".", "--only", "S15b", "--because", reason], Some(&bin));
    assert!(o.status.success(), "re-proving failed: {}", text(&o));

    let after = read_spec(&dir);
    let s = find(&after, "S15b");
    let new_ref = s.test.clone().expect("S15b names its new test");
    let new_file = ref_path(&new_ref);
    assert!(new_file.starts_with("tests/") && new_file.ends_with("_spec_2.rs"), "new file: {new_file}");
    assert_ne!(new_file, OLD_FILE);
    let new_src = read(&dir, &new_file);
    assert!(new_src.contains("#[test]"));

    let red = s.red_commit(&after).expect("S15b has a red commit").to_string();
    assert_ne!(Some(red.as_str()), find(&before, "S15b").red_commit(&before));
    assert!(git_ok(&dir, &["cat-file", "-e", &format!("{red}^{{commit}}")]), "red {red} is a commit");

    let old_src = read(&dir, OLD_FILE);
    assert!(!old_src.contains("fn s15b"), "{old_src}");
    assert!(!old_src.contains("/// S15b"), "{old_src}");
    assert!(old_src.contains("fn s15a"), "{old_src}");

    assert_eq!(after.sealed.get(OLD_FILE), Some(&sha256_hex(old_src.as_bytes())));
    assert_eq!(after.sealed.get(&new_file), Some(&sha256_hex(new_src.as_bytes())));

    assert_eq!(commit_count(&dir), commits + 1, "one commit holds it all");
    let msg = git(&dir, &["log", "-1", "--format=%B"]);
    assert!(msg.contains(OLD_FILE), "message names the old file: {msg}");
    assert!(msg.contains(&new_file), "message names the new file: {msg}");
    assert!(msg.contains(reason), "message names the reason: {msg}");
    assert_eq!(git(&dir, &["status", "--porcelain"]), "", "everything is committed");
}

/// S12: Re-proving a scenario that still has a test requires a reason
#[test]
fn s12_reprove_without_because_is_refused() {
    let (dir, _before) = reprove_fixture("s12");
    let bin = fake_model(
        "s12",
        "/// S15b: two replies are needed\n#[test]\nfn s15b() {\n    assert_eq!(fixture::replies(), 2);\n}\n",
    );
    let spec_before = read(&dir, &spec_path());
    let commits = commit_count(&dir);

    let o = hexa(&dir, &["spec", "tests", ADR, "--crate", ".", "--only", "S15b"], Some(&bin));

    assert!(!o.status.success(), "must be refused: {}", text(&o));
    let t = text(&o);
    assert!(t.contains("--because"), "the refusal names --because: {t}");
    assert!(!t.contains("unexpected argument"), "--only must be understood: {t}");
    assert_eq!(read(&dir, &spec_path()), spec_before);
    assert_eq!(read(&dir, OLD_FILE), OLD_TESTS);
    let tests: Vec<_> = std::fs::read_dir(dir.join("tests")).unwrap().collect();
    assert_eq!(tests.len(), 1, "no file written");
    assert_eq!(commit_count(&dir), commits);
    assert_eq!(git(&dir, &["status", "--porcelain"]), "");
}

/// S13: A scenario left without a test by re-acceptance may be named without a reason
#[test]
fn s13_untested_scenario_may_be_named_without_reason() {
    let dir = project("s13", "pub fn replies() -> usize { 1 }\n");
    let f1 = "tests/adr_2610281700_spec.rs";
    let f2 = "tests/adr_2610281700_spec_2.rs";
    let c1 = "/// S01: one\n#[test]\nfn s01() {\n    assert_eq!(fixture::replies(), 1);\n}\n";
    let c2 = "/// S02: not zero\n#[test]\nfn s02() {\n    assert_ne!(fixture::replies(), 0);\n}\n";
    write(&dir, f1, c1);
    write(&dir, f2, c2);
    let red = commit_all(&dir, "tests");
    let mut spec = accept_spec(
        spec_of(
            vec![
                scen("S01", Kind::Example, Some(&format!("{f1}::s01")), Some(&red)),
                scen("S02", Kind::Negative, Some(&format!("{f2}::s02")), Some(&red)),
                scen("S03", Kind::Example, None, None),
            ],
            None,
        ),
        "tester",
    );
    spec.sealed.insert(f1.into(), sha256_hex(c1.as_bytes()));
    spec.sealed.insert(f2.into(), sha256_hex(c2.as_bytes()));
    write_spec(&dir, &spec);
    commit_all(&dir, "spec");
    let bin = fake_model(
        "s13",
        "/// S03: two replies\n#[test]\nfn s03() {\n    assert_eq!(fixture::replies(), 2);\n}\n",
    );

    let o = hexa(&dir, &["spec", "tests", ADR, "--crate", ".", "--only", "S03"], Some(&bin));
    assert!(o.status.success(), "naming an untested scenario needs no reason: {}", text(&o));

    let after = read_spec(&dir);
    let s = find(&after, "S03");
    let new_file = ref_path(s.test.as_deref().expect("S03 is bound"));
    assert!(new_file.starts_with("tests/") && new_file.ends_with("_spec_3.rs"), "new file: {new_file}");
    assert!(s.red_commit(&after).is_some(), "S03 has a red commit");
    let new_src = read(&dir, &new_file);
    assert_eq!(after.sealed.get(&new_file), Some(&sha256_hex(new_src.as_bytes())));
    assert_eq!(read(&dir, f1), c1);
    assert_eq!(read(&dir, f2), c2);
}

/// S14: Scenarios not named are not regenerated or rebound
#[test]
fn s14_unnamed_scenarios_are_untouched() {
    let dir = project("s14", "pub fn replies() -> usize { 1 }\n");
    let f1 = "tests/adr_2610281700_spec.rs";
    let s01_fn = "/// S01: one\n#[test]\nfn s01() {\n    assert_eq!(fixture::replies(), 1);\n}\n";
    let content = format!("{s01_fn}\n/// S02: not zero\n#[test]\nfn s02() {{\n    assert_ne!(fixture::replies(), 0);\n}}\n");
    write(&dir, f1, &content);
    let red = commit_all(&dir, "tests");
    let mut spec = accept_spec(
        spec_of(
            vec![
                scen("S01", Kind::Example, Some(&format!("{f1}::s01")), None),
                scen("S02", Kind::Negative, Some(&format!("{f1}::s02")), None),
            ],
            Some(&red),
        ),
        "tester",
    );
    spec.sealed.insert(f1.into(), sha256_hex(content.as_bytes()));
    write_spec(&dir, &spec);
    commit_all(&dir, "spec");
    let bin = fake_model(
        "s14",
        "/// S02: two replies\n#[test]\nfn s02() {\n    assert_eq!(fixture::replies(), 2);\n}\n",
    );

    let o = hexa(&dir, &["spec", "tests", ADR, "--crate", ".", "--only", "S02", "--because", "changed"], Some(&bin));
    assert!(o.status.success(), "re-proving failed: {}", text(&o));

    let after = read_spec(&dir);
    let s01 = find(&after, "S01");
    assert_eq!(s01.test.as_deref(), Some(format!("{f1}::s01").as_str()));
    assert_eq!(s01.red_commit(&after), Some(red.as_str()));
    assert!(read(&dir, f1).contains(s01_fn), "S01's test function is unchanged");

    let new_file = ref_path(find(&after, "S02").test.as_deref().expect("S02 is rebound"));
    assert_ne!(new_file, f1);
    let new_src = read(&dir, &new_file);
    assert_eq!(new_src.matches("#[test]").count(), 1, "{new_src}");
    assert!(!new_src.contains("fn s01"), "{new_src}");
}

/// S15: without_tests removes exactly the named test functions
#[test]
fn s15_without_tests_property() {
    let mut rng = Rng(0x5151_2610_2816_2800);
    for _case in 0..300 {
        let n_items = 1 + rng.below(8) as usize;
        // (text, is_test, name)
        let mut items: Vec<(String, bool, Option<String>)> = Vec::new();
        for i in 0..n_items {
            let name = format!("p{i}");
            let other = format!("p{}", rng.below(10));
            match rng.below(3) {
                0 => {
                    let ignore = if rng.coin() { "#[ignore]\n" } else { "" };
                    let t = format!(
                        "/// doc for {name}\n#[test]\n{ignore}fn {name}() {{\n    let s = \"x {other} {{\";\n    assert!(!s.is_empty());\n}}\n"
                    );
                    items.push((t, true, Some(name)));
                }
                1 => {
                    let t = format!("/// helper {name}\nfn {name}() -> u32 {{\n    {i}\n}}\n");
                    items.push((t, false, Some(name)));
                }
                _ => {
                    let t = format!("const C{i}: u32 = {i};\n");
                    items.push((t, false, None));
                }
            }
        }
        let source: String = items.iter().map(|(t, _, _)| t.as_str()).collect::<Vec<_>>().join("\n");
        let names: Vec<String> = (0..10).filter(|_| rng.coin()).map(|i| format!("p{i}")).collect();

        assert_eq!(without_tests(&source, &[]), source);

        let out = without_tests(&source, &names);
        let mut cursor = 0usize;
        for (t, is_test, name) in &items {
            let removed = *is_test && name.as_ref().is_some_and(|n| names.contains(n));
            if removed {
                let n = name.as_ref().unwrap();
                assert!(!out.contains(&format!("fn {n}()")), "{n} must be gone:\n{out}");
                assert!(!out.contains(&format!("/// doc for {n}\n")), "{n}'s docs must be gone:\n{out}");
            } else {
                let at = out[cursor..].find(t.as_str());
                assert!(at.is_some(), "item must remain unchanged and in order:\n{t}\n--- in ---\n{out}");
                cursor += at.unwrap() + t.len();
            }
        }
    }
}

// ---------------------------------------------------------------- §3 --red-at

fn red_at_fixture(tag: &str) -> (PathBuf, String) {
    let dir = project(tag, "pub fn answer() -> u32 { 1 }\n");
    write_spec(
        &dir,
        &accept_spec(
            spec_of(vec![scen("S01", Kind::Example, None, None), scen("S02", Kind::Negative, None, None)], None),
            "tester",
        ),
    );
    let earlier = commit_all(&dir, "before the API");
    write(&dir, "src/lib.rs", "pub fn answer() -> u32 { 1 }\n\npub fn answer_two() -> u32 { 2 }\n");
    commit_all(&dir, "the API");
    (dir, earlier)
}

/// S16: A test that did not build at its red commit is red when the new file builds at HEAD
#[test]
fn s16_unbuilt_at_red_commit_counts_as_red() {
    let (dir, earlier) = red_at_fixture("s16");
    let bin = fake_model(
        "s16",
        "/// S01: two\n#[test]\nfn s01() {\n    assert_eq!(fixture::answer_two(), 2);\n}\n\n/// S02: not three\n#[test]\nfn s02() {\n    assert_ne!(fixture::answer_two(), 3);\n}\n",
    );

    let o = hexa(&dir, &["spec", "tests", ADR, "--crate", ".", "--red-at", &earlier], Some(&bin));
    assert!(o.status.success(), "a test written before its API is red: {}", text(&o));

    let after = read_spec(&dir);
    for id in ["S01", "S02"] {
        let s = find(&after, id);
        assert!(s.test.is_some(), "{id} is bound");
        assert!(red_matches(s.red_commit(&after), &earlier), "{id} is red at {earlier}: {:?}", s.red_commit(&after));
    }
}

/// S17: A test that builds and passes at the red commit is refused
#[test]
fn s17_passing_at_red_commit_is_refused() {
    let (dir, earlier) = red_at_fixture("s17");
    let bin = fake_model(
        "s17",
        "/// S01: one\n#[test]\nfn s01_passes_early() {\n    assert_eq!(fixture::answer(), 1);\n}\n\n/// S02: not three\n#[test]\nfn s02() {\n    assert_ne!(fixture::answer_two(), 3);\n}\n",
    );

    let o = hexa(&dir, &["spec", "tests", ADR, "--crate", ".", "--red-at", &earlier], Some(&bin));
    assert!(!o.status.success(), "a test green at the red commit must be refused: {}", text(&o));
    let t = text(&o);
    assert!(t.contains("s01_passes_early"), "the refusal names the passing test: {t}");

    let after = read_spec(&dir);
    for s in &after.specs {
        assert_eq!(s.test, None, "{} must not be bound", s.id);
    }
}

/// S18: A failure to build is not red when the new file does not build at HEAD
#[test]
fn s18_not_building_at_head_is_refused() {
    let (dir, earlier) = red_at_fixture("s18");
    let bin = fake_model(
        "s18",
        "/// S01: never\n#[test]\nfn s01() {\n    assert_eq!(fixture::no_such_fn(), 2);\n}\n\n/// S02: never either\n#[test]\nfn s02() {\n    assert_ne!(fixture::no_such_fn(), 3);\n}\n",
    );

    let o = hexa(&dir, &["spec", "tests", ADR, "--crate", ".", "--red-at", &earlier], Some(&bin));
    assert!(!o.status.success(), "a file that does not build at HEAD must be refused: {}", text(&o));
    let t = text(&o);
    assert!(t.contains("HEAD"), "the refusal names HEAD: {t}");
    assert!(t.to_lowercase().contains("build"), "the refusal says it does not build: {t}");

    let after = read_spec(&dir);
    for s in &after.specs {
        assert_eq!(s.test, None, "{} must not be bound", s.id);
    }
}
