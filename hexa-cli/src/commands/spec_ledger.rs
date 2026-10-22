//! `hexa spec check | accept | seal | verify` — an ADR's decisions, held to
//! the spec that states them as behaviour (ADR-2610221540).
//!
//! The coverage rule is the domain's, reached through `hexa_core::ports::spec`.
//! This file reads the ledger, hashes, and runs each scenario's test alone.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use colored::Colorize;
use hexa_core::ports::spec::{coverage, numbered_decisions, Acceptance, Spec};

use hexa_exec::spec_author::{acceptance_holds, content_hash, run_test_alone, sha256_hex, TestRun};

use super::adr::gate_path;

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
    let sha256 = content_hash(&spec);
    println!("  {} {} accepted by {} at {}", "\u{2713}".green(), a.id, by, &sha256[..12]);
    spec.accepted = Some(Acceptance { by, sha256 });
    save(&root, &spec)
}

/// The model for a role: the one named, else the tier's. No model id is
/// written here (founding goal G1).
fn model_for(named: Option<&str>, tier: &str) -> anyhow::Result<String> {
    named
        .map(str::to_string)
        .or_else(|| hexa_infer::tier_model(tier))
        .ok_or_else(|| anyhow::anyhow!("no model: pass --model, or set inference.tier_models.{tier} in .hexa/project.json"))
}

/// `hexa spec draft <ADR>` (§2).
pub async fn draft(adr: &str, model: Option<&str>, force: bool) -> anyhow::Result<()> {
    let root = std::env::current_dir()?;
    let a = find(&root, adr)?;
    if let Some(existing) = load(&root, &a.id)? {
        if existing.accepted.is_some() {
            anyhow::bail!("{} has an accepted spec; a requirement changes through the ADR, then a new draft", a.id);
        }
        if !force {
            anyhow::bail!("{} already has a draft; --force replaces it", a.id);
        }
    }
    let model = model_for(model, "t2.5")?;
    println!("{} {} from the ADR and its ports, on {}", "\u{2b21} spec draft".cyan().bold(), a.id, model);
    let d = hexa_exec::draft_spec(&model, &root, &a.id).await.map_err(|e| anyhow::anyhow!(e))?;
    for g in &d.given {
        println!("  {} given {}", "\u{25b8}".cyan(), g);
    }
    if d.given.is_empty() {
        println!("  {} the ADR names no port or domain file in backquotes; the author saw the ADR alone", "!".yellow());
    }
    std::fs::create_dir_all(root.join(SPECS))?;
    save(&root, &d.spec)?;
    println!("  {} {} scenarios → {}/{}.json", "\u{2713}".green(), d.spec.specs.len(), SPECS, a.id);
    for f in &d.findings {
        println!("  {} {}", "\u{2717}".red(), f.message());
    }
    println!("\n  Read every scenario. It is what \"correct\" will mean. Then `hexa spec accept {}`.", a.id);
    Ok(())
}

/// `hexa spec tests <ADR>` (§5): generate, require red, commit, record, seal.
pub async fn tests(adr: &str, krate: &str, model: Option<&str>) -> anyhow::Result<()> {
    let root = std::env::current_dir()?;
    let a = find(&root, adr)?;
    let mut spec = load(&root, &a.id)?.ok_or_else(|| anyhow::anyhow!("{} has no spec", a.id))?;
    let model = model_for(model, "t2")?;
    let path_env = gate_path(
        std::env::var_os("HOME").map(PathBuf::from).as_deref(),
        &std::env::var("PATH").unwrap_or_default(),
        &|p| p.is_dir(),
    );
    println!("{} {} on {}", "\u{2b21} spec tests".cyan().bold(), a.id, model);
    let g = hexa_exec::generate_spec_tests(&model, &root, &spec, Path::new(krate), &path_env)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    for s in spec.specs.iter_mut() {
        s.test = g.refs.get(&s.id).cloned();
    }
    spec.red = Some(g.red.clone());
    // The generated file is new and red at its own commit: a new seal. Every
    // other sealed file must still match.
    spec.sealed.remove(&g.file);
    save(&root, &spec)?;
    println!("  {} {} tests in {}, every one red at {}", "\u{2713}".green(), g.refs.len(), g.file, g.red);
    seal(&a.id)?;
    println!("\n  Commit {}/{}.json. The tests are sealed: build the decision until they pass.", SPECS, a.id);
    Ok(())
}

