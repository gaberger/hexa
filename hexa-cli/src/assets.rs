//! Embedded asset bundle for hexa-cli (ADR-2026-03-22-1522).
//!
//! All templates and schemas live as real files under `hexa-cli/assets/`
//! and are baked into the binary at compile time via `rust-embed`. This
//! replaces scattered `include_str!` and hardcoded template strings.
//!
//! Canonical template directories (`skills/`, `agents/`, `hooks/`) are
//! the single source of truth — hexa-nexus re-embeds them for deployment
//! to target projects via `POST /api/projects/init`.

use rust_embed::Embed;

#[derive(Embed)]
#[folder = "assets/"]
pub struct Assets;

impl Assets {
    /// Get a file's contents as a UTF-8 string.
    pub fn get_str(path: &str) -> Option<String> {
        Self::get(path).map(|f| String::from_utf8_lossy(&f.data).to_string())
    }

    /// Get a template and apply simple `{{key}}` substitutions.
    pub fn render_template(path: &str, vars: &[(&str, &str)]) -> Option<String> {
        let mut content = Self::get_str(path)?;
        for (key, value) in vars {
            content = content.replace(&format!("{{{{{}}}}}", key), value);
        }
        Some(content)
    }

    /// Extract all files under a prefix to a target directory.
    /// Skips files that already exist (don't overwrite user customizations).
    pub fn extract_to(prefix: &str, target: &std::path::Path) -> std::io::Result<Vec<String>> {
        let mut extracted = Vec::new();
        for path in Self::iter() {
            if let Some(relative) = path.strip_prefix(prefix) {
                // Skip .tmpl extension in output path
                let dest_name = relative.strip_suffix(".tmpl").unwrap_or(relative);
                let dest = target.join(dest_name);
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                if let Some(file) = Self::get(&path) {
                    if !dest.exists() {
                        std::fs::write(&dest, &file.data)?;
                        extracted.push(dest_name.to_string());
                    }
                }
            }
        }
        Ok(extracted)
    }

    /// Extract every file under `prefix` to `target`, overwriting the files
    /// hexa owns there (ADR-2610301129 §4). Returns the files written.
    ///
    /// Unlike [`Assets::extract_to`], an existing file is replaced when its
    /// bytes differ, through a same-directory temp file and `rename`, so a
    /// reader sees the whole old file or the whole new one. Plugin bodies go
    /// first and manifests last, so an interrupted run leaves the old
    /// manifest over a consistent set of files. A file outside the embedded
    /// set is left alone, so a mod file dropped in a later version stays as an
    /// orphan.
    pub fn extract_owned(prefix: &str, target: &std::path::Path) -> std::io::Result<Vec<String>> {
        let mut files: Vec<(String, String)> = Self::iter()
            .filter_map(|path| path.strip_prefix(prefix).map(|r| (r.to_string(), path.to_string())))
            .collect();
        files.sort_by(|a, b| owned_order(&a.0).cmp(&owned_order(&b.0)).then_with(|| a.0.cmp(&b.0)));
        std::fs::create_dir_all(target)?;
        let mut written = Vec::new();
        for (relative, path) in files {
            let rel = std::path::Path::new(&relative);
            if rel.is_absolute() || rel.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
                return Err(std::io::Error::other(format!("unsafe asset path {relative}")));
            }
            let Some(file) = Self::get(&path) else { continue };
            let dest = target.join(rel);
            refuse_symlinks(target, rel)?;
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            if std::fs::read(&dest).is_ok_and(|now| now == file.data.as_ref()) {
                continue;
            }
            replace_atomically(&dest, &file.data)?;
            written.push(relative);
        }
        Ok(written)
    }
}

/// Write order for [`Assets::extract_owned`]: bodies, then each plugin's
/// `plugin.json`, then the marketplace manifest.
fn owned_order(relative: &str) -> u8 {
    if relative == ".claude-plugin/marketplace.json" {
        2
    } else if relative.ends_with("/.claude-plugin/plugin.json") {
        1
    } else {
        0
    }
}

