//! Integration tests for ADR-2610281552: a new project is consistent on day one.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use hexa_core::domain::project_floor::{
    adr_ids, cited_ledger, gitignore_lines, merged_gitignore, CitedLedger,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn temp_dir(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let p = std::env::temp_dir().join(format!(
        "hexa-cli-adr2610281552-{tag}-{}-{nanos}-{n}",
        std::process::id()
    ));
    fs::create_dir_all(&p).expect("create temp dir");
    p
}

fn strings(xs: &[&str]) -> Vec<String> {
    xs.iter().map(|s| s.to_string()).collect()
}

fn hexa(dir: &Path, home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(args)
        .current_dir(dir)
        .env("HOME", home)
        .output()
        .expect("run hexa binary")
}

fn git_init(dir: &Path) {
    if !dir.join(".git").exists() {
        let _ = Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir)
            .output();
    }
}

/// Run `hexa adr doctor --json` and return (output, whitespace-free stdout).
fn doctor(dir: &Path, home: &Path) -> (Output, String) {
    let out = hexa(dir, home, &["adr", "doctor", "--json"]);
    let compact: String = String::from_utf8_lossy(&out.stdout)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    (out, compact)
}

const DANGLING_KIND: &str = "\"kind\":\"dangling_citation\"";

/// The JSON objects (roughly) that carry a dangling_citation kind.
fn dangling_chunks(compact: &str) -> Vec<String> {
    // Split on '{' so each chunk approximates one finding object.
    compact
        .split('{')
        .filter(|c| c.contains(DANGLING_KIND))
        .map(|c| c.to_string())
        .collect()
}

fn write_project_adr(dir: &Path, body_extra: &str) {
    let adrs = dir.join("docs").join("adrs");
    fs::create_dir_all(&adrs).unwrap();
    let text = format!(
        "---\nid: ADR-2610010000\nstatus: accepted\ndate: 2026-10-01\n---\n\
# ADR-2610010000: a sample decision\n\n\
**Status:** Accepted\n\
**Date:** 2026-10-01\n\n\
## Context\n\nA sample.\n\n\
## Decision\n\nDo the sample thing.{body_extra}\n"
    );
    fs::write(adrs.join("ADR-2610010000-sample.md"), text).unwrap();
}

fn whole_lines(text: &str) -> Vec<&str> {
    text.lines().map(|l| l.trim()).collect()
}

fn is_whole_line(text: &str, line: &str) -> bool {
    text.lines().any(|l| l.trim() == line)
}

const LANGS: &[&str] = &["rust", "ts", "go"];

fn sample_existing() -> Vec<Option<&'static str>> {
    vec![
        None,
        Some(""),
        Some("*.log"),
        Some("*.log\n"),
        Some("target/\n"),
        Some("  target/  \n"),
        Some("mytarget/\n"),
        Some("*.log\ntarget/\n"),
        Some("target/\n.hexa/loop.json\n.hexa/loop.lock\n"),
        Some("node_modules/\ndist/\n.hexa/loop.json\n.hexa/loop.lock\n"),
        Some(".hexa/loop.json\n.hexa/loop.lock\n"),
        Some(".hexa/loop.json\n.hexa/loop.lock"),
        Some("# comment\n.env\nnode_modules/\n"),
        Some("dist/\ndist/\n"),
    ]
}

// ---------------------------------------------------------------------------
// §1: the binary knows hexa's own decisions
// ---------------------------------------------------------------------------

/// S01: The build embeds the ids of hexa's own ADR files.
#[test]
fn s01_build_embeds_ids_of_hexa_adr_files() {
    let names = strings(&["ADR-2609121400.md", "ADR-2609211430.md"]);
    let ids = adr_ids(&names);
    assert_eq!(ids, strings(&["ADR-2609121400", "ADR-2609211430"]));
}

/// S02: A build without hexa's ADR directory embeds no ids.
#[test]
fn s02_build_without_adr_directory_embeds_no_ids() {
    let ids = adr_ids(&[]);
    assert!(ids.is_empty(), "expected no ids, got {ids:?}");
}

/// S03: Only files matching `ADR-*.md` count as decisions.
#[test]
fn s03_only_adr_md_files_count_as_decisions() {
    let names = strings(&["ADR-2609121400.md", "README.md", "ADR-2609211430.txt"]);
    let ids = adr_ids(&names);
    assert_eq!(ids, strings(&["ADR-2609121400"]));
    assert!(!ids.iter().any(|i| i.contains("README")));
    assert!(!ids.iter().any(|i| i == "ADR-2609211430"));
}

// ---------------------------------------------------------------------------
// §2: a citation resolves against the project's ledger, then hexa's
// ---------------------------------------------------------------------------

