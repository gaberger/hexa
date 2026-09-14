//! A playbook must be runnable, or it is a spec in a new hat.
//!
//! ADR-2609140844, decision 4. The whole reason `hexa hey` hands back a
//! procedure instead of prose is that prose cannot fail. A playbook whose steps
//! name verbs that do not exist, or which never reaches a gate, is the same
//! unrunnable document the ADR replaced — only now hexa prints it with
//! authority.
//!
//! So each shipped playbook must: parse, name only real verbs, carry at least
//! one proof step, and end at the architecture grade.

use std::path::{Path, PathBuf};
use std::process::Command;

use hexa_cli::playbook::{self, Playbook};

fn hexa_bin() -> PathBuf {
    let mut p = std::env::current_exe().expect("test exe");
    p.pop();
    p.pop();
    p.push("hexa");
    p
}

fn resolves(chain: &[String]) -> bool {
    Command::new(hexa_bin())
        .args(chain)
        .arg("--help")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// The verb chain of a `hexa …` step, or `None` when the step runs something
/// else. A placeholder, a flag or a quoted argument ends the chain.
fn hexa_chain(run: &str) -> Option<Vec<String>> {
    let mut tokens = run.split_whitespace();
    if tokens.next()? != "hexa" {
        return None;
    }
    let mut out = Vec::new();
    for t in tokens {
        let checkable = !t.is_empty()
            && !t.starts_with('-')
            && t.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !checkable {
            break;
        }
        out.push(t.to_string());
    }
    Some(out)
}

/// Read the asset directory, not the embedded copy. A file that fails to embed
/// must fail here too, rather than vanishing from the set being checked.
fn playbook_files() -> Vec<(String, Playbook)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/playbooks");
    let mut out = Vec::new();
    for e in std::fs::read_dir(&dir).expect("playbooks dir").flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "json") {
            let body = std::fs::read_to_string(&p).expect("read playbook");
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            let pb: Playbook = serde_json::from_str(&body)
                .unwrap_or_else(|e| panic!("{name} is not a valid playbook: {e}"));
            out.push((name, pb));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    assert!(
        out.len() >= 4,
        "found only {} playbook file(s); the walker is broken, which would make \
         every test in this file pass vacuously",
        out.len()
    );
    out
}

#[test]
fn every_playbook_step_names_a_real_verb() {
    let mut checked = 0usize;
    let mut dead: Vec<String> = Vec::new();
    for (file, pb) in playbook_files() {
        for (i, step) in pb.steps.iter().enumerate() {
            let Some(c) = hexa_chain(&step.run) else { continue };
            checked += 1;
            assert!(!c.is_empty(), "{file} step {}: `hexa` with no verb", i + 1);
            if !resolves(&c) {
                dead.push(format!("{file} step {}: hexa {}", i + 1, c.join(" ")));
            }
        }
    }
    assert!(checked >= 15, "only {checked} hexa steps found; the extractor is broken");
    assert!(dead.is_empty(), "{} dead verb(s) in playbooks:\n  {}", dead.len(), dead.join("\n  "));
}

/// Decision 4: every playbook ends at the architecture grade.
#[test]
fn every_playbook_ends_at_the_grade() {
    for (file, pb) in playbook_files() {
        let last = pb.steps.last().unwrap_or_else(|| panic!("{file} has no steps"));
        assert!(
            last.run.starts_with("hexa analyze"),
            "{file} ends on `{}`, not the architecture grade. A playbook that \
             does not reach a gate is prose.",
            last.run
        );
    }
}

/// Decision 4: and at least one step proves something before it gets there.
///
/// hexa has exactly three proof shapes — a verified claim, an evidence command,
/// and a gate. A playbook that uses none of them asserts its own success.
#[test]
fn every_playbook_carries_a_proof_step() {
    for (file, pb) in playbook_files() {
        let proves = pb.steps.iter().any(|s| {
            s.run.starts_with("hexa verify") || s.run.contains("--evidence") || s.run.contains("--gate")
        });
        assert!(
            proves,
            "{file} has no proof step: no `hexa verify`, no `--evidence`, no `--gate`"
        );
    }
}

#[test]
fn every_playbook_is_substantial_and_well_formed() {
    for (file, pb) in playbook_files() {
        assert!(!pb.name.is_empty(), "{file}: empty name");
        assert!(!pb.summary.is_empty(), "{file}: empty summary");
        assert!(pb.steps.len() >= 3, "{file}: {} step(s) is not a procedure", pb.steps.len());
        assert!(!pb.triggers.is_empty(), "{file}: no triggers, so nothing can route here");
        for t in &pb.triggers {
            assert_eq!(t.to_lowercase(), *t, "{file}: trigger `{t}` is not lowercase");
            assert!(!t.contains(' '), "{file}: trigger `{t}` is not one word");
        }
        for (i, s) in pb.steps.iter().enumerate() {
            assert!(!s.title.is_empty(), "{file} step {}: empty title", i + 1);
            assert!(!s.run.is_empty(), "{file} step {}: empty run", i + 1);
            assert!(!s.done_when.is_empty(), "{file} step {}: no done condition", i + 1);
        }
    }
}

/// The file name and the declared name must agree, or `hexa hey` prints one
/// name and the operator opens another file.
#[test]
fn every_playbook_file_is_named_after_its_playbook() {
    for (file, pb) in playbook_files() {
        assert_eq!(file, format!("{}.json", pb.name), "{file} declares name `{}`", pb.name);
    }
}

/// The embedded copy and the files on disk are the same set. A playbook that
/// ships in the repository but not in the binary helps nobody.
#[test]
fn every_playbook_file_is_embedded_in_the_binary() {
    let on_disk: Vec<String> = playbook_files().into_iter().map(|(_, p)| p.name).collect();
    let mut embedded: Vec<String> =
        playbook::load().expect("embedded playbooks load").into_iter().map(|p| p.name).collect();
    embedded.sort();
    let mut disk = on_disk;
    disk.sort();
    assert_eq!(disk, embedded, "the shipped playbooks and the embedded playbooks differ");
}
