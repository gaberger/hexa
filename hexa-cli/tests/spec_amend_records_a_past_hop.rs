//! `hexa spec amend --at <commit>…` records a past step of a sealed file's chain.
//!
//! Every move of a seal is an amendment (ADR-2611081151 §1). Where one went
//! unrecorded, because a hexa verb moved the seal before it recorded moves or a
//! hand-written test was edited before it was sealed, `spec verify` reports the
//! chain broken and no verb could put the step back. `--at` records it, and only
//! it: each step's hash is the file's blob at a named commit of this branch, the
//! steps run oldest first from the anchor to exactly the hash the chain already
//! needs, the diffs are shown, and nothing is written without `--confirm`.
//!
//! Ids are assembled at run time: the citation checker scans source.

use std::path::Path;
use std::process::Command;

fn run(root: &Path, args: &[&str]) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hexa")).args(args).current_dir(root).output().expect("run hexa");
    (
        out.status.code().unwrap_or(-1),
        format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)),
    )
}
fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .expect("git");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}
fn id() -> String {
    format!("ADR-{}", "2701050000")
}
const F: &str = "tests/rule.rs";
fn version(n: u32) -> String {
    format!("#[test]\nfn doubles() {{ assert_eq!({n} * 2, {n} * 2); }}\n")
}
fn spec_path(r: &Path) -> std::path::PathBuf {
    r.join("docs/specs").join(format!("{}.json", id()))
}
fn amendments(r: &Path) -> Vec<(String, String)> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(spec_path(r)).unwrap()).unwrap();
    v["amendments"].as_array().cloned().unwrap_or_default().iter().map(|a| (a["from"].as_str().unwrap().to_string(), a["to"].as_str().unwrap().to_string())).collect()
}
fn sha(bytes: &str) -> String {
    use std::io::Write;
    let mut c = Command::new("sha256sum").stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().unwrap();
    c.stdin.take().unwrap().write_all(bytes.as_bytes()).unwrap();
    String::from_utf8_lossy(&c.wait_with_output().unwrap().stdout).split_whitespace().next().unwrap().to_string()
}

/// The file committed as versions 1, 2, 3 (c1, c2, c3), then the spec accepted and
/// sealed at version 3: a seal whose chain skips version 1 -> 2 -> 3.
fn repo() -> (tempfile::TempDir, [String; 3]) {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    let f = id();
    for sub in ["docs/adrs", "docs/specs", "tests"] {
        std::fs::create_dir_all(r.join(sub)).unwrap();
    }
    std::fs::write(
        r.join("docs/adrs").join(format!("{f}-a-decision.md")),
        format!("---\nid: {f}\nstatus: accepted\ndate: 2027-01-01\n---\n# {f}: A decision\n\n**Status:** Accepted\n\n## Decision\n\n1. **It doubles.**\n\n## References\n\nnone\n"),
    )
    .unwrap();
    std::fs::write(r.join("Cargo.toml"), "[package]\nname = \"fx\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[workspace]\n").unwrap();
    git(r, &["init", "-q"]);
    let mut commits = Vec::new();
    for n in 1..=3 {
        std::fs::write(r.join(F), version(n)).unwrap();
        git(r, &["add", "-A"]);
        git(r, &["commit", "-qm", &format!("test v{n}")]);
        commits.push(git(r, &["rev-parse", "HEAD"]));
    }
    let s = |sid: &str, kind: &str| {
        format!("{{\"id\":\"{sid}\",\"decision\":[\"§1\"],\"kind\":\"{kind}\",\"description\":\"d\",\"given\":\"g\",\"when\":\"w\",\"then\":\"t\",\"test\":\"{F}::doubles\"}}")
    };
    std::fs::write(spec_path(r), format!("{{\"adr\":\"{f}\",\"feature\":\"f\",\"specs\":[{},{}]}}", s("S1", "example"), s("S2", "negative"))).unwrap();
    git(r, &["add", "docs"]);
    git(r, &["commit", "-qm", "the spec"]);
    assert_eq!(run(r, &["spec", "accept", &f, "--by", "a reviewer"]).0, 0);
    assert_eq!(run(r, &["spec", "seal", &f]).0, 0);
    git(r, &["add", "docs"]);
    git(r, &["commit", "-qm", "sealed at v3"]);
    let [a, b, c]: [String; 3] = commits.try_into().unwrap();
    (d, [a, b, c])
}

/// The first call's confirm hash, from the command it prints.
fn proposed(out: &str) -> String {
    out.split("--confirm ").nth(1).and_then(|t| t.split_whitespace().next()).unwrap_or("").to_string()
}

