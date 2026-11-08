//! Where Claude Code keeps what hexa reads: its project transcripts and its
//! auto-memory directory. One place, so `hexa insight` and `hexa doctor` agree
//! (https://code.claude.com/docs/en/memory, "Storage location").

use std::path::{Path, PathBuf};

/// Where Claude Code keeps its project transcripts: `$CLAUDE_CONFIG_DIR/projects`
/// when that variable is set and non-empty, else `~/.claude/projects`.
pub(crate) fn claude_projects_root(config_dir: Option<&str>, home: Option<&Path>) -> Option<PathBuf> {
    match config_dir.filter(|d| !d.is_empty()) {
        Some(d) => Some(Path::new(d).join("projects")),
        None => home.map(|h| h.join(".claude").join("projects")),
    }
}

/// The name Claude Code gives a project's directory under `projects/`: its
/// path with every character that is not a letter or a digit replaced by `-`.
pub(crate) fn project_dir_name(project_root: &Path) -> String {
    project_root
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// The auto-memory directory for a project: `<projects root>/<name>/memory`.
/// All worktrees of one repository share it.
pub(crate) fn auto_memory_dir(config_dir: Option<&str>, home: Option<&Path>, project_root: &Path) -> Option<PathBuf> {
    claude_projects_root(config_dir, home).map(|r| r.join(project_dir_name(project_root)).join("memory"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    /// Claude Code keeps its data under `$CLAUDE_CONFIG_DIR` when that is set;
    /// a profile such as `~/.claude-profiles/<name>/config` has its transcripts
    /// there, and `~/.claude` is only the default.
    #[test]
    fn the_config_dir_wins_when_it_is_set() {
        let home = Path::new("/home/u");
        assert_eq!(
            claude_projects_root(Some("/home/u/.claude-profiles/p/config"), Some(home)),
            Some(PathBuf::from("/home/u/.claude-profiles/p/config/projects"))
        );
    }

    #[test]
    fn an_unset_or_empty_config_dir_falls_back_to_the_default() {
        let home = Path::new("/home/u");
        let default = Some(PathBuf::from("/home/u/.claude/projects"));
        assert_eq!(claude_projects_root(None, Some(home)), default);
        assert_eq!(claude_projects_root(Some(""), Some(home)), default);
    }

    #[test]
    fn a_config_dir_needs_no_home_and_nothing_means_no_root() {
        assert_eq!(claude_projects_root(Some("/cfg"), None), Some(PathBuf::from("/cfg/projects")));
        assert_eq!(claude_projects_root(None, None), None);
        assert_eq!(claude_projects_root(Some(""), None), None);
    }

    /// Seen on a real install: `/Volumes/Crucial/projects/strata` is
    /// `-Volumes-Crucial-projects-strata`. Every character that is not a letter
    /// or digit becomes a dash.
    #[test]
    fn a_project_directory_name_replaces_every_non_alphanumeric() {
        assert_eq!(project_dir_name(Path::new("/Volumes/Crucial/projects/strata")), "-Volumes-Crucial-projects-strata");
        assert_eq!(project_dir_name(Path::new("/home/u/my.app_2")), "-home-u-my-app-2");
    }

    #[test]
    fn the_auto_memory_directory_is_under_the_projects_root() {
        assert_eq!(
            auto_memory_dir(Some("/cfg"), None, Path::new("/work/strata")),
            Some(PathBuf::from("/cfg/projects/-work-strata/memory"))
        );
        assert_eq!(
            auto_memory_dir(None, Some(Path::new("/home/u")), Path::new("/work/strata")),
            Some(PathBuf::from("/home/u/.claude/projects/-work-strata/memory"))
        );
        assert_eq!(auto_memory_dir(None, None, Path::new("/work/strata")), None);
    }
}
