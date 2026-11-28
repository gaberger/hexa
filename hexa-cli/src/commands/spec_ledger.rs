//! `hexa spec check | accept | seal | red | amend | verify` — an ADR's decisions, held to
//! the spec that states them as behaviour (ADR-2610221540).
//!
//! The coverage rule is the domain's, reached through `hexa_core::ports::spec`.
//! This file reads the ledger, hashes, and runs each scenario's test alone.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use colored::Colorize;
use serde_json::Value;
use hexa_core::ports::spec::{coverage, numbered_decisions, test_ref, Acceptance, Finding, Spec};

use hexa_core::ports::test_runner::{is_test_file, parse_test_ref};
use hexa_exec::spec_author::{acceptance_holds, content_hash, sha256_hex, TestRun};
use hexa_exec::test_exec::{env_prefix, full_commit, plan_test, remote_preflight, run_plans, test_config, Planned, TestConfig, Where};

use super::adr::gate_path;

/// ADRs accepted before this one carry no spec and are legacy (§9). From it
/// on, an accepted ADR without a spec fails `check`.
const SPEC_REQUIRED_FROM: u64 = 2610221540;

const ADRS: &str = "docs/adrs";
const SPECS: &str = "docs/specs";
/// Where else a ledger may keep its ADRs.
const ADR_DIRS: [&str; 2] = [ADRS, "docs/adr"];

/// `ADR-…` from a full id or bare digits.
fn normalise(id: &str) -> String {
    if id.starts_with("ADR-") { id.to_string() } else { format!("ADR-{id}") }
}

/// One ADR in the ledger.
#[derive(Clone)]
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
    let mut out: Vec<Adr> = Vec::new();
    let mut any = false;
    for dir in ADR_DIRS {
        let Ok(entries) = std::fs::read_dir(root.join(dir)) else { continue };
        any = true;
        out.extend(entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "md")).filter_map(|p| {
            let stem = p.file_stem()?.to_string_lossy().to_string();
            stem.starts_with("ADR-").then(|| {
                let status = status_of(&std::fs::read_to_string(&p).unwrap_or_default());
                Adr { id: adr_id_of(&stem), path: p, status }
            })
        }));
    }
    if !any {
        anyhow::bail!("no {ADRS}/: not found");
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out.dedup_by(|a, b| a.id == b.id);
    Ok(out)
}

fn find(root: &Path, id: &str) -> anyhow::Result<Adr> {
    let id = normalise(id);
    ledger(root)?.into_iter().find(|a| a.id == id).ok_or_else(|| anyhow::anyhow!("{id}: no such ADR in {ADRS}/"))
}

/// Where an ADR's spec is kept: `docs/specs/<ADR>.json`, else the first of the
/// other homes a ledger may use that exists.
/// Every place a spec may live, the preferred one first.
fn spec_homes(root: &Path, id: &str) -> [PathBuf; 4] {
    [
        root.join(SPECS).join(format!("{id}.json")),
        root.join(".hexa/specs").join(format!("{id}.json")),
        root.join("docs/adr").join(format!("{id}.spec.json")),
        root.join(ADRS).join(format!("{id}.spec.json")),
    ]
}

fn spec_path(root: &Path, id: &str) -> PathBuf {
    let homes = spec_homes(root, id);
    homes.iter().find(|p| p.exists()).cloned().unwrap_or_else(|| homes[0].clone())
}

fn load(root: &Path, id: &str) -> anyhow::Result<Option<Spec>> {
    Ok(load_text(root, id)?.map(|(_, spec)| spec))
}

/// A spec that keeps its scenarios under `scenarios` rather than `specs`. It
/// is read as a spec, and written back in its own shape.
fn is_alt(text: &str) -> bool {
    serde_json::from_str::<Value>(text).ok().is_some_and(|v| v.get("specs").is_none() && v.get("scenarios").is_some())
}

fn read_spec(text: &str) -> serde_json::Result<Spec> {
    let mut v: Value = serde_json::from_str(text)?;
    if is_alt(text) {
        if let Some(o) = v.as_object_mut() {
            if let Some(sc) = o.remove("scenarios") {
                o.insert("specs".into(), sc);
            }
            if !o.contains_key("feature") {
                let adr = o.get("adr").cloned().unwrap_or_else(|| Value::String(String::new()));
                o.insert("feature".into(), adr);
            }
        }
    }
    serde_json::from_value(v)
}

/// The amendments the spec file carries, oldest first.
fn amendments_of(text: &str) -> Vec<Value> {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|v| v.get("amendments").and_then(Value::as_array).cloned())
        .unwrap_or_default()
}

/// The spec and the exact text it was read from.
fn load_text(root: &Path, id: &str) -> anyhow::Result<Option<(String, Spec)>> {
    let p = spec_path(root, id);
    if !p.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&p)?;
    let spec = read_spec(&text).map_err(|e| anyhow::anyhow!("{}: not a spec: {e}", p.display()))?;
    Ok(Some((text, spec)))
}

/// Save `spec` only if the file still holds `read`, the text it was loaded
/// from. A write after a long wait would otherwise put back an old
/// acceptance over a person's edit.
fn save_unchanged(root: &Path, spec: &Spec, read: &str) -> anyhow::Result<()> {
    let p = spec_path(root, &spec.adr);
    if std::fs::read_to_string(&p).ok().as_deref() != Some(read) {
        anyhow::bail!(
            "{} changed while the tests were generated; it was not overwritten. Re-run against the current spec",
            p.display()
        );
    }
    save(root, spec)
}

/// The spec as written: its fields, then the amendments, which are not the
/// spec's behaviour and are kept apart from `Spec`.
#[derive(serde::Serialize)]
struct WithAmendments<'a> {
    #[serde(flatten)]
    spec: &'a Spec,
    #[serde(skip_serializing_if = "<[Value]>::is_empty")]
    amendments: &'a [Value],
    #[serde(skip_serializing_if = "<[Value]>::is_empty")]
    moves: &'a [Value],
}

fn save(root: &Path, spec: &Spec) -> anyhow::Result<()> {
    write_spec(root, spec, None)
}

/// Write `spec`. The amendments on disk are kept unless `amendments` replaces
/// them: a list that is append-only survives every other write.
fn write_spec(root: &Path, spec: &Spec, amendments: Option<&[Value]>) -> anyhow::Result<()> {
    write_records(root, spec, amendments, &[])
}

/// `write_spec`, appending `new_moves` to the moves on disk, which every write
/// keeps (ADR-2611251652 §2).
fn write_records(root: &Path, spec: &Spec, amendments: Option<&[Value]>, new_moves: &[Value]) -> anyhow::Result<()> {
    let p = spec_path(root, &spec.adr);
    let old: Option<Value> = std::fs::read_to_string(&p).ok().and_then(|t| serde_json::from_str(&t).ok());
    let kept: Vec<Value> = match amendments {
        Some(a) => a.to_vec(),
        None => old.as_ref().and_then(|o| o.get("amendments")).and_then(Value::as_array).cloned().unwrap_or_default(),
    };
    let mut moves: Vec<Value> = old.as_ref().and_then(|o| o.get("moves")).and_then(Value::as_array).cloned().unwrap_or_default();
    moves.extend_from_slice(new_moves);
    let text = match old {
        Some(Value::Object(mut file)) if !file.contains_key("specs") && file.contains_key("scenarios") => {
            let Value::Object(new) = serde_json::to_value(spec)? else { anyhow::bail!("a spec is an object") };
            for k in ["accepted", "sealed", "red"] {
                match new.get(k) {
                    Some(v) => {
                        file.insert(k.into(), v.clone());
                    }
                    None => {
                        file.remove(k);
                    }
                }
            }
            if let (Some(Value::Array(have)), Some(Value::Array(now))) = (file.get_mut("scenarios"), new.get("specs")) {
                for s in have.iter_mut().filter_map(Value::as_object_mut) {
                    let id = s.get("id").and_then(Value::as_str).unwrap_or("").to_string();
                    let Some(n) = now.iter().find(|n| n["id"] == id) else { continue };
                    for k in ["test", "red"] {
                        match n.get(k) {
                            Some(v) => {
                                s.insert(k.into(), v.clone());
                            }
                            None => {
                                s.remove(k);
                            }
                        }
                    }
                }
            }
            if kept.is_empty() {
                file.remove("amendments");
            } else {
                file.insert("amendments".into(), Value::Array(kept));
            }
            if !moves.is_empty() {
                file.insert("moves".into(), Value::Array(moves));
            }
            serde_json::to_string_pretty(&Value::Object(file))?
        }
        _ => serde_json::to_string_pretty(&WithAmendments { spec, amendments: &kept, moves: &moves })?,
    };
    std::fs::write(&p, text + "\n")?;
    Ok(())
}