#[test]
fn the_unrecorded_steps_are_recorded_and_verify_stops_objecting() {
    let (d, [_c1, c2, c3]) = repo();
    let r = d.path();
    let f = id();
    let (_, before) = run(r, &["spec", "verify", &f]);
    assert!(before.contains("seal rewritten") || before.contains("seal broken"), "precondition: the chain is broken:\n{before}");

    let args = ["spec", "amend", &f, "--file", F, "--reason", "hexa seal left these steps unrecorded", "--at", &c2, "--at", &c3];
    let (code, shown) = run(r, &args);
    assert_eq!(code, 0, "the first call only shows:\n{shown}");
    assert!(amendments(r).is_empty(), "the first call writes nothing");
    assert!(shown.contains("--confirm"), "{shown}");
    assert!(shown.contains("+") && shown.contains("-"), "the diffs are shown:\n{shown}");

    let digest = proposed(&shown);
    let mut confirm = args.to_vec();
    confirm.extend(["--confirm", digest.as_str()]);
    let (code, done) = run(r, &confirm);
    assert_eq!(code, 0, "recording:\n{done}");

    let got = amendments(r);
    let (h1, h2, h3) = (sha(&version(1)), sha(&version(2)), sha(&version(3)));
    assert_eq!(got, vec![(h1, h2.clone()), (h2, h3)], "both steps, anchor to the sealed hash");
    let (_, after) = run(r, &["spec", "verify", &f]);
    assert!(!after.contains("seal rewritten") && !after.contains("seal broken"), "verify still objects:\n{after}");
}

#[test]
fn it_fills_the_gap_before_a_link_that_is_already_recorded() {
    // Ferro's shape: a later amendment exists and continues from the hash the steps must reach.
    let (d, [_c1, c2, c3]) = repo();
    let r = d.path();
    let f = id();
    std::fs::write(r.join(F), version(4)).unwrap();
    let (_, shown) = run(r, &["spec", "amend", &f, "--file", F, "--reason", "later edit"]);
    let digest = proposed(&shown);
    let (code, out) = run(r, &["spec", "amend", &f, "--file", F, "--reason", "later edit", "--confirm", &digest]);
    assert_eq!(code, 0, "the ordinary amendment:\n{out}");
    git(r, &["add", "-A"]);
    git(r, &["commit", "-qm", "v4, amended"]);
    assert_eq!(amendments(r).len(), 1);

    let args = ["spec", "amend", &f, "--file", F, "--reason", "unrecorded before", "--at", &c2, "--at", &c3];
    let (_, shown) = run(r, &args);
    let digest = proposed(&shown);
    let mut confirm = args.to_vec();
    confirm.extend(["--confirm", digest.as_str()]);
    let (code, out) = run(r, &confirm);
    assert_eq!(code, 0, "{out}");
    let got = amendments(r);
    assert_eq!(got.len(), 3, "{got:?}");
    assert_eq!(got[0].0, sha(&version(1)), "the recorded steps come first, from the anchor");
    assert_eq!(got[2], (sha(&version(3)), sha(&version(4))), "the existing link is last and untouched");
    let (_, after) = run(r, &["spec", "verify", &f]);
    assert!(!after.contains("seal rewritten") && !after.contains("seal broken"), "{after}");
}

#[test]
fn what_it_will_not_record() {
    let (d, [_c1, c2, c3]) = repo();
    let r = d.path();
    let f = id();
    let spec_before = std::fs::read_to_string(spec_path(r)).unwrap();
    let refused = |args: &[&str], why: &str| {
        let (code, out) = run(r, args);
        assert_ne!(code, 0, "{why}: must be refused:\n{out}");
        assert_eq!(std::fs::read_to_string(spec_path(r)).unwrap(), spec_before, "{why}: nothing is written");
        out
    };
    // Steps that stop short of the hash the chain needs.
    let out = refused(&["spec", "amend", &f, "--file", F, "--reason", "r", "--at", &c2, "--confirm", "0"], "short of the target");
    assert!(out.contains("do not reach where the chain needs to"), "{out}");
    // Out of order.
    refused(&["spec", "amend", &f, "--file", F, "--reason", "r", "--at", &c3, "--at", &c2, "--confirm", "0"], "newest first");
    // Not a commit.
    refused(&["spec", "amend", &f, "--file", F, "--reason", "r", "--at", "deadbeefdeadbeef", "--confirm", "0"], "no such commit");
    // A confirmation for content that was not shown.
    refused(&["spec", "amend", &f, "--file", F, "--reason", "r", "--at", &c2, "--at", &c3, "--confirm", &"0".repeat(64)], "wrong confirm");
    // No reason, and no file.
    refused(&["spec", "amend", &f, "--file", F, "--reason", "  ", "--at", &c2, "--at", &c3], "empty reason");
    refused(&["spec", "amend", &f, "--reason", "r", "--at", &c2, "--at", &c3], "no --file");
}