/// `hexa spec seal <ADR>` — hash every test file the scenarios name (§6).
pub fn seal(adr: &str) -> anyhow::Result<()> {
    let root = std::env::current_dir()?;
    let a = find(&root, adr)?;
    let mut spec = load(&root, &a.id)?.ok_or_else(|| anyhow::anyhow!("{} has no spec to seal", a.id))?;
    if let Err(why) = acceptance_holds(&spec) {
        anyhow::bail!("{}: {why}. Tests are sealed against an accepted spec, never a draft", a.id);
    }
    // A seal freezes a file. Over a source file it would freeze the code the
    // test is meant to drive, so a sealed test lives in a `tests/` directory.
    let outside: Vec<String> =
        spec.test_files().into_iter().filter(|f| !Path::new(f).iter().any(|c| c == "tests")).collect();
    if !outside.is_empty() {
        anyhow::bail!(
            "{}: {} not under a tests/ directory; sealing them would freeze the implementation",
            a.id,
            outside.join(", ")
        );
    }
    let mut sealed = BTreeMap::new();
    for f in spec.test_files() {
        let bytes = std::fs::read(root.join(&f)).map_err(|e| anyhow::anyhow!("{f}: {e}"))?;
        let hash = sha256_hex(&bytes);
        // Re-sealing an edited test would launder the edit: a sealed test
        // changes through the spec, never by hand.
        if spec.sealed.get(&f).is_some_and(|s| *s != hash) {
            anyhow::bail!(
                "{}: seal broken: {f} changed since sealing. A sealed test changes through the spec, not by re-sealing",
                a.id
            );
        }
        println!("  {} sealed {}", "\u{2713}".green(), f);
        sealed.insert(f, hash);
    }
    spec.sealed = sealed;
    save(&root, &spec)
}

/// `hexa spec verify [ADR] [--ci]` — drift is a failure (§6, §7, §8).
pub async fn verify(adr: Option<&str>, ci: bool, prove: bool) -> anyhow::Result<()> {
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
        if let Err(why) = acceptance_holds(&spec) {
            println!("  {} {} {}", "\u{2717}".red(), a.id, why);
            fatal += 1;
        }
        for f in spec.test_files() {
            let now = std::fs::read(root.join(&f)).map(|b| sha256_hex(&b)).ok();
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
            match run_test_alone(&root, &path_env, r, None).await {
                TestRun::Passed => {
                    passed += 1;
                    println!("  {} {} {} — {}", "\u{2713}".green(), a.id, s.id, r.dimmed());
                }
                TestRun::Failed(tail) | TestRun::Broken { tail, .. } => {
                    failed += 1;
                    println!("  {} {} {} failed — {}", "\u{2717}".red(), a.id, s.id, r);
                    for line in tail.lines() {
                        println!("      {}", line.dimmed());
                    }
                }
                TestRun::NotFound(why) => {
                    failed += 1;
                    println!("  {} {} {} {}", "\u{2717}".red(), a.id, s.id, why);
                }
                TestRun::Vacuous => {
                    failed += 1;
                    println!("  {} {} {} ran 0 tests (vacuous): {}", "\u{2717}".red(), a.id, s.id, r);
                }
                TestRun::Ambiguous(n) => {
                    failed += 1;
                    println!("  {} {} {} matched {n} tests; a scenario names one: {}", "\u{2717}".red(), a.id, s.id, r);
                }
                TestRun::NotProvenHere(why) => {
                    unproven += 1;
                    let mark = if ci { "\u{2717}".red() } else { "?".yellow() };
                    println!("  {} {} {} not proven here: {}", mark, a.id, s.id, why);
                }
            }
        }
        if completed && ci && spec.specs.iter().any(|s| s.test.is_none()) {
            fatal += 1;
        }
        if prove {
            fatal += prove_red(&root, &path_env, a, &spec).await?;
        }
    }
    println!("\n  {passed} passed · {failed} failed · {unproven} not proven here · {pending} pending");
    let unproven_fails = if ci { unproven } else { 0 };
    if failed + fatal + unproven_fails > 0 {
        anyhow::bail!("spec verification failed");
    }
    Ok(())
}