/// Coverage findings for one ADR and its spec, printed. True when covered.
fn print_coverage(adr: &Adr, spec: &Spec, alt: bool) -> bool {
    let text = std::fs::read_to_string(&adr.path).unwrap_or_default();
    let mut findings = coverage(&adr.id, &numbered_decisions(&text), spec);
    // A spec in the `scenarios` shape states examples; it is not asked for negatives.
    findings.retain(|f| !(alt && matches!(f, Finding::NoNegative(_))));
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
        let loaded = load_text(&root, &a.id)?;
        match loaded.map(|(t, s)| (is_alt(&t), s)) {
            Some((alt, spec)) if print_coverage(a, &spec, alt) => {
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

/// A spec is drafted and accepted only where a test could be bound to it:
/// a Cargo.toml (generated Rust tests), a package.json (vitest), a Python
/// manifest (the built-in pytest runner, ADR-2611221935 §1), or a configured
/// runner (ADR-2611191120 §9). Said before a paid `spec draft`.
fn require_a_rust_project(root: &Path, id: &str, verb: &str) -> anyhow::Result<()> {
    if root.join("Cargo.toml").is_file() || root.join("package.json").is_file() {
        return Ok(());
    }
    if ["pyproject.toml", "setup.py", "setup.cfg"].iter().any(|m| root.join(m).is_file()) {
        return Ok(());
    }
    if test_config(root).is_ok_and(|c| !c.runners.is_empty()) {
        return Ok(());
    }
    anyhow::bail!(
        "{id}: {verb} refused: no Cargo.toml, package.json, Python manifest (pyproject.toml, setup.py or setup.cfg) or test.runners at {}. \
         `hexa spec tests` generates Rust tests only; \
         a test in another language is written by hand and bound with `hexa spec red <ADR> <commit> --test <S>=<file>::<name>`, \
         which needs a runner for its file: vitest under a package.json, pytest for .py, bash for .sh, or one in test.runners in .hexa/project.json",
        root.display()
    )
}

/// `hexa spec accept <ADR>` — a person approves the behaviour (§4).
pub fn accept(adr: &str, by: Option<&str>) -> anyhow::Result<()> {
    let root = std::env::current_dir()?;
    let a = find(&root, adr)?;
    let (text, mut spec) = load_text(&root, &a.id)?.ok_or_else(|| anyhow::anyhow!("{} has no spec to accept", a.id))?;
    // A spec accepted here could never be sealed (ADR-2611161600 §2).
    require_a_rust_project(&root, &a.id, "`hexa spec accept`")?;
    require_committed(&root, &a.id)?;

    if !print_coverage(&a, &spec, is_alt(&text)) {
        anyhow::bail!("{}: the spec does not cover the decision; accept a complete one", a.id);
    }
    let by = acceptor(
        by,
        || {
            let out = std::process::Command::new("git").args(["config", "user.name"]).output().ok()?;
            Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
        },
        || std::env::var("USER").ok(),
    )?;
    let previous = last_accepted(&root, &a.id);
    if record_acceptance(&mut spec, by.clone(), previous.as_ref()) {
        let unbound: Vec<&str> = spec.specs.iter().filter(|s| s.test.is_none()).map(|s| s.id.as_str()).collect();
        if previous.is_some() && !unbound.is_empty() {
            println!(
                "  {} {} changed where {} changed: those lose their tests, and the files that held them their seals; \
                 every other scenario keeps its proof. Run `hexa spec tests {} --only {}`, or commit each amended test \
                 failing and run `hexa spec red {} <commit> --test <scenario>=<file>.rs::<fn>`",
                "!".yellow(),
                a.id,
                unbound.join(", "),
                a.id,
                unbound.join(","),
                a.id
            );
        } else {
            println!(
                "  {} {} the accepted behaviour changed: its tests, red commit and seal are dropped. Run `hexa spec tests {}`",
                "!".yellow(),
                a.id,
                a.id
            );
        }
    }
    let sha256 = content_hash(&spec);
    println!("  {} {} accepted by {} at {}", "\u{2713}".green(), a.id, by, &sha256[..12]);
    save(&root, &spec)
}

/// Record `by`'s acceptance of `spec` as it stands. Tests, red commits and
/// seals bind the behaviour accepted before them: when this acceptance
/// replaces one of different behaviour, they are dropped, so the new
/// acceptance needs new tests and a new seal. True when any were dropped.
fn record_acceptance(spec: &mut Spec, by: String, previous: Option<&Spec>) -> bool {
    let sha256 = content_hash(spec);
    let changed = spec.accepted.as_ref().is_some_and(|a| a.sha256 != sha256);
    let bound = spec.red.is_some() || !spec.sealed.is_empty() || spec.specs.iter().any(|s| s.test.is_some() || s.red.is_some());
    // Only what changed loses its proof (ADR-2610281628 §1): each scenario
    // identical to its accepted predecessor keeps its test and red, and a
    // seal goes only from a file that held a changed or removed one's test.
    if let (true, Some(prev)) = (changed, previous) {
        let kept = hexa_core::ports::spec::rebind(prev, spec);
        let lost: std::collections::BTreeSet<String> = prev
            .specs
            .iter()
            .filter_map(|p| p.test.as_deref())
            .filter(|t| !kept.specs.iter().any(|k| k.test.as_deref() == Some(*t)))
            .filter_map(|t| t.split("::").next().map(str::to_string))
            .collect();
        let mut sealed = prev.sealed.clone();
        sealed.retain(|f, _| !lost.contains(f));
        spec.specs = kept.specs;
        spec.sealed = sealed;
        spec.red = prev.red.clone();
        spec.accepted = Some(Acceptance { by, sha256 });
        return !lost.is_empty();
    }
    if changed {
        for s in spec.specs.iter_mut() {
            s.test = None;
            s.red = None;
        }
        spec.red = None;
        spec.sealed.clear();
    }
    spec.accepted = Some(Acceptance { by, sha256 });
    changed && bound
}

/// The spec file's path from the repository root, as git names it.
fn spec_rel(root: &Path, id: &str) -> String {
    let p = spec_path(root, id);
    p.strip_prefix(root).unwrap_or(&p).to_string_lossy().replace('\\', "/")
}

/// The spec at `rev`, if that commit holds it and it parses.
fn spec_at(root: &Path, rev: &str, rel: &str) -> Option<Spec> {
    let out = std::process::Command::new("git").args(["show", &format!("{rev}:{rel}")]).current_dir(root).output().ok()?;
    serde_json::from_slice(&out.stdout).ok().filter(|_| out.status.success())
}

/// The commits that changed the spec file, newest first, at most `limit`.
fn spec_revisions(root: &Path, rel: &str, limit: usize) -> Vec<String> {
    std::process::Command::new("git")
        .args(["log", "--format=%H", &format!("-{limit}"), "--", rel])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().map(str::to_string).collect())
        .unwrap_or_default()
}

/// HEAD holds the spec and a person's acceptance of it as it stands there.
fn accepted_at_head(root: &Path, id: &str) -> bool {
    spec_at(root, "HEAD", &spec_rel(root, id)).is_some_and(|s| acceptance_holds(&s).is_ok())
}

/// The spec as last accepted in history: the newest committed version whose
/// acceptance still matches its content. What a re-acceptance compares
/// against (ADR-2610281628 §1). Not only HEAD's: an amendment commits the
/// edited scenarios before a person re-accepts them (ADR-2611161600 §1),
/// and that commit holds no acceptance that matches.
fn last_accepted(root: &Path, id: &str) -> Option<Spec> {
    let rel = spec_rel(root, id);
    spec_revisions(root, &rel, 200).iter().find_map(|rev| spec_at(root, rev, &rel).filter(|s| acceptance_holds(s).is_ok()))
}

/// An acceptance binds text a commit holds (ADR-2611161600 §1). A spec that
/// is untracked, or modified while HEAD holds no acceptance of it, is refused.
/// A re-acceptance of an edited spec is not: its predecessor is in HEAD and
/// only the changed scenarios lose their proof.
fn require_committed(root: &Path, id: &str) -> anyhow::Result<()> {
    let rel = spec_rel(root, id);
    let git = |args: &[&str]| std::process::Command::new("git").args(args).current_dir(root).output();
    let tracked = git(&["ls-files", "--error-unmatch", "--", &rel])?;
    if !tracked.status.success() {
        anyhow::bail!(
            "{id}: accept refused: {rel} is untracked, so no commit holds the text being accepted. \
             `git add {rel}` and `git commit` it, then accept"
        );
    }
    let same = git(&["diff", "--quiet", "HEAD", "--", &rel])?;
    if !same.status.success() && !accepted_at_head(root, id) {
        anyhow::bail!(
            "{id}: accept refused: {rel} is modified and HEAD holds no acceptance of it, so no commit holds the text \
             being accepted. `git add {rel}` and `git commit` it, then accept"
        );
    }
    Ok(())
}

/// Who accepts: `--by` if given (a blank one is refused, not replaced), else
/// git `user.name`, else `$USER` — each fallback only when it is not blank.
fn acceptor(
    by: Option<&str>,
    git_name: impl FnOnce() -> Option<String>,
    user: impl FnOnce() -> Option<String>,
) -> anyhow::Result<String> {
    let named = |s: &String| !s.trim().is_empty();
    match by {
        Some(b) if b.trim().is_empty() => anyhow::bail!("--by is blank: name who is accepting"),
        Some(b) => Ok(b.to_string()),
        None => git_name()
            .filter(named)
            .or_else(|| user().filter(named))
            .ok_or_else(|| anyhow::anyhow!("who is accepting? pass --by")),
    }
}

/// The model for a role: the one named, else the tier's. No model id is
/// written here (founding goal G1).
fn model_for(named: Option<&str>, tier: &str) -> anyhow::Result<String> {
    // No tier: the tool-less route, which can read nothing but its prompt.
    Ok(named.map(str::to_string).or_else(|| hexa_infer::tier_model(tier)).unwrap_or_else(hexa_infer::tool_less_default))
}

/// Stop before the first inference call when no completion model is
/// reachable: the frontier CLI does not count, the spec author and the test
/// writer refuse it (ADR-2610221540 §2). The dead end prints its guide
/// (ADR-2610301500 §2); the command still fails.
fn require_a_completion_path(root: &Path, verb: &str, model: &str) -> anyhow::Result<()> {
    use hexa_infer::ports::Discovery;
    let found = hexa_infer::discovery().discover();
    if completion_path_open(&found, model) {
        return Ok(());
    }
    if let Some(g) = hexa_core::ports::guide::find("no-completion-model") {
        super::gap::hit(&g, root);
    }
    anyhow::bail!(
        "no completion model is reachable, so {verb} cannot run. Open paths: {}. `hexa doctor` shows what was probed",
        hexa_infer::reach::path_words(&found)
    )
}

/// Whether `model` can be answered as a completion over `found`. Any open
/// non-frontier path is one. The frontier CLI is one only for `claude-text`,
/// which runs it tool-less (ADR-2610270901); for an agent model it is the
/// author that reads the code (ADR-2610221540 §2).
fn completion_path_open(found: &[hexa_infer::ports::Found], model: &str) -> bool {
    let text = hexa_infer::wiring::claude_text_model(model).is_some();
    found.iter().any(|f| f.open() && (f.kind != "frontier" || text))
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
    require_a_completion_path(&root, "`hexa spec draft`", &model)?;
    // Free checks first, then this one, then the paid call: a spec drafted
    // here could never be sealed.
    require_a_rust_project(&root, &a.id, "`hexa spec draft`")?;
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
    // `spec accept` refuses an uncommitted first acceptance (ADR-2611161600
    // §1), so the instruction says to commit first (ADR-2611230831 §2).
    println!(
        "\n  Read every scenario in {}/{}.json. It is what \"correct\" will mean. Commit it, then `hexa spec accept {}`.",
        SPECS, a.id, a.id
    );
    Ok(())
}

/// `hexa spec tests <ADR>` (§5): generate, require red, commit, record, seal.
pub async fn tests(
    adr: &str,
    krate: &str,
    model: Option<&str>,
    red_at: Option<String>,
    replace: bool,
    because: Option<String>,
) -> anyhow::Result<()> {
    let root = std::env::current_dir()?;
    let a = find(&root, adr)?;
    let (read, mut spec) = load_text(&root, &a.id)?.ok_or_else(|| anyhow::anyhow!("{} has no spec", a.id))?;
    // A sealed file is replaced only through the verb, and only for a stated
    // reason (ADR-2610281442 §4).
    let replace = if replace {
        let because = because
            .filter(|b| !b.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("--replace needs --because \"<reason>\": a sealed test is replaced only for a stated reason"))?;
        let old_file = spec
            .sealed
            .keys()
            .next()
            .cloned()
            .or_else(|| spec.test_files().into_iter().next())
            .ok_or_else(|| anyhow::anyhow!("{} has no sealed test file to replace", a.id))?;
        Some(hexa_exec::spec_author::Replace { old_file, because })
    } else {
        if because.is_some() {
            anyhow::bail!("--because goes with --replace");
        }
        None
    };
    // What each scenario is anchored to, before this generation moves any. A
    // binding that already fails is refused before a model is paid for.
    let all: Vec<String> = spec.specs.iter().map(|s| s.id.clone()).collect();
    let anchored = match &replace {
        Some(_) => bindings_before(&root, &a.id, &spec, &all)?,
        None => BTreeMap::new(),
    };
    let options = hexa_exec::spec_author::TestOptions { red_at, replace: replace.clone(), only: None, because: None };
    let model = model_for(model, "t2")?;
    require_a_completion_path(&root, "`hexa spec tests`", &model)?;
    let path_env = gate_path(
        std::env::var_os("HOME").map(PathBuf::from).as_deref(),
        &std::env::var("PATH").unwrap_or_default(),
        &|p| p.is_dir(),
    );
    println!("{} {} on {}", "\u{2b21} spec tests".cyan().bold(), a.id, model);
    let g = match hexa_exec::generate_spec_tests(&model, &root, &spec, Path::new(krate), &path_env, &options).await {
        Ok(g) => g,
        Err(e) => {
            // A generation refused for tests that already pass is the guard
            // gap: say so and count it.
            if e.contains("passes before the code exists") {
                if let Some(g) = hexa_core::ports::guide::find("guard-test-is-a-mirror") {
                    super::gap::hit(&g, &root);
                }
            }
            anyhow::bail!(e);
        }
    };
    report_guards(&root, &g.guards);
    let replaced_hash = replace.as_ref().and_then(|r| spec.sealed.get(&r.old_file).cloned());
    if let Some(r) = &replace {
        spec.sealed.remove(&r.old_file);
    }
    record_generated(&mut spec, &g.refs, &g.red, &g.file);
    let made = match &replace {
        Some(r) => moves_made(&root, &spec, &anchored, &r.because)?,
        None => Vec::new(),
    };
    save_unchanged(&root, &spec, &read)?;
    if !made.is_empty() {
        write_records(&root, &spec, None, &made)?;
    }
    println!("  {} {} tests in {}, every one red at {}", "\u{2713}".green(), g.refs.len(), g.file, g.red);
    seal(&a.id)?;
    // A replacement at the same path moves the seal, so it is recorded as an
    // amendment: every move of a seal is in the chain (ADR-2611081151 §1).
    if let (Some(r), Some(from)) = (&replace, replaced_hash) {
        if r.old_file == g.file {
            record_replacement(&root, &a.id, &r.old_file, &from, &r.because)?;
        }
    }
    println!("\n  Commit {}/{}.json. The tests are sealed: build the decision until they pass.", SPECS, a.id);
    Ok(())
}

/// Scenarios whose test passed before the code: their tests were left out
/// and the scenarios stay pending. Said, and counted as the gap it is.
fn report_guards(root: &Path, guards: &[String]) {
    if guards.is_empty() {
        return;
    }
    println!("  {} {} scenario(s) already hold, so their tests were left out and they stay pending:", "!".yellow(), guards.len());
    for g in guards {
        println!("      {g}");
    }
    if let Some(g) = hexa_core::ports::guide::find("guard-test-is-a-mirror") {
        super::gap::hit(&g, root);
    }
}

/// Append the amendment a `--replace` at the same path makes: the seal moved
/// from `from` to the file's new hash, for `because` (ADR-2611081151 §1).
fn record_replacement(root: &Path, id: &str, file: &str, from: &str, because: &str) -> anyhow::Result<()> {
    let (text, mut spec) = load_text(root, id)?.ok_or_else(|| anyhow::anyhow!("{id} has no spec"))?;
    let Some(to) = spec.sealed.get(file).cloned() else { return Ok(()) };
    if to == from {
        return Ok(());
    }
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let mut amended = amendments_of(&text);
    amended.push(serde_json::json!({
        "file": file,
        "from": from,
        "to": to,
        "by": git(&["config", "user.name"]).or_else(|| std::env::var("USER").ok()).unwrap_or_else(|| "hexa spec tests".to_string()),
        "reason": format!("replaced by `hexa spec tests --replace`: {because}"),
        "at": chrono::Utc::now().to_rfc3339(),
        "base_commit": git(&["rev-parse", "HEAD"]).unwrap_or_default(),
    }));
    spec.sealed.insert(file.to_string(), to);
    write_spec(root, &spec, Some(&amended))
}

/// The `#[test]` function a `/// <id>:` doc line names in `text`, if any.
fn documented_test(text: &str, id: &str) -> Option<String> {
    let mut pending = false;
    for line in text.lines() {
        let t = line.trim();
        if let Some(doc) = t.strip_prefix("///") {
            let first = doc.split_whitespace().next().unwrap_or("").trim_end_matches(':');
            pending |= first == id;
            continue;
        }
        if !pending || t.is_empty() || t.starts_with("#[") {
            continue;
        }
        let words: Vec<&str> = t.split_whitespace().collect();
        let name = words.iter().position(|w| *w == "fn").and_then(|i| words.get(i + 1));
        return name.map(|n| n.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect());
    }
    None
}

/// `hexa spec tests <ADR> --only <ids>` (ADR-2610281628 §2): those scenarios
/// alone, into a new file, proved red; their old tests removed from the
/// files that held them; everything rebound, sealed and committed at once.
pub async fn reprove(
    adr: &str,
    krate: &str,
    model: Option<&str>,
    red_at: Option<String>,
    only: &[String],
    because: Option<String>,
) -> anyhow::Result<()> {
    let root = std::env::current_dir()?;
    let a = find(&root, adr)?;
    let (_, mut spec) = load_text(&root, &a.id)?.ok_or_else(|| anyhow::anyhow!("{} has no spec", a.id))?;
    let unknown: Vec<&String> = only.iter().filter(|id| !spec.specs.iter().any(|s| &s.id == *id)).collect();
    if !unknown.is_empty() {
        anyhow::bail!("{}: no scenario {}", a.id, unknown.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "));
    }
    let bound: Vec<&str> = spec
        .specs
        .iter()
        .filter(|s| only.contains(&s.id) && s.test.is_some())
        .map(|s| s.id.as_str())
        .collect();
    let because = because.filter(|b| !b.trim().is_empty());
    if !bound.is_empty() && because.is_none() {
        anyhow::bail!(
            "{}: {} still {} a sealed test; re-proving replaces it, so give --because \"<reason>\"",
            a.id,
            bound.join(", "),
            if bound.len() == 1 { "has" } else { "have" }
        );
    }
    let anchored = bindings_before(&root, &a.id, &spec, only)?;
    if !anchored.is_empty() && because.is_none() {
        let ids: Vec<&str> = anchored.keys().map(String::as_str).collect();
        anyhow::bail!("{}: re-proving moves the binding of {}; give --because \"<reason>\", which its move records", a.id, ids.join(", "));
    }
    let model = model_for(model, "t2")?;
    let path_env = gate_path(
        std::env::var_os("HOME").map(PathBuf::from).as_deref(),
        &std::env::var("PATH").unwrap_or_default(),
        &|p| p.is_dir(),
    );
    println!("{} {} {} on {}", "\u{2b21} spec tests".cyan().bold(), a.id, only.join(", "), model);
    let options =
        hexa_exec::spec_author::TestOptions { red_at, replace: None, only: Some(only.to_vec()), because: because.clone() };
    let g = match hexa_exec::generate_spec_tests(&model, &root, &spec, Path::new(krate), &path_env, &options).await {
        Ok(g) => g,
        Err(e) => {
            // A generation refused for tests that already pass is the guard
            // gap: say so and count it.
            if e.contains("passes before the code exists") {
                if let Some(g) = hexa_core::ports::guide::find("guard-test-is-a-mirror") {
                    super::gap::hit(&g, &root);
                }
            }
            anyhow::bail!(e);
        }
    };
    report_guards(&root, &g.guards);

    // What each file was sealed at, before the old tests are cut from it.
    let sealed_before = spec.sealed.clone();
    // The superseded tests leave the files that held them.
    let mut by_file: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for s in spec.specs.iter().filter(|s| only.contains(&s.id)) {
        if let Some((file, f)) = s.test.as_deref().and_then(hexa_core::ports::spec::test_ref) {
            by_file.entry(file.to_string()).or_default().push(f.rsplit("::").next().unwrap_or(f).to_string());
        }
    }
    // A scenario re-acceptance left unbound still has its old test in one of
    // the spec's files, documented with its id: that goes too.
    for id in only.iter().filter(|id| spec.specs.iter().any(|s| &s.id == *id && s.test.is_none())) {
        for file in spec.test_files() {
            let text = std::fs::read_to_string(root.join(&file)).unwrap_or_default();
            if let Some(f) = documented_test(&text, id) {
                by_file.entry(file.clone()).or_default().push(f);
            }
        }
    }
    for (file, names) in &by_file {
        let text = std::fs::read_to_string(root.join(file)).map_err(|e| anyhow::anyhow!("{file}: {e}"))?;
        let left = hexa_core::ports::spec::without_tests(&text, names);
        // A file left with no test is gone, not kept as a husk.
        if !hexa_core::ports::spec::test_fns(&left).is_empty() {
            std::fs::write(root.join(file), left)?;
        } else {
            std::fs::remove_file(root.join(file))?;
        }
        spec.sealed.remove(file);
    }
    for s in spec.specs.iter_mut().filter(|s| only.contains(&s.id)) {
        s.test = g.refs.get(&s.id).cloned();
        s.red = Some(g.red.clone());
    }
    spec.sealed.remove(&g.file);
    spec.sealed = seal_files_of(&root, &a.id, &spec, spec.test_files())?;
    let reason = because.unwrap_or_else(|| "left without a test when the spec was re-accepted".to_string());
    let made = moves_made(&root, &spec, &anchored, &reason)?;
    // A file the old tests were cut from moves its seal, so that is amended:
    // every move of a seal is in the chain (ADR-2611081151 §1).
    let mut amended = amendments_of(&std::fs::read_to_string(spec_path(&root, &a.id))?);
    let by = git_user_or_env(None)?;
    for file in by_file.keys() {
        // The hash the file's seal had. A re-acceptance drops the seal of a file
        // that held a changed scenario's test, so there may be none recorded; the
        // move still happened, and it chains from where the file's amendments
        // end, or from its anchor in history when it has none. Skipping it left
        // the chain broken for good ("does not continue the chain").
        let from = sealed_before.get(file).cloned().or_else(|| chain_end(&amended, file)).or_else(|| seal_anchor(&root, file));
        let (Some(from), Some(to)) = (from, spec.sealed.get(file)) else { continue };
        if from != *to {
            amended.push(serde_json::json!({
                "file": file,
                "from": from,
                "to": to,
                "by": by,
                "reason": format!("tests cut by `hexa spec tests --only`: {reason}"),
                "at": chrono::Utc::now().to_rfc3339(),
                "base_commit": git_head(&root),
            }));
        }
    }
    write_records(&root, &spec, Some(&amended), &made)?;

    let old: Vec<&str> = by_file.keys().map(String::as_str).collect();
    let msg = format!(
        "test(spec): {} re-proves {} in {}\n\n{}{}\n\nEvery new test fails at {} (ADR-2610281628 §2).",
        a.id,
        only.join(", "),
        g.file,
        if old.is_empty() { String::new() } else { format!("Removed from {}: ", old.join(", ")) },
        reason,
        g.red
    );
    let spec_file = format!("{SPECS}/{}.json", a.id);
    let mut paths: Vec<&str> = vec![g.file.as_str(), spec_file.as_str()];
    paths.extend(old.iter().copied());
    let run = |args: &[&str]| -> anyhow::Result<()> {
        let o = std::process::Command::new("git").args(args).current_dir(&root).output()?;
        if !o.status.success() {
            anyhow::bail!("git {}: {}", args.join(" "), String::from_utf8_lossy(&o.stderr).trim());
        }
        Ok(())
    };
    let mut add = vec!["add", "--"];
    add.extend(paths.iter().copied());
    run(&add)?;
    let mut commit = vec!["-c", "user.name=hexa-spec", "-c", "user.email=spec@hexa.local", "commit", "-q", "-m", &msg, "--"];
    commit.extend(paths.iter().copied());
    run(&commit)?;
    println!("  {} {} re-proved in {}, red at {}", "\u{2713}".green(), only.join(", "), g.file, g.red);
    Ok(())
}

/// Point every scenario at its new test, red at the new commit. A scenario's
/// own `red` overrides the spec's, so an old one is cleared: it names a commit
/// at which the new test does not exist.
fn record_generated(spec: &mut Spec, refs: &BTreeMap<String, String>, red: &str, file: &str) {
    for s in spec.specs.iter_mut() {
        s.test = refs.get(&s.id).cloned();
        s.red = None;
    }
    spec.red = Some(red.to_string());
    // The generated file is new and red at its own commit: a new seal. Every
    // other sealed file must still match.
    spec.sealed.remove(file);
}

/// `hexa spec seal <ADR>` — hash every test file the scenarios name (§6).
/// `hexa spec seal`: seal, then name any sealed file no commit holds. Such a
/// seal is made (the hash is recorded) but history cannot anchor it, so `spec
/// verify --ci` fails it as unanchored until the file is committed.
pub fn seal_verb(adr: &str) -> anyhow::Result<()> {
    seal(adr)?;
    let root = std::env::current_dir()?;
    let a = find(&root, adr)?;
    if let Some((_, spec)) = load_text(&root, &a.id)? {
        for f in spec.sealed.keys().filter(|f| seal_anchor(&root, f).is_none()) {
            println!(
                "  {} {f} is not committed: `hexa spec verify --ci` reports this seal unanchored until you commit it",
                "!".yellow()
            );
        }
        // The chain starts at the blob of the commit that first added the file. A
        // test edited after that commit and sealed at the edited text has a step
        // no amendment records, and `verify` says so only later. Ask the same
        // judge `verify` uses, so the two cannot disagree.
        use hexa_core::ports::spec::{judge_seal, SealVerdict};
        let raw: Value = std::fs::read_to_string(spec_path(&root, &a.id)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
        let amendments = amendment_chain(&raw);
        for (f, recorded) in &spec.sealed {
            let Some(anchor) = seal_anchor(&root, f) else { continue };
            let current = std::fs::read(root.join(f)).ok().map(|b| sha256_hex(&b));
            if matches!(judge_seal(f, Some(&anchor), &amendments, Some(recorded), current.as_deref()), SealVerdict::Rewritten | SealVerdict::BrokenChain(_)) {
                println!(
                    "  {} {f} was edited since it was first committed (at {}…), and no amendment records that: `hexa spec verify` will report this seal's chain broken. A test generated by `hexa spec tests` is sealed as first committed; commit a hand-written one at its final text in one commit before sealing",
                    "!".yellow(),
                    &anchor[..anchor.len().min(12)]
                );
            }
        }
    }
    Ok(())
}

pub fn seal(adr: &str) -> anyhow::Result<()> {
    let root = std::env::current_dir()?;
    let a = find(&root, adr)?;
    let (text, mut spec) = load_text(&root, &a.id)?.ok_or_else(|| anyhow::anyhow!("{} has no spec to seal", a.id))?;
    if let Err(why) = acceptance_holds(&spec) {
        anyhow::bail!("{}: {why}. Tests are sealed against an accepted spec, never a draft", a.id);
    }
    spec.sealed = seal_files_of(&root, &a.id, &spec, test_files(&root, &spec, &text))?;
    save(&root, &spec)
}

/// The test files the scenarios name. In the `scenarios` shape a test may be a
/// bare function name: its file is the scenario's `file`, else the one file
/// under a `tests/` directory that declares it.
fn test_files(root: &Path, spec: &Spec, text: &str) -> std::collections::BTreeSet<String> {
    let mut out = spec.test_files();
    if !is_alt(text) {
        return out;
    }
    let raw: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    for (s, r) in spec.specs.iter().zip(raw["scenarios"].as_array().into_iter().flatten()) {
        let Some(t) = s.test.as_deref().filter(|t| parse_test_ref(t).is_none()) else { continue };
        match r["file"].as_str() {
            Some(f) => {
                out.insert(f.to_string());
            }
            None => out.extend(find_test_file(root, root, t)),
        }
    }
    out
}

/// The first `.rs` file under a `tests/` directory in `dir` that declares `fn name(`.
fn find_test_file(root: &Path, dir: &Path, name: &str) -> Option<String> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        let n = p.file_name()?.to_string_lossy().to_string();
        if p.is_dir() {
            if matches!(n.as_str(), ".git" | "target" | "node_modules") {
                continue;
            }
            if let Some(f) = find_test_file(root, &p, name) {
                return Some(f);
            }
        } else if p.extension().is_some_and(|x| x == "rs") {
            let rel = p.strip_prefix(root).ok()?;
            if rel.iter().any(|c| c == "tests")
                && std::fs::read_to_string(&p).is_ok_and(|t| t.contains(&format!("fn {name}(")))
            {
                return Some(rel.to_string_lossy().to_string());
            }
        }
    }
    None
}

/// Hash every test file `spec`'s scenarios name. A spec naming none would
/// seal nothing and still report success, so it is refused.
#[cfg(test)]
fn seal_files(root: &Path, id: &str, spec: &Spec) -> anyhow::Result<BTreeMap<String, String>> {
    seal_files_of(root, id, spec, spec.test_files())
}

fn seal_files_of(
    root: &Path,
    id: &str,
    spec: &Spec,
    files: std::collections::BTreeSet<String>,
) -> anyhow::Result<BTreeMap<String, String>> {
    if files.is_empty() {
        anyhow::bail!("{id}: no scenario names a test, so there is nothing to seal. Run `hexa spec tests {id} --crate <dir>` first");
    }
    // A seal freezes a file. Over a source file it would freeze the code the
    // test is meant to drive, so a sealed test lives in a `tests/` directory.
    let outside: Vec<String> = files.iter().filter(|&f| !is_test_file(f)).cloned().collect();
    if !outside.is_empty() {
        anyhow::bail!(
            "{}: {} not a test file; sealing it would freeze the implementation: a sealed test lives under a tests/, test/ or __tests__/ directory, is named *.test.* or *.spec.*, or is a .sh gate script (ADR-2611191120 §8)",
            id,
            outside.join(", ")
        );
    }
    let mut sealed = BTreeMap::new();
    for f in files {
        let bytes = std::fs::read(root.join(&f)).map_err(|e| anyhow::anyhow!("{f}: {e}"))?;
        let hash = sha256_hex(&bytes);
        // Re-sealing an edited test would launder the edit: a sealed test
        // changes through the spec, never by hand.
        if spec.sealed.get(&f).is_some_and(|s| *s != hash) {
            anyhow::bail!(
                "{}: seal broken: {f} changed since sealing. A sealed test changes through the spec, not by re-sealing; \
                 if the scenarios are unchanged and the edit is deliberate, `hexa spec amend {} --reason \"<why>\"`",
                id,
                id
            );
        }
        println!("  {} sealed {}", "\u{2713}".green(), f);
        sealed.insert(f, hash);
    }
    Ok(sealed)
}

/// `hexa spec verify [ADR] [--ci]` — drift is a failure (§6, §7, §8).
/// The bytes of `file` at `commit`, if that commit holds it.
fn blob_at(root: &Path, commit: &str, file: &str) -> Option<Vec<u8>> {
    std::process::Command::new("git")
        .args(["cat-file", "blob", &format!("{commit}:./{file}")])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| o.stdout)
}

/// The `to` of the last amendment recorded for `file`: the hash its seal had
/// when it was last moved.
fn chain_end(amendments: &[Value], file: &str) -> Option<String> {
    amendments.iter().rev().find(|a| a["file"] == file).and_then(|a| a["to"].as_str()).map(str::to_string)
}

/// The sha256 a sealed file is anchored to (ADR-2611081151 §1, §2): its blob
/// at the first commit in the branch's history that added it. Every clone
/// computes the same anchor (a red commit kept off the branch is not in a
/// clone), and every later change must be a recorded amendment. `None` when
/// no commit holds it: unanchored.
fn seal_anchor(root: &Path, file: &str) -> Option<String> {
    let adds = std::process::Command::new("git")
        .args(["log", "--diff-filter=A", "--format=%H", "--", file])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let first = String::from_utf8_lossy(&adds.stdout).lines().last()?.to_string();
    blob_at(root, &first, file).map(|b| sha256_hex(&b))
}

/// The spec's amendments as the domain reads them, oldest first.
fn amendment_chain(raw: &Value) -> Vec<hexa_core::ports::spec::Amendment> {
    let s = |e: &Value, k: &str| e.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    raw.get("amendments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|e| hexa_core::ports::spec::Amendment {
            file: s(e, "file"),
            from: s(e, "from"),
            to: s(e, "to"),
            by: s(e, "by"),
            reason: s(e, "reason"),
            at: s(e, "at"),
            base_commit: s(e, "base_commit"),
        })
        .collect()
}

