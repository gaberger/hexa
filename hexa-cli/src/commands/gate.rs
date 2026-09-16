//! `hexa gate coverage` — which stated requirement is verified by nothing?
//!
//! ADR-2609160300 §1. The project's rule is that the executable gate replaces
//! the written spec. It had no way to ask whether the gate covers the spec.
//! `evidence_is_vacuous` catches a gate that runs zero tests; it cannot catch
//! a gate that runs eleven good checks and misses a twelfth requirement.
//!
//! Measured on 2026-09-15: deleting an entire required adapter from a trial
//! implementation left its gate printing PASS. A competing method found that
//! by reading the spec. This finds it by running.
//!
//! The method is mutation testing at component granularity. For each module,
//! remove it, run the gate, restore it. A module whose deletion the gate
//! survives is verified by nothing.

use colored::Colorize;
use std::path::{Path, PathBuf};

/// One module and what happened when it was removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Coverage {
    pub(crate) module: String,
    /// True when the gate still passed with this module deleted, which means
    /// nothing the gate runs depends on it.
    pub(crate) gate_survived: bool,
}

/// The module names of every row the gate survived, in the order given.
pub(crate) fn uncovered(rows: &[Coverage]) -> Vec<String> {
    rows.iter().filter(|r| r.gate_survived).map(|r| r.module.clone()).collect()
}

/// The files worth deleting one at a time. Source only; never the composition
/// root, which is wiring rather than a subject; never a test, which is the gate
/// itself and would make the measurement circular.
pub(crate) fn candidate_modules(files: &[PathBuf], composition_root: &str) -> Vec<String> {
    let mut out = Vec::new();
    for f in files {
        let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !matches!(ext, "ts" | "tsx" | "js" | "rs" | "go") {
            continue;
        }
        let rel = f.to_string_lossy().replace('\\', "/");
        if rel == composition_root {
            continue;
        }
        let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.contains(".test.") || name.ends_with("_test.go") {
            continue;
        }
        if f.components().any(|c| {
            let c = c.as_os_str().to_string_lossy();
            c == "test" || c == "tests"
        }) {
            continue;
        }
        out.push(rel);
    }
    out
}

/// Restores a deleted file when it goes out of scope, however it goes out of
/// scope. A wrong answer is recoverable; a source file left deleted on disk
/// because the gate panicked is not.
struct Restore {
    path: PathBuf,
    bytes: Vec<u8>,
}

impl Drop for Restore {
    fn drop(&mut self) {
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&self.path, &self.bytes);
    }
}

/// Collect every file under `root`, skipping the directories that hold build
/// output rather than source.
fn collect_files(root: &Path, out: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    const SKIP: [&str; 6] = ["target", "node_modules", ".git", "dist", "build", "graph-out"];
    let entries = match std::fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if SKIP.contains(&name.as_str()) {
                continue;
            }
            collect_files(&path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

/// Delete each candidate in turn, run the gate, restore it, and record whether
/// the gate noticed.
pub(crate) async fn measure(
    root: &Path,
    gate: &str,
    composition_root: &str,
) -> anyhow::Result<Vec<Coverage>> {
    let mut files = Vec::new();
    collect_files(root, &mut files)?;

    let relative: Vec<PathBuf> = files
        .iter()
        .map(|f| f.strip_prefix(root).unwrap_or(f).to_path_buf())
        .collect();

    let mut rows = Vec::new();
    for module in candidate_modules(&relative, composition_root) {
        let path = root.join(&module);
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let guard = Restore { path: path.clone(), bytes };
        std::fs::remove_file(&path)?;
        let (ok, _out) = hexa_exec::direct_exec::run_evidence(gate, root).await;
        drop(guard);
        rows.push(Coverage { module, gate_survived: ok });
    }
    Ok(rows)
}

/// `hexa gate coverage` — name every module the gate does not verify.
pub(crate) async fn run(
    target: String,
    gate: String,
    composition_root: Option<String>,
) -> anyhow::Result<()> {
    let root = PathBuf::from(&target);

    let composition_root = match composition_root {
        Some(c) => c,
        None => ["src/main.ts", "src/main.rs", "main.go", "src/index.ts"]
            .iter()
            .find(|c| root.join(c).exists())
            .map(|c| c.to_string())
            .unwrap_or_default(),
    };

    let rows = measure(&root, &gate, &composition_root).await;
    let rows = rows?;

    println!("{} {}", "deletion coverage of".bold(), gate.cyan());

    for row in &rows {
        if row.gate_survived {
            println!(
                "  {} {} {}",
                "✗".red(),
                row.module,
                "— the gate passes without it".dimmed()
            );
        } else {
            println!("  {} {}", "✓".green(), row.module);
        }
    }

    let missed = uncovered(&rows);
    println!("{} of {} modules are verified by nothing", missed.len(), rows.len());

    if missed.is_empty() {
        Ok(())
    } else {
        anyhow::bail!("{} modules are verified by nothing", missed.len())
    }
}

#[cfg(test)]
mod gate_coverage {
    //! ADR-2609160300 §1: a gate that survives the deletion of a required
    //! component is a failed gate, by the same rule that rejects a vacuous one.
    use super::{candidate_modules, uncovered, Coverage};
    use std::path::PathBuf;

    fn cov(m: &str, survived: bool) -> Coverage {
        Coverage { module: m.to_string(), gate_survived: survived }
    }

    #[test]
    fn a_module_whose_deletion_the_gate_survives_is_uncovered() {
        let rows = vec![
            cov("src/adapters/secondary/cache.ts", true),
            cov("src/adapters/secondary/store.ts", false),
            cov("src/usecases/shorten.ts", false),
        ];
        assert_eq!(uncovered(&rows), vec!["src/adapters/secondary/cache.ts".to_string()]);
    }

    #[test]
    fn a_gate_that_survives_every_deletion_reports_every_module() {
        let rows = vec![cov("a.ts", true), cov("b.ts", true)];
        assert_eq!(uncovered(&rows).len(), 2, "a gate nothing depends on covers nothing");
    }

    #[test]
    fn a_fully_covered_tree_reports_nothing() {
        let rows = vec![cov("a.ts", false), cov("b.ts", false)];
        assert!(uncovered(&rows).is_empty());
    }

    #[test]
    fn candidates_skip_the_composition_root_and_tests_and_non_source() {
        let files: Vec<PathBuf> = [
            "src/main.ts",
            "src/adapters/secondary/store.ts",
            "src/domain/link.ts",
            "test/store.test.ts",
            "tests/http.test.ts",
            "src/adapters/secondary/store.test.ts",
            "README.md",
            "package.json",
        ].iter().map(PathBuf::from).collect();
        let got = candidate_modules(&files, "src/main.ts");
        assert_eq!(got, vec![
            "src/adapters/secondary/store.ts".to_string(),
            "src/domain/link.ts".to_string(),
        ], "the composition root is the wiring, tests are the gate, neither is a subject");
    }

    #[test]
    fn rust_and_go_sources_are_candidates_too() {
        let files: Vec<PathBuf> = ["src/lib.rs", "src/store.go", "src/lib_test.go", "src/main.rs"]
            .iter().map(PathBuf::from).collect();
        let got = candidate_modules(&files, "src/main.rs");
        assert_eq!(got, vec!["src/lib.rs".to_string(), "src/store.go".to_string()]);
    }
}
