//! hexa self-update — download and atomically replace the running binary.
//!
//! `hexa self-update [--check] [--version <tag>] [--yes]`
//!
//! ADR-2026-04-08-0929

use anyhow::{bail, Context};
use colored::Colorize;
use sha2::{Digest, Sha256};

const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const GITHUB_RELEASES_API: &str =
    "https://api.github.com/repos/gaberger/hexa/releases?per_page=30";
const GITHUB_RELEASES_BASE: &str =
    "https://github.com/gaberger/hexa/releases/download";

pub async fn run(check_only: bool, version: Option<String>, yes: bool) -> anyhow::Result<()> {
    let client = reqwest::Client::builder()
        .user_agent(format!("hexa/{}", CURRENT_VERSION))
        .build()?;

    // 1. Determine target version
    let explicit_version = version.is_some();
    let latest_tag = match version {
        Some(ref v) => v.clone(),
        None => fetch_latest_tag(&client).await?,
    };
    let latest_ver = latest_tag.trim_start_matches('v');

    println!(
        "  Current version:  {}",
        CURRENT_VERSION.cyan()
    );
    println!(
        "  Latest version:   {}",
        latest_ver.cyan()
    );

    if CURRENT_VERSION == latest_ver {
        println!("\n  {} Already up to date.", "✓".green());
        return Ok(());
    }

    // Downgrade guard: GitHub's "latest" flag races on tags pushed together,
    // so an unnamed update must never move backwards (ADR-2609151700).
    if !explicit_version {
        if let (Some(latest), Some(current)) =
            (parse_version(&latest_tag), parse_version(CURRENT_VERSION))
        {
            if latest < current {
                println!(
                    "\n  {} Installed version {} is newer than the newest published release {}; nothing to do.",
                    "✓".green(),
                    CURRENT_VERSION,
                    latest_ver
                );
                return Ok(());
            }
        }
    }

    if check_only {
        println!(
            "\n  {} Update available: {} → {}",
            "↑".yellow(),
            CURRENT_VERSION,
            latest_ver
        );
        println!("  Run `hexa self-update` to install.");
        return Ok(());
    }

    println!(
        "\n  {} Update available: {} → {}",
        "↑".yellow(),
        CURRENT_VERSION,
        latest_ver
    );

    // 2. Confirm
    if !yes {
        use dialoguer::Confirm;
        let ok = Confirm::new()
            .with_prompt("Install update?")
            .default(true)
            .interact()
            .unwrap_or(false);
        if !ok {
            println!("  Cancelled.");
            return Ok(());
        }
    }

    // 3. Determine platform
    let platform = detect_platform()?;
    let tarball_name = format!("hexa-{}-{}.tar.gz", latest_tag.trim_start_matches('v'), platform);
    let tarball_url = format!("{}/{}/{}", GITHUB_RELEASES_BASE, latest_tag, tarball_name);
    let sums_url = format!("{}/{}/SHA256SUMS.txt", GITHUB_RELEASES_BASE, latest_tag);

    // 4. Download tarball
    println!("  {} Downloading {}...", "→".cyan(), tarball_name);
    let tarball_bytes = download_bytes(&client, &tarball_url).await
        .with_context(|| format!("Failed to download {}", tarball_url))?;

    // 5. Download + verify SHA256
    println!("  {} Verifying checksum...", "→".cyan());
    let sums_text = download_text(&client, &sums_url).await
        .with_context(|| format!("Failed to download {}", sums_url))?;

    verify_sha256(&tarball_bytes, &tarball_name, &sums_text)?;
    println!("  {} Checksum OK", "✓".green());

    // 6. Extract to temp dir
    let tmp_dir = std::env::temp_dir().join(format!("hexa-update-{}", std::process::id()));
    std::fs::create_dir_all(&tmp_dir).context("Failed to create temp dir")?;
    let tarball_path = tmp_dir.join(&tarball_name);
    std::fs::write(&tarball_path, &tarball_bytes)?;

    let status = std::process::Command::new("tar")
        .args(["xzf", tarball_path.to_str().unwrap(), "-C", tmp_dir.to_str().unwrap()])
        .status()
        .context("Failed to run tar")?;
    if !status.success() {
        let _ = std::fs::remove_dir_all(&tmp_dir);
        bail!("tar extraction failed");
    }

    // 7. Find extracted hexa binary
    let extracted_bin = tmp_dir.join("hexa");
    if !extracted_bin.exists() {
        bail!("Extracted archive does not contain a 'hexa' binary");
    }

    // 8. Atomic replace — same filesystem rename
    let current_bin = std::env::current_exe().context("Cannot determine current binary path")?;
    let new_path = current_bin.with_extension("new");

    std::fs::copy(&extracted_bin, &new_path)
        .with_context(|| format!("Cannot write to {}", new_path.display()))?;

    // Make the new binary executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&new_path, std::fs::Permissions::from_mode(0o755))?;
    }

    // Atomic rename (same filesystem)
    std::fs::rename(&new_path, &current_bin)
        .with_context(|| format!("Failed to replace {}", current_bin.display()))?;

    // 9. Verify new version
    let output = std::process::Command::new(&current_bin)
        .arg("--version")
        .output();

    println!(
        "\n  {} hexa {} installed to {}",
        "✓".green(),
        latest_ver.green(),
        current_bin.display()
    );

    if let Ok(o) = output {
        let ver_line = String::from_utf8_lossy(&o.stdout);
        let ver_line = ver_line.trim();
        if !ver_line.is_empty() {
            println!("  {} {}", "→".cyan(), ver_line);
        }
    }

    let _ = std::fs::remove_dir_all(&tmp_dir);


    Ok(())
}