/// The spec's moves as the domain reads them, oldest first.
fn move_chain(raw: &Value) -> Vec<hexa_core::ports::spec::Move> {
    let s = |e: &Value, k: &str| e.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    raw.get("moves")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|e| hexa_core::ports::spec::Move {
            scenario: s(e, "scenario"),
            from: s(e, "from"),
            to: s(e, "to"),
            by: s(e, "by"),
            reason: s(e, "reason"),
            at: s(e, "at"),
            base_commit: s(e, "base_commit"),
        })
        .collect()
}

/// Every committed revision of one spec along the branch, oldest first:
/// whether its acceptance holds, the spec, and the moves it records.
///
/// The branch is its first-parent line. History simplification would drop the
/// line a merge did not take the spec from, and with it a binding nothing
/// recorded leaving; flattening both sides of a merge would put their
/// revisions in no true order. A side branch's changes show at its merge.
struct History(Vec<(bool, Spec, Vec<hexa_core::ports::spec::Move>)>);

impl History {
    fn of(root: &Path, id: &str) -> History {
        // Every home a spec may have: moving it between them keeps its past.
        let homes: Vec<String> = spec_homes(root, id)
            .iter()
            .map(|p| p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('\\', "/"))
            .collect();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(root)
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        };
        if git(&["rev-parse", "--is-shallow-repository"]).is_some_and(|s| s.trim() == "true") {
            println!("  {} {id} shallow clone: bindings older than its history cannot be checked", "?".yellow());
        }
        let mut args = vec!["log", "--first-parent", "--reverse", "--format=%H", "--"];
        args.extend(homes.iter().map(String::as_str));
        History(
            git(&args)
                .unwrap_or_default()
                .lines()
                .filter_map(|rev| {
                    let text = String::from_utf8(homes.iter().find_map(|rel| blob_at(root, rev, rel))?).ok()?;
                    let spec = read_spec(&text).ok()?;
                    let moves = serde_json::from_str::<Value>(&text).map(|v| move_chain(&v)).unwrap_or_default();
                    Some((acceptance_holds(&spec).is_ok(), spec, moves))
                })
                .collect(),
        )
    }

    /// `s`'s anchor (ADR-2611251652 §1), and the moves already recorded in the
    /// revision it was taken from: those led to it, or to an older text's
    /// anchor, and are not applied to it again.
    fn anchor(&self, s: &hexa_core::ports::spec::Scenario) -> (Option<String>, Vec<hexa_core::ports::spec::Move>) {
        let revisions: Vec<(bool, Spec)> = self.0.iter().map(|(h, spec, _)| (*h, spec.clone())).collect();
        let mut spent = Vec::new();
        for (_, spec, moves) in self.0.iter().rev().filter(|(holds, _, _)| *holds) {
            let Some(then) = spec.specs.iter().find(|then| then.same_text(s)) else { break };
            if then.test.is_some() {
                spent = moves.clone();
            }
        }
        (hexa_core::ports::spec::binding_anchor(s, &revisions), spent)
    }
}

