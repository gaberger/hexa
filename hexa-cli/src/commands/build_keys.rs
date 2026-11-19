//! `hexa dev build-keys`: does a native build script get the same cache key
//! under every cargo command? (ADR-2611151330)
//!
//! It runs no compiler. It asks cargo for the unit graph of each selection
//! and compares them (`hexa_core::ports::build_keys`). A selection that cannot
//! be graphed is an error, never a skipped selection: a check that could not
//! look has not passed.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::Command;

use colored::Colorize;
use hexa_core::ports::build_keys::{analyze, Report};
use hexa_core::ports::ledger::LedgerError;
use serde_json::{json, Value};

/// How many selection names a group prints before it says "and N more".
const SHOWN: usize = 3;

pub async fn run(selections: Vec<String>, json_out: bool, text: bool) -> anyhow::Result<()> {
    let as_json = json_out || (!text && !std::io::stdout().is_terminal());
    let dir = std::env::current_dir()?;
    match compute(&dir, &selections) {
        Ok(report) => {
            if as_json {
                let mut v = serde_json::to_value(&report)?;
                v["ok"] = json!(true);
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                print_text(&report);
            }
            if !report.stable {
                crate::commands::events::exit(1);
            }
            Ok(())
        }
        Err(e) => {
            if as_json {
                let mut v = serde_json::to_value(&e)?;
                v["ok"] = json!(false);
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                eprintln!("\u{2717} {}\n  hint: {}\n  code: {}", e.message, e.hint, e.code);
            }
            crate::commands::events::exit(2);
        }
    }
}

/// Whether the project asks `hexa dev ci` to run this check.
pub fn enabled(dir: &Path) -> bool {
    project_config(dir)
        .and_then(|c| c.get("build_keys")?.get("enabled")?.as_bool())
        .unwrap_or(false)
}

/// Graph every selection and compare. `explicit` empty means the project's
/// `build_keys.selections`, else the default set.
pub fn compute(dir: &Path, explicit: &[String]) -> Result<Report, LedgerError> {
    let root = cargo_root(dir).ok_or_else(|| {
        err(
            "not_cargo_project",
            format!("no Cargo.toml in {} or above", dir.display()),
            "Run it from a Rust project, or from inside one.",
        )
    })?;
    let selections = if !explicit.is_empty() {
        explicit.to_vec()
    } else if let Some(s) = configured_selections(dir) {
        s
    } else {
        default_selections(dir)?
    };

    let locked = root.join("Cargo.lock").is_file();
    let mut graphs: Vec<(String, Value)> = Vec::new();
    for sel in &selections {
        graphs.push((sel.clone(), unit_graph(dir, sel, locked)?));
    }
    if graphs.len() < 2 {
        return Err(err(
            "too_few_selections",
            format!("{} selection(s) given; two are needed to compare", graphs.len()),
            "Pass --selection at least twice, or set build_keys.selections in .hexa/project.json to two or more.",
        ));
    }
    Ok(analyze(&graphs))
}

fn err(code: &str, message: String, hint: &str) -> LedgerError {
    LedgerError::new(code, message, hint.to_string(), None, false)
}

fn cargo_root(dir: &Path) -> Option<PathBuf> {
    dir.ancestors().find(|d| d.join("Cargo.toml").is_file()).map(Path::to_path_buf)
}

fn project_config(dir: &Path) -> Option<Value> {
    let f = dir.ancestors().map(|d| d.join(".hexa/project.json")).find(|f| f.is_file())?;
    serde_json::from_str(&std::fs::read_to_string(f).ok()?).ok()
}

fn configured_selections(dir: &Path) -> Option<Vec<String>> {
    let sel: Vec<String> = project_config(dir)?
        .get("build_keys")?
        .get("selections")?
        .as_array()?
        .iter()
        .filter_map(|s| s.as_str().map(str::to_string))
        .collect();
    (!sel.is_empty()).then_some(sel)
}