/// `--prove` (§5, §8): each tested scenario's test must exist and fail at its
/// red commit. Returns how many did not. One worktree per commit;
/// builds share a target directory under `target/`, so a second commit
/// rebuilds what changed, not the world.
async fn prove_red(root: &Path, path_env: &str, a: &Adr, spec: &Spec) -> anyhow::Result<usize> {
    let mut by_commit: BTreeMap<String, Vec<(&str, &str)>> = BTreeMap::new();
    let mut unproven = 0;
    for s in &spec.specs {
        let Some(r) = s.test.as_deref() else { continue };
        match s.red_commit(spec) {
            Some(sha) => by_commit.entry(sha.to_string()).or_default().push((&s.id, r)),
            None => {
                unproven += 1;
                println!("  {} {} {} has no red commit: nothing shows it was written first", "\u{2717}".red(), a.id, s.id);
            }
        }
    }
    let target = root.join("target/hexa-prove");
    for (sha, scenarios) in by_commit {
        // Under the ignored target/, beside the shared build. A worktree left
        // by an interrupted run is removed first.
        let wt = target.join(format!("wt-{sha}"));
        let _ = std::process::Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(&wt)
            .current_dir(root)
            .output();
        let add = std::process::Command::new("git")
            .args(["worktree", "add", "-q", "--detach"])
            .arg(&wt)
            .arg(&sha)
            .current_dir(root)
            .output()?;
        if !add.status.success() {
            unproven += scenarios.len();
            println!(
                "  {} {} red commit {sha} cannot be checked out: {}",
                "\u{2717}".red(),
                a.id,
                String::from_utf8_lossy(&add.stderr).trim()
            );
            continue;
        }
        for (sid, r) in scenarios {
            let outcome = run_test_alone(&wt, path_env, r, Some(&target)).await;
            let (ok, what) = match outcome {
                TestRun::Failed(_) => (true, format!("red at {sha}")),
                // Written before the code it calls existed: red, and labelled.
                TestRun::Broken { compile: true, .. } => (true, format!("red at {sha} (did not build: written first)")),
                TestRun::Broken { compile: false, tail } => {
                    (false, format!("red not shown at {sha}: the test never ran: {}", tail.lines().last().unwrap_or("")))
                }
                TestRun::Passed | TestRun::Ambiguous(_) => {
                    (false, format!("mirror: passes at its red commit {sha}, so it was not written first"))
                }
                TestRun::NotFound(why) => (false, format!("not at {sha}: {why}")),
                TestRun::Vacuous => (false, format!("not at {sha}: it ran 0 tests there")),
                TestRun::NotProvenHere(why) => (false, format!("red not proven here at {sha}: {why}")),
            };
            if ok {
                println!("  {} {} {} {}", "\u{2713}".green(), a.id, sid, what);
            } else {
                unproven += 1;
                println!("  {} {} {} {}", "\u{2717}".red(), a.id, sid, what);
            }
        }
        let _ = std::process::Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(&wt)
            .current_dir(root)
            .output();
    }
    Ok(unproven)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_is_the_timestamp_or_the_stem() {
        assert_eq!(adr_id_of("ADR-2610221540-a-slug"), "ADR-2610221540");
        assert_eq!(adr_id_of("ADR-001"), "ADR-001");
    }
}