/// One scenario's binding against history and the recorded moves.
struct Binding {
    /// The ref the anchored chain reaches; `None` with no anchor, or when the
    /// chain is broken.
    reached: Option<String>,
    /// What `verify` reports, when the binding does not hold.
    problem: Option<String>,
}

fn binding(history: &History, moves: &[hexa_core::ports::spec::Move], s: &hexa_core::ports::spec::Scenario) -> Binding {
    use hexa_core::ports::spec::{judge_binding, BindingVerdict};
    let (anchor, spent) = history.anchor(s);
    let Some(anchor) = anchor else { return Binding { reached: None, problem: None } };
    let (at, live): (Vec<usize>, Vec<_>) =
        moves.iter().enumerate().filter(|(_, m)| !spent.contains(m)).map(|(i, m)| (i, m.clone())).unzip();
    // No ref is empty, so judging the empty one asks where the chain reaches.
    let (reached, broken) = match judge_binding(&s.id, Some(&anchor), &live, Some("")) {
        BindingVerdict::Moved { expected } => (Some(expected), None),
        BindingVerdict::BrokenChain(i) => (None, Some(at[i])),
        BindingVerdict::Holds => (None, None),
    };
    let problem = match (broken, s.test.as_deref()) {
        (Some(i), held) => Some(format!(
            "{} binding chain broken: move {i} does not continue from the ref history reaches (anchor {anchor}); the spec holds {}",
            s.id,
            held.unwrap_or("no test")
        )),
        (None, Some(held)) if reached.as_deref() != Some(held) => Some(format!(
            "{} binding moved without a record: history expects {}, the spec holds {held}; record it with `hexa spec move`",
            s.id,
            reached.as_deref().unwrap_or("?")
        )),
        _ => None,
    };
    Binding { reached, problem }
}

/// Judge every scenario's binding (ADR-2611251652 §2), say how many moves the
/// spec records, and return how many bindings do not hold.
fn report_bindings(root: &Path, id: &str, spec: &Spec, raw: &Value, verbose: bool) -> usize {
    let moves = move_chain(raw);
    if !moves.is_empty() {
        let n = moves.len();
        println!("  {} {id} {n} move{}", "\u{25cb}".dimmed(), if n == 1 { "" } else { "s" });
        if verbose {
            for m in &moves {
                println!("      {} {} → {} by {} at {}: {}", m.scenario, m.from, m.to, m.by, m.at, m.reason);
            }
        }
    }
    let history = History::of(root, id);
    let mut failed = 0;
    // A reviewed move's content must be in the new file's seal chain: else
    // one version was shown and another committed (ADR-2611251652 §4).
    let amendments = amendment_chain(raw);
    for m in raw.get("moves").and_then(Value::as_array).into_iter().flatten() {
        let (Some(reviewed), Some(to)) = (m["sha256"].as_str(), m["to"].as_str()) else { continue };
        let Some(file) = parse_test_ref(to).map(|t| t.file) else { continue };
        let Some(anchor) = seal_anchor(root, &file) else { continue };
        if anchor != reviewed && !amendments.iter().any(|a| a.file == file && a.to == reviewed) {
            println!(
                "  {} {id} {} binding move to {to}: the content reviewed for it is not in {file}'s history",
                "\u{2717}".red(),
                m["scenario"].as_str().unwrap_or("?")
            );
            failed += 1;
        }
    }
    for s in &spec.specs {
        if let Some(problem) = binding(&history, &moves, s).problem {
            println!("  {} {id} {problem}", "\u{2717}".red());
            failed += 1;
        }
    }
    failed
}

/// The ref each of `ids` is anchored to now, before a verb moves any of them.
/// A binding that already does not hold is refused: a move recorded on top of
/// it would hide it (ADR-2611251652 §3).
fn bindings_before(root: &Path, id: &str, spec: &Spec, ids: &[String]) -> anyhow::Result<BTreeMap<String, String>> {
    let raw: Value = serde_json::from_str(&std::fs::read_to_string(spec_path(root, id))?)?;
    let (moves, history) = (move_chain(&raw), History::of(root, id));
    let mut out = BTreeMap::new();
    for s in spec.specs.iter().filter(|s| ids.contains(&s.id)) {
        let b = binding(&history, &moves, s);
        if let Some(problem) = b.problem {
            anyhow::bail!("{id}: {problem}. Nothing was changed");
        }
        if let Some(reached) = b.reached {
            out.insert(s.id.clone(), reached);
        }
    }
    Ok(out)
}

fn git_head(root: &Path) -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn move_record(root: &Path, scenario: &str, from: &str, to: &str, by: &str, reason: &str) -> Value {
    serde_json::json!({
        "scenario": scenario,
        "from": from,
        "to": to,
        "by": by,
        "reason": reason,
        "at": chrono::Utc::now().to_rfc3339(),
        "base_commit": git_head(root),
    })
}

/// One move for each anchored scenario whose ref `spec` changes.
fn moves_made(root: &Path, spec: &Spec, before: &BTreeMap<String, String>, reason: &str) -> anyhow::Result<Vec<Value>> {
    let by = git_user_or_env(None)?;
    Ok(spec
        .specs
        .iter()
        .filter_map(|s| {
            let (from, to) = (before.get(&s.id)?, s.test.as_deref()?);
            (from != to).then(|| move_record(root, &s.id, from, to, &by, reason))
        })
        .collect())
}

fn git_user_or_env(by: Option<&str>) -> anyhow::Result<String> {
    acceptor(
        by,
        || {
            let out = std::process::Command::new("git").args(["config", "user.name"]).output().ok()?;
            Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
        },
        || std::env::var("USER").ok(),
    )
}

/// The newest committed version of `file`, if history holds one.
fn last_version(root: &Path, file: &str) -> Option<Vec<u8>> {
    let log = std::process::Command::new("git").args(["log", "--format=%H", "--", file]).current_dir(root).output().ok()?;
    String::from_utf8_lossy(&log.stdout).lines().find_map(|rev| blob_at(root, rev, file))
}

