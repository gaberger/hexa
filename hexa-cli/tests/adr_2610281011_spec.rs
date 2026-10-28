//! Integration tests for ADR-2610281011: the design stage of the hexa loop,
//! review coverage, `hexa design adr`, ADR design lines, `hexa go`'s design
//! suggestion, `hexa drift --since`, routing and guidance.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use hexa_core::domain::design::{self, Stage};
use hexa_core::domain::intent::{self, Routed};
use hexa_core::domain::workflow::{self, DesignNext, DesignState, LoopStage};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const DRAFT: &str = "docs/design/idea.md";
const STAGE_WORDS: [&str; 6] = ["design", "decide", "gate", "build", "harden", "done"];

static COUNTER: AtomicUsize = AtomicUsize::new(0);

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
    let bitlen = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
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
        let mut a = h;
        for i in 0..64 {
            let s1 = a[4].rotate_right(6) ^ a[4].rotate_right(11) ^ a[4].rotate_right(25);
            let ch = (a[4] & a[5]) ^ (!a[4] & a[6]);
            let t1 = a[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a[0].rotate_right(2) ^ a[0].rotate_right(13) ^ a[0].rotate_right(22);
            let maj = (a[0] & a[1]) ^ (a[0] & a[2]) ^ (a[1] & a[2]);
            let t2 = s0.wrapping_add(maj);
            a[7] = a[6];
            a[6] = a[5];
            a[5] = a[4];
            a[4] = a[3].wrapping_add(t1);
            a[3] = a[2];
            a[2] = a[1];
            a[1] = a[0];
            a[0] = t1.wrapping_add(t2);
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(a[i]);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}

/// A review report with an optional hash line and a section per stage number.
fn report(sha: Option<&str>, stages: &[u8]) -> String {
    let mut out = String::from("# Elephant-Goldfish review\n\nDraft: docs/design/idea.md\n");
    if let Some(sha) = sha {
        out.push_str(&format!("Draft sha256: {sha}\n"));
    }
    out.push('\n');
    for n in stages {
        let stage = design::stage_from_number(*n).expect("stage number 5 to 9");
        out.push_str(&format!("## Stage {n}: {}\n\n", design::stage_name(stage)));
        out.push_str(&format!("- S{n}-1: A finding for stage {n}.\n\n"));
    }
    out
}

fn text(o: &Output) -> String {
    format!("{}\n{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

struct Project {
    base: PathBuf,
    root: PathBuf,
    home: PathBuf,
}

impl Project {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let base = std::env::temp_dir().join(format!("hexa-it-{tag}-{}-{n}-{nanos}", std::process::id()));
        let root = base.join("proj");
        let home = base.join("home");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        let p = Project { base, root, home };
        p.git(&["init", "-q", "-b", "main"]);
        p.git(&["config", "user.email", "test@example.com"]);
        p.git(&["config", "user.name", "Test"]);
        p.git(&["config", "commit.gpgsign", "false"]);
        p
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    fn write(&self, rel: &str, contents: &str) {
        let path = self.path(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    fn git(&self, args: &[&str]) -> Output {
        let o = Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .env("HOME", &self.home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .expect("git runs");
        assert!(o.status.success(), "git {args:?} failed: {}", text(&o));
        o
    }

    fn commit_all(&self, msg: &str) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "-m", msg]);
    }

    fn hexa(&self, args: &[&str]) -> Output {
        let home = &self.home;
        Command::new(env!("CARGO_BIN_EXE_hexa"))
            .args(args)
            .current_dir(&self.root)
            .env("HOME", home)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("XDG_DATA_HOME", home.join(".local/share"))
            .env("XDG_CACHE_HOME", home.join(".cache"))
            .env("XDG_STATE_HOME", home.join(".local/state"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("OPENAI_API_KEY")
            .env_remove("GEMINI_API_KEY")
            .env_remove("OPENROUTER_API_KEY")
            .output()
            .expect("hexa runs")
    }

    fn loop_text(&self) -> String {
        text(&self.hexa(&["loop"]))
    }

    /// The stage word the loop's `stage` line shows.
    fn stage(&self) -> Option<String> {
        for line in self.loop_text().lines() {
            let l = line.to_lowercase();
            let Some(i) = l.find("stage") else { continue };
            let after = &l[i + 5..];
            for w in after.split(|c: char| !c.is_alphanumeric()) {
                if STAGE_WORDS.contains(&w) {
                    return Some(w.to_string());
                }
            }
        }
        None
    }

    fn snapshot(&self) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut out = BTreeMap::new();
        walk(&self.root, &mut |p| {
            if let Ok(bytes) = std::fs::read(p) {
                out.insert(p.to_path_buf(), bytes);
            }
        });
        out
    }

    /// The text of every file that is new or changed since `before`.
    fn fresh_since(&self, before: &BTreeMap<PathBuf, Vec<u8>>) -> String {
        let mut out = String::new();
        for (path, bytes) in self.snapshot() {
            if before.get(&path) != Some(&bytes) {
                out.push_str(&String::from_utf8_lossy(&bytes));
                out.push('\n');
            }
        }
        out
    }

    fn adrs(&self) -> BTreeMap<String, Vec<u8>> {
        let mut out = BTreeMap::new();
        if let Ok(rd) = std::fs::read_dir(self.path("docs/adrs")) {
            for e in rd.flatten() {
                if e.path().is_file() {
                    out.insert(e.file_name().to_string_lossy().to_string(), std::fs::read(e.path()).unwrap());
                }
            }
        }
        out
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

fn walk(dir: &Path, f: &mut dyn FnMut(&Path)) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if name == ".git" || name == "target" || name == "node_modules" {
            continue;
        }
        if p.is_dir() {
            walk(&p, f);
        } else {
            f(&p);
        }
    }
}

const EXISTING_ADR: &str = "# ADR-2610010900: Existing decision\n\n**Status:** Accepted\n**Date:** 2026-10-01\n\n## Context\n\nSomething.\n\n## Decision\n\n1. We do the thing.\n\n## Consequences\n\nFine.\n";

fn adr_with(id: &str, title: &str, design_line: Option<&str>) -> String {
    let mut out = format!("# {id}: {title}\n\n**Status:** Accepted\n**Date:** 2026-10-01\n");
    if let Some(d) = design_line {
        out.push_str(&format!("**Design:** {d}\n"));
    }
    out.push_str("\n## Context\n\nSomething.\n\n## Decision\n\n1. We do the thing.\n\n## Consequences\n\nFine.\n");
    out
}

const DRAFT_V1: &str = "# Offline sync\n\nA design for an offline sync mode.\n\n## Proposal\n\nQueue writes locally.\n";
const DRAFT_V2: &str = "# Offline sync\n\nA design for an offline sync mode.\n\n## Proposal\n\nQueue writes locally and replay them on reconnect.\n";

/// A project with CLAUDE.md naming the artifact directory, an existing ADR
/// and a draft with the given contents.
fn design_project(tag: &str, draft: &str) -> Project {
    let p = Project::new(tag);
    p.write("CLAUDE.md", "# Project\n\nEGM artifact directory: design/egm\n");
    p.write("docs/adrs/ADR-2610010900-existing-decision.md", EXISTING_ADR);
    p.write(DRAFT, draft);
    p
}

fn workspace_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .ancestors()
        .find(|a| a.join("hexa-core").is_dir())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| manifest.parent().unwrap_or(&manifest).to_path_buf())
}

fn skill_text() -> String {
    let root = workspace_root();
    let mut found: Option<PathBuf> = None;
    let mut fallback: Option<PathBuf> = None;
    walk(&root, &mut |p| {
        let s = p.to_string_lossy();
        if s.contains("hexa-workflow") {
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if name == "SKILL.md" && found.is_none() {
                found = Some(p.to_path_buf());
            } else if name == "hexa-workflow.md" && fallback.is_none() {
                fallback = Some(p.to_path_buf());
            }
        }
    });
    let path = found.or(fallback).expect("the /hexa-workflow skill exists in the workspace");
    std::fs::read_to_string(path).unwrap()
}

const HOOK_NEEDLES: [&str; 3] = [
    r#"Design (`hexa design brainstorm "<idea>"`, optional) → Decide"#,
    r#"Design (`hexa design brainstorm \"<idea>\"`, optional) → Decide"#,
    r#"Design (`hexa design brainstorm \"<idea>\"`, optional) \u{2192} Decide"#,
];

/// Source files (under a `src` directory) holding the prompt hook's design guidance.
fn hook_sources_with_design_guidance() -> Vec<PathBuf> {
    let root = workspace_root();
    let mut out = Vec::new();
    walk(&root, &mut |p| {
        let in_src = p.components().any(|c| c.as_os_str() == "src");
        if !in_src || p.extension().map(|e| e != "rs").unwrap_or(true) {
            return;
        }
        if let Ok(s) = std::fs::read_to_string(p) {
            if HOOK_NEEDLES.iter().any(|n| s.contains(n)) {
                out.push(p.to_path_buf());
            }
        }
    });
    out
}

fn assert_design_adr_exists(p: &Project) {
    let o = p.hexa(&["design", "adr", "--help"]);
    assert!(o.status.success(), "`hexa design adr` exists: {}", text(&o));
}

// ---------------------------------------------------------------------------
// §1: loop stages
// ---------------------------------------------------------------------------

/// S01: The loop's stages are design, decide, gate, build, harden, done, in that order
#[test]
fn s01_loop_stages_in_order() {
    use LoopStage::*;
    let stages = workflow::loop_stages();
    assert_eq!(stages, vec![Design, Decide, Gate, Build, Harden, Done]);
    let words: Vec<&str> = stages.iter().map(|s| workflow::stage_word(*s)).collect();
    assert_eq!(words, STAGE_WORDS.to_vec());
}

/// S02: parse_stage reads the design stage
#[test]
fn s02_parse_stage_design() {
    assert_eq!(workflow::parse_stage("design"), Some(LoopStage::Design));
}

/// S03: parse_stage refuses a word that names no stage
#[test]
fn s03_parse_stage_refuses_unknown_words() {
    for w in ["", "brainstorm", "review", "designing"] {
        assert_eq!(workflow::parse_stage(w), None, "{w:?} names no stage");
    }
    assert_eq!(workflow::parse_stage("decide"), Some(LoopStage::Decide));
}

/// S04: stage_word and parse_stage round-trip for every stage, and the stage order matches the enum order
#[test]
fn s04_stage_words_round_trip() {
    let stages = workflow::loop_stages();
    assert!(!stages.is_empty());
    for s in &stages {
        assert_eq!(workflow::parse_stage(workflow::stage_word(*s)), Some(*s));
    }
    assert!(stages.windows(2).all(|w| w[0] < w[1]), "strictly increasing: {stages:?}");
    let words: BTreeSet<&str> = stages.iter().map(|s| workflow::stage_word(*s)).collect();
    assert_eq!(words.len(), stages.len(), "no two stages share a word");
}

/// S05: hexa loop design records a draft inside the project and sets the stage to design
#[test]
fn s05_loop_design_records_draft() {
    let p = design_project("s05", DRAFT_V1);
    p.commit_all("init");
    let _ = p.hexa(&["loop", "stage", "gate"]);
    let o = p.hexa(&["loop", "design", DRAFT]);
    assert!(o.status.success(), "hexa loop design succeeds: {}", text(&o));
    assert_eq!(p.stage().as_deref(), Some("design"), "loop: {}", p.loop_text());
    assert!(p.loop_text().contains(DRAFT), "the loop records the draft: {}", p.loop_text());
}

/// S06: hexa loop design refuses a draft that is not a file inside the project
#[test]
fn s06_loop_design_refuses_outside_or_missing() {
    let p = Project::new("s06");
    std::fs::write(p.base.join("outside.md"), "# Outside\n").unwrap();
    p.write("docs/designs/d.md", "# D\n\nA draft.\n");
    p.commit_all("init");
    let _ = p.hexa(&["loop", "stage", "build"]);
    let before = p.stage();
    for bad in ["ghost.md", "../outside.md", "/etc/hosts"] {
        let o = p.hexa(&["loop", "design", bad]);
        let t = text(&o);
        assert!(!o.status.success(), "`hexa loop design {bad}` fails: {t}");
        assert!(t.contains(bad), "the error names {bad}: {t}");
        assert!(t.to_lowercase().contains("inside the project"), "the error says not a file inside the project: {t}");
        assert_eq!(p.stage(), before, "the stage is unchanged after {bad}");
        assert!(!p.loop_text().contains(bad), "no draft {bad} is recorded");
    }
    let o = p.hexa(&["loop", "design", "docs/designs/d.md"]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(p.loop_text().contains("docs/designs/d.md"));
}

/// S07: Design is optional: a loop with no design recorded starts at decide
#[test]
fn s07_fresh_loop_starts_at_decide() {
    let p = Project::new("s07");
    p.write("README.md", "# Project\n");
    p.commit_all("init");
    assert_eq!(p.stage().as_deref(), Some("decide"), "loop: {}", p.loop_text());
}

// ---------------------------------------------------------------------------
// §2: review coverage
// ---------------------------------------------------------------------------

/// S08: reviewed_sha reads back the recorded draft hash
#[test]
fn s08_reviewed_sha_reads_hash() {
    let r = "# Review\n\nSome intro.\nDraft sha256: ab12cd34\n\n## Stage 5: Initial zoom out\n\n- S5-1: x\n";
    assert_eq!(design::reviewed_sha(r), Some("ab12cd34".to_string()));
}

/// S09: reviewed_sha finds nothing in a report with no hash line
#[test]
fn s09_reviewed_sha_none_without_line() {
    assert_eq!(design::reviewed_sha(&report(None, &[5, 6, 7, 8])), None);
}

/// S10: review_covers accepts a report with the matching hash and sections for stages 5 to 8
#[test]
fn s10_review_covers_ok() {
    let r = report(Some("H"), &[5, 6, 7, 8]);
    assert_eq!(design::review_covers(&r, "H"), Ok(()));
    assert_eq!(
        design::reviewed_stages(&r),
        vec![Stage::ZoomOut, Stage::Comprehension, Stage::Critic, Stage::IntermediateZoomOut]
    );
}

/// S11: review_covers refuses a report recorded against a different hash
#[test]
fn s11_review_covers_refuses_other_hash() {
    let r = report(Some("H1"), &[5, 6, 7, 8]);
    let err = design::review_covers(&r, "H2").expect_err("a different hash");
    let l = err.to_lowercase();
    assert!(l.contains("differ") || l.contains("match"), "says the hash differs: {err}");
}

/// S12: review_covers refuses a report with no hash
#[test]
fn s12_review_covers_refuses_missing_hash() {
    let r = report(None, &[5, 6, 7, 8]);
    let err = design::review_covers(&r, "H").expect_err("no hash");
    let l = err.to_lowercase();
    assert!(l.contains("missing") || l.contains("no draft sha256") || l.contains("not record"), "says the hash is missing: {err}");
}

/// S13: review_covers names the missing stage numbers
#[test]
fn s13_review_covers_names_missing_stages() {
    let r = report(Some("H"), &[5, 7]);
    let err = design::review_covers(&r, "H").expect_err("stages missing");
    assert!(err.contains('6') && err.contains('8'), "names stages 6 and 8: {err}");
}

/// S14: A stage-9-only report does not cover a draft
#[test]
fn s14_stage_nine_only_does_not_cover() {
    let r = report(Some("H"), &[9]);
    let err = design::review_covers(&r, "H").expect_err("stage 9 only");
    for n in ['5', '6', '7', '8'] {
        assert!(err.contains(n), "names stage {n}: {err}");
    }
}

/// S15: review_covers is Ok exactly when the hash matches and stages 5 to 8 are all present
#[test]
fn s15_review_covers_property() {
    let shas = [None, Some("H"), Some("H1")];
    let stage_sets: Vec<Vec<u8>> = vec![
        vec![],
        vec![5],
        vec![5, 7],
        vec![9],
        vec![5, 6, 7],
        vec![5, 6, 7, 8],
        vec![5, 6, 7, 8, 9],
        vec![8, 7, 6, 5],
        vec![6, 7, 8, 9],
    ];
    let needed = [Stage::ZoomOut, Stage::Comprehension, Stage::Critic, Stage::IntermediateZoomOut];
    for sha in shas {
        for stages in &stage_sets {
            let r = report(sha, stages);
            for draft in ["H", "H1", "H2"] {
                let reviewed = design::reviewed_stages(&r);
                let expect = design::reviewed_sha(&r).as_deref() == Some(draft)
                    && needed.iter().all(|s| reviewed.contains(s));
                assert_eq!(design::review_covers(&r, draft).is_ok(), expect, "report {r:?} draft {draft}");
                if expect && !stages.contains(&9) {
                    let mut with9 = stages.clone();
                    with9.push(9);
                    assert!(design::review_covers(&report(sha, &with9), draft).is_ok(), "stage 9 keeps Ok");
                }
            }
        }
    }
}

/// S16: hexa design review records the hash of the draft bytes it read
#[test]
fn s16_design_review_records_draft_hash() {
    let p = design_project("s16", DRAFT_V1);
    p.commit_all("init");
    let h = sha256_hex(DRAFT_V1.as_bytes());
    let before = p.snapshot();
    let o = p.hexa(&["design", "review", DRAFT]);
    let fresh = format!("{}\n{}", p.fresh_since(&before), text(&o));
    assert!(fresh.contains(&format!("Draft sha256: {h}")), "the report records `Draft sha256: {h}`: {fresh}");
}

// ---------------------------------------------------------------------------
// §3: hexa design adr
// ---------------------------------------------------------------------------

/// S17: A reviewed, committed draft becomes a proposed ADR that cites it
#[test]
fn s17_design_adr_writes_proposed_adr() {
    let p = design_project("s17", DRAFT_V1);
    let h = sha256_hex(DRAFT_V1.as_bytes());
    p.write("design/egm/review-idea.md", &report(Some(&h), &[5, 6, 7, 8]));
    p.commit_all("draft and review");
    let _ = p.hexa(&["loop", "design", DRAFT]);
    let adrs_before = p.adrs();
    let before = p.snapshot();
    let o = p.hexa(&["design", "adr", DRAFT]);
    assert!(o.status.success(), "hexa design adr succeeds: {}", text(&o));
    let adrs_after = p.adrs();
    let new: Vec<&String> = adrs_after.keys().filter(|k| !adrs_before.contains_key(*k)).collect();
    assert_eq!(new.len(), 1, "exactly one new ADR: {new:?}");
    let name = new[0].clone();
    assert!(name.starts_with("ADR-") && name.ends_with(".md"), "named ADR-<id>-<slug>.md: {name}");
    let body = String::from_utf8_lossy(&adrs_after[&name]).to_string();
    assert!(body.contains("**Status:** Proposed"), "{body}");
    assert!(!body.contains("**Status:** Accepted"), "not accepted: {body}");
    assert!(body.contains(&format!("**Design:** {DRAFT}")), "{body}");
    let decision = body.find("## Decision").expect("a Decision section");
    assert!(
        body[decision..].lines().any(|l| l.trim_start().starts_with("1.")),
        "numbered decisions: {body}"
    );
    // The earlier ADR is untouched.
    assert_eq!(adrs_before.iter().find(|(k, _)| **k != name).map(|(_, v)| v), adrs_after.get("ADR-2610010900-existing-decision.md"));
    // No spec is written.
    for (path, bytes) in p.snapshot() {
        if before.get(&path) != Some(&bytes) {
            let s = path.to_string_lossy().to_lowercase();
            assert!(!s.contains("spec"), "no spec is written: {s}");
        }
    }
    assert_eq!(p.stage().as_deref(), Some("decide"), "loop: {}", p.loop_text());
    let id: String = name.split('-').take(2).collect::<Vec<_>>().join("-");
    assert!(p.loop_text().contains(&id), "the loop records {id}: {}", p.loop_text());
}

/// S18: hexa design adr refuses a draft with uncommitted changes
#[test]
fn s18_design_adr_refuses_uncommitted() {
    let p = design_project("s18", DRAFT_V1);
    p.commit_all("init");
    p.write(DRAFT, DRAFT_V2);
    let h = sha256_hex(DRAFT_V2.as_bytes());
    p.write("design/egm/review-idea.md", &report(Some(&h), &[5, 6, 7, 8]));
    let _ = p.hexa(&["loop", "design", DRAFT]);
    assert_design_adr_exists(&p);
    let adrs_before = p.adrs();
    let stage_before = p.stage();
    let loop_before = p.loop_text();
    let o = p.hexa(&["design", "adr", DRAFT]);
    assert!(!o.status.success(), "fails: {}", text(&o));
    assert_eq!(p.adrs(), adrs_before, "nothing written under docs/adrs/");
    assert_eq!(p.stage(), stage_before, "the loop's stage is unchanged");
    assert_eq!(p.loop_text().contains("ADR-"), loop_before.contains("ADR-"), "no ADR recorded");
}

/// S19: hexa design adr refuses a draft that git does not track
#[test]
fn s19_design_adr_refuses_untracked() {
    let p = Project::new("s19");
    p.write("CLAUDE.md", "# Project\n\nEGM artifact directory: design/egm\n");
    p.write("docs/adrs/ADR-2610010900-existing-decision.md", EXISTING_ADR);
    p.commit_all("init");
    p.write(DRAFT, DRAFT_V1);
    let h = sha256_hex(DRAFT_V1.as_bytes());
    p.write("design/egm/review-idea.md", &report(Some(&h), &[5, 6, 7, 8]));
    assert_design_adr_exists(&p);
    let adrs_before = p.adrs();
    let o = p.hexa(&["design", "adr", DRAFT]);
    let t = text(&o);
    assert!(!o.status.success(), "fails: {t}");
    assert!(t.to_lowercase().contains("not tracked"), "says the draft is not tracked by git: {t}");
    assert_eq!(p.adrs(), adrs_before, "no ADR written");
}

/// S20: A draft revised after its review must be reviewed again
#[test]
fn s20_design_adr_refuses_stale_review() {
    let p = design_project("s20", DRAFT_V2);
    let h1 = sha256_hex(DRAFT_V1.as_bytes());
    p.write("design/egm/review-idea.md", &report(Some(&h1), &[5, 6, 7, 8]));
    p.commit_all("revised draft");
    assert_design_adr_exists(&p);
    let adrs_before = p.adrs();
    let o = p.hexa(&["design", "adr", DRAFT]);
    let t = text(&o);
    assert!(!o.status.success(), "fails: {t}");
    assert!(t.to_lowercase().contains("no review covers"), "says no review covers the current bytes: {t}");
    assert_eq!(p.adrs(), adrs_before, "no ADR written");
}

/// S21: No other hexa design command writes under docs/adrs/
#[test]
fn s21_other_design_commands_leave_adrs_alone() {
    let p = design_project("s21", DRAFT_V1);
    p.write("docs/adrs/ADR-2610011000-another.md", &adr_with("ADR-2610011000", "Another", None));
    p.commit_all("init");
    assert_design_adr_exists(&p);
    let before = p.adrs();
    let runs: [&[&str]; 4] = [
        &["design", "brainstorm", "an offline sync mode"],
        &["design", "draft", DRAFT],
        &["design", "review", DRAFT],
        &["design", "resolve", DRAFT],
    ];
    for args in runs {
        let _ = p.hexa(args);
        assert_eq!(p.adrs(), before, "`hexa {}` leaves docs/adrs/ unchanged", args.join(" "));
    }
}

// ---------------------------------------------------------------------------
// §4: ADR design lines
// ---------------------------------------------------------------------------

/// S22: An ADR whose Design line resolves passes the doctor
#[test]
fn s22_adr_doctor_accepts_resolving_design() {
    let p = Project::new("s22");
    p.write("docs/design/idea.md", DRAFT_V1);
    p.write(
        "docs/adrs/ADR-2610011000-with-design.md",
        &adr_with("ADR-2610011000", "With design", Some("docs/design/idea.md")),
    );
    p.commit_all("init");
    let t = text(&p.hexa(&["adr", "doctor"]));
    assert!(
        !t.lines().any(|l| l.contains("docs/design/idea.md")),
        "no error for the design line: {t}"
    );
    std::fs::remove_file(p.path("docs/design/idea.md")).unwrap();
    let t = text(&p.hexa(&["adr", "doctor"]));
    let hits: Vec<&str> = t
        .lines()
        .filter(|l| l.contains("docs/design/idea.md"))
        .collect();
    assert!(!hits.is_empty(), "an error naming the path: {t}");
    assert!(t.contains("ADR-2610011000"), "the error names the ADR: {t}");
}

/// S23: hexa adr doctor reports a Design line that does not resolve
#[test]
fn s23_adr_doctor_reports_missing_design() {
    let p = Project::new("s23");
    p.write(
        "docs/adrs/ADR-2610011100-gone-design.md",
        &adr_with("ADR-2610011100", "Gone design", Some("docs/design/gone.md")),
    );
    p.commit_all("init");
    let t = text(&p.hexa(&["adr", "doctor"]));
    assert!(t.contains("docs/design/gone.md"), "names the path: {t}");
    assert!(t.contains("ADR-2610011100"), "names the ADR: {t}");
}

/// S24: An ADR with no Design line is not an error
#[test]
fn s24_adr_without_design_is_fine() {
    let p = Project::new("s24");
    p.write("docs/adrs/ADR-2610011200-no-design.md", &adr_with("ADR-2610011200", "No design", None));
    p.write(
        "docs/adrs/ADR-2610011300-missing-design.md",
        &adr_with("ADR-2610011300", "Missing design", Some("docs/design/absent.md")),
    );
    p.commit_all("init");
    let t = text(&p.hexa(&["adr", "doctor"]));
    assert!(t.contains("docs/design/absent.md") && t.contains("ADR-2610011300"), "error for the second ADR: {t}");
    assert!(
        !t.lines().any(|l| l.contains("ADR-2610011200") && l.to_lowercase().contains("design")),
        "no design error about the first ADR: {t}"
    );
}

// ---------------------------------------------------------------------------
// §5: next design action
// ---------------------------------------------------------------------------

fn state(draft: Option<&str>, sha: Option<&str>, reviews: Vec<String>, committed: bool) -> DesignState {
    DesignState {
        draft: draft.map(str::to_string),
        draft_sha256: sha.map(str::to_string),
        reviews,
        committed,
    }
}

/// S25: Each design state maps to its next action
#[test]
fn s25_design_states_map_to_actions() {
    let d = "docs/design/d.md";
    let cov = report(Some("H"), &[5, 6, 7, 8]);
    assert_eq!(workflow::next_design_action(&state(None, None, vec![], false)), DesignNext::Nothing);
    assert_eq!(
        workflow::next_design_action(&state(Some(d), Some("H"), vec![], true)),
        DesignNext::Review(d.to_string())
    );
    assert_eq!(
        workflow::next_design_action(&state(Some(d), Some("H"), vec![cov.clone()], false)),
        DesignNext::Commit(d.to_string())
    );
    assert_eq!(
        workflow::next_design_action(&state(Some(d), Some("H"), vec![cov], true)),
        DesignNext::ToAdr(d.to_string())
    );
}

/// S26: A review of an earlier revision does not advance past Review, even when the draft is committed
#[test]
fn s26_stale_review_stays_at_review() {
    let d = "docs/design/d.md";
    let s = state(Some(d), Some("H2"), vec![report(Some("H1"), &[5, 6, 7, 8])], true);
    assert_eq!(workflow::next_design_action(&s), DesignNext::Review(d.to_string()));
}

/// S27: hexa go prints no design suggestion when no draft is recorded
#[test]
fn s27_go_without_draft_suggests_no_design() {
    let p = design_project("s27", DRAFT_V1);
    p.commit_all("init");
    let t = text(&p.hexa(&["go"]));
    for s in ["hexa design review", "hexa design adr", "git commit"] {
        assert!(!t.contains(s), "no `{s}` suggestion: {t}");
    }
    let o = p.hexa(&["loop", "design", DRAFT]);
    assert!(o.status.success(), "{}", text(&o));
    let t = text(&p.hexa(&["go"]));
    assert!(t.contains(&format!("hexa design review {DRAFT}")), "{t}");
}

/// S28: hexa go prints the verb that matches the next action
#[test]
fn s28_go_prints_matching_verb() {
    let p = Project::new("s28");
    p.write("CLAUDE.md", "# Project\n\nEGM artifact directory: design/egm\n");
    p.write("docs/adrs/ADR-2610010900-existing-decision.md", EXISTING_ADR);
    p.commit_all("init");
    p.write(DRAFT, DRAFT_V1);
    let o = p.hexa(&["loop", "design", DRAFT]);
    assert!(o.status.success(), "{}", text(&o));

    let t = text(&p.hexa(&["go"]));
    assert!(t.contains(&format!("hexa design review {DRAFT}")), "Review: {t}");

    let h = sha256_hex(DRAFT_V1.as_bytes());
    p.write("design/egm/review-idea.md", &report(Some(&h), &[5, 6, 7, 8]));
    let t = text(&p.hexa(&["go"]));
    assert!(t.contains(&format!("git commit {DRAFT}")), "Commit: {t}");

    p.commit_all("draft and review");
    let t = text(&p.hexa(&["go"]));
    assert!(t.contains(&format!("hexa design adr {DRAFT}")), "ToAdr: {t}");
}

/// S29: The next action follows from coverage and commit state for every state
#[test]
fn s29_next_action_property() {
    let d = "docs/design/d.md";
    let review_sets: Vec<Vec<String>> = vec![
        vec![],
        vec![report(Some("H"), &[5, 6, 7, 8])],
        vec![report(Some("H1"), &[5, 6, 7, 8])],
        vec![report(Some("H"), &[9])],
        vec![report(Some("H"), &[5, 7])],
        vec![report(None, &[5, 6, 7, 8])],
        vec![report(Some("H1"), &[5, 6, 7, 8]), report(Some("H"), &[5, 6, 7, 8, 9])],
    ];
    for draft in [None, Some(d)] {
        for sha in [None, Some("H"), Some("H1")] {
            for reviews in &review_sets {
                for committed in [false, true] {
                    let s = state(draft, sha, reviews.clone(), committed);
                    let got = workflow::next_design_action(&s);
                    let covered = sha.is_some_and(|h| reviews.iter().any(|r| design::review_covers(r, h).is_ok()));
                    let want = match draft {
                        None => DesignNext::Nothing,
                        Some(d) if !covered => DesignNext::Review(d.to_string()),
                        Some(d) if !committed => DesignNext::Commit(d.to_string()),
                        Some(d) => DesignNext::ToAdr(d.to_string()),
                    };
                    assert_eq!(got, want, "state {s:?}");
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// §6: drift
// ---------------------------------------------------------------------------

/// S30: docs_touched returns changed documents and documents that name changed paths, in docs order
#[test]
fn s30_docs_touched_example() {
    let docs = vec![
        ("a.md".to_string(), vec!["src/x.rs".to_string()]),
        ("b.md".to_string(), vec!["src/y.rs".to_string()]),
        ("c.md".to_string(), vec![]),
    ];
    let changed = vec!["c.md".to_string(), "src/x.rs".to_string()];
    assert_eq!(design::docs_touched(&docs, &changed), vec!["a.md".to_string(), "c.md".to_string()]);
}

/// S31: docs_touched returns nothing for a change that touches no document
#[test]
fn s31_docs_touched_empty() {
    let docs = vec![("a.md".to_string(), vec!["src/x.rs".to_string()])];
    assert!(design::docs_touched(&docs, &["src/z.rs".to_string()]).is_empty());
    assert_eq!(design::docs_touched(&docs, &["src/x.rs".to_string()]), vec!["a.md".to_string()]);
}

fn drift_project(tag: &str) -> Project {
    let p = Project::new(tag);
    p.write("docs/a.md", "# A\n\nThis document describes `src/x.rs`.\n");
    p.write("docs/b.md", "# B\n\nThis document describes `src/y.rs`.\n");
    p.write("docs/c.md", "# C\n\nThis document describes `src/w.rs`.\n");
    p.write("src/x.rs", "pub fn x() {}\n");
    p.write("src/y.rs", "pub fn y() {}\n");
    p.write("src/w.rs", "pub fn w() {}\n");
    p.write("src/z.rs", "pub fn z() {}\n");
    p.commit_all("init");
    p.git(&["tag", "base"]);
    p
}

/// S32: hexa drift --since makes no completion when nothing is touched
#[test]
fn s32_drift_since_nothing_touched() {
    let p = drift_project("s32");
    p.write("src/z.rs", "pub fn z() { let _ = 1; }\n");
    p.commit_all("change z");
    let before = p.snapshot();
    let o = p.hexa(&["drift", "--since", "base"]);
    let fresh = format!("{}\n{}", text(&o), p.fresh_since(&before));
    assert!(fresh.contains("No document touched since base"), "{fresh}");
    assert!(o.status.success(), "no completion is needed, so it succeeds without a provider: {fresh}");
}

/// S33: hexa drift --since runs one goldfish per touched document and no others
#[test]
fn s33_drift_since_checks_touched_only() {
    let p = drift_project("s33");
    p.write("src/x.rs", "pub fn x() { let _ = 1; }\n");
    p.write("src/y.rs", "pub fn y() { let _ = 2; }\n");
    p.commit_all("change x and y");
    let before = p.snapshot();
    let o = p.hexa(&["drift", "--since", "base"]);
    let fresh = format!("{}\n{}", text(&o), p.fresh_since(&before));
    assert!(!fresh.contains("No document touched"), "{fresh}");
    assert!(fresh.contains("docs/a.md"), "a.md is checked: {fresh}");
    assert!(fresh.contains("docs/b.md"), "b.md is checked: {fresh}");
    assert!(!fresh.contains("docs/c.md"), "c.md is not checked: {fresh}");
}

/// S34: Without --since, drift checks every document
#[test]
fn s34_drift_without_since_checks_all() {
    let p = drift_project("s34");
    let before = p.snapshot();
    let o = p.hexa(&["drift"]);
    let fresh = format!("{}\n{}", text(&o), p.fresh_since(&before));
    for d in ["docs/a.md", "docs/b.md", "docs/c.md"] {
        assert!(fresh.contains(d), "{d} is checked: {fresh}");
    }
    // `--since` is accepted by the same command.
    let o = p.hexa(&["drift", "--help"]);
    assert!(text(&o).contains("--since"), "{}", text(&o));
}

/// S35: docs_touched is exactly the ordered, deduplicated set of documents changed or naming a change
#[test]
fn s35_docs_touched_property() {
    let pool = ["a.md", "b.md", "c.md", "src/x.rs", "src/y.rs", "src/z.rs"];
    let docs_pool = ["a.md", "b.md", "c.md", "d.md"];
    let mut seed: u64 = 0x9e3779b97f4a7c15;
    let mut next = move |n: usize| -> usize {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((seed >> 33) as usize) % n
    };
    for _ in 0..300 {
        let ndocs = next(5);
        let docs: Vec<(String, Vec<String>)> = (0..ndocs)
            .map(|_| {
                let name = docs_pool[next(docs_pool.len())].to_string();
                let names = (0..next(3)).map(|_| pool[next(pool.len())].to_string()).collect();
                (name, names)
            })
            .collect();
        let changed: Vec<String> = (0..next(4)).map(|_| pool[next(pool.len())].to_string()).collect();
        let got = design::docs_touched(&docs, &changed);
        let mut want: Vec<String> = Vec::new();
        for (doc, names) in &docs {
            let touched = changed.contains(doc) || names.iter().any(|n| changed.contains(n));
            if touched && !want.contains(doc) {
                want.push(doc.clone());
            }
        }
        assert_eq!(got, want, "docs {docs:?} changed {changed:?}");
        assert!(design::docs_touched(&docs, &[]).is_empty());
    }
}

// ---------------------------------------------------------------------------
// §7: routing
// ---------------------------------------------------------------------------

/// S36: A brainstorm request routes to design brainstorm with the idea, behind confirmation
#[test]
fn s36_route_brainstorm() {
    match intent::route("brainstorm an offline sync mode") {
        Routed::Hexa(r) => {
            assert!(r.command.starts_with("design brainstorm \""), "{}", r.command);
            let quoted = &r.command["design brainstorm \"".len()..];
            let idea = quoted.split('"').next().unwrap_or("");
            assert!(idea.contains("offline sync mode"), "{}", r.command);
            assert!(r.destructive);
        }
        other => panic!("expected a hexa route, got {other:?}"),
    }
}

/// S37: A design review request with a path routes to design review
#[test]
fn s37_route_design_review() {
    match intent::route("review the design in docs/design/idea.md") {
        Routed::Hexa(r) => {
            assert_eq!(r.command, "design review docs/design/idea.md");
            assert!(!r.destructive);
        }
        other => panic!("expected a hexa route, got {other:?}"),
    }
}

/// S38: A design review request without a path has no route
#[test]
fn s38_route_design_review_needs_path() {
    assert_eq!(intent::route("review my design"), Routed::None);
    match intent::route("review my design docs/designs/d.md") {
        Routed::Hexa(r) => assert_eq!(r.command, "design review docs/designs/d.md"),
        other => panic!("expected a hexa route, got {other:?}"),
    }
}

/// S39: Drift requests route to drift
#[test]
fn s39_route_drift() {
    for req in ["check for drift", "do the docs match the code?"] {
        match intent::route(req) {
            Routed::Hexa(r) => {
                assert_eq!(r.command, "drift", "{req}");
                assert!(!r.destructive, "{req}");
            }
            other => panic!("{req}: expected a hexa route, got {other:?}"),
        }
    }
}

/// S40: Every design or drift route is a command the table lists
#[test]
fn s40_design_and_drift_routes_are_listed() {
    let table = intent::hexa_commands();
    for want in ["drift", "design review <path>", "design brainstorm <idea>"] {
        assert!(table.iter().any(|c| c == want), "the table holds {want}: {table:?}");
    }
    let prefixes: Vec<Vec<String>> = table
        .iter()
        .map(|c| c.split_whitespace().take_while(|w| !w.starts_with('<')).map(str::to_string).collect())
        .collect();
    let requests = [
        "brainstorm an offline sync mode",
        "review the design in docs/design/idea.md",
        "review my design docs/designs/d.md",
        "check for drift",
        "do the docs match the code?",
    ];
    for req in requests {
        let Routed::Hexa(r) = intent::route(req) else { panic!("{req} routes to hexa") };
        if !(r.command.starts_with("design") || r.command.starts_with("drift")) {
            panic!("{req} routes to a design or drift command, got {}", r.command);
        }
        let words: Vec<&str> = r.command.split_whitespace().collect();
        assert!(
            prefixes.iter().any(|p| {
                let has_arg = table.iter().any(|c| c.split_whitespace().take(p.len()).eq(p.iter().map(String::as_str)) && c.contains('<'));
                words.len() >= p.len()
                    && words.iter().take(p.len()).copied().eq(p.iter().map(String::as_str))
                    && (words.len() == p.len() || has_arg)
            }),
            "{} matches a table entry",
            r.command
        );
    }
}

// ---------------------------------------------------------------------------
// §8: guidance
// ---------------------------------------------------------------------------

/// S41: The feature-sized prompt hook names design first
#[test]
fn s41_prompt_hook_names_design_first() {
    let files = hook_sources_with_design_guidance();
    assert!(
        !files.is_empty(),
        "the prompt hook's guidance contains Design (`hexa design brainstorm \"<idea>\"`, optional) → Decide"
    );
    for f in files {
        let s = std::fs::read_to_string(&f).unwrap();
        let needle = HOOK_NEEDLES.iter().find(|n| s.contains(*n)).unwrap();
        assert!(needle.find("Design").unwrap() < needle.find("Decide").unwrap());
    }
}

/// S42: The skill and CLAUDE.md cover the design verbs and drift --since
#[test]
fn s42_skill_and_claude_md_cover_design() {
    let skill = skill_text();
    let lines: Vec<&str> = skill.lines().collect();
    let decide = lines
        .iter()
        .position(|l| l.trim_start().starts_with('#') && l.contains("Decide"))
        .expect("the skill has a Decide section");
    let design_h = lines[..decide]
        .iter()
        .position(|l| l.trim_start().starts_with('#') && l.contains("Design"))
        .expect("the skill has a Design section before Decide");
    let section = lines[design_h..decide].join("\n");
    assert!(section.contains("hexa design brainstorm"), "{section}");
    for v in ["draft", "review", "resolve", "adr"] {
        assert!(
            section.contains(&format!("hexa design {v}")) || section.contains(&format!("`{v}`")),
            "the design section names {v}: {section}"
        );
    }
    assert!(skill.contains("hexa drift --since"), "the ship step runs hexa drift --since <base>");

    let claude = std::fs::read_to_string(workspace_root().join("CLAUDE.md")).expect("CLAUDE.md");
    let step0 = claude.lines().position(|l| {
        let t = l.trim_start().trim_start_matches(['*', '-', ' ', '#']);
        (t.starts_with("0.") || t.starts_with("0)") || l.contains("Step 0") || l.contains("**0")) && l.contains("Design")
    });
    let step0 = step0.expect("CLAUDE.md's pipeline has a step 0, Design");
    let rest = claude.lines().skip(step0).take(30).collect::<Vec<_>>().join("\n");
    assert!(rest.contains("hexa design brainstorm"), "{rest}");
    for v in ["draft", "review", "resolve", "adr"] {
        assert!(rest.contains(&format!("design {v}")) || rest.contains(&format!("`{v}`")), "step 0 names {v}: {rest}");
    }
}

/// S43: The guidance no longer starts at Decide
#[test]
fn s43_guidance_no_longer_starts_at_decide() {
    assert!(
        !hook_sources_with_design_guidance().is_empty(),
        "the prompt hook's guidance names Design before Decide"
    );
    let skill = skill_text();
    let first_design = skill.find("hexa design brainstorm").expect("the skill names hexa design brainstorm");
    let first_decide_heading = skill
        .lines()
        .scan(0usize, |off, l| {
            let start = *off;
            *off += l.len() + 1;
            Some((start, l))
        })
        .find(|(_, l)| l.trim_start().starts_with('#') && l.contains("Decide"))
        .map(|(s, _)| s)
        .expect("the skill has a Decide section");
    assert!(first_design < first_decide_heading, "design appears before Decide in the skill");
}
