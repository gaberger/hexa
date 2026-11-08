//! `hexa spec check | accept | seal | verify` — an ADR's decisions, held to
//! the spec that states them as behaviour (ADR-2610221540).
//!
//! The coverage rule is the domain's, reached through `hexa_core::ports::spec`.
//! This file reads the ledger, hashes, and runs each scenario's test alone.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use colored::Colorize;
use serde_json::Value;
use hexa_core::ports::spec::{coverage, numbered_decisions, test_ref, Acceptance, Finding, Spec};

use hexa_exec::spec_author::{acceptance_holds, content_hash, run_test_alone, sha256_hex, TestRun};

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
fn spec_path(root: &Path, id: &str) -> PathBuf {
    let homes = [
        root.join(SPECS).join(format!("{id}.json")),
        root.join(".hexa/specs").join(format!("{id}.json")),
        root.join("docs/adr").join(format!("{id}.spec.json")),
        root.join(ADRS).join(format!("{id}.spec.json")),
    ];
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
}

fn save(root: &Path, spec: &Spec) -> anyhow::Result<()> {
    write_spec(root, spec, None)
}

/// Write `spec`. The amendments on disk are kept unless `amendments` replaces
/// them: a list that is append-only survives every other write.
fn write_spec(root: &Path, spec: &Spec, amendments: Option<&[Value]>) -> anyhow::Result<()> {
    let p = spec_path(root, &spec.adr);
    let old: Option<Value> = std::fs::read_to_string(&p).ok().and_then(|t| serde_json::from_str(&t).ok());
    let kept: Vec<Value> = match amendments {
        Some(a) => a.to_vec(),
        None => old.as_ref().and_then(|o| o.get("amendments")).and_then(Value::as_array).cloned().unwrap_or_default(),
    };
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
            serde_json::to_string_pretty(&Value::Object(file))?
        }
        _ => serde_json::to_string_pretty(&WithAmendments { spec, amendments: &kept })?,
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

