//! Integration tests for ADR-2610270853: build provenance and staleness.
//!
//! The domain scenarios call `hexa_core::domain::provenance` directly. The
//! binary scenarios run the `hexa` binary with an empty `HOME` and from a
//! directory outside any git repository.
//!
//! For the release-feed scenarios (S17, S18, S22, S27), every proxy variable
//! points at a local TCP listener. The listener accepts connections and never
//! answers, so the tests can count feed requests and simulate a slow feed.
//!
//! The command for the session-start hook is not among the signatures given.
//! `SESSION_START_ARGS` is a best guess. Change it if the CLI names the hook
//! differently.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hexa_core::domain::provenance::{advice, staleness, Build, Checkout, Staleness};

const HEXA: &str = env!("CARGO_BIN_EXE_hexa");
const PKG_VERSION: &str = env!("CARGO_PKG_VERSION");
const MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

/// Best guess at the session-start hook command (not shown in the signatures).
const SESSION_START_ARGS: &[&str] = &["hook", "session-start"];

const REBUILD: &str = "cargo build -p hexa-cli --release";
const SELF_UPDATE: &str = "hexa self-update";

const FULL_COMMIT: &str = "a4d8d3c0b1e2f3a4b5c6d7e8f9a0b1c2d3e4f5a6";
const OTHER_COMMIT: &str = "d4eb5860a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6";

// ---------------------------------------------------------------- helpers --

fn build(version: &str, commit: Option<&str>, dirty: bool) -> Build {
    Build {
        version: version.to_string(),
        commit: commit.map(str::to_string),
        dirty,
    }
}

fn checkout(head: &str, knows_build: bool, build_is_ancestor: bool, behind: u64) -> Checkout {
    Checkout {
        head: head.to_string(),
        knows_build,
        build_is_ancestor,
        source_commits_behind: behind,
    }
}

fn has_behind_checkout(r: &[Staleness]) -> bool {
    r.iter().any(|s| matches!(s, Staleness::BehindCheckout { .. }))
}

fn has_behind_release(r: &[Staleness]) -> bool {
    r.iter().any(|s| matches!(s, Staleness::BehindRelease { .. }))
}

fn has_unknown(r: &[Staleness]) -> bool {
    r.iter().any(|s| matches!(s, Staleness::Unknown { .. }))
}

fn has_current(r: &[Staleness]) -> bool {
    r.contains(&Staleness::Current)
}

fn sample_builds() -> Vec<Build> {
    let mut out = Vec::new();
    for v in ["26.10.0", "26.9.0", "26.11.0", "1.2.3", "0.1.0"] {
        for c in [None, Some(FULL_COMMIT), Some(OTHER_COMMIT)] {
            for d in [false, true] {
                out.push(build(v, c, d));
            }
        }
    }
    out
}

fn sample_checkouts() -> Vec<Option<Checkout>> {
    vec![
        None,
        Some(checkout(FULL_COMMIT, true, true, 0)),
        Some(checkout(OTHER_COMMIT, true, true, 47)),
        Some(checkout(OTHER_COMMIT, true, false, 5)),
        Some(checkout(OTHER_COMMIT, false, false, 0)),
    ]
}

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(MANIFEST_DIR)
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The full HEAD commit of the tree hexa-cli was built from, if it is a git tree.
fn git_head() -> Option<String> {
    git(&["rev-parse", "HEAD"]).filter(|h| !h.is_empty())
}

/// Whether that tree has uncommitted changes.
fn git_dirty() -> Option<bool> {
    git(&["status", "--porcelain"]).map(|s| !s.is_empty())
}

fn unique_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "hexa-provenance-{label}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A fake release feed: accepts connections, counts them, never answers.
struct SilentFeed {
    url: String,
    count: Arc<AtomicUsize>,
}

impl SilentFeed {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let count = Arc::new(AtomicUsize::new(0));
        let held = Arc::new(Mutex::new(Vec::new()));
        let c = Arc::clone(&count);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                if let Ok(s) = stream {
                    c.fetch_add(1, Ordering::SeqCst);
                    held.lock().unwrap().push(s);
                }
            }
        });
        SilentFeed {
            url: format!("http://127.0.0.1:{port}"),
            count,
        }
    }

    fn requests(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }
}

