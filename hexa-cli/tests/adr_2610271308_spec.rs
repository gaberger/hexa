//! Integration tests for ADR-2610271308: plain-English intent routing
//! (`hexa_core::domain::intent`) and the `hexa hey` command.

use hexa_core::domain::intent::{hexa_commands, route, Route, Routed};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const HEXA: &str = env!("CARGO_BIN_EXE_hexa");

const DEAD_VERBS: &[&str] = &["brain", "sched", "inbox", "brief", "report", "secrets"];

/// A varied corpus of request texts used by the property scenarios.
fn corpus() -> Vec<String> {
    let mut texts: Vec<String> = [
        "",
        " ",
        "grade the architecture",
        "what's the architecture grade",
        "show me the git status",
        "what's the git status",
        "show recent commits",
        "what changed (diff)",
        "harden nowhere/at-all",
        "harden src/parser",
        "harden it",
        "harden",
        "bug-hunt src/parser",
        "bug-hunt",
        "search memory for retry backoff",
        "search memory",
        "memory search",
        "where does the work stand",
        "what should I do next",
        "check every decision's gate",
        "check my install",
        "update hexa",
        "update hexa now",
        "self-update",
        "clean up the dev worktrees",
        "validate the brain",
        "show my inbox",
        "give me the brief",
        "run the report audit",
        "secrets status",
        "scheduler status",
        "make me a sandwich",
        "doctr plz",
        "HARDEN SRC/PARSER",
        "Grade The Architecture",
        "analyze",
        "loop",
        "go",
        "doctor",
        "adr gates",
        "please harden lib/a.rs and lib/b.rs",
        "search memory for 🦀 unicode",
        "git",
        "status",
        "\n\t\r",
        "!!!???",
        "rm -rf /",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    texts.extend(random_strings(40, "abcdefghijklmnopqrstuvwxyz /.-"));
    texts
}

/// Deterministic pseudo-random strings over `alphabet`.
fn random_strings(n: usize, alphabet: &str) -> Vec<String> {
    let chars: Vec<char> = alphabet.chars().collect();
    let mut state: u64 = 0x2610_2713_08u64;
    let mut next = move || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (state >> 33) as usize
    };
    (0..n)
        .map(|_| {
            let len = next() % 24;
            (0..len).map(|_| chars[next() % chars.len()]).collect()
        })
        .collect()
}

/// The leading verb/subcommand words of a table entry or command, stopping at
/// the first argument, placeholder or flag.
fn verb_words(command: &str) -> Vec<String> {
    command
        .split_whitespace()
        .take_while(|w| {
            let first = w.chars().next().unwrap_or('-');
            first.is_ascii_alphabetic()
                && w.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
        .map(|w| w.to_string())
        .collect()
}

fn hexa_route(r: &Routed) -> Option<&Route> {
    match r {
        Routed::Hexa(route) => Some(route),
        _ => None,
    }
}

fn any_route(r: &Routed) -> Option<&Route> {
    match r {
        Routed::Hexa(route) | Routed::Shell(route) => Some(route),
        Routed::None => None,
    }
}

fn expect_hexa(text: &str, command: &str) -> Route {
    match route(text) {
        Routed::Hexa(r) => {
            assert_eq!(r.command, command, "route({text:?}) command");
            r
        }
        other => panic!("route({text:?}) = {other:?}, expected Routed::Hexa({command:?})"),
    }
}

fn expect_shell(text: &str, command: &str) -> Route {
    match route(text) {
        Routed::Shell(r) => {
            assert_eq!(r.command, command, "route({text:?}) command");
            r
        }
        other => panic!("route({text:?}) = {other:?}, expected Routed::Shell({command:?})"),
    }
}

/// Runs `hexa <verb words> --help` for each entry; returns the failing entries.
fn check_table(entries: &[String]) -> Vec<String> {
    entries
        .iter()
        .filter(|entry| {
            let words = verb_words(entry);
            if words.is_empty() {
                return true;
            }
            let status = Command::new(HEXA)
                .args(&words)
                .arg("--help")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            !matches!(status, Ok(s) if s.success())
        })
        .cloned()
        .collect()
}

static TMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn temp_dir(tag: &str) -> PathBuf {
    let n = TMP_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "hexa-intent-test-{}-{}-{}",
        tag,
        std::process::id(),
        n
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Every file under `dir` with its contents.
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        if let Ok(entries) = fs::read_dir(&d) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    out.insert(p.clone(), Vec::new());
                    stack.push(p);
                } else {
                    out.insert(p.clone(), fs::read(&p).unwrap_or_default());
                }
            }
        }
    }
    out
}