/// S04: An id in the project's ledger resolves to Project.
#[test]
fn s04_project_id_resolves_to_project() {
    let project = strings(&["ADR-2610010000"]);
    let hexa_ids = strings(&["ADR-2609121400"]);
    assert_eq!(
        cited_ledger("ADR-2610010000", &project, &hexa_ids),
        CitedLedger::Project
    );
}

/// S05: An id that only hexa's ledger holds resolves to Hexa.
#[test]
fn s05_hexa_only_id_resolves_to_hexa() {
    let project = strings(&["ADR-2610010000"]);
    let hexa_ids = strings(&["ADR-2609121400"]);
    assert_eq!(
        cited_ledger("ADR-2609121400", &project, &hexa_ids),
        CitedLedger::Hexa
    );
}

/// S06: An id that neither ledger holds is Dangling.
#[test]
fn s06_unknown_id_is_dangling() {
    let project = strings(&["ADR-2610010000"]);
    let hexa_ids = strings(&["ADR-2609121400"]);
    assert_eq!(
        cited_ledger("ADR-1999010100", &project, &hexa_ids),
        CitedLedger::Dangling
    );
}

/// S07: The project's ledger takes precedence over hexa's.
#[test]
fn s07_project_ledger_takes_precedence() {
    let project = strings(&["ADR-2609121400"]);
    let hexa_ids = strings(&["ADR-2609121400"]);
    let r = cited_ledger("ADR-2609121400", &project, &hexa_ids);
    assert_eq!(r, CitedLedger::Project);
    assert_ne!(r, CitedLedger::Hexa);
}

/// S08: With two empty ledgers, every id is Dangling.
#[test]
fn s08_empty_ledgers_make_every_id_dangling() {
    assert_eq!(
        cited_ledger("ADR-2609121400", &[], &[]),
        CitedLedger::Dangling
    );
}