/// `hexa spec move <ADR> --scenario <id>=<ref>` (ADR-2611251652 §4): the first
/// call shows the diff, `--confirm` records the move.
pub fn move_binding(adr: &str, scenario: &str, reason: &str, confirm: Option<&str>, by: Option<&str>) -> anyhow::Result<()> {
    let root = std::env::current_dir()?;
    let a = find(&root, adr)?;
    let id = a.id.as_str();
    if reason.trim().is_empty() {
        anyhow::bail!("{id}: --reason is empty: say why the test binding moves");
    }
    let (read, mut spec) = load_text(&root, id)?.ok_or_else(|| anyhow::anyhow!("{id} has no spec"))?;
    if let Err(why) = acceptance_holds(&spec) {
        anyhow::bail!("{id}: {why}. A move is recorded against an accepted spec");
    }
    let (sid, to) = scenario
        .split_once('=')
        .map(|(a, b)| (a.trim(), b.trim()))
        .ok_or_else(|| anyhow::anyhow!("{id}: --scenario {scenario}: expected <scenario>=<file>::<name>"))?;
    let at = spec
        .specs
        .iter()
        .position(|s| s.id == sid)
        .ok_or_else(|| anyhow::anyhow!("{id}: {sid} is not a scenario of this spec"))?;
    let parsed = parse_test_ref(to).ok_or_else(|| anyhow::anyhow!("{id}: {to} is not one test ref"))?;
    let file = parsed.file.clone();
    if !is_test_file(&file) {
        anyhow::bail!("{id}: {file} is not a test file (ADR-2611191120 §8)");
    }
    let now = std::fs::read(root.join(&file)).map_err(|e| anyhow::anyhow!("{id}: {file}: {e}"))?;
    let text = String::from_utf8_lossy(&now).into_owned();
    let cfg = test_config(&root).map_err(|e| anyhow::anyhow!("{id}: {e}"))?;
    let named = parsed.name.as_deref().map(|n| n.rsplit("::").next().unwrap_or(n));
    // Rust is read, not planned: a plan needs a crate around the file, and
    // the question here is only whether the file holds the test.
    let missing = if file.ends_with(".rs") {
        named.is_some_and(|n| !hexa_core::ports::spec::test_fns(&text).iter().any(|f| f == n))
    } else {
        matches!(plan_test(&root, to, &cfg, ""), Err(TestRun::NotFound(_)))
    };
    if missing {
        anyhow::bail!("{id}: {to}: that test is not in {file}");
    }
    let raw: Value = serde_json::from_str(&read)?;
    let b = binding(&History::of(&root, id), &move_chain(&raw), &spec.specs[at]);
    let Some(from) = b.reached else {
        match b.problem {
            Some(problem) => anyhow::bail!("{id}: {problem}. Remove the record that breaks the chain first"),
            None => anyhow::bail!("{id}: {sid} has no anchor in history, so there is no binding to move"),
        }
    };
    if from == to {
        anyhow::bail!("{id}: nothing to record: {sid}'s chain already reaches {to}");
    }
    let old_file = parse_test_ref(&from).map(|t| t.file).ok_or_else(|| anyhow::anyhow!("{id}: {from} is not one test ref"))?;
    let old = last_version(&root, &old_file)
        .ok_or_else(|| anyhow::anyhow!("{id}: {old_file} has no version in git history, so the move cannot be shown"))?;
    let hash = sha256_hex(&now);
    let Some(given) = confirm else {
        println!("{} {id} {sid}: {from} → {to}", "\u{2b21} spec move".cyan().bold());
        if old == now {
            println!("  {old_file} at its last version and {file} have the same content.");
        } else {
            let diff = unified_diff(&file, &old, &now)?;
            let hunks: Vec<&str> = diff.lines().skip(2).collect();
            println!("--- {old_file} (last in history)\n+++ {file} (the move's target)\n{}", hunks.join("\n"));
        }
        println!("  The diff above and your reason are the review: this verb cannot tell whether a move weakened a test.");
        println!("  To record it, as shown:\n\n    hexa spec move {id} --scenario {sid}={to} --reason {reason:?} --confirm {hash}\n");
        return Ok(());
    };
    if !given.trim().eq_ignore_ascii_case(&hash) {
        anyhow::bail!(
            "{id}: --confirm {given} is not the sha256 of {file} as it is now, so it confirms content that was not shown. \
             Run again without --confirm to see the diff of the file as it stands"
        );
    }
    let by = git_user_or_env(by)?;
    let mut record = move_record(&root, sid, &from, to, &by, reason);
    // What was reviewed, which `verify` holds the new file's history to.
    record["sha256"] = Value::String(hash.clone());
    spec.specs[at].test = Some(to.to_string());
    if std::fs::read_to_string(spec_path(&root, id)).ok().as_deref() != Some(read.as_str()) {
        anyhow::bail!("{id}: the spec changed while the move was prepared; nothing was written");
    }
    write_records(&root, &spec, None, &[record])?;
    println!("  {} {id} {sid} moved to {to} by {by}: {reason}", "\u{2713}".green());
    Ok(())
}

/// Judge every sealed file of one spec against history (ADR-2611081151 §1,
/// §2), print each that does not hold, and return how many fail. `also` names
/// test files the scenarios point at that the `sealed` map may lack.
fn report_seals(root: &Path, id: &str, raw: &Value, also: &[String], ci: bool) -> usize {
    use hexa_core::ports::spec::{judge_seal, SealVerdict};
    let sealed = raw.get("sealed").and_then(Value::as_object);
    let mut files: Vec<String> = sealed.map(|m| m.keys().cloned().collect()).unwrap_or_default();
    files.extend(also.iter().cloned());
    files.sort();
    files.dedup();
    let amendments = amendment_chain(raw);
    let mut fatal = 0;
    for f in &files {
        let Some(recorded) = sealed.and_then(|m| m.get(f)).and_then(Value::as_str) else {
            println!("  {} {} {} is not sealed (`hexa spec seal`)", "\u{2717}".red(), id, f);
            fatal += 1;
            continue;
        };
        let current = std::fs::read(root.join(f)).ok().map(|b| sha256_hex(&b));
        let anchor = seal_anchor(root, f);
        let verdict = judge_seal(f, anchor.as_deref(), &amendments, Some(recorded), current.as_deref());
        let (line, fails) = match verdict {
            SealVerdict::Holds => continue,
            SealVerdict::Edited => (format!("seal broken: {f} changed since sealing. A sealed test changes through the spec"), true),
            SealVerdict::Gone => (format!("seal broken: {f} is gone"), true),
            SealVerdict::Rewritten => (
                format!(
                    "seal rewritten: {f}'s recorded hash moved without an amendment; history anchors it at {}…, the record says {}…",
                    anchor.as_deref().unwrap_or("?").chars().take(12).collect::<String>(),
                    recorded.chars().take(12).collect::<String>()
                ),
                true,
            ),
            SealVerdict::BrokenChain(i) => {
                (format!("seal broken: amendment {i} of {f} does not continue the chain from the anchored hash"), true)
            }
            // No anchor never hides an edit: the file is still held to its
            // record, the check this replaces (ADR-2610221540 §6).
            SealVerdict::Unanchored if current.as_deref() != Some(recorded) => match current {
                None => (format!("seal broken: {f} is gone"), true),
                Some(_) => (format!("seal broken: {f} changed since sealing. A sealed test changes through the spec"), true),
            },
            SealVerdict::Unanchored => (
                format!("seal unanchored: no commit in history holds {f}; commit it, so history can anchor the seal"),
                ci,
            ),
        };
        let mark = if fails { "\u{2717}".red() } else { "?".yellow() };
        println!("  {} {} {}", mark, id, line);
        fatal += usize::from(fails);
    }
    fatal
}

pub async fn verify(adr: Option<&str>, ci: bool, prove: bool, verbose: bool, local: bool) -> anyhow::Result<()> {
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
    // Where the tests run: here, or every one through the project's remote
    // template for HEAD, in one run (ADR-2611191120 §3–§5). Refused before
    // anything runs when the remote could not describe this tree.
    let cfg = test_config(&root).map_err(|e| anyhow::anyhow!(e))?;
    let remote = cfg.remote.clone().filter(|_| !local);
    let head = match &remote {
        Some(_) => {
            let head = full_commit(&root, "HEAD").ok_or_else(|| anyhow::anyhow!("HEAD is not a commit: nothing to send to the remote runner"))?;
            remote_preflight(&root, &head, true).map_err(|e| anyhow::anyhow!(e))?;
            println!("  {} every test goes to the remote runner (test.remote) for {}; --local runs them here", "\u{2192}".cyan(), &head[..12]);
            Some(head)
        }
        None => None,
    };
    let prefix = if remote.is_some() { env_prefix(None, None) } else { env_prefix(Some(&path_env), None) };
    let (mut passed, mut failed, mut pending, mut unproven) = (0, 0, 0, 0);
    let mut fatal = 0;
    // Each tested scenario: (ADR, scenario, ref, its plan or why it cannot run).
    let mut jobs: Vec<(String, String, String, Result<Planned, TestRun>)> = Vec::new();
    let mut to_prove: Vec<(Adr, Spec)> = Vec::new();
    // Scenarios found, whatever became of them: the empty-run check is about
    // finding none (ADR-2611230830 §2), not about how many ran.
    let mut scenarios_found = 0usize;
    for a in &adrs {
        // The seals are judged from the raw record, so a spec this cannot
        // otherwise parse still guards its tests (ADR-2611081151 §1).
        let raw: Option<Value> =
            std::fs::read_to_string(spec_path(&root, &a.id)).ok().and_then(|t| serde_json::from_str(&t).ok());
        let (text, spec) = match load_text(&root, &a.id) {
            Ok(Some(loaded)) => loaded,
            Ok(None) => {
                println!("  {} {} has no spec", "\u{2717}".red(), a.id);
                fatal += 1;
                continue;
            }
            Err(e) => {
                if let Some(raw) = &raw {
                    fatal += report_seals(&root, &a.id, raw, &[], ci);
                }
                println!("  {} {} spec does not parse: {e}", "\u{2717}".red(), a.id);
                fatal += 1;
                continue;
            }
        };
        scenarios_found += spec.specs.len();
        let alt = is_alt(&text);
        if !print_coverage(a, &spec, alt) {
            fatal += 1;
        }
        if let Err(why) = acceptance_holds(&spec) {
            println!("  {} {} {}", "\u{2717}".red(), a.id, why);
            fatal += 1;
        }
        fatal += report_bindings(&root, &a.id, &spec, raw.as_ref().unwrap_or(&Value::Null), verbose);
        let amended = amendments_of(&text);
        if !amended.is_empty() {
            let n = amended.len();
            println!("  {} {} {} amendment{}", "\u{25cb}".dimmed(), a.id, n, if n == 1 { "" } else { "s" });
            if verbose {
                for e in &amended {
                    let short = |k: &str| e[k].as_str().unwrap_or("?").chars().take(12).collect::<String>();
                    println!(
                        "      {} {}…→{}… by {} at {}: {}",
                        e["file"].as_str().unwrap_or("?"),
                        short("from"),
                        short("to"),
                        e["by"].as_str().unwrap_or("?"),
                        e["at"].as_str().unwrap_or("?"),
                        e["reason"].as_str().unwrap_or("")
                    );
                }
            }
        }
        let named: Vec<String> = test_files(&root, &spec, &text).into_iter().collect();
        let raw = raw.unwrap_or(Value::Null);
        fatal += report_seals(&root, &a.id, &raw, &named, ci);
        let completed = a.status == "completed";
        for s in &spec.specs {
            let Some(r) = s.test.as_deref() else {
                pending += 1;
                let mark = if completed && ci { "\u{2717}".red() } else { "\u{25cb}".dimmed() };
                println!("  {} {} {} pending: no test yet", mark, a.id, s.id);
                continue;
            };
            if alt && parse_test_ref(r).is_none() {
                println!("  {} {} {} not run here: `{}` is a bare test name, not `<path>.rs::<fn>`", "\u{25cb}".dimmed(), a.id, s.id, r);
                continue;
            }
            jobs.push((a.id.clone(), s.id.clone(), r.to_string(), plan_test(&root, r, &cfg, &prefix)));
        }
        if completed && ci && spec.specs.iter().any(|s| s.test.is_none()) {
            fatal += 1;
        }
        if prove {
            to_prove.push((a.clone(), spec.clone()));
        }
    }
    let plans: Vec<Planned> = jobs.iter().filter_map(|j| j.3.as_ref().ok().cloned()).collect();
    let at = match (&remote, &head) {
        (Some(template), Some(commit)) => Where::Remote { template, commit },
        _ => Where::Here,
    };
    let mut runs = run_plans(&root, at, &plans).await.into_iter();
    for (adr_id, sid, r, plan) in jobs {
        let outcome = match plan {
            Ok(_) => runs.next().unwrap_or_else(|| TestRun::RunnerFailed("no result came back for this test".into())),
            Err(why) => why,
        };
        match outcome {
            TestRun::Passed => {
                passed += 1;
                println!("  {} {} {} — {}", "\u{2713}".green(), adr_id, sid, r.dimmed());
            }
            TestRun::Failed(tail) | TestRun::Broken { tail, .. } => {
                failed += 1;
                println!("  {} {} {} failed — {}", "\u{2717}".red(), adr_id, sid, r);
                for line in tail.lines() {
                    println!("      {}", line.dimmed());
                }
            }
            TestRun::NotFound(why) => {
                failed += 1;
                println!("  {} {} {} {}", "\u{2717}".red(), adr_id, sid, why);
            }
            TestRun::Vacuous => {
                failed += 1;
                println!("  {} {} {} ran 0 tests (vacuous): {}", "\u{2717}".red(), adr_id, sid, r);
            }
            TestRun::Ambiguous(n) => {
                failed += 1;
                println!("  {} {} {} matched {n} tests; a scenario names one: {}", "\u{2717}".red(), adr_id, sid, r);
            }
            TestRun::NotProvenHere(why) => {
                unproven += 1;
                let mark = if ci { "\u{2717}".red() } else { "?".yellow() };
                println!("  {} {} {} not proven here: {}", mark, adr_id, sid, why);
            }
            TestRun::RunnerFailed(why) => {
                failed += 1;
                println!("  {} {} {} not run: {}", "\u{2717}".red(), adr_id, sid, why.lines().next().unwrap_or(""));
            }
        }
    }
    for (a, spec) in &to_prove {
        fatal += prove_red(&root, &path_env, a, spec, &cfg, local).await?;
    }
    println!("\n  {passed} passed · {failed} failed · {unproven} not proven here · {pending} pending");
    let unproven_fails = if ci { unproven } else { 0 };
    // A whole-repository --ci run that found no scenario checked nothing, and
    // a vacuous gate is a failed gate (ADR-2611230830 §2). Said first, so the
    // reason is the one a reader needs.
    if ci && adr.is_none() && scenarios_found == 0 {
        anyhow::bail!("`hexa spec verify --ci`: no spec holds a scenario, so nothing was checked");
    }
    if failed + fatal + unproven_fails > 0 {
        anyhow::bail!("spec verification failed");
    }
    // Nothing passed and work is pending: no scenario was verified, and the
    // workflow uses this verb as the loop's gate. A gate that measured nothing
    // is a failed gate, as "running 0 tests" is.
    if passed == 0 && pending > 0 {
        anyhow::bail!(
            "spec verification proved nothing: 0 passed and {pending} pending. Generate the tests (`hexa spec tests <ADR> --crate <dir>`) before using this as a gate"
        );
    }
    Ok(())
}