fn run_hey(dir: &Path, args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(HEXA)
        .arg("hey")
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn hexa hey");
    {
        let mut si = child.stdin.take().expect("stdin");
        let _ = si.write_all(stdin.as_bytes());
    }
    child.wait_with_output().expect("wait hexa hey")
}

fn combined(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn make_parser_tree(dir: &Path) {
    fs::create_dir_all(dir.join("src/parser")).unwrap();
    fs::write(dir.join("src/parser/mod.rs"), "pub fn parse() {}\n").unwrap();
}

// ---------------------------------------------------------------------------
// §1
// ---------------------------------------------------------------------------

/// S01: route answers a hexa request from text alone
#[test]
fn s01_route_answers_hexa_request_from_text() {
    let r = expect_hexa("grade the architecture", "analyze .");
    assert!(!r.description.trim().is_empty(), "description must be non-empty");
    assert!(!r.description.contains('\n'), "description must be one line");
}

/// S02: route answers a non-hexa request with a Shell route
#[test]
fn s02_route_answers_non_hexa_with_shell() {
    expect_shell("show me the git status", "git status");
}

/// S03: route does not consult the filesystem
#[test]
fn s03_route_does_not_consult_filesystem() {
    assert!(!Path::new("nowhere/at-all").exists());
    expect_hexa("harden nowhere/at-all", "harden nowhere/at-all");
}

/// S04: route is a pure function of its text
#[test]
fn s04_route_is_pure() {
    let cwd = std::env::current_dir().unwrap();
    let watched = temp_dir("s04");
    fs::write(watched.join("marker"), "x").unwrap();
    let before_tmp = snapshot(&watched);
    let before_cwd: Vec<_> = fs::read_dir(&cwd)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();

    for text in corpus() {
        let a = route(&text);
        let b = route(&text);
        assert_eq!(a, b, "route({text:?}) not deterministic");
    }

    let after_cwd: Vec<_> = fs::read_dir(&cwd)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    let mut bc = before_cwd.clone();
    let mut ac = after_cwd.clone();
    bc.sort();
    ac.sort();
    assert_eq!(bc, ac, "route changed the working directory");
    assert_eq!(before_tmp, snapshot(&watched), "route changed files");
    let _ = fs::remove_dir_all(&watched);
}

// ---------------------------------------------------------------------------
// §2
// ---------------------------------------------------------------------------

/// S05: the table lists the core verbs and each one exists
#[test]
fn s05_table_lists_core_verbs_and_each_exists() {
    let cmds = hexa_commands();
    let verbs: Vec<Vec<String>> = cmds.iter().map(|c| verb_words(c)).collect();
    let required: &[&[&str]] = &[
        &["analyze"],
        &["loop"],
        &["go"],
        &["adr", "gates"],
        &["doctor"],
        &["self-update"],
        &["harden"],
        &["memory", "search"],
    ];
    for req in required {
        let req: Vec<String> = req.iter().map(|s| s.to_string()).collect();
        assert!(
            verbs.contains(&req),
            "hexa_commands() lacks {:?}: {cmds:?}",
            req.join(" ")
        );
    }
    let failing = check_table(&cmds);
    assert!(failing.is_empty(), "entries whose --help failed: {failing:?}");
}

/// S06: a removed verb in the table fails the check
#[test]
fn s06_removed_verb_fails_check() {
    let mut entries = hexa_commands();
    entries.push("brain validate".to_string());
    let status = Command::new(HEXA)
        .args(["brain", "validate", "--help"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("run hexa");
    assert!(!status.success(), "`hexa brain validate --help` should fail");
    let failing = check_table(&entries);
    assert!(
        failing.contains(&"brain validate".to_string()),
        "check did not report brain validate: {failing:?}"
    );
}

/// S07: every hexa route the router produces is in the checked table
#[test]
fn s07_every_hexa_route_is_in_table() {
    let table: Vec<Vec<String>> = hexa_commands().iter().map(|c| verb_words(c)).collect();
    for text in corpus() {
        if let Routed::Hexa(r) = route(&text) {
            let words: Vec<String> = r.command.split_whitespace().map(String::from).collect();
            let ok = table
                .iter()
                .any(|e| !e.is_empty() && words.len() >= e.len() && words[..e.len()] == e[..]);
            assert!(ok, "route({text:?}) = {:?} not in table {table:?}", r.command);
        }
    }
}

// ---------------------------------------------------------------------------
// §3
// ---------------------------------------------------------------------------

/// S08: git state requests route to shell git commands
#[test]
fn s08_git_requests_route_to_shell() {
    expect_shell("what's the git status", "git status");
    expect_shell("show recent commits", "git log --oneline -20");
    expect_shell("what changed (diff)", "git diff --stat");
}

/// S09: requests for removed systems get no hexa route
#[test]
fn s09_removed_systems_get_no_hexa_route() {
    let texts = [
        "validate the brain",
        "show my inbox",
        "give me the brief",
        "run the report audit",
        "secrets status",
        "scheduler status",
    ];
    for text in texts {
        if let Routed::Hexa(r) = route(text) {
            for dead in DEAD_VERBS {
                assert!(
                    !r.command.starts_with(dead),
                    "route({text:?}) gave dead hexa command {:?}",
                    r.command
                );
            }
        }
    }
}

/// S10: no route, and no table entry, names a dead verb
#[test]
fn s10_no_dead_verbs_anywhere() {
    for entry in hexa_commands() {
        let first = entry.split_whitespace().next().unwrap_or("");
        assert!(!DEAD_VERBS.contains(&first), "table entry {entry:?} is dead");
    }
    for text in corpus() {
        if let Routed::Hexa(r) = route(&text) {
            let first = r.command.split_whitespace().next().unwrap_or("");
            assert!(
                !DEAD_VERBS.contains(&first),
                "route({text:?}) gave dead command {:?}",
                r.command
            );
        }
    }
}

// ---------------------------------------------------------------------------
// §4
// ---------------------------------------------------------------------------

/// S11: the read-only core verbs are routed
#[test]
fn s11_read_only_core_verbs_routed() {
    expect_hexa("what's the architecture grade", "analyze .");
    expect_hexa("where does the work stand", "loop");
    expect_hexa("what should I do next", "go");
    expect_hexa("check every decision's gate", "adr gates");
    expect_hexa("check my install", "doctor");
    expect_hexa("update hexa", "self-update --check");
}

/// S12: verbs that take an argument carry it from the request
#[test]
fn s12_arguments_carried_from_request() {
    expect_hexa("harden src/parser", "harden src/parser");
    expect_hexa("bug-hunt src/parser", "harden src/parser");
    expect_hexa("search memory for retry backoff", "memory search retry backoff");
}

/// S13: a required argument the request lacks is not invented
#[test]
fn s13_missing_argument_not_invented() {
    for text in ["harden it", "search memory"] {
        if let Routed::Hexa(r) = route(text) {
            assert!(
                !r.command.starts_with("harden") && !r.command.starts_with("memory search"),
                "route({text:?}) invented an argument: {:?}",
                r.command
            );
        }
    }
}

/// S14: route arguments always come from the request
#[test]
fn s14_route_arguments_come_from_request() {
    for text in corpus() {
        if let Routed::Hexa(r) = route(&text) {
            let arg = if r.command == "harden" || r.command.starts_with("harden ") {
                Some(r.command["harden".len()..].trim().to_string())
            } else if r.command == "memory search" || r.command.starts_with("memory search ") {
                Some(r.command["memory search".len()..].trim().to_string())
            } else {
                None
            };
            if let Some(arg) = arg {
                assert!(!arg.is_empty(), "route({text:?}) gave empty argument");
                assert!(
                    text.contains(&arg),
                    "route({text:?}) argument {arg:?} not in text"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// §5
// ---------------------------------------------------------------------------

/// S15: an unmatched request returns None
#[test]
fn s15_unmatched_returns_none() {
    assert_eq!(route("make me a sandwich"), Routed::None);
}

/// S16: hey says when it found no verb
#[test]
fn s16_hey_reports_no_verb() {
    let dir = temp_dir("s16");
    let before = snapshot(&dir);
    let out = run_hey(&dir, &["make me a sandwich"], "");
    let text = combined(&out);
    assert!(text.contains("hexa --help"), "no pointer at hexa --help: {text}");
    assert!(
        text.to_lowercase().contains("verb"),
        "does not say it found no verb: {text}"
    );
    assert_eq!(before, snapshot(&dir), "hey changed files");
    let _ = fs::remove_dir_all(&dir);
}

/// S17: hey never runs a nearest guess, even with --yes
#[test]
fn s17_hey_never_runs_nearest_guess() {
    assert_eq!(route("doctr plz"), Routed::None);
    let dir = temp_dir("s17");
    let before = snapshot(&dir);
    let out = run_hey(&dir, &["--yes", "doctr plz"], "");
    let text = combined(&out);
    assert!(text.contains("hexa --help"), "no pointer at hexa --help: {text}");
    assert!(
        text.to_lowercase().contains("verb"),
        "no no-verb message: {text}"
    );
    assert_eq!(before, snapshot(&dir), "hey ran something that changed files");
    let _ = fs::remove_dir_all(&dir);
}

/// S18: text that matches no rule always gives None
#[test]
fn s18_untriggered_text_gives_none() {
    let mut texts = vec![String::new(), " ".to_string(), "\t\n".to_string()];
    texts.extend(random_strings(200, "bcfjkpqvwxz "));
    for text in texts {
        assert_eq!(route(&text), Routed::None, "route({text:?}) should be None");
    }
}

// ---------------------------------------------------------------------------
// §6
// ---------------------------------------------------------------------------

/// S19: changing routes are marked destructive
#[test]
fn s19_changing_routes_destructive() {
    for (text, cmd) in [
        ("harden src/parser", "harden src/parser"),
        ("clean up the dev worktrees", "dev worktree cleanup"),
    ] {
        let r = any_route(&route(text))
            .cloned()
            .unwrap_or_else(|| panic!("route({text:?}) returned None"));
        assert_eq!(r.command, cmd);
        assert!(r.destructive, "route({text:?}) should be destructive");
    }
}

/// S20: --yes does not skip confirmation for a destructive route
#[test]
fn s20_yes_does_not_skip_destructive_confirmation() {
    let r = expect_hexa("harden src/parser", "harden src/parser");
    assert!(r.destructive);

    let dir = temp_dir("s20");
    make_parser_tree(&dir);
    let before = snapshot(&dir);
    let out = run_hey(&dir, &["--yes", "harden src/parser"], "n\n");
    let text = combined(&out);
    let lower = text.to_lowercase();
    assert!(
        lower.contains("confirm")
            || lower.contains("[y/n]")
            || lower.contains("proceed")
            || lower.contains("continue")
            || text.contains('?'),
        "no confirmation prompt: {text}"
    );
    assert!(
        text.contains("harden src/parser"),
        "prompt does not name the command: {text}"
    );
    assert_eq!(before, snapshot(&dir), "harden ran despite refusal");
    let _ = fs::remove_dir_all(&dir);
}

/// S21: read-only routes are not marked destructive
#[test]
fn s21_read_only_routes_not_destructive() {
    let a = expect_hexa("update hexa", "self-update --check");
    let b = expect_hexa("check my install", "doctor");
    let c = expect_shell("what's the git status", "git status");
    for r in [a, b, c] {
        assert!(!r.destructive, "{:?} should not be destructive", r.command);
    }
}

/// S22: every tree-, git- or install-changing route is destructive
#[test]
fn s22_changing_routes_always_destructive() {
    let mut saw_any = false;
    for text in corpus() {
        if let Some(r) = any_route(&route(&text)) {
            let words: Vec<&str> = r.command.split_whitespace().collect();
            let changing = r.command.starts_with("harden")
                || (words.first() == Some(&"self-update") && !words.contains(&"--check"))
                || r.command.starts_with("dev worktree cleanup");
            if changing {
                saw_any = true;
                assert!(
                    r.destructive,
                    "route({text:?}) = {:?} must be destructive",
                    r.command
                );
            }
        }
    }
    assert!(saw_any, "corpus produced no changing route to check");
    let _ = hexa_route(&Routed::None);
}
