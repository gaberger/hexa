//! `hexa spec check | accept | seal | verify` — an ADR's decisions, held to
//! the spec that states them as behaviour (ADR-2610221540).
//!
//! The coverage rule is the domain's, reached through `hexa_core::ports::spec`.
//! This file reads the ledger, hashes, and runs each scenario's test alone.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use colored::Colorize;
use hexa_core::ports::spec::{coverage, numbered_decisions, test_ref, Acceptance, Spec};
use sha2::{Digest, Sha256};

use super::adr::{failure_tail, gate_path, NOT_PROVEN_HERE};

/// ADRs accepted before this one carry no spec and are legacy (§9). From it
/// on, an accepted ADR without a spec fails `check`.
const SPEC_REQUIRED_FROM: u64 = 2610221540;

const ADRS: &str = "docs/adrs";
const SPECS: &str = "docs/specs";

/// `ADR-…` from a full id or bare digits.
fn normalise(id: &str) -> String {
    if id.starts_with("ADR-") { id.to_string() } else { format!("ADR-{id}") }
}

/// One ADR in the ledger.
struct Adr {
    id: String,
    path: PathBuf,
    status: String,
}

impl Adr {
    /// Accepted and Completed decisions bind; the rest do not need a spec.
    fn binds(&self) -> bool {
        matches!(self.status.as_str(), "accepted" | "completed")
    }

    /// Made before specs were required. An id that is not the ten-digit
    /// timestamp form predates them too.
    fn legacy(&self) -> bool {
        self.id.trim_start_matches("ADR-").parse::<u64>().map_or(true, |n| n < SPEC_REQUIRED_FROM)
    }
}

/// The id is the filename up to the slug: `ADR-<10 digits>` or the whole stem.
fn adr_id_of(stem: &str) -> String {
    let digits: String = stem.trim_start_matches("ADR-").chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.len() == 10 { format!("ADR-{digits}") } else { stem.to_string() }
}

fn status_of(text: &str) -> String {
    text.lines()
        .find_map(|l| l.strip_prefix("**Status:**"))
        .and_then(|s| s.split_whitespace().next())
        .unwrap_or("")
        .to_lowercase()
}

fn ledger(root: &Path) -> anyhow::Result<Vec<Adr>> {
    let mut out: Vec<Adr> = std::fs::read_dir(root.join(ADRS))
        .map_err(|e| anyhow::anyhow!("no {ADRS}/: {e}"))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .filter_map(|p| {
            let stem = p.file_stem()?.to_string_lossy().to_string();
            stem.starts_with("ADR-").then(|| {
                let status = status_of(&std::fs::read_to_string(&p).unwrap_or_default());
                Adr { id: adr_id_of(&stem), path: p, status }
            })
        })
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

fn find(root: &Path, id: &str) -> anyhow::Result<Adr> {
    let id = normalise(id);
    ledger(root)?.into_iter().find(|a| a.id == id).ok_or_else(|| anyhow::anyhow!("{id}: no such ADR in {ADRS}/"))
}

fn spec_path(root: &Path, id: &str) -> PathBuf {
    root.join(SPECS).join(format!("{id}.json"))
}

fn load(root: &Path, id: &str) -> anyhow::Result<Option<Spec>> {
    let p = spec_path(root, id);
    if !p.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&p)?;
    serde_json::from_str(&text).map(Some).map_err(|e| anyhow::anyhow!("{}: not a spec: {e}", p.display()))
}