/// Parse a release tag such as `v26.9.11` or `26.9.11` into a comparable
/// `(major, minor, patch)` triple. Exactly three numeric parts are required.
pub(crate) fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.strip_prefix('v').unwrap_or(s);
    let mut parts = s.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Pick the published release with the highest semantic version, ignoring
/// drafts, prereleases, and tags that do not parse as `vX.Y.Z`.
pub(crate) fn newest_release_tag(releases: &[serde_json::Value]) -> Option<String> {
    releases
        .iter()
        .filter(|r| !r["draft"].as_bool().unwrap_or(false))
        .filter(|r| !r["prerelease"].as_bool().unwrap_or(false))
        .filter_map(|r| {
            let tag = r["tag_name"].as_str()?;
            parse_version(tag).map(|v| (v, tag.to_string()))
        })
        .max_by_key(|(v, _)| *v)
        .map(|(_, tag)| tag)
}

async fn fetch_latest_tag(client: &reqwest::Client) -> anyhow::Result<String> {
    let resp = client
        .get(GITHUB_RELEASES_API)
        .send()
        .await
        .context("GitHub releases API request failed")?;

    if !resp.status().is_success() {
        bail!("GitHub API returned {}", resp.status());
    }

    let json: serde_json::Value = resp.json().await?;
    let releases = json
        .as_array()
        .context("GitHub releases response was not a JSON array")?;

    newest_release_tag(releases).context("no published release found")
}

async fn download_bytes(client: &reqwest::Client, url: &str) -> anyhow::Result<Vec<u8>> {
    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        bail!("Download failed: {} returned {}", url, resp.status());
    }
    Ok(resp.bytes().await?.to_vec())
}

async fn download_text(client: &reqwest::Client, url: &str) -> anyhow::Result<String> {
    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        bail!("Download failed: {} returned {}", url, resp.status());
    }
    Ok(resp.text().await?)
}

fn verify_sha256(data: &[u8], filename: &str, sums_text: &str) -> anyhow::Result<()> {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let computed = format!("{:x}", hasher.finalize());

    for line in sums_text.lines() {
        let parts: Vec<&str> = line.splitn(2, ' ').collect();
        if parts.len() == 2 {
            let expected_hash = parts[0].trim();
            let expected_file = parts[1].trim().trim_start_matches('*');
            if expected_file == filename {
                if computed == expected_hash {
                    return Ok(());
                } else {
                    bail!(
                        "SHA256 mismatch for {}:\n  expected: {}\n  computed: {}",
                        filename,
                        expected_hash,
                        computed
                    );
                }
            }
        }
    }

    bail!("No SHA256 entry found for {} in SHA256SUMS.txt", filename)
}

fn detect_platform() -> anyhow::Result<&'static str> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return Ok("aarch64-apple-darwin");

    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return Ok("x86_64-apple-darwin");

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return Ok("x86_64-unknown-linux-gnu");

    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    return Ok("aarch64-unknown-linux-gnu");

    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
    )))]
    bail!("Unsupported platform — use install.sh manually")
}

#[cfg(test)]
mod latest_release_tests {
    //! GitHub's "latest" flag is the most recently *created* release, not the
    //! highest version. Two tags pushed together race, and the older one can
    //! win the flag. self-update must pick the highest version itself and must
    //! never downgrade unless a version was named (ADR-2609151700).
    use super::{newest_release_tag, parse_version};
    use serde_json::json;

    fn rel(tag: &str, draft: bool, pre: bool) -> serde_json::Value {
        json!({ "tag_name": tag, "draft": draft, "prerelease": pre })
    }

    #[test]
    fn version_parses_tag_with_or_without_v() {
        assert_eq!(parse_version("v26.9.11"), Some((26, 9, 11)));
        assert_eq!(parse_version("26.9.11"), Some((26, 9, 11)));
        assert_eq!(parse_version("26.10.0"), Some((26, 10, 0)));
        assert_eq!(parse_version("nightly"), None);
    }

    #[test]
    fn highest_version_wins_regardless_of_list_order() {
        let rels = vec![rel("v26.9.10", false, false), rel("v26.9.11", false, false), rel("v26.9.9", false, false)];
        assert_eq!(newest_release_tag(&rels).as_deref(), Some("v26.9.11"));
    }

    #[test]
    fn numeric_not_lexical_ordering() {
        let rels = vec![rel("v26.9.9", false, false), rel("v26.10.0", false, false), rel("v26.9.11", false, false)];
        assert_eq!(newest_release_tag(&rels).as_deref(), Some("v26.10.0"));
    }

    #[test]
    fn drafts_prereleases_and_unparseable_tags_are_skipped() {
        let rels = vec![
            rel("v27.0.0", true, false),
            rel("v26.12.0", false, true),
            rel("nightly", false, false),
            rel("v26.9.11", false, false),
        ];
        assert_eq!(newest_release_tag(&rels).as_deref(), Some("v26.9.11"));
        assert_eq!(newest_release_tag(&[]), None);
    }
}