/// Refuse to write through a symlink anywhere between `target` and the file.
fn refuse_symlinks(target: &std::path::Path, relative: &std::path::Path) -> std::io::Result<()> {
    let mut at = target.to_path_buf();
    for part in relative.components() {
        at.push(part);
        match std::fs::symlink_metadata(&at) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err(std::io::Error::other(format!("refusing to write through symlink {}", at.display())));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Replace `dest` with `bytes` through a temp file in the same directory.
/// The temp name carries the pid and a counter, so two hexa processes, or two
/// threads, never share one.
fn replace_atomically(dest: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = dest.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let tmp = dest.with_file_name(format!("{name}.hexa-tmp.{}.{n}", std::process::id()));
    let result = std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, dest));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workplan_schema_is_embedded() {
        let schema = Assets::get_str("schemas/workplan.schema.json");
        assert!(schema.is_some(), "workplan schema should be embedded");
        let content = schema.unwrap();
        assert!(content.contains("\"title\": \"hexa workplan\""), "schema should carry the hexa title");
    }

    #[test]
    fn settings_template_is_embedded() {
        let settings = Assets::get_str("templates/hexa-claude-settings.json");
        assert!(settings.is_some(), "settings template should be embedded");
    }

    #[test]
    fn render_template_substitutes() {
        // Create a simple test — we know the CLAUDE.md template has {{project_name}}
        let result = Assets::render_template("templates/hexa-claude-settings.json", &[]);
        assert!(result.is_some());
    }

    const MODS: &str = "claude-mods/";
    const HOOKS: &str = "question-alert/hooks/hooks.json";

    fn embedded(relative: &str) -> Vec<u8> {
        Assets::get(&format!("{MODS}{relative}")).expect("embedded").data.to_vec()
    }

    fn temp_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.to_string_lossy().contains(".hexa-tmp.") {
                    out.push(p);
                }
            }
        }
        out
    }

    #[test]
    fn extract_owned_overwrites_stale_files_and_keeps_orphans() {
        let dir = tempfile::tempdir().unwrap();
        let hooks = dir.path().join(HOOKS);
        let mine = dir.path().join("question-alert/mine.txt");
        std::fs::create_dir_all(hooks.parent().unwrap()).unwrap();
        std::fs::write(&hooks, "{\"stale\": true}").unwrap();
        std::fs::write(&mine, "mine").unwrap();
        let written = Assets::extract_owned(MODS, dir.path()).unwrap();
        assert!(written.iter().any(|w| w == HOOKS), "{written:?}");
        assert_eq!(std::fs::read(&hooks).unwrap(), embedded(HOOKS));
        assert_eq!(std::fs::read_to_string(&mine).unwrap(), "mine");
        assert!(dir.path().join(".claude-plugin/marketplace.json").is_file());
        assert!(temp_files(dir.path()).is_empty());
    }

    #[test]
    fn extract_owned_leaves_an_identical_file_alone() {
        let dir = tempfile::tempdir().unwrap();
        Assets::extract_owned(MODS, dir.path()).unwrap();
        let hooks = dir.path().join(HOOKS);
        let before = std::fs::metadata(&hooks).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        let written = Assets::extract_owned(MODS, dir.path()).unwrap();
        assert!(written.is_empty(), "an unchanged machine writes nothing: {written:?}");
        assert_eq!(std::fs::metadata(&hooks).unwrap().modified().unwrap(), before);
    }

    #[test]
    fn extract_owned_refuses_a_symlinked_folder() {
        let dir = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), dir.path().join("question-alert")).unwrap();
        assert!(Assets::extract_owned(MODS, dir.path()).is_err());
        assert_eq!(std::fs::read_dir(elsewhere.path()).unwrap().count(), 0, "wrote through the symlink");
        assert!(temp_files(dir.path()).is_empty());
    }

    #[test]
    fn extract_owned_writes_manifests_last() {
        let mut names = [
            ".claude-plugin/marketplace.json",
            "question-alert/.claude-plugin/plugin.json",
            "question-alert/hooks/hooks.json",
            "question-alert/types/index.d.ts",
        ];
        names.sort_by_key(|n| (owned_order(n), *n));
        assert_eq!(names.last(), Some(&".claude-plugin/marketplace.json"));
        assert_eq!(names.get(names.len() - 2), Some(&"question-alert/.claude-plugin/plugin.json"));
    }

    #[test]
    fn extract_owned_is_never_seen_half_written() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().to_path_buf();
        Assets::extract_owned(MODS, &target).unwrap();
        let hooks = target.join(HOOKS);
        let want = embedded(HOOKS);
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let writers: Vec<_> = (0..2)
            .map(|_| {
                let target = target.clone();
                let hooks = hooks.clone();
                std::thread::spawn(move || {
                    for _ in 0..50 {
                        // Make each pass rewrite, so the rename races the reader.
                        replace_atomically(&hooks, b"{\"stale\": true}").unwrap();
                        Assets::extract_owned(MODS, &target).unwrap();
                    }
                })
            })
            .collect();
        let reader = {
            let done = done.clone();
            std::thread::spawn(move || {
                while !done.load(std::sync::atomic::Ordering::Relaxed) {
                    if let Ok(bytes) = std::fs::read(&hooks) {
                        let whole = bytes == want || bytes == b"{\"stale\": true}";
                        assert!(whole, "torn read: {} bytes", bytes.len());
                    }
                }
            })
        };
        for w in writers {
            w.join().unwrap();
        }
        done.store(true, std::sync::atomic::Ordering::Relaxed);
        reader.join().unwrap();
        assert!(temp_files(&target).is_empty());
    }

    #[test]
    fn iter_lists_assets() {
        let count = Assets::iter().count();
        assert!(count >= 3, "should have at least 3 embedded assets, got {}", count);
    }

    #[test]
    fn skills_are_embedded_and_parseable() {
        let skill_paths: Vec<_> = Assets::iter()
            .filter(|p| p.starts_with("skills/") && p.ends_with(".md"))
            .collect();
        println!("skill paths: {:?}", skill_paths);
        assert!(!skill_paths.is_empty(), "no .md files found under skills/ prefix");
        // Verify at least one parses successfully
        let parsed = skill_paths.iter().filter_map(|p| {
            Assets::get_str(p).and_then(|c| {
                let c = c.trim().to_string();
                if !c.starts_with("---") { return None; }
                let rest = &c[3..];
                let end = rest.find("\n---")?;
                let fm = &rest[..end];
                let name: String = fm.lines()
                    .find_map(|l| l.strip_prefix("name:").map(|v| v.trim().to_string()))?;
                Some(name)
            })
        }).collect::<Vec<_>>();
        println!("parsed skill names: {:?}", parsed);
        assert!(!parsed.is_empty(), "no skills parsed successfully from embedded assets");
    }
}
