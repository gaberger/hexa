//! hexa's own `.claude/` carries exactly what hexa ships.
//!
//! This repository's installed skills drifted from the embedded ones and
//! nothing failed. Fifteen skills that `hexa init` no longer installs sat in
//! `.claude/skills/`, ten of them describing SpacetimeDB, HexFlo and a daemon
//! deleted in the collapse, and one telling the agent to run `hexa summarize`,
//! which does not exist. The agent working on hexa was reading them.
//!
//! `hexa-cli/tests/shipped_docs_name_real_verbs.rs` checks the *embedded*
//! assets, which were clean the whole time. Nothing checked the copies that
//! are actually loaded. ADR-2609122048: a document that cannot fail is
//! indistinguishable from one that is wrong, so this makes it fail.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

/// Every markdown file under a skills directory, as a path relative to it.
fn skill_set(dir: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "md") {
                if let Ok(rel) = p.strip_prefix(dir) {
                    out.insert(rel.display().to_string());
                }
            }
        }
    }
    out
}

#[test]
fn the_installed_skills_are_the_shipped_skills() {
    let root = root();
    let shipped = skill_set(&root.join("hexa-cli/assets/skills"));
    let installed = skill_set(&root.join(".claude/skills"));

    assert!(
        shipped.len() >= 8,
        "found only {} shipped skills; the walker is broken",
        shipped.len()
    );

    let extra: Vec<&String> = installed.difference(&shipped).collect();
    let missing: Vec<&String> = shipped.difference(&installed).collect();

    assert!(
        extra.is_empty(),
        "{} skill(s) installed in .claude/skills that hexa does not ship. \
         Remove them, or add them to hexa-cli/assets/skills:\n  {:?}",
        extra.len(),
        extra
    );
    assert!(
        missing.is_empty(),
        "{} shipped skill(s) missing from .claude/skills. Run `hexa assets sync`:\n  {:?}",
        missing.len(),
        missing
    );
}

#[test]
fn the_installed_skills_match_the_shipped_bytes() {
    let root = root();
    let ship_dir = root.join("hexa-cli/assets/skills");
    let inst_dir = root.join(".claude/skills");

    let mut stale = Vec::new();
    for rel in skill_set(&ship_dir) {
        let a = std::fs::read_to_string(ship_dir.join(&rel)).unwrap_or_default();
        let b = match std::fs::read_to_string(inst_dir.join(&rel)) {
            Ok(v) => v,
            Err(_) => continue, // absence is the other test's job
        };
        if a != b {
            stale.push(rel);
        }
    }
    assert!(
        stale.is_empty(),
        "{} installed skill(s) differ from the shipped copy. Run `hexa assets sync`:\n  {:?}",
        stale.len(),
        stale
    );
}

/// The hooks are what put the loop in front of the agent. Without this file
/// no hook fires at all, which is how this repository ran until now.
#[test]
fn the_hook_wiring_is_committed_and_names_real_events() {
    let settings = root().join(".claude/settings.json");
    assert!(
        settings.is_file(),
        ".claude/settings.json is missing, so no hook fires in hexa's own repo"
    );
    let body = std::fs::read_to_string(&settings).expect("read settings.json");
    let json: serde_json::Value = serde_json::from_str(&body).expect("settings.json is JSON");

    let hooks = json.get("hooks").and_then(|h| h.as_object()).expect("hooks object");
    for event in ["SessionStart", "UserPromptSubmit", "PreToolUse"] {
        assert!(hooks.contains_key(event), "no {event} hook is wired");
    }

    // The status line ran `node scripts/hexa-statusline.cjs`, a file that has
    // never existed in this repository.
    let status = json
        .get("statusLine")
        .and_then(|s| s.get("command"))
        .and_then(|c| c.as_str())
        .expect("statusLine command");
    assert!(
        status.starts_with("hexa "),
        "the status line must run hexa, not {status:?}"
    );
}