/// The commit that first added `file` to this branch's history: where its seal's
/// chain is anchored (ADR-2611081151 §1).
fn anchor_commit(root: &Path, file: &str) -> Option<String> {
    let adds = std::process::Command::new("git")
        .args(["log", "--diff-filter=A", "--format=%H", "--", file])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    String::from_utf8_lossy(&adds.stdout).lines().last().map(str::to_string)
}

/// `hexa spec amend <ADR> --file <f> --reason <why> --at <commit>…`: record steps
/// of a sealed file's chain that went unrecorded (every move of a seal is an
/// amendment, ADR-2611081151 §1, and a hexa verb or a test edited before it was
/// sealed left some out). It adds no way to reach a new state: each step's hash
/// is the file's blob at a named commit of this branch, the commits run oldest
/// first from the anchor, and they must end at exactly the hash the chain already
/// needs (where its first recorded amendment starts, else the recorded seal). The
/// first call shows the diffs and writes nothing; `--confirm` with the digest it
/// printed records them, before the amendments already there.
pub fn amend_history(adr: &str, reason: &str, file: Option<&str>, at: &[String], confirm: Option<&str>, by: Option<&str>) -> anyhow::Result<()> {
    let root = std::env::current_dir()?;
    let a = find(&root, adr)?;
    let id = a.id.as_str();
    if reason.trim().is_empty() {
        anyhow::bail!("{id}: --reason is empty: say why these steps were not recorded when they happened");
    }
    let Some(file) = file.map(|f| f.trim_start_matches("./")) else {
        anyhow::bail!("{id}: --file <path> is required with --at: steps are recorded for one sealed file at a time");
    };
    let (text, spec) = load_text(&root, id)?.ok_or_else(|| anyhow::anyhow!("{id} has no spec to amend"))?;
    if let Err(why) = acceptance_holds(&spec) {
        anyhow::bail!("{id}: {why}. Steps are recorded against an accepted spec");
    }
    let Some(recorded) = spec.sealed.get(file).cloned() else {
        let all: Vec<&str> = spec.sealed.keys().map(String::as_str).collect();
        anyhow::bail!("{id}: {file} is not a sealed file of this spec (sealed: {})", all.join(", "));
    };
    if !is_test_file(file) {
        anyhow::bail!("{id}: {file} is not a test file (ADR-2611191120 §8)");
    }
    let git = |args: &[&str]| -> Option<String> {
        let o = std::process::Command::new("git").args(args).current_dir(&root).output().ok()?;
        o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let Some(anchor_at) = anchor_commit(&root, file) else {
        anyhow::bail!("{id}: no commit in history holds {file}, so its chain has no anchor to continue from: commit it first");
    };
    let Some(anchor_bytes) = blob_at(&root, &anchor_at, file) else {
        anyhow::bail!("{id}: {file} has no version at its anchor {anchor_at}");
    };
    let anchor = sha256_hex(&anchor_bytes);
    let mut amended = amendments_of(&text);
    // Where the recorded chain has to be reached: the hash its first amendment starts from.
    let first_existing = amended.iter().position(|m| m["file"] == file);
    let target = match first_existing {
        Some(i) => amended[i]["from"].as_str().unwrap_or_default().to_string(),
        None => recorded.clone(),
    };

    // The commits: real, on this branch, oldest first, none before the anchor.
    let mut steps: Vec<(String, String, Vec<u8>)> = Vec::new(); // (commit, hash, bytes)
    let mut earlier = anchor_at.clone();
    for c in at {
        let sha = git(&["rev-parse", "--verify", "--quiet", &format!("{c}^{{commit}}")]).ok_or_else(|| anyhow::anyhow!("{id}: {c} is not a commit"))?;
        if git(&["merge-base", "--is-ancestor", &sha, "HEAD"]).is_none() {
            anyhow::bail!("{id}: {c} is not an ancestor of HEAD: a step is recorded from this branch's history");
        }
        if git(&["merge-base", "--is-ancestor", &earlier, &sha]).is_none() {
            anyhow::bail!("{id}: {c} is not after the previous commit (or the anchor {}): name the commits oldest first", &anchor_at[..anchor_at.len().min(8)]);
        }
        let bytes = blob_at(&root, &sha, file).ok_or_else(|| anyhow::anyhow!("{id}: {file} does not exist at {c}"))?;
        steps.push((sha.clone(), sha256_hex(&bytes), bytes));
        earlier = sha;
    }
    // Only the steps that change the file are steps.
    let mut hops: Vec<(String, String, String, Vec<u8>, Vec<u8>)> = Vec::new(); // (commit, from, to, old, new)
    let (mut cur, mut cur_bytes) = (anchor.clone(), anchor_bytes);
    for (sha, h, bytes) in &steps {
        if *h != cur {
            hops.push((sha.clone(), cur.clone(), h.clone(), cur_bytes.clone(), bytes.clone()));
            cur = h.clone();
            cur_bytes = bytes.clone();
        }
    }
    if cur != target {
        anyhow::bail!(
            "{id}: these commits do not reach where the chain needs to: they end at {}…, and {file}'s record needs {}… ({}). Name the commit at which the file had that hash as the last --at",
            &cur[..cur.len().min(12)],
            &target[..target.len().min(12)],
            if first_existing.is_some() { "where its first recorded amendment starts" } else { "its recorded seal" }
        );
    }
    if hops.is_empty() {
        anyhow::bail!("{id}: nothing to record: {file}'s chain already starts at its anchor and reaches {}…", &target[..target.len().min(12)]);
    }
    // One digest over the steps as shown, so --confirm confirms exactly these.
    let digest = sha256_hex(hops.iter().map(|h| format!("{}>{}@{}\n", h.1, h.2, h.0)).collect::<String>().as_bytes());
    let Some(given) = confirm else {
        println!("{} {id} {file}: {} unrecorded step(s) from its anchor", "\u{2b21} spec amend --at".cyan().bold(), hops.len());
        for (i, (sha, from, to, old, new)) in hops.iter().enumerate() {
            println!("\nstep {} at {}: {}… → {}…", i + 1, &sha[..sha.len().min(10)], &from[..from.len().min(12)], &to[..to.len().min(12)]);
            let diff = unified_diff(file, old, new)?;
            let hunks: Vec<&str> = diff.lines().skip(2).collect();
            println!("{}", hunks.join("\n"));
        }
        println!("\n  The diffs above and your reason are the review: this verb cannot tell whether a step weakened a test. It records steps history already holds and reaches no state the record did not already need.");
        let ats: String = at.iter().map(|c| format!(" --at {c}")).collect();
        println!("  To record them, as shown:\n\n    hexa spec amend {id} --file {file} --reason {reason:?}{ats} --confirm {digest}\n");
        return Ok(());
    };
    if !given.trim().eq_ignore_ascii_case(&digest) {
        anyhow::bail!("{id}: --confirm {given} is not the digest of the steps as they stand, so it confirms steps that were not shown. Run again without --confirm to see them");
    }
    let by = git_user_or_env(by)?;
    let now = chrono::Utc::now().to_rfc3339();
    let records: Vec<Value> = hops
        .iter()
        .map(|(sha, from, to, _, _)| {
            serde_json::json!({
                "file": file,
                "from": from,
                "to": to,
                "by": by,
                "reason": format!("recorded from history (the step went unrecorded when it happened): {reason}"),
                "at": now,
                "base_commit": sha,
            })
        })
        .collect();
    // Before the amendments already there: they continue from where these end.
    let at_index = first_existing.unwrap_or(amended.len());
    for (k, r) in records.into_iter().enumerate() {
        amended.insert(at_index + k, r);
    }
    write_records(&root, &spec, Some(&amended), &[])?;
    println!("  {} {id} {file}: {} step(s) recorded by {by}, oldest first, before any amendment already there", "\u{2713}".green(), hops.len());
    println!("\n  Commit {}.", spec_rel(&root, id));
    Ok(())
}

/// `hexa spec amend <ADR> --reason <why> [--file <f>] [--confirm <sha256>]`
/// (ADR-2611062141). The scenarios are unchanged and a sealed file's content
/// has to change. The first call shows the diff and writes nothing; the call
/// with the hash of the file as shown records the amendment and moves the seal.
pub fn amend(
    adr: &str,
    reason: &str,
    file: Option<&str>,
    confirm: Option<&str>,
    by: Option<&str>,
) -> anyhow::Result<()> {
    let root = std::env::current_dir()?;
    let a = find(&root, adr)?;
    let id = a.id.as_str();
    if reason.trim().is_empty() {
        anyhow::bail!("{id}: --reason is empty: say why the sealed test is being changed");
    }
    let (text, mut spec) = load_text(&root, id)?.ok_or_else(|| anyhow::anyhow!("{id} has no spec to amend"))?;
    match acceptance_holds(&spec) {
        Ok(()) => {}
        Err(why) if spec.accepted.is_none() => anyhow::bail!("{id}: {why}"),
        Err(_) => anyhow::bail!(
            "{id}: the scenarios changed since they were accepted. That goes through the spec, not through amend: \
             a person runs `hexa spec accept {id}` again, then `hexa spec tests {id} --crate <dir>`"
        ),
    }
    if spec.sealed.is_empty() {
        anyhow::bail!("{id}: nothing is sealed, so there is nothing to amend");
    }
    // What differs from its seal, with the file as it is now.
    let mut differing: Vec<(String, Option<Vec<u8>>)> = Vec::new();
    for (f, sealed) in &spec.sealed {
        let now = std::fs::read(root.join(f)).ok();
        if now.as_deref().map(sha256_hex).as_deref() != Some(sealed.as_str()) {
            differing.push((f.clone(), now));
        }
    }
    let chosen = match file.map(|f| f.trim_start_matches("./")) {
        Some(f) => {
            if !spec.sealed.contains_key(f) {
                let all: Vec<&str> = spec.sealed.keys().map(String::as_str).collect();
                anyhow::bail!("{id}: {f} is not a sealed file of this spec (sealed: {})", all.join(", "));
            }
            if !differing.iter().any(|d| d.0 == f) {
                anyhow::bail!("{id}: nothing to amend: {f} matches its seal");
            }
            f.to_string()
        }
        None => match differing.as_slice() {
            [] => anyhow::bail!("{id}: nothing to amend: every sealed file matches its seal"),
            [(f, _)] => f.clone(),
            many => {
                let names: Vec<&str> = many.iter().map(|d| d.0.as_str()).collect();
                anyhow::bail!(
                    "{id}: {} sealed files differ from their seal: {}. --file <path> is required: one file is amended per confirmation",
                    many.len(),
                    names.join(", ")
                )
            }
        },
    };
    let now_bytes = differing
        .into_iter()
        .find(|d| d.0 == chosen)
        .and_then(|d| d.1)
        .ok_or_else(|| anyhow::anyhow!("{id}: {chosen} is gone; a deleted sealed test cannot be amended"))?;
    let now_hash = sha256_hex(&now_bytes);
    if !is_test_file(&chosen) {
        anyhow::bail!("{id}: {chosen} is not a test file; only a sealed test file is amended: a sealed test lives under a tests/, test/ or __tests__/ directory, is named *.test.* or *.spec.*, or is a .sh gate script (ADR-2611191120 §8)");
    }
    let sealed_hash = spec.sealed[&chosen].clone();
    let mut amended = amendments_of(&text);
    let Some((base_commit, base_bytes)) = sealed_version(&root, &chosen, &sealed_hash, &amended) else {
        anyhow::bail!("{id}: {chosen}: the sealed version is not in git history, so the edit cannot be shown");
    };

    // Every test function a scenario names in this file must survive the edit.
    let before = String::from_utf8_lossy(&base_bytes).to_string();
    let after = String::from_utf8_lossy(&now_bytes).to_string();
    let raw: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    let scenario_files: Vec<Option<String>> = raw["scenarios"]
        .as_array()
        .or_else(|| raw["specs"].as_array())
        .into_iter()
        .flatten()
        .map(|s| s["file"].as_str().map(str::to_string))
        .collect();
    let mut missing: Vec<String> = Vec::new();
    for (i, s) in spec.specs.iter().enumerate() {
        let Some(t) = s.test.as_deref() else { continue };
        let (belongs, leaf) = match test_ref(t) {
            Some((_, f)) if f.ends_with("::") => continue,
            Some((p, f)) => (p == chosen, f.rsplit("::").next().unwrap_or(f)),
            None => {
                let named = scenario_files.get(i).cloned().flatten();
                (named.map_or_else(|| before.contains(&format!("fn {t}(")), |n| n == chosen), t)
            }
        };
        if belongs && !after.contains(&format!("fn {leaf}(")) {
            missing.push(format!("{leaf} ({})", s.id));
        }
    }
    if !missing.is_empty() {
        anyhow::bail!(
            "{id}: {chosen} no longer has the test function(s) the scenarios name: {}. A scenario keeps its test; changing that goes through the spec",
            missing.join(", ")
        );
    }

    let Some(given) = confirm else {
        let diff = unified_diff(&chosen, &base_bytes, &now_bytes)?;
        println!("{} {id} {chosen}, against its sealed version at {}", "\u{2b21} spec amend".cyan().bold(), &base_commit[..base_commit.len().min(12)]);
        println!("{diff}");
        println!("  The diff above and your reason are the review: this verb cannot tell whether an edit weakened an assertion.");
        println!("  To record it, as shown:\n\n    hexa spec amend {id} --file {chosen} --reason {reason:?} --confirm {now_hash}\n");
        return Ok(());
    };
    if !given.trim().eq_ignore_ascii_case(&now_hash) {
        anyhow::bail!(
            "{id}: --confirm {given} is not the sha256 of {chosen} as it is now, so it confirms content that was not shown. \
             Run again without --confirm to see the diff of the file as it stands"
        );
    }
    let by = acceptor(
        by,
        || {
            let out = std::process::Command::new("git").args(["config", "user.name"]).output().ok()?;
            Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
        },
        || std::env::var("USER").ok(),
    )?;
    amended.push(serde_json::json!({
        "file": chosen,
        "from": sealed_hash,
        "to": now_hash,
        "by": by,
        "reason": reason,
        "at": chrono::Utc::now().to_rfc3339(),
        "base_commit": base_commit,
    }));
    spec.sealed.insert(chosen.clone(), now_hash.clone());
    write_spec(&root, &spec, Some(&amended))?;
    println!("  {} {id} amended {chosen} to {} by {by}: {reason}", "\u{2713}".green(), &now_hash[..12]);
    Ok(())
}

/// The sealed version of `file`: the revision in git history whose sha256 is
/// `hash`. A seal that an earlier amendment moved may name a version nobody
/// committed; it falls back to the version that amendment replaced, which a
/// person was shown, so the diff then spans every edit since.
fn sealed_version(root: &Path, file: &str, hash: &str, amended: &[Value]) -> Option<(String, Vec<u8>)> {
    let mut want = hash.to_string();
    for _ in 0..=amended.len() {
        if let Some(found) = in_history(root, file, &want) {
            return Some(found);
        }
        let prior = amended
            .iter()
            .rev()
            .find(|e| e["file"].as_str() == Some(file) && e["to"].as_str() == Some(want.as_str()))
            .and_then(|e| e["from"].as_str())?;
        want = prior.to_string();
    }
    None
}

fn in_history(root: &Path, file: &str, hash: &str) -> Option<(String, Vec<u8>)> {
    let log = std::process::Command::new("git").args(["log", "--format=%H", "--"]).arg(file).current_dir(root).output().ok()?;
    if !log.status.success() {
        return None;
    }
    for rev in String::from_utf8_lossy(&log.stdout).lines() {
        let shown = std::process::Command::new("git").arg("show").arg(format!("{rev}:./{file}")).current_dir(root).output().ok()?;
        if shown.status.success() && sha256_hex(&shown.stdout) == hash {
            return Some((rev.to_string(), shown.stdout));
        }
    }
    None
}

/// A unified diff of `old` to `new`, from `git diff --no-index`.
fn unified_diff(file: &str, old: &[u8], new: &[u8]) -> anyhow::Result<String> {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!("hexa-amend-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let out = (|| {
        std::fs::write(dir.join("sealed"), old)?;
        std::fs::write(dir.join("current"), new)?;
        std::process::Command::new("git")
            .args(["diff", "--no-index", "--no-color", "--no-ext-diff", "--", "sealed", "current"])
            .current_dir(&dir)
            .output()
    })();
    let _ = std::fs::remove_dir_all(&dir);
    let out = out?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let hunks: Vec<&str> = text.lines().skip_while(|l| !l.starts_with("@@")).collect();
    if hunks.is_empty() {
        anyhow::bail!("{file}: git produced no diff between the sealed version and the file");
    }
    Ok(format!("--- {file} (sealed)\n+++ {file} (current)\n{}", hunks.join("\n")))
}

