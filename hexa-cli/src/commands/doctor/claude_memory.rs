//! `hexa doctor`'s section on Claude Code's memory (https://code.claude.com/docs/en/memory):
//! is `CLAUDE.md` short enough to be followed, does the auto-memory index fit
//! what Claude Code loads, and are agent worktrees kept out of git. Advisory:
//! nothing here fails the doctor.

use std::path::Path;

use crate::commands::claude_paths::auto_memory_dir;
use crate::commands::refresh::{END_MARKER, START_MARKER};

/// Claude Code advises keeping each CLAUDE.md under this many lines.
pub(crate) const CLAUDE_MD_LINES: usize = 200;
/// It loads only this much of the auto-memory index `MEMORY.md`; the rest is dropped.
pub(crate) const MEMORY_INDEX_LINES: usize = 200;
pub(crate) const MEMORY_INDEX_BYTES: usize = 25 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Level {
    Ok,
    Warn,
    Info,
}

#[derive(Debug, Clone)]
pub(crate) struct Line {
    pub level: Level,
    pub text: String,
}

fn line(level: Level, text: String) -> Line {
    Line { level, text }
}

/// The lines in a CLAUDE.md, and how many of them are hexa's managed block
/// (the markers included).
pub(crate) fn claude_md_lines(text: &str) -> (usize, usize) {
    let all: Vec<&str> = text.lines().collect();
    let start = all.iter().position(|l| l.trim() == START_MARKER);
    let end = all.iter().position(|l| l.trim() == END_MARKER);
    let managed = match (start, end) {
        (Some(s), Some(e)) if e >= s => e - s + 1,
        _ => 0,
    };
    (all.len(), managed)
}

/// Whether a `.gitignore` covers `.claude/worktrees`: the directory itself or
/// all of `.claude`, with or without a leading or trailing slash.
pub(crate) fn worktrees_ignored(gitignore: &str) -> bool {
    gitignore.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).any(|l| {
        let l = l.trim_start_matches('/').trim_end_matches('/');
        l == ".claude/worktrees" || l == ".claude"
    })
}

