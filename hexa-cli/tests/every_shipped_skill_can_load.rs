//! A skill the harness cannot load is not a skill.
//!
//! Four of hexa's own shipped skills — `hexa-adr-create`, `-review`, `-search`
//! and `-status` — were bare `.md` files in the skills root, carrying
//! frontmatter and a `trigger:` field from a format nothing reads any more.
//! Claude Code loads `<name>/SKILL.md` and nothing else, so all four were
//! invisible. `hexa skill list` counted them anyway and reported twelve.
//!
//! Nothing failed, because a file that is never read cannot be wrong. So this
//! reads them.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("workspace root").to_path_buf()
}

struct Finding {
    /// Skills at `<name>/SKILL.md`, by directory name.
    loadable: BTreeSet<String>,
    /// Markdown files in the skills root, which the harness never reads.
    stranded: Vec<String>,
    /// A `SKILL.md` whose frontmatter name disagrees with its directory.
    misnamed: Vec<String>,
}

/// The frontmatter `name:` of a skill file.
fn declared_name(body: &str) -> Option<String> {
    let rest = body.trim_start().strip_prefix("---")?;
    let end = rest.find("\n---")?;
    rest[..end].lines().find_map(|l| {
        l.strip_prefix("name:")
            .map(|v| v.trim().trim_matches('"').trim_matches('\'').to_string())
    })
}

fn inspect(skills_root: &Path) -> Finding {
    let mut f =
        Finding { loadable: BTreeSet::new(), stranded: Vec::new(), misnamed: Vec::new() };
    let Ok(entries) = std::fs::read_dir(skills_root) else { return f };
    for e in entries.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if p.is_dir() {
            let skill = p.join("SKILL.md");
            if !skill.is_file() {
                f.stranded.push(format!("{name}/ (no SKILL.md)"));
                continue;
            }
            let body = std::fs::read_to_string(&skill).expect("read SKILL.md");
            match declared_name(&body) {
                Some(d) if d == name => {
                    f.loadable.insert(name);
                }
                Some(d) => f.misnamed.push(format!("{name}/SKILL.md declares `{d}`")),
                None => f.misnamed.push(format!("{name}/SKILL.md declares no name")),
            }
        } else if p.extension().is_some_and(|x| x == "md") {
            f.stranded.push(name);
        }
    }
    f
}

fn assert_all_loadable(label: &str, skills_root: &Path) {
    let f = inspect(skills_root);
    assert!(
        f.loadable.len() >= 10,
        "{label}: found only {} loadable skill(s); the walker is broken, which \
         would make this test pass vacuously",
        f.loadable.len()
    );
    assert!(
        f.stranded.is_empty(),
        "{label}: {} file(s) the harness will never load. A skill lives at \
         <name>/SKILL.md:\n  {}",
        f.stranded.len(),
        f.stranded.join("\n  ")
    );
    assert!(
        f.misnamed.is_empty(),
        "{label}: {} skill(s) whose name disagrees with their directory:\n  {}",
        f.misnamed.len(),
        f.misnamed.join("\n  ")
    );
}

/// The skills baked into the binary and written by `hexa init`.
#[test]
fn every_shipped_skill_is_loadable() {
    assert_all_loadable("shipped", &root().join("hexa-cli/assets/skills"));
}

/// And hexa's own installed copies, which are the ones an agent here reads.
#[test]
fn every_installed_skill_is_loadable() {
    assert_all_loadable("installed", &root().join(".claude/skills"));
}

/// The two sets are the same skills. `installed_assets_match_shipped.rs`
/// compares file paths; this compares the names a user would actually type.
#[test]
fn the_installed_skills_are_the_shipped_skills() {
    let shipped = inspect(&root().join("hexa-cli/assets/skills")).loadable;
    let installed = inspect(&root().join(".claude/skills")).loadable;
    assert_eq!(
        shipped, installed,
        "the skills hexa ships and the skills hexa has installed here are different sets"
    );
}

/// CLAUDE.md names a slash command for every skill, and no others.
///
/// Before this test, hexa's own operator manual promised `/hexa-feature-dev`,
/// `/hexa-validate` and `/cargo-fast`, none of which exist, and omitted three
/// that do. An agent reading that file would reach for a command that is not
/// there and never learn about the ones that are.
#[test]
fn claude_md_names_exactly_the_shipped_skills() {
    let md = std::fs::read_to_string(root().join("CLAUDE.md")).expect("read CLAUDE.md");
    let start = md.find("**Slash commands**").expect("CLAUDE.md lists slash commands");
    let block = &md[start..start + md[start..].find("\n\n").unwrap_or(md.len() - start)];

    let mut named: BTreeSet<String> = BTreeSet::new();
    let mut rest = block;
    while let Some(i) = rest.find("`/") {
        let after = &rest[i + 2..];
        let Some(j) = after.find('`') else { break };
        named.insert(after[..j].to_string());
        rest = &after[j + 1..];
    }
    assert!(named.len() >= 5, "found only {} slash command(s); the extractor is broken", named.len());

    let shipped = inspect(&root().join("hexa-cli/assets/skills")).loadable;
    let missing: Vec<&String> = shipped.difference(&named).collect();
    let invented: Vec<&String> = named.difference(&shipped).collect();
    assert!(
        missing.is_empty() && invented.is_empty(),
        "CLAUDE.md's slash commands do not match the shipped skills.\n  \
         not listed: {missing:?}\n  do not exist: {invented:?}"
    );
}