/// `--prove` (§5, §8): each tested scenario's test must exist and fail at its
/// red commit. Returns how many did not. One worktree per commit;
/// builds share a target directory under `target/`, so a second commit
/// rebuilds what changed, not the world.
async fn prove_red(root: &Path, path_env: &str, a: &Adr, spec: &Spec, cfg: &TestConfig, local: bool) -> anyhow::Result<usize> {
    let remote = cfg.remote.as_deref().filter(|_| !local);
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
        // Here: a checkout of the red commit under the ignored target/, beside
        // the shared build. The path is this run's own, so a concurrent
        // `--prove` never removes it mid-test; a stale one at this path (a
        // reused pid) is removed first. Remote: none. The template runs from
        // the repository root, and the runner checks the commit out on its
        // side (ADR-2611191120 §3, gaberger/hexa#68): an old commit's copy of
        // the runner script may not exist, and nothing is built here.
        let wt = if remote.is_some() {
            root.to_path_buf()
        } else {
            match checkout_at(root, &target, &sha) {
                Ok(wt) => wt,
                Err(e) => {
                    unproven += scenarios.len();
                    println!("  {} {} red commit {sha} cannot be checked out: {e}", "\u{2717}".red(), a.id);
                    continue;
                }
            }
        };
        // A test proved red at a commit from before it was written
        // (`--red-at`, ADR-2610281628 §3) is checked there as written: the
        // file is added to that tree, as the red check added it. A remote
        // checkout of that commit cannot hold it (ADR-2611191120 §5).
        let mut copied: Vec<&str> = Vec::new();
        for (sid, r) in &scenarios {
            if let Some(t) = parse_test_ref(r) {
                if remote.is_some() {
                    if !file_at(root, &sha, &t.file) && root.join(&t.file).exists() {
                        copied.push(sid);
                    }
                } else if !wt.join(&t.file).exists() {
                    if let Ok(body) = std::fs::read(root.join(&t.file)) {
                        let _ = std::fs::create_dir_all(wt.join(&t.file).parent().unwrap_or(&wt));
                        let _ = std::fs::write(wt.join(&t.file), body);
                        copied.push(sid);
                    }
                }
            }
        }
        let full = full_commit(root, &sha).unwrap_or_else(|| sha.clone());
        let refused = match remote {
            Some(_) => remote_preflight(root, &full, false).err(),
            None => None,
        };
        let prefix = if remote.is_some() { env_prefix(None, None) } else { env_prefix(Some(path_env), Some(&target)) };
        let mut planned: Vec<(&str, &str, Result<Planned, TestRun>)> = Vec::new();
        for (sid, r) in &scenarios {
            let plan = match (&refused, remote.is_some() && copied.contains(sid)) {
                (Some(why), _) => Err(TestRun::RunnerFailed(why.clone())),
                (None, true) => Err(TestRun::RunnerFailed(format!(
                    "the test was written after {sha} and copied into it to prove red, which a remote checkout cannot do: prove it with --local"
                ))),
                (None, false) => plan_test(&wt, r, cfg, &prefix),
            };
            planned.push((sid, r, plan));
        }
        let plans: Vec<Planned> = planned.iter().filter_map(|p| p.2.as_ref().ok().cloned()).collect();
        let at = match remote {
            Some(template) => Where::Remote { template, commit: &full },
            None => Where::Here,
        };
        let mut runs = run_plans(&wt, at, &plans).await.into_iter();
        for (sid, _r, plan) in planned {
            let outcome = match plan {
                Ok(_) => runs.next().unwrap_or_else(|| TestRun::RunnerFailed("no result came back".into())),
                Err(why) => why,
            };
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
                TestRun::RunnerFailed(why) => {
                    (false, format!("red not shown at {sha}: {}", why.lines().next().unwrap_or("")))
                }
            };
            if ok {
                println!("  {} {} {} {}", "\u{2713}".green(), a.id, sid, what);
            } else {
                unproven += 1;
                println!("  {} {} {} {}", "\u{2717}".red(), a.id, sid, what);
            }
        }
        if remote.is_none() {
            remove_checkout(root, &wt);
        }
    }
    Ok(unproven)
}