/// S09: The doctor passes on a fresh project whose shipped files cite hexa's decisions.
#[test]
fn s09_doctor_passes_on_fresh_project() {
    let dir = temp_dir("s09-proj");
    let home = temp_dir("s09-home");
    let made = hexa(&dir, &home, &["dev", "new", "--lang", "rust"]);
    assert!(
        made.status.success(),
        "hexa dev new failed: {}\n{}",
        String::from_utf8_lossy(&made.stdout),
        String::from_utf8_lossy(&made.stderr)
    );
    git_init(&dir);

    let (out, compact) = doctor(&dir, &home);
    assert!(
        !compact.contains(DANGLING_KIND),
        "fresh project reported dangling citations: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(
        out.status.success(),
        "doctor failed on fresh project (code {:?}): {}\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// S10: The doctor still reports a citation that resolves nowhere.
#[test]
fn s10_doctor_reports_citation_resolving_nowhere() {
    let dir = temp_dir("s10-proj");
    let home = temp_dir("s10-home");
    write_project_adr(&dir, "");
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("src").join("lib.rs"),
        "// Per ADR-1999010100 we do this.\n// Per ADR-2609121400 the project runs.\npub fn f() {}\n",
    )
    .unwrap();
    git_init(&dir);

    let (out, compact) = doctor(&dir, &home);
    let chunks = dangling_chunks(&compact);
    assert_eq!(
        chunks.len(),
        1,
        "expected exactly one DanglingCitation, got: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(chunks[0].contains("ADR-1999010100"));
    assert!(
        !chunks.iter().any(|c| c.contains("ADR-2609121400")),
        "hexa's own decision was reported as dangling"
    );
    assert!(!out.status.success(), "doctor should fail on a dangling citation");
}

/// S11: A user's own citation of a hexa decision is not reported.
#[test]
fn s11_user_citation_of_hexa_decision_not_reported() {
    let dir = temp_dir("s11-proj");
    let home = temp_dir("s11-home");
    write_project_adr(&dir, "\n\nSee ADR-2609151930: every cited id must resolve.");
    git_init(&dir);

    let (out, compact) = doctor(&dir, &home);
    let chunks = dangling_chunks(&compact);
    assert!(
        !chunks.iter().any(|c| c.contains("ADR-2609151930")),
        "ADR-2609151930 reported as dangling: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(
        !compact.contains(DANGLING_KIND),
        "unexpected dangling citation: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// S12: Resolution order holds over all inputs.
#[test]
fn s12_resolution_order_property() {
    let universe = ["ADR-2609121400", "ADR-2610010000", "ADR-1999010100", "ADR-2026-04-05-0900"];
    let n = universe.len();
    for pmask in 0..(1u32 << n) {
        for hmask in 0..(1u32 << n) {
            let project: Vec<String> = (0..n)
                .filter(|i| pmask & (1 << i) != 0)
                .map(|i| universe[i].to_string())
                .collect();
            let hexa_ids: Vec<String> = (0..n)
                .filter(|i| hmask & (1 << i) != 0)
                .map(|i| universe[i].to_string())
                .collect();
            for id in universe.iter().chain(["ADR-0000000000", ""].iter()) {
                let in_p = project.iter().any(|p| p == id);
                let in_h = hexa_ids.iter().any(|h| h == id);
                let r = cited_ledger(id, &project, &hexa_ids);
                assert_eq!(r == CitedLedger::Project, in_p, "id={id} p={project:?} h={hexa_ids:?}");
                assert_eq!(r == CitedLedger::Hexa, !in_p && in_h, "id={id} p={project:?} h={hexa_ids:?}");
                assert_eq!(r == CitedLedger::Dangling, !in_p && !in_h, "id={id} p={project:?} h={hexa_ids:?}");
            }
        }
    }
}

/// S13: When the two ledgers are the same set, as in hexa's own repository, nothing goes unchecked.
#[test]
fn s13_same_ledgers_never_resolve_to_hexa() {
    let universe = ["ADR-2609121400", "ADR-2610010000", "ADR-1999010100", "ADR-2026-04-05-0900"];
    let n = universe.len();
    for mask in 0..(1u32 << n) {
        let l: Vec<String> = (0..n)
            .filter(|i| mask & (1 << i) != 0)
            .map(|i| universe[i].to_string())
            .collect();
        for id in universe.iter().chain(["ADR-0000000000"].iter()) {
            let r = cited_ledger(id, &l, &l);
            assert_ne!(r, CitedLedger::Hexa, "id={id} L={l:?}");
            if l.iter().any(|x| x == id) {
                assert_eq!(r, CitedLedger::Project);
            } else {
                assert_eq!(r, CitedLedger::Dangling);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// §3: a new project ignores its build output
// ---------------------------------------------------------------------------

fn as_set(v: Vec<&'static str>) -> BTreeSet<&'static str> {
    v.into_iter().collect()
}

/// S14: Rust ignores target/ and the session state.
#[test]
fn s14_rust_gitignore_lines() {
    let lines = gitignore_lines("rust");
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert_eq!(
        as_set(lines),
        ["target/", ".hexa/loop.json", ".hexa/loop.lock"].into_iter().collect()
    );
}

/// S15: TS ignores node_modules/, dist/ and the session state.
#[test]
fn s15_ts_gitignore_lines() {
    let lines = gitignore_lines("ts");
    assert_eq!(lines.len(), 4, "{lines:?}");
    assert_eq!(
        as_set(lines),
        ["node_modules/", "dist/", ".hexa/loop.json", ".hexa/loop.lock"]
            .into_iter()
            .collect()
    );
}

/// S16: Go adds no build-output lines, only the session state.
#[test]
fn s16_go_gitignore_lines() {
    let lines = gitignore_lines("go");
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(!lines.contains(&"target/"));
    assert!(!lines.contains(&"node_modules/"));
    assert!(!lines.contains(&"dist/"));
    assert_eq!(
        as_set(lines),
        [".hexa/loop.json", ".hexa/loop.lock"].into_iter().collect()
    );
}

/// S17: With no existing file, the merge gives every needed line.
#[test]
fn s17_merge_with_no_existing_file() {
    let text = merged_gitignore(None, "rust").expect("expected Some");
    let lines = whole_lines(&text);
    for l in ["target/", ".hexa/loop.json", ".hexa/loop.lock"] {
        assert_eq!(lines.iter().filter(|x| **x == l).count(), 1, "{l} in {text:?}");
    }
}

/// S18: The merge keeps the existing text and appends only the missing lines.
#[test]
fn s18_merge_keeps_existing_and_appends_missing() {
    let text = merged_gitignore(Some("*.log\ntarget/\n"), "rust").expect("expected Some");
    assert!(text.starts_with("*.log\ntarget/\n"), "{text:?}");
    let rest = &text["*.log\ntarget/\n".len()..];
    assert!(is_whole_line(rest, ".hexa/loop.json"), "{text:?}");
    assert!(is_whole_line(rest, ".hexa/loop.lock"), "{text:?}");
    assert_eq!(whole_lines(&text).iter().filter(|x| **x == "target/").count(), 1);
}

/// S19: Nothing is written when nothing is missing.
#[test]
fn s19_nothing_written_when_nothing_missing() {
    assert_eq!(
        merged_gitignore(Some("target/\n.hexa/loop.json\n.hexa/loop.lock\n"), "rust"),
        None
    );
}

/// S20: A line counts as present only when it matches a whole line, not a substring.
#[test]
fn s20_presence_is_whole_line_not_substring() {
    let existing = "mytarget/\n.hexa/loop.json\n.hexa/loop.lock\n";
    let text = merged_gitignore(Some(existing), "rust").expect("expected Some");
    assert!(text.starts_with(existing), "{text:?}");
    assert!(text.lines().any(|l| l == "target/"), "{text:?}");
}

/// S21: `hexa dev new` writes the .gitignore, so the first commit holds no build output.
#[test]
fn s21_dev_new_writes_gitignore_and_target_not_staged() {
    let dir = temp_dir("s21-proj");
    let home = temp_dir("s21-home");
    let made = hexa(&dir, &home, &["dev", "new", "--lang", "rust"]);
    assert!(
        made.status.success(),
        "hexa dev new failed: {}\n{}",
        String::from_utf8_lossy(&made.stdout),
        String::from_utf8_lossy(&made.stderr)
    );

    let build = Command::new(env!("CARGO"))
        .arg("build")
        .current_dir(&dir)
        .env_remove("CARGO_TARGET_DIR")
        .env("HOME", std::env::var("HOME").unwrap_or_default())
        .output()
        .expect("run cargo build");
    assert!(
        build.status.success(),
        "cargo build failed: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    assert!(dir.join("target").exists(), "cargo build produced no target/");

    git_init(&dir);
    let add = Command::new("git")
        .args(["add", "-A"])
        .current_dir(&dir)
        .output()
        .expect("git add");
    assert!(add.status.success(), "{}", String::from_utf8_lossy(&add.stderr));

    let gi = fs::read_to_string(dir.join(".gitignore")).expect(".gitignore written");
    for l in ["target/", ".hexa/loop.json", ".hexa/loop.lock"] {
        assert!(is_whole_line(&gi, l), "{l} missing from .gitignore: {gi:?}");
    }

    let ls = Command::new("git")
        .args(["ls-files"])
        .current_dir(&dir)
        .output()
        .expect("git ls-files");
    let staged = String::from_utf8_lossy(&ls.stdout);
    assert!(
        !staged.lines().any(|l| l.starts_with("target/") || l == "target"),
        "target/ was staged:\n{staged}"
    );
}

/// S22: `hexa init` leaves an already complete .gitignore unchanged.
#[test]
fn s22_init_leaves_complete_gitignore_unchanged() {
    let dir = temp_dir("s22-proj");
    let home = temp_dir("s22-home");
    let mut content = String::from("# my ignores\n*.log\n.env\n");
    for l in gitignore_lines("rust") {
        content.push_str(l);
        content.push('\n');
    }
    content.push_str("scratch/\n");
    fs::write(dir.join(".gitignore"), &content).unwrap();

    let out = hexa(&dir, &home, &["init", ".", "--lang", "rust"]);
    assert!(
        out.status.success(),
        "hexa init failed: {}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let after = fs::read(dir.join(".gitignore")).expect(".gitignore still present");
    assert_eq!(after, content.as_bytes(), "gitignore changed");
}

/// S23: Every language ignores the session state.
#[test]
fn s23_every_language_ignores_session_state() {
    for lang in LANGS {
        let lines = gitignore_lines(lang);
        assert!(lines.contains(&".hexa/loop.json"), "{lang}: {lines:?}");
        assert!(lines.contains(&".hexa/loop.lock"), "{lang}: {lines:?}");
    }
}

/// S24: The merge is complete, preserving and duplicate-free.
#[test]
fn s24_merge_complete_preserving_duplicate_free() {
    for lang in LANGS {
        for e in sample_existing() {
            let Some(text) = merged_gitignore(e, lang) else { continue };
            let base = e.unwrap_or("");
            assert!(text.starts_with(base), "lang={lang} e={e:?} text={text:?}");
            for l in gitignore_lines(lang) {
                assert!(is_whole_line(&text, l), "lang={lang} e={e:?}: {l} missing in {text:?}");
            }
            let appended: Vec<&str> = text[base.len()..]
                .lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                .collect();
            let mut seen = BTreeSet::new();
            for a in &appended {
                assert!(
                    !is_whole_line(base, a),
                    "lang={lang} e={e:?}: appended {a} already present"
                );
                assert!(seen.insert(*a), "lang={lang} e={e:?}: {a} appended twice");
            }
        }
    }
}

/// S25: The merge is idempotent.
#[test]
fn s25_merge_is_idempotent() {
    for lang in LANGS {
        for e in sample_existing() {
            if let Some(text) = merged_gitignore(e, lang) {
                assert_eq!(
                    merged_gitignore(Some(&text), lang),
                    None,
                    "lang={lang} e={e:?} text={text:?}"
                );
            }
        }
    }
}

/// S26: None means nothing is missing.
#[test]
fn s26_none_means_nothing_missing() {
    for lang in LANGS {
        for e in sample_existing() {
            let complete = match e {
                Some(t) => gitignore_lines(lang).iter().all(|l| is_whole_line(t, l)),
                None => false,
            };
            let r = merged_gitignore(e, lang);
            assert_eq!(r.is_none(), complete, "lang={lang} e={e:?} r={r:?}");
        }
    }
}