fn hexa_cmd(home: &Path, cwd: &Path, feed: &SilentFeed) -> Command {
    let mut cmd = Command::new(HEXA);
    cmd.current_dir(cwd)
        .env("HOME", home)
        .env_remove("NO_PROXY")
        .env_remove("no_proxy");
    for var in [
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        cmd.env(var, &feed.url);
    }
    cmd
}

fn combined(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn version_output() -> String {
    let out = Command::new(HEXA).arg("--version").output().unwrap();
    assert!(out.status.success(), "hexa --version failed: {out:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

// ------------------------------------------------------------- §1 binary --

/// S01: A clean build from a git tree prints its version and short commit
#[test]
fn s01_clean_build_prints_version_and_short_commit() {
    let head = git_head().expect("S01 needs hexa-cli built from inside a git tree");
    let short = &head[..7];
    let out = version_output();
    if git_dirty() == Some(false) {
        assert_eq!(out, format!("hexa {PKG_VERSION} ({short})"));
    } else {
        assert!(
            out.starts_with(&format!("hexa {PKG_VERSION} ({short}")),
            "got {out:?}"
        );
    }
}

/// S02: A dirty build is marked dirty
#[test]
fn s02_dirty_build_is_marked_dirty() {
    let head = git_head().expect("S02 needs hexa-cli built from inside a git tree");
    let short = &head[..7];
    let out = version_output();
    assert!(out.contains(PKG_VERSION), "got {out:?}");
    assert!(out.contains(short), "got {out:?}");
    let dirty = git_dirty().unwrap_or(false);
    assert_eq!(
        out.contains("-dirty"),
        dirty,
        "tree dirty = {dirty}, but output is {out:?}"
    );
}

/// S03: A build outside a git tree claims no commit
#[test]
fn s03_build_outside_git_claims_no_commit() {
    let out = version_output();
    assert!(out.contains(PKG_VERSION), "got {out:?}");
    match git_head() {
        None => {
            assert_eq!(out, format!("hexa {PKG_VERSION}"), "no commit, no -dirty");
            assert!(!out.contains("-dirty"));
            assert!(!out.contains('('), "no commit expected, got {out:?}");
        }
        Some(head) => {
            // Built inside a git tree, so it must name its commit instead.
            assert!(out.contains(&head[..7]), "got {out:?}");
        }
    }
}

/// S04: A clean build is not marked dirty
#[test]
fn s04_clean_build_is_not_marked_dirty() {
    let head = git_head().expect("S04 needs hexa-cli built from inside a git tree");
    let out = version_output();
    assert!(out.contains(&head[..7]), "got {out:?}");
    if git_dirty() == Some(false) {
        assert!(!out.contains("-dirty"), "got {out:?}");
    }
}

// ----------------------------------------------------- §2 behind checkout --

/// S05: The case from Drivers: a binary 47 source commits behind its checkout
#[test]
fn s05_binary_47_source_commits_behind_checkout() {
    let b = build("26.10.0", Some(OTHER_COMMIT), false);
    let c = checkout(FULL_COMMIT, true, true, 47);
    let r = staleness(&b, Some(&c), Some("26.10.0"));
    assert!(r.contains(&Staleness::BehindCheckout { commits: 47 }), "{r:?}");
    assert!(!has_current(&r), "{r:?}");
}

/// S06: Only commits that change the binary's sources are counted
#[test]
fn s06_only_source_commits_are_counted() {
    let b = build("26.10.0", Some(OTHER_COMMIT), false);
    let c = checkout(FULL_COMMIT, true, true, 12);
    let r = staleness(&b, Some(&c), Some("26.10.0"));
    assert!(r.contains(&Staleness::BehindCheckout { commits: 12 }), "{r:?}");
    assert!(!r.contains(&Staleness::BehindCheckout { commits: 30 }), "{r:?}");
}

/// S07: A docs-only gap is not reported as behind
#[test]
fn s07_docs_only_gap_is_not_behind() {
    let b = build("26.10.0", Some(OTHER_COMMIT), false);
    let c = checkout(FULL_COMMIT, true, true, 0);
    let r = staleness(&b, Some(&c), Some("26.10.0"));
    assert!(!has_behind_checkout(&r), "{r:?}");
    assert!(!has_unknown(&r), "{r:?}");
    for s in &r {
        assert_eq!(advice(s), None, "finding {s:?} gave advice");
    }
}

/// S08: A binary built from HEAD is not behind
#[test]
fn s08_binary_built_from_head_is_not_behind() {
    let b = build("26.10.0", Some(FULL_COMMIT), false);
    let c = checkout(FULL_COMMIT, true, true, 0);
    let r = staleness(&b, Some(&c), Some("26.10.0"));
    assert!(!has_behind_checkout(&r), "{r:?}");
}

// ------------------------------------------------------------ §3 branches --

/// S09: A build commit the repository has, but not on the current branch, is named OffBranch
#[test]
fn s09_off_branch_build_is_named() {
    let b = build("26.10.0", Some(OTHER_COMMIT), false);
    let c = checkout(FULL_COMMIT, true, false, 0);
    let r = staleness(&b, Some(&c), Some("26.10.0"));
    assert!(r.contains(&Staleness::OffBranch), "{r:?}");
}

/// S10: An off-branch build is not judged behind and not called current
#[test]
fn s10_off_branch_is_not_behind_nor_current() {
    let b = build("26.10.0", Some(OTHER_COMMIT), false);
    let c = checkout(FULL_COMMIT, true, false, 5);
    let r = staleness(&b, Some(&c), Some("26.10.0"));
    assert!(!has_behind_checkout(&r), "{r:?}");
    assert!(!has_current(&r), "{r:?}");
}

/// S11: A repository without the build commit gets no checkout claim
#[test]
fn s11_unknown_build_commit_gets_no_checkout_claim() {
    let b = build("26.10.0", Some(OTHER_COMMIT), false);
    let c = checkout(FULL_COMMIT, false, false, 0);
    let r = staleness(&b, Some(&c), Some("26.10.0"));
    assert!(!has_behind_checkout(&r), "{r:?}");
    assert!(!r.contains(&Staleness::OffBranch), "{r:?}");
}

// ------------------------------------------------------------ §4 releases --

/// S12: A higher published release is reported with its version
#[test]
fn s12_higher_release_is_reported() {
    let b = build("26.10.0", Some(FULL_COMMIT), false);
    let r = staleness(&b, None, Some("26.11.0"));
    assert!(
        r.contains(&Staleness::BehindRelease { latest: "26.11.0".into() }),
        "{r:?}"
    );
}

/// S13: Release ordering compares versions numerically, as in ADR-2609151700
#[test]
fn s13_release_ordering_is_numeric() {
    let b = build("26.9.0", Some(FULL_COMMIT), false);
    let r = staleness(&b, None, Some("26.10.0"));
    assert!(
        r.contains(&Staleness::BehindRelease { latest: "26.10.0".into() }),
        "{r:?}"
    );
}

/// S14: Equal versions are not behind, whatever the commit
#[test]
fn s14_equal_versions_are_not_behind() {
    for commit in [FULL_COMMIT, OTHER_COMMIT] {
        for dirty in [false, true] {
            let b = build("26.10.0", Some(commit), dirty);
            for c in sample_checkouts() {
                let r = staleness(&b, c.as_ref(), Some("26.10.0"));
                assert!(!has_behind_release(&r), "{b:?} {c:?} -> {r:?}");
            }
        }
    }
}

/// S15: A lower release is not behind, and the comparison is not lexical
#[test]
fn s15_lower_release_is_not_behind() {
    let b = build("26.10.0", Some(FULL_COMMIT), false);
    let r = staleness(&b, None, Some("26.9.0"));
    assert!(!has_behind_release(&r), "{r:?}");
}

/// S16: No build is behind a release equal to its own version
#[test]
fn s16_property_never_behind_own_version() {
    for b in sample_builds() {
        for c in sample_checkouts() {
            let r = staleness(&b, c.as_ref(), Some(b.version.as_str()));
            assert!(!has_behind_release(&r), "{b:?} {c:?} -> {r:?}");
        }
    }
}

/// S17: The session hook asks the release feed at most once in 24 hours
#[test]
fn s17_session_hook_uses_cached_release() {
    let home = unique_dir("s17-home");
    let cwd = unique_dir("s17-cwd");
    let feed = SilentFeed::start();

    // The first session fills the cache in ~/.hexa/.
    let _ = hexa_cmd(&home, &cwd, &feed)
        .args(SESSION_START_ARGS)
        .output()
        .unwrap();
    assert!(
        home.join(".hexa").is_dir(),
        "the session hook should cache the release answer in ~/.hexa/"
    );
    let after_first = feed.requests();

    // A session within 24 hours must not ask the feed again.
    let out = hexa_cmd(&home, &cwd, &feed)
        .args(SESSION_START_ARGS)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", combined(&out));
    assert_eq!(
        feed.requests(),
        after_first,
        "a cached answer under 24h old must not trigger a feed request"
    );
}

/// S18: A failed or slow fetch is not retried in the same window and does not hold up the session
#[test]
fn s18_slow_fetch_gives_up_and_is_not_retried() {
    let home = unique_dir("s18-home");
    let cwd = unique_dir("s18-cwd");
    let feed = SilentFeed::start();

    let start = Instant::now();
    let _ = hexa_cmd(&home, &cwd, &feed)
        .args(SESSION_START_ARGS)
        .output()
        .unwrap();
    let elapsed = start.elapsed();
    assert!(
        feed.requests() >= 1,
        "the first session with an empty cache should try the release feed (via the proxy)"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "the session must give up on the fetch after about two seconds, took {elapsed:?}"
    );

    let after_first = feed.requests();
    let _ = hexa_cmd(&home, &cwd, &feed)
        .args(SESSION_START_ARGS)
        .output()
        .unwrap();
    assert_eq!(
        feed.requests(),
        after_first,
        "a failed fetch must not be retried within 24 hours"
    );
}

// ------------------------------------------------------------- §5 unknown --

/// S19: A build with no commit and no cached release is Unknown
#[test]
fn s19_no_commit_no_release_is_unknown() {
    let b = build("26.10.0", None, false);
    for c in sample_checkouts() {
        let r = staleness(&b, c.as_ref(), None);
        assert!(
            r.iter()
                .any(|s| matches!(s, Staleness::Unknown { why } if !why.trim().is_empty())),
            "{c:?} -> {r:?}"
        );
    }
}

/// S20: A check that could not be made is never reported as Current
#[test]
fn s20_unmade_check_is_not_current() {
    let b = build("26.10.0", None, false);
    for c in sample_checkouts() {
        let r = staleness(&b, c.as_ref(), None);
        assert!(!has_current(&r), "{c:?} -> {r:?}");
    }
}

/// S21: No comparison is never a pass
#[test]
fn s21_property_no_comparison_is_never_a_pass() {
    for b in sample_builds().into_iter().filter(|b| b.commit.is_none()) {
        for c in sample_checkouts() {
            let r = staleness(&b, c.as_ref(), None);
            assert!(!r.is_empty(), "{b:?} {c:?}");
            assert!(has_unknown(&r), "{b:?} {c:?} -> {r:?}");
            assert!(!has_current(&r), "{b:?} {c:?} -> {r:?}");
        }
    }
}

/// S22: hexa doctor prints Unknown
#[test]
fn s22_doctor_prints_unknown() {
    let home = unique_dir("s22-home");
    let cwd = unique_dir("s22-cwd");
    let feed = SilentFeed::start();
    let out = hexa_cmd(&home, &cwd, &feed).arg("doctor").output().unwrap();
    let text = combined(&out);
    let line = text
        .lines()
        .find(|l| {
            let l = l.to_lowercase();
            l.contains("stale") && l.contains("unknown")
        })
        .unwrap_or_else(|| panic!("no 'stale ... unknown' line in doctor output:\n{text}"));
    let lower = line.to_lowercase();
    let rest = lower
        .split("unknown")
        .skip(1)
        .collect::<String>()
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_string();
    assert!(!rest.is_empty(), "the line must say why: {line:?}");
}

// -------------------------------------------------------------- §6 advice --

/// S23: Checkout-gap advice is one line with the rebuild command
#[test]
fn s23_checkout_gap_advice() {
    let line = advice(&Staleness::BehindCheckout { commits: 47 }).expect("advice expected");
    assert!(!line.contains('\n'), "{line:?}");
    assert!(line.contains("47"), "{line:?}");
    assert!(line.contains(REBUILD), "{line:?}");
}

/// S24: Release-gap advice is one line with the update command
#[test]
fn s24_release_gap_advice() {
    let line = advice(&Staleness::BehindRelease { latest: "26.11.0".into() })
        .expect("advice expected");
    assert!(!line.contains('\n'), "{line:?}");
    assert!(line.contains("26.11.0"), "{line:?}");
    assert!(line.contains(SELF_UPDATE), "{line:?}");
}

/// S25: The session-start hook prints one advice line per finding
#[test]
fn s25_one_advice_line_per_finding() {
    // Domain level: the hook prints `advice` for each finding.
    let b = build("26.10.0", Some(OTHER_COMMIT), false);
    let c = checkout(FULL_COMMIT, true, true, 3);
    let r = staleness(&b, Some(&c), Some("26.11.0"));
    let lines: Vec<String> = r.iter().filter_map(advice).collect();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines.iter().all(|l| !l.contains('\n')), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains(REBUILD)), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains(SELF_UPDATE)), "{lines:?}");
}