/// `hexa spec red <ADR> <commit> [--test <S>=<file>::<fn>]…` — record the
/// commit a test was red at, only after watching it fail there. Each `--test`
/// binds a pending scenario: one a re-acceptance left without a test because
/// a person changed it. With none, every tested scenario lacking a red commit
/// is checked. A test file whose seal the re-acceptance dropped gets back the
/// seal its last record held, so the edit since is reviewed through
/// `hexa spec amend` (gap `spec-amend-scenario-path`).
pub async fn red(adr: &str, commit: &str, tests: &[String], local: bool) -> anyhow::Result<()> {
    let root = std::env::current_dir()?;
    let a = find(&root, adr)?;
    let id = a.id.as_str();
    let (read, mut spec) = load_text(&root, id)?.ok_or_else(|| anyhow::anyhow!("{id} has no spec"))?;
    if let Err(why) = acceptance_holds(&spec) {
        anyhow::bail!("{id}: {why}. A red commit is recorded against an accepted spec");
    }
    let git = |args: &[&str]| -> Option<String> {
        let o = std::process::Command::new("git").args(args).current_dir(&root).output().ok()?;
        o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let sha = git(&["rev-parse", "--verify", "--quiet", &format!("{commit}^{{commit}}")])
        .ok_or_else(|| anyhow::anyhow!("{id}: {commit} is not a commit"))?;
    if git(&["merge-base", "--is-ancestor", &sha, "HEAD"]).is_none() {
        anyhow::bail!("{id}: {commit} is not an ancestor of HEAD: a red commit is one this branch grew from");
    }
    let short = git(&["rev-parse", "--short", &sha]).unwrap_or_else(|| sha.clone());

    // What is checked: each scenario named, with the test it is bound to.
    let mut targets: Vec<(String, String, bool)> = Vec::new();
    let history = History::of(&root, id);
    let moves = serde_json::from_str::<Value>(&read).map(|v| move_chain(&v)).unwrap_or_default();
    for t in tests {
        let (sid, r) = t
            .split_once('=')
            .map(|(a, b)| (a.trim(), b.trim()))
            .ok_or_else(|| anyhow::anyhow!("{id}: --test {t}: expected <scenario>=<file>::<name>"))?;
        let s = spec
            .specs
            .iter()
            .find(|s| s.id == sid)
            .ok_or_else(|| anyhow::anyhow!("{id}: {sid} is not a scenario of this spec"))?;
        // A binding history anchors moves only by a record (ADR-2611251652 §3).
        if let Some(anchored) = binding(&history, &moves, s).reached.filter(|anchored| anchored != r) {
            anyhow::bail!(
                "{id}: {sid} is anchored to {anchored} in history; `spec red` does not move it to {r}. \
                 Record the move with `hexa spec move {id} --scenario {sid}={r} --reason \"<why>\"`"
            );
        }
        if let Some(have) = &s.test {
            anyhow::bail!(
                "{id}: {sid} already has its test, {have}. A scenario's test changes through the spec: a person \
                 re-accepts the changed scenario, which leaves it pending"
            );
        }
        let Some(parsed) = parse_test_ref(r).filter(|t| !t.name.as_deref().is_some_and(|n| n.ends_with("::"))) else {
            anyhow::bail!(
                "{id}: --test {t}: {r} is not one test: `<file>.rs::<fn>`, `<file>.ts::<test name>`, `<file>.sh[::<arg>]` \
                 or `<file>.<ext>::<name>` for a runner in test.runners (ADR-2611191120 §1)"
            );
        };
        let file = parsed.file.as_str();
        if !is_test_file(file) {
            anyhow::bail!(
                "{id}: {file} is not a test file; a scenario's test is sealed, and sealing a source file would freeze the \
                 implementation: a sealed test lives under a tests/, test/ or __tests__/ directory, is named *.test.* or \
                 *.spec.*, or is a .sh gate script (ADR-2611191120 §8)"
            );
        }
        if targets.iter().any(|(s, _, _)| s == sid) {
            anyhow::bail!("{id}: {sid} is named twice");
        }
        targets.push((sid.to_string(), r.to_string(), true));
    }
    if tests.is_empty() {
        targets = spec
            .specs
            .iter()
            .filter(|s| s.red_commit(&spec).is_none())
            .filter_map(|s| s.test.clone().map(|t| (s.id.clone(), t, false)))
            .collect();
        if targets.is_empty() {
            anyhow::bail!(
                "{id}: nothing to record: every tested scenario has a red commit. Bind a pending one with --test <scenario>=<file>.rs::<fn>"
            );
        }
    }

    println!("{} {id} at {short}: each named test must fail there", "\u{2b21} spec red".cyan().bold());
    let path_env = gate_path(
        std::env::var_os("HOME").map(PathBuf::from).as_deref(),
        &std::env::var("PATH").unwrap_or_default(),
        &|p| p.is_dir(),
    );
    // Where the tests run: here, or through the remote for this commit
    // (ADR-2611191120 §4, §5), refused before anything runs when unpushed.
    let cfg = test_config(&root).map_err(|e| anyhow::anyhow!("{id}: {e}"))?;
    let remote = cfg.remote.as_deref().filter(|_| !local);
    if remote.is_some() {
        remote_preflight(&root, &sha, false).map_err(|e| anyhow::anyhow!("{id}: nothing recorded: {e}"))?;
        println!("  {} the tests go to the remote runner (test.remote) for {short}; --local runs them here", "\u{2192}".cyan());
    }
    // Remote: no checkout here. The template runs from the repository root
    // and the runner checks the commit out (ADR-2611191120 §3, gaberger/hexa#68);
    // the test must be in that commit as written, which git answers without one.
    let target = root.join("target/hexa-prove");
    let wt = match remote {
        Some(_) => root.clone(),
        None => checkout_at(&root, &target, &sha).map_err(|e| anyhow::anyhow!("{id}: {short} cannot be checked out: {e}"))?,
    };
    let prefix = if remote.is_some() { env_prefix(None, None) } else { env_prefix(Some(&path_env), Some(&target)) };
    let planned: Vec<Result<Planned, TestRun>> = targets
        .iter()
        .map(|(_, r, _)| match parse_test_ref(r) {
            Some(t) if remote.is_some() && !file_at(&root, &sha, &t.file) => {
                Err(TestRun::NotFound(format!("{} is not in {short}", t.file)))
            }
            _ => plan_test(&wt, r, &cfg, &prefix),
        })
        .collect();
    let plans: Vec<Planned> = planned.iter().filter_map(|p| p.as_ref().ok().cloned()).collect();
    let at = match remote {
        Some(template) => Where::Remote { template, commit: &sha },
        None => Where::Here,
    };
    let mut runs = run_plans(&wt, at, &plans).await.into_iter();
    let mut not_red: Vec<String> = Vec::new();
    for ((sid, r, _), plan) in targets.iter().zip(planned) {
        // The commit must hold the test as written: nothing is copied in.
        let outcome = match plan {
            Ok(_) => runs.next().unwrap_or_else(|| TestRun::RunnerFailed("no result came back".into())),
            Err(why) => why,
        };
        let why = match outcome {
            TestRun::Failed(_) => None,
            TestRun::Broken { compile: true, .. } => None,
            TestRun::Passed | TestRun::Ambiguous(_) => Some(format!("passes at {short}, so it was not written first")),
            TestRun::Broken { compile: false, tail } => {
                Some(format!("never ran at {short}: {}", tail.lines().last().unwrap_or("")))
            }
            TestRun::NotFound(w) => Some(format!("not at {short}: {w}")),
            TestRun::Vacuous => Some(format!("ran 0 tests at {short}")),
            TestRun::NotProvenHere(w) => Some(format!("not proven here at {short}: {w}")),
            TestRun::RunnerFailed(w) => Some(format!("not shown red at {short}: {}", w.lines().next().unwrap_or(""))),
        };
        match why {
            None => println!("  {} {id} {sid} red at {short} — {}", "\u{2713}".green(), r.dimmed()),
            Some(w) => {
                println!("  {} {id} {sid} {w} — {r}", "\u{2717}".red());
                not_red.push(format!("{sid} {w}"));
            }
        }
    }
    if remote.is_none() {
        remove_checkout(&root, &wt);
    }
    if !not_red.is_empty() {
        anyhow::bail!("{id}: nothing recorded: {}", not_red.join("; "));
    }

    let mut files: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (sid, r, bind) in &targets {
        if let Some(s) = spec.specs.iter_mut().find(|s| &s.id == sid) {
            if *bind {
                s.test = Some(r.clone());
            }
            s.red = Some(short.clone());
        }
        if let Some(t) = parse_test_ref(r) {
            files.insert(t.file);
        }
    }
    let unsealed: Vec<String> = files.into_iter().filter(|f| !spec.sealed.contains_key(f)).collect();
    for f in &unsealed {
        match last_seal(&root, id, f) {
            Some(hash) => {
                let now = std::fs::read(root.join(f)).ok().map(|b| sha256_hex(&b));
                println!("  {} {id} seal of {f} put back at {}…, its last record", "\u{2713}".green(), &hash[..hash.len().min(12)]);
                if now.as_deref() != Some(hash.as_str()) {
                    println!(
                        "  {} {f} changed since that seal: review and record the edit with `hexa spec amend {id} --file {f} --reason \"<why>\"`",
                        "!".yellow()
                    );
                }
                spec.sealed.insert(f.clone(), hash);
            }
            None => println!("  {} {f} has never been sealed: `hexa spec seal {id}`", "!".yellow()),
        }
    }
    save_unchanged(&root, &spec, &read)?;
    println!("\n  Commit {}.", spec_rel(&root, id));
    Ok(())
}

/// The hash the newest committed version of the spec recorded for `file`.
fn last_seal(root: &Path, id: &str, file: &str) -> Option<String> {
    let rel = spec_rel(root, id);
    spec_revisions(root, &rel, 200).iter().find_map(|rev| spec_at(root, rev, &rel).and_then(|s| s.sealed.get(file).cloned()))
}

/// Whether `file` is in the tree of commit `sha`: asked of git, so a remote
/// run needs no checkout here to know it.
fn file_at(root: &Path, sha: &str, file: &str) -> bool {
    std::process::Command::new("git")
        .args(["cat-file", "-e", &format!("{sha}:{file}")])
        .current_dir(root)
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// A detached worktree of `sha` under `target`, at a path no other run uses.
fn checkout_at(root: &Path, target: &Path, sha: &str) -> Result<PathBuf, String> {
    let wt = prove_worktree(target, sha);
    remove_checkout(root, &wt);
    let add = std::process::Command::new("git")
        .args(["worktree", "add", "-q", "--detach"])
        .arg(&wt)
        .arg(sha)
        .current_dir(root)
        .output()
        .map_err(|e| e.to_string())?;
    if !add.status.success() {
        return Err(String::from_utf8_lossy(&add.stderr).trim().to_string());
    }
    hexa_exec::test_exec::link_node_modules(root, &wt);
    Ok(wt)
}

fn remove_checkout(root: &Path, wt: &Path) {
    let _ = std::process::Command::new("git").args(["worktree", "remove", "--force"]).arg(wt).current_dir(root).output();
}

/// A worktree path no other live run uses: the red sha alone is shared by
/// every process proving that commit at once.
fn prove_worktree(target: &Path, sha: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    target.join(format!("wt-{sha}-{}-{n}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(kind: &'static str, reachable: Option<bool>) -> hexa_infer::ports::Found {
        hexa_infer::ports::Found {
            kind,
            name: kind.to_string(),
            detail: String::new(),
            via: String::new(),
            reachable,
            models: Vec::new(),
        }
    }

    /// `claude-text` is a tool-less completion served through the frontier
    /// CLI (ADR-2610270901), so an open frontier is a completion path for it;
    /// for an agent model it is not (ADR-2610221540 §2).
    #[test]
    fn claude_text_through_an_open_frontier_is_a_completion_path() {
        let frontier = [found("frontier", Some(true))];
        assert!(completion_path_open(&frontier, "claude-text:opus"));
        assert!(completion_path_open(&frontier, "claude-text"));
        assert!(!completion_path_open(&frontier, "claude-code/opus"), "an agent CLI is not a completion");
        assert!(!completion_path_open(&[found("frontier", Some(false))], "claude-text:opus"), "a closed frontier serves nothing");
        assert!(completion_path_open(&[found("local", None)], "qwen3:32b"), "an open non-frontier path still counts");
        assert!(!completion_path_open(&[], "claude-text:opus"));
    }

    #[test]
    fn an_id_is_the_timestamp_or_the_stem() {
        assert_eq!(adr_id_of("ADR-2610221540-a-slug"), "ADR-2610221540");
        assert_eq!(adr_id_of("ADR-001"), "ADR-001");
    }

    #[test]
    fn regenerated_tests_are_red_at_the_new_commit_not_a_scenarios_old_one() {
        let mut spec: Spec = serde_json::from_value(serde_json::json!({
            "adr": "ADR-1", "feature": "f", "red": "old0000",
            "specs": [{ "id": "s1", "decision": ["1"], "kind": "example", "description": "d",
                        "test": "tests/old.rs::t", "red": "f4268d3" }]
        }))
        .unwrap();
        let refs = BTreeMap::from([("s1".to_string(), "tests/new.rs::t".to_string())]);
        record_generated(&mut spec, &refs, "new1111", "tests/new.rs");
        assert_eq!(spec.specs[0].test.as_deref(), Some("tests/new.rs::t"));
        assert_eq!(spec.specs[0].red_commit(&spec), Some("new1111"));
    }

    #[test]
    fn a_spec_edited_during_generation_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(SPECS)).unwrap();
        let p = spec_path(dir.path(), "ADR-1");
        let accepted = r#"{"adr":"ADR-1","feature":"f","specs":[],"accepted":{"by":"p","sha256":"x"}}"#;
        std::fs::write(&p, accepted).unwrap();
        let (read, mut spec) = load_text(dir.path(), "ADR-1").unwrap().unwrap();
        // A person edits the spec while the model works; acceptance is gone.
        let edited = r#"{"adr":"ADR-1","feature":"edited","specs":[]}"#;
        std::fs::write(&p, edited).unwrap();
        record_generated(&mut spec, &BTreeMap::new(), "red0000", "tests/t.rs");
        assert!(save_unchanged(dir.path(), &spec, &read).is_err());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), edited, "the edit and its lost acceptance must survive");
        // Unchanged, it saves.
        std::fs::write(&p, accepted).unwrap();
        save_unchanged(dir.path(), &spec, &read).unwrap();
        assert_eq!(load(dir.path(), "ADR-1").unwrap().unwrap().red.as_deref(), Some("red0000"));
    }

    #[test]
    fn a_spec_with_no_tested_scenario_seals_nothing_and_fails() {
        let spec: Spec = serde_json::from_value(serde_json::json!({
            "adr": "ADR-1", "feature": "f",
            "specs": [{ "id": "s1", "decision": ["1"], "kind": "example", "description": "d" }]
        }))
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let err = seal_files(dir.path(), "ADR-1", &spec).expect_err("an empty seal must not succeed");
        assert!(err.to_string().contains("nothing to seal"), "{err}");
    }

    #[test]
    fn re_accepting_changed_behaviour_drops_the_old_tests_and_seal() {
        let mut spec: Spec = serde_json::from_value(serde_json::json!({
            "adr": "ADR-1", "feature": "f", "red": "old0000",
            "sealed": { "tests/old.rs": "h" },
            "specs": [{ "id": "s1", "decision": ["1"], "kind": "example", "description": "d",
                        "then": "the old outcome", "test": "tests/old.rs::t", "red": "f4268d3" }]
        }))
        .unwrap();
        // A first acceptance keeps tests bound before it; re-accepting the
        // same behaviour keeps what was generated for it.
        assert!(!record_acceptance(&mut spec, "p".into(), None));
        assert!(!record_acceptance(&mut spec, "p".into(), None));
        assert_eq!(spec.specs[0].test.as_deref(), Some("tests/old.rs::t"));
        assert!(!spec.sealed.is_empty());
        // The behaviour is hand-edited after sealing, then accepted again.
        spec.specs[0].then = "a new outcome".into();
        assert!(record_acceptance(&mut spec, "p".into(), None));
        assert!(acceptance_holds(&spec).is_ok());
        assert_eq!(spec.specs[0].test, None, "an old test must not hold over new behaviour");
        assert_eq!(spec.specs[0].red_commit(&spec), None);
        assert!(spec.sealed.is_empty(), "a new acceptance needs a new seal");
    }

    #[test]
    fn a_blank_acceptor_is_refused_never_recorded() {
        let none = || None;
        let blank = || Some(String::new());
        let err = acceptor(None, blank, blank).expect_err("an empty $USER must not accept");
        assert!(err.to_string().contains("pass --by"), "{err}");
        assert!(acceptor(None, none, || Some("  ".into())).is_err());
        // A blank --by is refused, not silently replaced by $USER.
        assert!(acceptor(Some("  "), || Some("git".into()), || Some("user".into())).is_err());
        assert_eq!(acceptor(Some("p"), none, none).unwrap(), "p");
        assert_eq!(acceptor(None, || Some("git".into()), || Some("user".into())).unwrap(), "git");
        assert_eq!(acceptor(None, blank, || Some("user".into())).unwrap(), "user");
    }

    #[test]
    fn two_runs_proving_one_red_commit_never_share_a_worktree() {
        let target = Path::new("/r/target/hexa-prove");
        let a = prove_worktree(target, "f4268d3");
        let b = prove_worktree(target, "f4268d3");
        assert_ne!(a, b, "a concurrent run would force-remove the other's live worktree");
        assert_ne!(a, target.join("wt-f4268d3"));
        assert!(a.starts_with(target) && b.starts_with(target));
        assert!(a.file_name().unwrap().to_string_lossy().contains(std::process::id().to_string().as_str()));
    }
}