/// The workspace's three commands, plus `-p M` for each member when there is
/// more than one.
fn default_selections(dir: &Path) -> Result<Vec<String>, LedgerError> {
    let out = Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(dir)
        .env("RUSTC_BOOTSTRAP", "1")
        .output()
        .map_err(|e| err("no_unit_graph", format!("cannot run cargo: {e}"), "Install cargo."))?;
    if !out.status.success() {
        return Err(err(
            "no_unit_graph",
            format!("cargo metadata failed: {}", first_line(&out.stderr)),
            "Fix the workspace so `cargo metadata` succeeds.",
        ));
    }
    let meta: Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| err("no_unit_graph", format!("cargo metadata printed no JSON: {e}"), "Check the cargo version."))?;
    let mut members: Vec<String> = meta["packages"]
        .as_array()
        .map(|a| a.iter().filter_map(|p| p["name"].as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    members.sort();
    let mut sel = vec!["build --workspace".to_string(), "test --no-run --workspace".to_string(), "clippy --workspace --all-targets".to_string()];
    if members.len() > 1 {
        for m in &members {
            sel.push(format!("build -p {m}"));
            sel.push(format!("test --no-run -p {m}"));
        }
    }
    Ok(sel)
}

fn first_line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string()
}

/// `cargo <selection> --unit-graph`, offline first. `RUSTC_BOOTSTRAP=1` is
/// what lets a stable cargo accept `-Z unstable-options`; a cargo that drops
/// `--unit-graph` fails here, loudly.
fn unit_graph(dir: &Path, selection: &str, locked: bool) -> Result<Value, LedgerError> {
    let mut last = String::new();
    for offline in [true, false] {
        let mut cmd = Command::new("cargo");
        cmd.args(selection.split_whitespace());
        if locked {
            cmd.arg("--locked");
        }
        if offline {
            cmd.arg("--offline");
        }
        cmd.args(["--unit-graph", "-Z", "unstable-options"]).current_dir(dir).env("RUSTC_BOOTSTRAP", "1");
        match cmd.output() {
            Ok(o) if o.status.success() => match serde_json::from_slice::<Value>(&o.stdout) {
                Ok(v) if v.get("units").is_some() => return Ok(v),
                _ => last = "cargo printed no unit graph".to_string(),
            },
            Ok(o) => last = first_line(&o.stderr),
            Err(e) => last = format!("cannot run cargo: {e}"),
        }
    }
    Err(err(
        "no_unit_graph",
        format!("`cargo {selection}` gave no unit graph: {last}"),
        "Fix the selection or the workspace; a selection that cannot be graphed is not skipped.",
    ))
}

fn print_text(r: &Report) {
    println!(
        "{} build keys: {} selections, {} native build script(s), {} unstable",
        "\u{2b21}".cyan(),
        r.summary.selections,
        r.summary.native_scripts,
        r.summary.unstable
    );
    println!();
    for s in &r.scripts {
        if s.stable {
            println!("  {} {}: one key across {} selection(s)", "\u{2713}".green(), s.package, s.selections);
            continue;
        }
        println!("  {} {}: {} different keys across {} selections (unstable)", "\u{2717}".red(), s.package, s.groups.len(), s.selections);
        for (i, g) in s.groups.iter().enumerate() {
            let shown: Vec<&str> = g.iter().take(SHOWN).map(String::as_str).collect();
            let more = g.len().saturating_sub(SHOWN);
            println!(
                "      key {}: {}{}",
                i + 1,
                shown.join(", "),
                if more > 0 { format!(" and {more} more") } else { String::new() }
            );
        }
        for d in &s.differences {
            println!("      {} ({}): {}", d.unit, d.kind, d.detail);
        }
        if let Some(h) = &s.hint {
            println!("      hint: {h}");
        }
    }
    for n in &r.notes {
        println!("  {} {n}", "\u{b7}".dimmed());
    }
}