/// S26: Current gives no advice
#[test]
fn s26_current_gives_no_advice() {
    assert_eq!(advice(&Staleness::Current), None);
}

/// S27: Commands other than session start and doctor print no staleness warning
#[test]
fn s27_other_commands_print_no_staleness_advice() {
    let checkout_advice =
        advice(&Staleness::BehindCheckout { commits: 47 }).expect("advice expected");
    let home = unique_dir("s27-home");
    let feed = SilentFeed::start();
    // Run from inside the source checkout so a stale binary would have
    // something to warn about.
    let out = hexa_cmd(&home, Path::new(MANIFEST_DIR), &feed)
        .arg("--version")
        .output()
        .unwrap();
    let text = combined(&out);
    assert!(!text.contains(&checkout_advice), "{text}");
    assert!(!text.contains(REBUILD), "{text}");
    assert!(!text.contains(SELF_UPDATE), "{text}");
    assert_eq!(feed.requests(), 0, "only session start may ask the release feed");
}

// ------------------------------------------------------------ §7 combined --

/// S28: A binary behind both its checkout and a release reports both
#[test]
fn s28_behind_both_reports_both() {
    let b = build("26.10.0", Some(OTHER_COMMIT), false);
    let c = checkout(FULL_COMMIT, true, true, 3);
    let r = staleness(&b, Some(&c), Some("26.11.0"));
    assert!(r.contains(&Staleness::BehindCheckout { commits: 3 }), "{r:?}");
    assert!(
        r.contains(&Staleness::BehindRelease { latest: "26.11.0".into() }),
        "{r:?}"
    );
}

/// S29: One finding does not hide another
#[test]
fn s29_one_finding_does_not_hide_another() {
    let b = build("26.10.0", Some(OTHER_COMMIT), false);
    let c = checkout(FULL_COMMIT, true, true, 3);
    let r = staleness(&b, Some(&c), Some("26.11.0"));
    assert_eq!(
        has_behind_checkout(&r),
        has_behind_release(&r),
        "only one finding reported: {r:?}"
    );
    assert!(has_behind_checkout(&r) && has_behind_release(&r), "{r:?}");
    assert!(!has_current(&r), "{r:?}");
}

/// S30: Detection is pure and deterministic
#[test]
fn s30_property_detection_is_pure_and_deterministic() {
    let releases = [None, Some("26.10.0"), Some("26.11.0"), Some("26.9.0"), Some("garbage")];
    for b in sample_builds() {
        for c in sample_checkouts() {
            for rel in releases {
                let first = staleness(&b, c.as_ref(), rel);
                let second = staleness(&b, c.as_ref(), rel);
                assert_eq!(first, second, "{b:?} {c:?} {rel:?}");
            }
        }
    }
}
