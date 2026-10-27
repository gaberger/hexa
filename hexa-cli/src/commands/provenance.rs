//! Which build this binary is, read from what was embedded at compile time,
//! the git tree it runs in, and the cached release feed (ADR-2610270853).
//! The judgement is `hexa_core::ports::provenance`'s.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hexa_core::ports::provenance::{advice, staleness, Build, Checkout, Staleness};

/// How long a release answer, or a failed attempt at one, stands (§4).
const RELEASE_WINDOW: Duration = Duration::from_secs(24 * 60 * 60);
/// How long the session waits on the release feed (§4).
const FEED_TIMEOUT: Duration = Duration::from_secs(2);

/// Files the binary is built from; a commit touching none of them changes
/// nothing a rebuild would (§2).
const SOURCES: &[&str] = &[
    "Cargo.toml",
    "Cargo.lock",
    ":(glob)hexa-*/Cargo.toml",
    ":(glob)hexa-*/src/**",
    ":(glob)hexa-*/build.rs",
    "hexa-cli/assets",
];

/// This binary's build, as embedded by `build.rs` (§1).
pub fn build() -> Build {
    Build {
        version: env!("CARGO_PKG_VERSION").to_string(),
        commit: Some(env!("HEXA_BUILD_COMMIT")).filter(|c| !c.is_empty()).map(str::to_string),
        dirty: !env!("HEXA_BUILD_DIRTY").is_empty(),
    }
}

/// The git tree at `dir`, as it relates to `build`. `None` outside a git
/// tree, or when the build records no commit to look for.
pub fn checkout(dir: &Path, build: &Build) -> Option<Checkout> {
    let commit = build.commit.as_deref()?;
    let git = |args: &[&str]| {
        std::process::Command::new("git").arg("-C").arg(dir).args(args).output().ok()
    };
    let head = git(&["rev-parse", "HEAD"]).filter(|o| o.status.success())?;
    let head = String::from_utf8_lossy(&head.stdout).trim().to_string();
    // 0: an ancestor; 1: known, not an ancestor; anything else: not in this repository.
    let ancestry = git(&["merge-base", "--is-ancestor", commit, "HEAD"])?.status.code();
    let (knows_build, build_is_ancestor) = match ancestry {
        Some(0) => (true, true),
        Some(1) => (true, false),
        _ => (false, false),
    };
    let source_commits_behind = if build_is_ancestor {
        let range = format!("{commit}..HEAD");
        let mut args = vec!["rev-list", "--count", range.as_str(), "--"];
        args.extend(SOURCES);
        git(&args)
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
            .unwrap_or(0)
    } else {
        0
    };
    Some(Checkout { head, knows_build, build_is_ancestor, source_commits_behind })
}

fn hexa_home() -> PathBuf {
    std::env::var("HEXA_HOME").map(PathBuf::from).unwrap_or_else(|_| {
        PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_string())).join(".hexa")
    })
}

fn cache_path() -> PathBuf {
    hexa_home().join("release-check.json")
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The cached release answer: when it was asked, and the answer, which is
/// `None` when the ask failed.
fn cached() -> Option<(u64, Option<String>)> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(cache_path()).ok()?).ok()?;
    Some((v["checked_at"].as_u64()?, v["latest"].as_str().map(str::to_string)))
}

/// The newest release, asked at most once per window, given two seconds;
/// a failed ask is recorded and stands for the window like an answer (§4).
pub async fn latest_release_refreshing() -> Option<String> {
    if let Some((at, latest)) = cached() {
        if now().saturating_sub(at) < RELEASE_WINDOW.as_secs() {
            return latest;
        }
    }
    let latest = tokio::time::timeout(FEED_TIMEOUT, crate::commands::update::latest_release_tag())
        .await
        .ok()
        .and_then(Result::ok);
    let record = serde_json::json!({ "checked_at": now(), "latest": latest });
    let _ = std::fs::create_dir_all(hexa_home());
    let _ = std::fs::write(cache_path(), record.to_string());
    latest
}

/// The cached release answer, never asking the feed.
pub fn latest_release_cached() -> Option<String> {
    cached().and_then(|(_, latest)| latest)
}

/// The lines a session prints: one advice line per finding (§6).
pub async fn session_lines(dir: &Path) -> Vec<String> {
    let b = build();
    let latest = latest_release_refreshing().await;
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
    staleness(&b, checkout(dir, &b).as_ref(), latest.as_deref())
        .iter()
        .filter_map(|s| {
            let line = advice(s)?;
            Some(match s {
                Staleness::BehindCheckout { .. } if !exe.is_empty() => {
                    format!("{line}, then install -m 755 target/release/hexa {exe}")
                }
                _ => line,
            })
        })
        .collect()
}

/// Every finding, `Unknown` and `Current` included, for `hexa doctor` (§5, §6).
pub fn doctor_lines(dir: &Path) -> Vec<String> {
    let b = build();
    let latest = latest_release_cached();
    staleness(&b, checkout(dir, &b).as_ref(), latest.as_deref())
        .iter()
        .map(|s| match s {
            Staleness::Current => "current".to_string(),
            Staleness::Unknown { why } => format!("unknown — {why}"),
            other => advice(other).unwrap_or_default(),
        })
        .collect()
}