fn save(root: &Path, spec: &Spec) -> anyhow::Result<()> {
    let p = spec_path(root, &spec.adr);
    std::fs::write(&p, serde_json::to_string_pretty(spec)? + "\n")?;
    Ok(())
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// What a person accepted: the hash of the behaviour, not of what follows it.
fn content_hash(spec: &Spec) -> anyhow::Result<String> {
    Ok(sha256(serde_json::to_string(&spec.accepted_content())?.as_bytes()))
}

/// Coverage findings for one ADR and its spec, printed. True when covered.
fn print_coverage(adr: &Adr, spec: &Spec) -> bool {
    let text = std::fs::read_to_string(&adr.path).unwrap_or_default();
    let findings = coverage(&adr.id, &numbered_decisions(&text), spec);
    for e in spec.exempt.iter().filter(|e| !e.reason.trim().is_empty()) {
        println!("  {} {} {} exempt: {}", "\u{25cb}".dimmed(), adr.id, e.decision, e.reason);
    }
    for f in &findings {
        println!("  {} {} {}", "\u{2717}".red(), adr.id, f.message());
    }
    findings.is_empty()
}

/// `hexa spec check [ADR]` — coverage, with no model involved (§3, §9).
pub fn check(adr: Option<&str>) -> anyhow::Result<()> {
    let root = std::env::current_dir()?;
    println!("{} every numbered decision has an example and a negative", "\u{2b21} spec check".cyan().bold());
    let adrs = match adr {
        Some(id) => vec![find(&root, id)?],
        None => ledger(&root)?,
    };
    let (mut covered, mut failed, mut legacy) = (0, 0, 0);
    for a in &adrs {
        match load(&root, &a.id)? {
            Some(spec) if print_coverage(a, &spec) => {
                covered += 1;
                println!("  {} {} covered ({} scenarios)", "\u{2713}".green(), a.id, spec.specs.len());
            }
            Some(_) => failed += 1,
            None if adr.is_none() && !a.binds() => {}
            None if adr.is_none() && a.legacy() => legacy += 1,
            None => {
                failed += 1;
                println!("  {} {} has no spec: {}/{}.json", "\u{2717}".red(), a.id, SPECS, a.id);
            }
        }
    }
    println!("\n  {covered} covered · {failed} failed · {legacy} legacy (accepted before ADR-{SPEC_REQUIRED_FROM}, no spec)");
    if failed > 0 {
        anyhow::bail!("{failed} ADR(s) not covered by a spec");
    }
    Ok(())
}

/// `hexa spec accept <ADR>` — a person approves the behaviour (§4).
pub fn accept(adr: &str, by: Option<&str>) -> anyhow::Result<()> {
    let root = std::env::current_dir()?;
    let a = find(&root, adr)?;
    let mut spec = load(&root, &a.id)?.ok_or_else(|| anyhow::anyhow!("{} has no spec to accept", a.id))?;
    if !print_coverage(&a, &spec) {
        anyhow::bail!("{}: the spec does not cover the decision; accept a complete one", a.id);
    }
    let by = by
        .map(str::to_string)
        .or_else(|| {
            let out = std::process::Command::new("git").args(["config", "user.name"]).output().ok()?;
            Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
        })
        .filter(|s| !s.trim().is_empty())
        .or_else(|| std::env::var("USER").ok())
        .ok_or_else(|| anyhow::anyhow!("who is accepting? pass --by"))?;
    let sha256 = content_hash(&spec)?;
    println!("  {} {} accepted by {} at {}", "\u{2713}".green(), a.id, by, &sha256[..12]);
    spec.accepted = Some(Acceptance { by, sha256 });
    save(&root, &spec)
}

/// Is `spec` accepted as it stands? `Err` names why not.
fn acceptance_holds(spec: &Spec) -> anyhow::Result<Result<(), String>> {
    Ok(match &spec.accepted {
        None => Err("not accepted: no person has accepted this spec (`hexa spec accept`)".into()),
        Some(a) if a.sha256 != content_hash(spec)? => {
            Err(format!("changed since it was accepted by {}: accept it again", a.by))
        }
        Some(_) => Ok(()),
    })
}

/// `hexa spec seal <ADR>` — hash every test file the scenarios name (§6).
pub fn seal(adr: &str) -> anyhow::Result<()> {
    let root = std::env::current_dir()?;
    let a = find(&root, adr)?;
    let mut spec = load(&root, &a.id)?.ok_or_else(|| anyhow::anyhow!("{} has no spec to seal", a.id))?;
    if let Err(why) = acceptance_holds(&spec)? {
        anyhow::bail!("{}: {why}. Tests are sealed against an accepted spec, never a draft", a.id);
    }
    let mut sealed = BTreeMap::new();
    for f in spec.test_files() {
        let bytes = std::fs::read(root.join(&f)).map_err(|e| anyhow::anyhow!("{f}: {e}"))?;
        println!("  {} sealed {}", "\u{2713}".green(), f);
        sealed.insert(f, sha256(&bytes));
    }
    spec.sealed = sealed;
    save(&root, &spec)
}

/// One scenario's test, run alone.
enum Run {
    Passed,
    Failed(String),
    NotFound(String),
    Vacuous,
    Ambiguous(u64),
    NotProvenHere(String),
}

/// The nearest directory at or above `file`, inside `root`, with a Cargo.toml.
fn crate_of(root: &Path, file: &Path) -> Option<PathBuf> {
    let mut dir = file.parent()?;
    loop {
        if dir.join("Cargo.toml").exists() {
            return Some(dir.to_path_buf());
        }
        if dir == root {
            return None;
        }
        dir = dir.parent()?;
    }
}

/// The cargo arguments that run `f` in `file` and nothing else: an
/// integration test by its target, a library test by its module path.
fn cargo_args(krate: &Path, file: &Path, f: &str) -> Option<Vec<String>> {
    let rel = file.strip_prefix(krate).ok()?;
    let manifest = krate.join("Cargo.toml").display().to_string();
    let mut parts: Vec<String> = rel.with_extension("").iter().map(|s| s.to_string_lossy().to_string()).collect();
    let (target, name) = match parts.first().map(String::as_str) {
        Some("tests") if parts.len() == 2 => (vec!["--test".to_string(), parts[1].clone()], f.to_string()),
        Some("src") => {
            parts.remove(0);
            if matches!(parts.last().map(String::as_str), Some("lib" | "mod" | "main")) {
                parts.pop();
            }
            parts.push(f.to_string());
            (vec!["--lib".to_string()], parts.join("::"))
        }
        _ => return None,
    };
    let mut args = vec!["test".to_string(), "--manifest-path".to_string(), manifest];
    args.extend(target);
    args.extend(["--".to_string(), name, "--exact".to_string()]);
    Some(args)
}

async fn run_one(root: &Path, path_env: &str, r: &str) -> Run {
    let Some((file, f)) = test_ref(r) else { return Run::NotFound(format!("{r} is not `<path>.rs::<test_fn>`")) };
    let abs = root.join(file);
    let leaf = f.rsplit("::").next().unwrap_or(f);
    let text = std::fs::read_to_string(&abs).unwrap_or_default();
    if !text.contains(&format!("fn {leaf}(")) {
        return Run::NotFound(format!("{leaf} not found in {file}"));
    }
    let Some(args) = crate_of(root, &abs).and_then(|k| cargo_args(&k, &abs, f)) else {
        return Run::NotFound(format!("{file} is in no crate's tests/ or src/"));
    };
    let quoted: Vec<String> = args.iter().map(|a| format!("{a:?}")).collect();
    let shell = format!("export PATH={path_env:?} HEXA_NO_SKIP=1; cargo {}", quoted.join(" "));
    let (ok, out) = hexa_exec::direct_exec::run_evidence(&shell, root).await;
    if let Some(line) = out.lines().find(|l| l.contains(NOT_PROVEN_HERE)) {
        let reason = line.split(NOT_PROVEN_HERE).nth(1).unwrap_or("").trim();
        return Run::NotProvenHere(reason.trim_matches(|c| c == '"' || c == '\'').to_string());
    }
    if !ok {
        return Run::Failed(failure_tail(&out, 10));
    }
    let passed: u64 = out
        .lines()
        .filter_map(|l| l.strip_prefix("test result: ok. "))
        .filter_map(|l| l.split(" passed").next()?.trim().parse::<u64>().ok())
        .sum();
    match passed {
        0 => Run::Vacuous,
        1 => Run::Passed,
        n => Run::Ambiguous(n),
    }
}

/// `hexa spec verify [ADR] [--ci]` — drift is a failure (§6, §7, §8).
pub async fn verify(adr: Option<&str>, ci: bool) -> anyhow::Result<()> {
    let root = std::env::current_dir()?;
    println!("{} each scenario's sealed test, run alone", "\u{2b21} spec verify".cyan().bold());
    let adrs = match adr {
        Some(id) => vec![find(&root, id)?],
        None => ledger(&root)?.into_iter().filter(|a| spec_path(&root, &a.id).exists()).collect(),
    };
    let path_env = gate_path(
        std::env::var_os("HOME").map(PathBuf::from).as_deref(),
        &std::env::var("PATH").unwrap_or_default(),
        &|p| p.is_dir(),
    );
    let (mut passed, mut failed, mut pending, mut unproven) = (0, 0, 0, 0);
    let mut fatal = 0;
    for a in &adrs {
        let Some(spec) = load(&root, &a.id)? else {
            println!("  {} {} has no spec", "\u{2717}".red(), a.id);
            fatal += 1;
            continue;
        };
        if !print_coverage(a, &spec) {
            fatal += 1;
        }
        if let Err(why) = acceptance_holds(&spec)? {
            println!("  {} {} {}", "\u{2717}".red(), a.id, why);
            fatal += 1;
        }
        for f in spec.test_files() {
            let now = std::fs::read(root.join(&f)).map(|b| sha256(&b)).ok();
            match (spec.sealed.get(&f), now) {
                (None, _) => {
                    println!("  {} {} {} is not sealed (`hexa spec seal`)", "\u{2717}".red(), a.id, f);
                    fatal += 1;
                }
                (Some(_), None) => {
                    println!("  {} {} seal broken: {} is gone", "\u{2717}".red(), a.id, f);
                    fatal += 1;
                }
                (Some(s), Some(n)) if *s != n => {
                    println!(
                        "  {} {} seal broken: {} changed since sealing. A sealed test changes through the spec",
                        "\u{2717}".red(),
                        a.id,
                        f
                    );
                    fatal += 1;
                }
                _ => {}
            }
        }
        let completed = a.status == "completed";
        for s in &spec.specs {
            let Some(r) = s.test.as_deref() else {
                pending += 1;
                let mark = if completed && ci { "\u{2717}".red() } else { "\u{25cb}".dimmed() };
                println!("  {} {} {} pending: no test yet", mark, a.id, s.id);
                continue;
            };
            match run_one(&root, &path_env, r).await {
                Run::Passed => {
                    passed += 1;
                    println!("  {} {} {} — {}", "\u{2713}".green(), a.id, s.id, r.dimmed());
                }
                Run::Failed(tail) => {
                    failed += 1;
                    println!("  {} {} {} failed — {}", "\u{2717}".red(), a.id, s.id, r);
                    for line in tail.lines() {
                        println!("      {}", line.dimmed());
                    }
                }
                Run::NotFound(why) => {
                    failed += 1;
                    println!("  {} {} {} {}", "\u{2717}".red(), a.id, s.id, why);
                }
                Run::Vacuous => {
                    failed += 1;
                    println!("  {} {} {} ran 0 tests (vacuous): {}", "\u{2717}".red(), a.id, s.id, r);
                }
                Run::Ambiguous(n) => {
                    failed += 1;
                    println!("  {} {} {} matched {n} tests; a scenario names one: {}", "\u{2717}".red(), a.id, s.id, r);
                }
                Run::NotProvenHere(why) => {
                    unproven += 1;
                    let mark = if ci { "\u{2717}".red() } else { "?".yellow() };
                    println!("  {} {} {} not proven here: {}", mark, a.id, s.id, why);
                }
            }
        }
        if completed && ci && spec.specs.iter().any(|s| s.test.is_none()) {
            fatal += 1;
        }
    }
    println!("\n  {passed} passed · {failed} failed · {unproven} not proven here · {pending} pending");
    let unproven_fails = if ci { unproven } else { 0 };
    if failed + fatal + unproven_fails > 0 {
        anyhow::bail!("spec verification failed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_library_test_runs_by_its_module_path() {
        let k = Path::new("/w/hexa-cli");
        let a = cargo_args(k, &k.join("src/commands/adr/mod.rs"), "adr_gates::f").unwrap();
        assert!(a.ends_with(&["--lib".into(), "--".into(), "commands::adr::adr_gates::f".into(), "--exact".into()]), "{a:?}");
        let a = cargo_args(k, &k.join("tests/x.rs"), "f").unwrap();
        assert!(a.ends_with(&["--test".into(), "x".into(), "--".into(), "f".into(), "--exact".into()]), "{a:?}");
        assert!(cargo_args(k, &k.join("benches/b.rs"), "f").is_none());
    }

    #[test]
    fn an_id_is_the_timestamp_or_the_stem() {
        assert_eq!(adr_id_of("ADR-2610221540-a-slug"), "ADR-2610221540");
        assert_eq!(adr_id_of("ADR-001"), "ADR-001");
    }
}
