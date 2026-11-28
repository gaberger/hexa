//! A seal that a re-acceptance dropped is re-made by `hexa spec tests --only`
//! with the link recorded, so the amendment chain still runs from the anchored hash.
//!
//! Every move of a seal is in the chain (ADR-2611081151 §1). `--only` cuts the
//! superseded tests out of the sealed file and records the move from the hash the
//! seal had. After a re-acceptance the seal is gone (ADR-2610281628 §1), so there
//! was no hash to move from, the move went unrecorded, and `spec verify` said
//! "seal broken: amendment 0 ... does not continue the chain from the anchored hash".
//!
//! Ids are assembled at run time: the citation checker scans source.
#![allow(unused)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use hexa_core::domain::spec::{Kind, Scenario, Spec};
use hexa_exec::spec_author::{accept as accept_spec, sha256_hex};

fn adr() -> String {
    format!("ADR-{}", "2701030000")
}
static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn fresh_dir(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("hexa-chain-{tag}-{}-{n}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
fn write(dir: &Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}
fn read(dir: &Path, rel: &str) -> String {
    std::fs::read_to_string(dir.join(rel)).unwrap_or_else(|e| panic!("reading {rel}: {e}"))
}
fn git(dir: &Path, args: &[&str]) -> String {
    let o = Command::new("git").args(args).current_dir(dir).output().expect("git runs");
    assert!(o.status.success(), "git {args:?} failed: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}
fn commit_all(dir: &Path, msg: &str) -> String {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", msg]);
    git(dir, &["rev-parse", "HEAD"])
}
fn fake_model(tag: &str, rust: &str) -> PathBuf {
    let bin = fresh_dir(&format!("{tag}-bin"));
    let reply = serde_json::json!({ "type": "result", "subtype": "success", "is_error": false, "result": format!("```rust\n{rust}\n```\n") });
    let reply_path = bin.join("reply.json");
    std::fs::write(&reply_path, serde_json::to_string(&reply).unwrap()).unwrap();
    let claude = bin.join("claude");
    std::fs::write(&claude, format!("#!/bin/sh\ncat >/dev/null\ncat '{}'\n", reply_path.display())).unwrap();
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
fn scen(id: &str, then: &str, test: &str) -> Scenario {
    Scenario {
        id: id.to_string(),
        decision: vec!["§1".to_string()],
        kind: if id == "S3" { Kind::Negative } else { Kind::Example },
        description: format!("{id} does its thing"),
        given: format!("given for {id}"),
        when: format!("when for {id}"),
        then: then.to_string(),
        test: Some(test.to_string()),
        red: None,
    }
}

const FILE: &str = "tests/rule_spec.rs";
const TESTS: &str = "/// S1: one\n#[test]\nfn s1() {\n    assert_eq!(fixture::n(), 1);\n}\n\n/// S2: not zero\n#[test]\nfn s2() {\n    assert_ne!(fixture::n(), 0);\n}\n\n/// S3: small\n#[test]\nfn s3() {\n    assert!(fixture::n() < 9);\n}\n";

/// The amendments for `file`, as (from, to).
fn chain(dir: &Path, file: &str) -> Vec<(String, String)> {
    let v: serde_json::Value = serde_json::from_str(&read(dir, &format!("docs/specs/{}.json", adr()))).unwrap();
    v["amendments"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter(|a| a["file"] == file)
        .map(|a| (a["from"].as_str().unwrap().to_string(), a["to"].as_str().unwrap().to_string()))
        .collect()
}

#[test]
fn a_seal_a_reacceptance_dropped_is_re_made_with_its_link_recorded() {
    let id = adr();
    let dir = fresh_dir("chain");
    write(&dir, "Cargo.toml", "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n");
    write(&dir, "src/lib.rs", "pub fn n() -> usize { 1 }\n");
    write(&dir, ".gitignore", "target/\n");
    write(&dir, &format!("docs/adrs/{id}-fixture.md"), &format!("---\nid: {id}\nstatus: proposed\ndate: 2027-01-03\n---\n# {id}: fixture\n\n## Context\n\nA fixture.\n\n## Decision\n\n1. **One.** It answers.\n\n## Implementation\n\n`src/lib.rs`.\n"));
    git(&dir, &["init", "-q"]);
    git(&dir, &["config", "user.name", "tester"]);
    git(&dir, &["config", "user.email", "tester@example.invalid"]);
    git(&dir, &["config", "commit.gpgsign", "false"]);
    write(&dir, FILE, TESTS);
    let red = commit_all(&dir, "tests");
    let anchor = sha256_hex(TESTS.as_bytes());

    let mut spec = Spec {
        adr: id.clone(),
        feature: "fixture".into(),
        description: String::new(),
        universal: vec![],
        exempt: vec![],
        accepted: None,
        sealed: BTreeMap::new(),
        red: Some(red.clone()),
        specs: vec![scen("S1", "then 1", &format!("{FILE}::s1")), scen("S2", "then 2", &format!("{FILE}::s2")), scen("S3", "then 3", &format!("{FILE}::s3"))],
    };
    let mut spec = accept_spec(spec, "tester");
    spec.sealed.insert(FILE.into(), anchor.clone());
    write(&dir, &format!("docs/specs/{id}.json"), &serde_json::to_string_pretty(&spec).unwrap());
    commit_all(&dir, "spec");

    // 1. Cut S1's test out of the sealed file: the seal still exists, the move is recorded.
    let bin = fake_model("s1", "/// S1: two\n#[test]\nfn s1() {\n    assert_eq!(fixture::n(), 2);\n}\n");
    let o = hexa(&dir, &["spec", "tests", &id, "--crate", ".", "--only", "S1", "--because", "S1 changed"], Some(&bin));
    assert!(o.status.success(), "first re-proof: {}", text(&o));
    let first = chain(&dir, FILE);
    assert_eq!(first.len(), 1, "the first cut is recorded: {first:?}");
    assert_eq!(first[0].0, anchor, "and chains from the anchored hash");

    // 2. A person re-accepts after S2's behaviour changes: the file that held S2's test is unsealed.
    let path = format!("docs/specs/{id}.json");
    let edited = read(&dir, &path).replace("then 2", "then 2, amended");
    write(&dir, &path, &edited);
    commit_all(&dir, "S2 amended");
    let o = hexa(&dir, &["spec", "accept", &id, "--by", "reviewer"], None);
    assert!(o.status.success(), "re-accept: {}", text(&o));
    let after_accept: serde_json::Value = serde_json::from_str(&read(&dir, &path)).unwrap();
    assert!(after_accept["sealed"].get(FILE).is_none(), "the re-acceptance unsealed {FILE}: {:?}", after_accept["sealed"]);
    commit_all(&dir, "re-accepted");

    // 3. Re-prove S2: its test is cut from the same file and the file is sealed again.
    let bin = fake_model("s2", "/// S2: five, amended\n#[test]\nfn s2() {\n    assert_eq!(fixture::n(), 5);\n}\n");
    let o = hexa(&dir, &["spec", "tests", &id, "--crate", ".", "--only", "S2", "--because", "S2 amended"], Some(&bin));
    assert!(o.status.success(), "second re-proof: {}", text(&o));

    let now = chain(&dir, FILE);
    let sealed_now: serde_json::Value = serde_json::from_str(&read(&dir, &path)).unwrap();
    let recorded = sealed_now["sealed"][FILE].as_str().expect("the file is sealed again").to_string();
    assert_eq!(sha256_hex(read(&dir, FILE).as_bytes()), recorded, "the record matches the file");
    // The chain runs unbroken from the anchor to the recorded hash.
    let mut at = anchor.clone();
    for (i, (from, to)) in now.iter().enumerate() {
        assert_eq!(*from, at, "amendment {i} does not continue the chain: {now:?}");
        at = to.clone();
    }
    assert_eq!(at, recorded, "the chain ends at the sealed hash: {now:?}");
    assert_eq!(now.len(), 2, "both cuts are recorded: {now:?}");

    // And hexa agrees.
    let v = hexa(&dir, &["spec", "verify", &id], None);
    let t = text(&v);
    assert!(!t.contains("seal broken") && !t.contains("seal rewritten"), "verify reports the chain broken:\n{t}");
}