/// What `hexa doctor` says about Claude Code's memory for the project at
/// `project_root`. Never a failure: a missing file or directory is information.
pub(crate) fn memory_lines(project_root: &Path, config_dir: Option<&str>, home: Option<&Path>) -> Vec<Line> {
    let mut out = Vec::new();

    match std::fs::read_to_string(project_root.join("CLAUDE.md")) {
        Err(_) => out.push(line(Level::Info, "CLAUDE.md: none".to_string())),
        Ok(text) => {
            let (total, managed) = claude_md_lines(&text);
            if total > CLAUDE_MD_LINES {
                out.push(line(
                    Level::Warn,
                    format!("CLAUDE.md: {total} lines, over the {CLAUDE_MD_LINES} Claude Code advises ({managed} are hexa's managed block); move what is not needed every session to .claude/rules/ or a skill"),
                ));
            } else {
                out.push(line(Level::Ok, format!("CLAUDE.md: {total} lines ({managed} hexa's managed block; advised under {CLAUDE_MD_LINES})")));
            }
        }
    }

    match auto_memory_dir(config_dir, home, project_root) {
        None => out.push(line(Level::Info, "auto memory: no config directory or home to look in".to_string())),
        Some(dir) => match std::fs::read(dir.join("MEMORY.md")) {
            Err(_) => out.push(line(Level::Info, format!("auto memory: no MEMORY.md at {} (auto memory off, or nothing saved here yet)", dir.display()))),
            Ok(bytes) => {
                let lines = String::from_utf8_lossy(&bytes).lines().count();
                if lines > MEMORY_INDEX_LINES || bytes.len() > MEMORY_INDEX_BYTES {
                    out.push(line(
                        Level::Warn,
                        format!("auto memory: MEMORY.md is {lines} lines, {} bytes; Claude Code loads only the first {MEMORY_INDEX_LINES} lines or 25KB and drops the rest, so shorten the index", bytes.len()),
                    ));
                } else {
                    out.push(line(Level::Ok, format!("auto memory: MEMORY.md {lines} lines, {} bytes (loaded up to {MEMORY_INDEX_LINES} lines or 25KB)", bytes.len())));
                }
            }
        },
    }

    if project_root.join(".claude").join("worktrees").is_dir() {
        let ignore = std::fs::read_to_string(project_root.join(".gitignore")).unwrap_or_default();
        if worktrees_ignored(&ignore) {
            out.push(line(Level::Ok, ".claude/worktrees is ignored by git".to_string()));
        } else {
            out.push(line(Level::Warn, ".claude/worktrees exists and is not in .gitignore (agent worktrees are full copies of the tree)".to_string()));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn write(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    fn auto_memory_dir_for(cfg: &Path, root: &Path) -> std::path::PathBuf {
        crate::commands::claude_paths::auto_memory_dir(cfg.to_str(), None, root).unwrap()
    }
    fn lines_of(n: usize) -> String {
        (0..n).map(|i| format!("line {i}\n")).collect()
    }
    fn find<'a>(ls: &'a [Line], needle: &str) -> &'a Line {
        ls.iter().find(|l| l.text.contains(needle)).unwrap_or_else(|| panic!("no line with {needle:?} in {ls:?}"))
    }

    #[test]
    fn the_managed_block_is_counted_between_hexa_markers() {
        let text = format!("# p\n\n{}\na\nb\nc\n{}\ntail\n", crate::commands::refresh::START_MARKER, crate::commands::refresh::END_MARKER);
        assert_eq!(claude_md_lines(&text), (8, 5));
        assert_eq!(claude_md_lines("one\ntwo\n"), (2, 0));
    }

    #[test]
    fn a_long_claude_md_is_a_warning_that_says_how_much_is_hexas() {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), "CLAUDE.md", &lines_of(250));
        let ls = memory_lines(d.path(), Some("/nowhere"), None);
        let l = find(&ls, "CLAUDE.md");
        assert_eq!(l.level, Level::Warn);
        assert!(l.text.contains("250") && l.text.contains("200"), "{}", l.text);
    }

    #[test]
    fn a_short_claude_md_is_ok_and_a_missing_one_is_information() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(find(&memory_lines(d.path(), Some("/nowhere"), None), "CLAUDE.md").level, Level::Info);
        write(d.path(), "CLAUDE.md", &lines_of(199));
        assert_eq!(find(&memory_lines(d.path(), Some("/nowhere"), None), "CLAUDE.md").level, Level::Ok);
    }

    /// Only the first 200 lines or 25KB of MEMORY.md are loaded; the rest is
    /// dropped without a word, so being over is a warning.
    #[test]
    fn the_memory_index_is_checked_against_what_claude_code_loads() {
        let cfg = tempfile::tempdir().unwrap();
        let root = Path::new("/work/proj");
        let mem = auto_memory_dir_for(cfg.path(), root);
        let check = |text: &str| {
            write(&mem, "MEMORY.md", text);
            let ls = memory_lines(root, Some(cfg.path().to_str().unwrap()), None);
            find(&ls, "MEMORY.md").level
        };
        assert_eq!(check(&lines_of(200)), Level::Ok);
        assert_eq!(check(&lines_of(201)), Level::Warn);
        assert_eq!(check(&"x".repeat(25 * 1024 + 1)), Level::Warn);
    }

    #[test]
    fn a_missing_memory_directory_is_information_not_a_failure() {
        let cfg = tempfile::tempdir().unwrap();
        let ls = memory_lines(Path::new("/work/none"), Some(cfg.path().to_str().unwrap()), None);
        assert_eq!(find(&ls, "auto memory").level, Level::Info);
    }

    #[test]
    fn agent_worktrees_must_be_ignored_by_git_when_they_exist() {
        let d = tempfile::tempdir().unwrap();
        assert!(memory_lines(d.path(), Some("/nowhere"), None).iter().all(|l| !l.text.contains("worktrees")), "no line when there are none");
        std::fs::create_dir_all(d.path().join(".claude/worktrees/a")).unwrap();
        assert_eq!(find(&memory_lines(d.path(), Some("/nowhere"), None), "worktrees").level, Level::Warn);
        write(d.path(), ".gitignore", "target/\n.claude/worktrees/\n");
        assert_eq!(find(&memory_lines(d.path(), Some("/nowhere"), None), "worktrees").level, Level::Ok);
    }

    #[test]
    fn the_gitignore_forms_that_cover_worktrees() {
        for covered in [".claude/worktrees", ".claude/worktrees/", "/.claude/worktrees/", ".claude/", ".claude", "/.claude/"] {
            assert!(worktrees_ignored(&format!("target/\n{covered}\n")), "{covered}");
        }
        for open in ["worktrees/", "# .claude/worktrees/", "src/.claude/worktrees/x", ""] {
            assert!(!worktrees_ignored(&format!("target/\n{open}\n")), "{open:?}");
        }
    }
}