/// `hexa spec accept <ADR>` — a person approves the behaviour (§4).
pub fn accept(adr: &str, by: Option<&str>) -> anyhow::Result<()> {
    let root = std::env::current_dir()?;
    let a = find(&root, adr)?;
    let (text, mut spec) = load_text(&root, &a.id)?.ok_or_else(|| anyhow::anyhow!("{} has no spec to accept", a.id))?;
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
    let previous = accepted_at_head(&root, &a.id);
    if record_acceptance(&mut spec, by.clone(), previous.as_ref()) {
        let unbound: Vec<&str> = spec.specs.iter().filter(|s| s.test.is_none()).map(|s| s.id.as_str()).collect();
        if previous.is_some() && !unbound.is_empty() {
            println!(
                "  {} {} changed where {} changed: those lose their tests, and the files that held them their seals; \
                 every other scenario keeps its proof. Run `hexa spec tests {} --only {}`",
                "!".yellow(),
                a.id,
                unbound.join(", "),
                a.id,
                unbound.join(",")
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

/// The spec as last accepted and committed, if HEAD holds one whose
/// acceptance still matches its content: what a re-acceptance compares
/// against (ADR-2610281628 §1).
fn accepted_at_head(root: &Path, id: &str) -> Option<Spec> {
    let out = std::process::Command::new("git")
        .args(["show", &format!("HEAD:{SPECS}/{id}.json")])
        .current_dir(root)
        .output()
        .ok()?;
    let spec: Spec = serde_json::from_slice(&out.stdout).ok().filter(|_| out.status.success())?;
    acceptance_holds(&spec).ok()?;
    Some(spec)
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
    let options = hexa_exec::spec_author::TestOptions { red_at, replace: replace.clone(), only: None, because: None };
    let model = model_for(model, "t2")?;
    require_a_completion_path(&root, "`hexa spec tests`", &model)?;
    let path_env = gate_path(
        std::env::var_os("HOME").map(PathBuf::from).as_deref(),
        &std::env::var("PATH").unwrap_or_default(),
        &|p| p.is_dir(),
    );
    println!("{} {} on {}", "\u{2b21} spec tests".cyan().bold(), a.id, model);
    let g = hexa_exec::generate_spec_tests(&model, &root, &spec, Path::new(krate), &path_env, &options)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    if let Some(r) = &replace {
        spec.sealed.remove(&r.old_file);
    }
    record_generated(&mut spec, &g.refs, &g.red, &g.file);
    save_unchanged(&root, &spec, &read)?;
    println!("  {} {} tests in {}, every one red at {}", "\u{2713}".green(), g.refs.len(), g.file, g.red);
    seal(&a.id)?;
    println!("\n  Commit {}/{}.json. The tests are sealed: build the decision until they pass.", SPECS, a.id);
    Ok(())
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
    let model = model_for(model, "t2")?;
    let path_env = gate_path(
        std::env::var_os("HOME").map(PathBuf::from).as_deref(),
        &std::env::var("PATH").unwrap_or_default(),
        &|p| p.is_dir(),
    );
    println!("{} {} {} on {}", "\u{2b21} spec tests".cyan().bold(), a.id, only.join(", "), model);
    let options =
        hexa_exec::spec_author::TestOptions { red_at, replace: None, only: Some(only.to_vec()), because: because.clone() };
    let g = hexa_exec::generate_spec_tests(&model, &root, &spec, Path::new(krate), &path_env, &options)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

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
    save(&root, &spec)?;

    let reason = because.unwrap_or_else(|| "left without a test when the spec was re-accepted".to_string());
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
        let Some(t) = s.test.as_deref().filter(|t| test_ref(t).is_none()) else { continue };
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
    let outside: Vec<String> =
        files.iter().filter(|&f| !Path::new(f).iter().any(|c| c == "tests")).cloned().collect();
    if !outside.is_empty() {
        anyhow::bail!(
            "{}: {} not under a tests/ directory; sealing them would freeze the implementation",
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
        .args(["cat-file", "blob", &format!("{commit}:{file}")])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| o.stdout)
}

/// The sha256 a sealed file is anchored to (ADR-2611081151 §1, §2): its blob
/// at the red commit of the scenarios it holds, else at the spec's red
/// commit, else at the first commit in history that added it. `None` when no
/// commit holds it: unanchored.
fn seal_anchor(root: &Path, raw: &Value, file: &str) -> Option<String> {
    let scenario_reds = raw
        .get("specs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|s| s.get("test").and_then(Value::as_str).is_some_and(|t| t.starts_with(file)))
        .filter_map(|s| s.get("red").and_then(Value::as_str));
    let reds: Vec<&str> = scenario_reds.chain(raw.get("red").and_then(Value::as_str)).filter(|r| !r.trim().is_empty()).collect();
    if let Some(bytes) = reds.iter().find_map(|r| blob_at(root, r, file)) {
        return Some(sha256_hex(&bytes));
    }
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
        let anchor = seal_anchor(root, raw, f);
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
            SealVerdict::Unanchored => (
                format!("seal unanchored: no commit in history holds {f}; re-anchor it with `hexa spec tests --red-at <commit>`"),
                ci,
            ),
        };
        let mark = if fails { "\u{2717}".red() } else { "?".yellow() };
        println!("  {} {} {}", mark, id, line);
        fatal += usize::from(fails);
    }
    fatal
}

pub async fn verify(adr: Option<&str>, ci: bool, prove: bool, verbose: bool) -> anyhow::Result<()> {
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
        let alt = is_alt(&text);
        if !print_coverage(a, &spec, alt) {
            fatal += 1;
        }
        if let Err(why) = acceptance_holds(&spec) {
            println!("  {} {} {}", "\u{2717}".red(), a.id, why);
            fatal += 1;
        }
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
            if alt && test_ref(r).is_none() {
                println!("  {} {} {} not run here: `{}` is a bare test name, not `<path>.rs::<fn>`", "\u{25cb}".dimmed(), a.id, s.id, r);
                continue;
            }
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
    if !Path::new(&chosen).iter().any(|c| c == "tests") {
        anyhow::bail!("{id}: {chosen} is not under a tests/ directory; only a sealed test file is amended");
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
        // Under the ignored target/, beside the shared build. The path is this
        // run's own, so a concurrent `--prove` never removes it mid-test; a
        // stale one at this path (a reused pid) is removed first.
        let wt = prove_worktree(&target, &sha);
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
        // A test proved red at a commit from before it was written
        // (`--red-at`, ADR-2610281628 §3) is checked there as written: the
        // file is added to that tree, as the red check added it.
        for (_, r) in &scenarios {
            if let Some((file, _)) = hexa_core::ports::spec::test_ref(r) {
                if !wt.join(file).exists() {
                    if let Ok(body) = std::fs::read(root.join(file)) {
                        let _ = std::fs::create_dir_all(wt.join(file).parent().unwrap_or(&wt));
                        let _ = std::fs::write(wt.join(file), body);
                    }
                }
            }
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
