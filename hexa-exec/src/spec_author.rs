//! The spec author and the test writer (ADR-2610221540 §2, §4, §5).
//!
//! Both are tool-less completions, so each sees exactly its prompt: the ADR
//! and the port and domain files the ADR names, never the implementation. A
//! model served through an agent CLI is refused, because an agent can read
//! the repository for itself. Every generated test must fail against the
//! unimplemented port before it is kept, and the commit that holds it is the
//! spec's `red`.
//!
//! Running one scenario's test alone also lives here, so `hexa spec verify`
//! and the generator judge a test the same way.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use hexa_core::ports::spec::{coverage, numbered_decisions, test_ref, Acceptance, Finding, Spec};
use sha2::{Digest, Sha256};

/// What a skip site prints when `HEXA_NO_SKIP` forbids the skip.
pub const NOT_PROVEN_HERE: &str = "not proven here:";

// ── acceptance (§4) ─────────────────────────────────────────────────────────

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// The hash of what a person accepts: the behaviour, not what follows it.
pub fn content_hash(spec: &Spec) -> String {
    sha256_hex(serde_json::to_string(&spec.accepted_content()).unwrap_or_default().as_bytes())
}

/// `spec`, recorded as accepted by `by` as it stands.
pub fn accept(mut spec: Spec, by: &str) -> Spec {
    let sha256 = content_hash(&spec);
    spec.accepted = Some(Acceptance { by: by.to_string(), sha256 });
    spec
}

/// `Ok` when a person accepted this spec as it stands; `Err` says why not.
pub fn acceptance_holds(spec: &Spec) -> Result<(), String> {
    match &spec.accepted {
        None => Err("not accepted: no person has accepted this spec (`hexa spec accept`)".into()),
        Some(a) if a.sha256 != content_hash(spec) => {
            Err(format!("changed since it was accepted by {}: accept it again", a.by))
        }
        Some(_) => Ok(()),
    }
}

// ── one scenario's test, run alone (§8) ─────────────────────────────────────

pub enum TestRun {
    Passed,
    /// The test ran and failed.
    Failed(String),
    /// It never ran: the build failed (`compile` is true) or cargo did not.
    Broken { compile: bool, tail: String },
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
/// integration test by its target, a library or binary test by its module
/// path in the target that compiles it. A module below `src/` may be compiled
/// by the library or by `main.rs`, so it is run in each one the crate has.
fn cargo_args(krate: &Path, file: &Path, f: &str) -> Option<Vec<String>> {
    let rel = file.strip_prefix(krate).ok()?;
    let manifest = krate.join("Cargo.toml").display().to_string();
    let mut parts: Vec<String> = rel.with_extension("").iter().map(|s| s.to_string_lossy().to_string()).collect();
    let (target, name) = match parts.first().map(String::as_str) {
        Some("tests") if parts.len() == 2 => (vec!["--test".to_string(), parts[1].clone()], f.to_string()),
        Some("src") if parts.get(1).map(String::as_str) == Some("bin") && parts.len() > 2 => {
            // src/bin/x.rs, or src/bin/x/main.rs and the modules beside it.
            let bin = parts[2].clone();
            let mut path: Vec<String> = parts.split_off(3);
            if matches!(path.last().map(String::as_str), Some("main" | "mod")) {
                path.pop();
            }
            path.push(f.to_string());
            (vec!["--bin".to_string(), bin], path.join("::"))
        }
        Some("src") => {
            parts.remove(0);
            let root = parts.len() == 1;
            let target = match parts.last().map(String::as_str) {
                Some("main") if root => vec!["--bins".to_string()],
                Some("lib") if root => vec!["--lib".to_string()],
                _ => {
                    let src = krate.join("src");
                    let has_bin = src.join("main.rs").is_file() || src.join("bin").is_dir();
                    let mut t = Vec::new();
                    if src.join("lib.rs").is_file() || !has_bin {
                        t.push("--lib".to_string());
                    }
                    if has_bin {
                        t.push("--bins".to_string());
                    }
                    t
                }
            };
            if matches!(parts.last().map(String::as_str), Some("lib" | "mod" | "main")) {
                parts.pop();
            }
            parts.push(f.to_string());
            (target, parts.join("::"))
        }
        _ => return None,
    };
    let mut args = vec!["test".to_string(), "--manifest-path".to_string(), manifest];
    args.extend(target);
    // A module ref (`…::`) selects every test under it; a fn ref exactly one.
    let module = name.ends_with("::");
    args.extend(["--".to_string(), name]);
    if !module {
        args.push("--exact".to_string());
    }
    Some(args)
}

/// Run the test `r` (`<path>.rs::<fn>`) alone under `HEXA_NO_SKIP`, from
/// `root`, building into `target_dir` when given.
pub async fn run_test_alone(root: &Path, path_env: &str, r: &str, target_dir: Option<&Path>) -> TestRun {
    let Some((file, f)) = test_ref(r) else { return TestRun::NotFound(format!("{r} is not `<path>.rs::<test_fn>`")) };
    let abs = root.join(file);
    let module = f.ends_with("::");
    let leaf = f.trim_end_matches("::").rsplit("::").next().unwrap_or(f);
    let text = std::fs::read_to_string(&abs).unwrap_or_default();
    let declared = if module { format!("mod {leaf} ") } else { format!("fn {leaf}(") };
    if !text.contains(&declared) {
        return TestRun::NotFound(format!("{leaf} not found in {file}"));
    }
    let Some(args) = crate_of(root, &abs).and_then(|k| cargo_args(&k, &abs, f)) else {
        return TestRun::NotFound(format!("{file} is in no crate's tests/ or src/"));
    };
    let quoted: Vec<String> = args.iter().map(|a| format!("{a:?}")).collect();
    let target = target_dir.map(|t| format!(" CARGO_TARGET_DIR={:?}", t.display().to_string())).unwrap_or_default();
    let shell = format!("export PATH={path_env:?} HEXA_NO_SKIP=1{target}; cargo {}", quoted.join(" "));
    let (ok, out) = crate::direct_exec::run_evidence(&shell, root).await;
    if let Some(line) = out.lines().find(|l| l.contains(NOT_PROVEN_HERE)) {
        let reason = line.split(NOT_PROVEN_HERE).nth(1).unwrap_or("").trim();
        return TestRun::NotProvenHere(reason.trim_matches(|c| c == '"' || c == '\'').to_string());
    }
    if !ok {
        if out.contains("test result: FAILED") {
            return TestRun::Failed(tail(&out, 10));
        }
        let compile = out.contains("could not compile") || out.contains("error[E");
        return TestRun::Broken { compile, tail: tail(&out, 10) };
    }
    let passed: u64 = out
        .lines()
        .filter_map(|l| l.strip_prefix("test result: ok. "))
        .filter_map(|l| l.split(" passed").next()?.trim().parse::<u64>().ok())
        .sum();
    match passed {
        0 => TestRun::Vacuous,
        1 => TestRun::Passed,
        // A module scenario passes when every test in it ran and passed.
        _ if module => TestRun::Passed,
        n => TestRun::Ambiguous(n),
    }
}

fn tail(out: &str, n: usize) -> String {
    let lines: Vec<&str> = out.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n") + "\n"
}

// ── what the author may see (§2) ────────────────────────────────────────────

/// The ADR's file, by id, under `docs/adrs/`.
fn adr_text(root: &Path, adr_id: &str) -> Result<String, String> {
    let dirs = [root.join("docs/adrs"), root.join("docs/adr")];
    for e in dirs.iter().flat_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten()) {
        let name = e.file_name().to_string_lossy().to_string();
        if name == format!("{adr_id}.md") || name.starts_with(&format!("{adr_id}-")) && name.ends_with(".md") {
            return std::fs::read_to_string(e.path()).map_err(|e| e.to_string());
        }
    }
    Err(format!("{adr_id}: no such ADR in docs/adrs/"))
}

/// Every other `.rs` file the ADR names in backquotes that exists, in order
/// of first mention: the writer sees its public signatures, never a body
/// (ADR-2610281442 §1).
fn signature_files(exists: &dyn Fn(&str) -> bool, adr_text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let pieces: Vec<&str> = adr_text.split('`').collect();
    for &span in pieces.iter().take(pieces.len().saturating_sub(1)).skip(1) {
        let span = span.trim();
        if span.ends_with(".rs")
            && !span.contains(char::is_whitespace)
            && !is_contract(span)
            && exists(span)
            && !out.iter().any(|o| o == span)
        {
            out.push(span.to_string());
        }
    }
    out
}

fn signature_block(read: &dyn Fn(&str) -> Option<String>, files: &[String]) -> String {
    files
        .iter()
        .map(|f| {
            let src = read(f).unwrap_or_default();
            format!("--- {f} (public signatures; bodies elided) ---\n{}\n", hexa_core::domain::spec::public_signatures(&src))
        })
        .collect()
}

/// A port or domain file: a path component, or the file's stem, names it.
fn is_contract(path: &str) -> bool {
    let p = Path::new(path);
    p.iter().any(|c| c == "ports" || c == "domain")
        || p.file_stem().is_some_and(|s| s == "ports" || s == "domain")
}

/// The files the author is given: each backquoted path in the ADR that
/// exists and is a port or domain file, in order of first mention. Nothing
/// else is ever read for the prompt. Every piece between two backticks is a
/// candidate, not only the odd ones: one stray backtick, or a four-backtick
/// fence, flips the parity. Prose between spans is never a path: it holds
/// whitespace, which no span counted here may.
fn given_files(root: &Path, adr_text: &str) -> Vec<String> {
    given_files_in(&|p| root.join(p).is_file(), adr_text)
}

/// [`given_files`], with what exists decided by `exists`: the working tree,
/// or the tree of the commit red is proved at.
fn given_files_in(exists: &dyn Fn(&str) -> bool, adr_text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let pieces: Vec<&str> = adr_text.split('`').collect();
    for &span in pieces.iter().take(pieces.len().saturating_sub(1)).skip(1) {
        let pathlike = span.contains('/') && Path::new(span).extension().is_some() && !span.contains(char::is_whitespace);
        if pathlike && is_contract(span) && exists(span) && !out.iter().any(|g| g == span) {
            out.push(span.to_string());
        }
    }
    out
}

fn given_block(root: &Path, given: &[String]) -> String {
    given_block_from(&|g| std::fs::read_to_string(root.join(g)).ok(), given)
}

fn given_block_from(read: &dyn Fn(&str) -> Option<String>, given: &[String]) -> String {
    given.iter().map(|g| format!("--- {g} ---\n{}\n", read(g).unwrap_or_default())).collect()
}

/// A file as it stands in `commit`'s tree.
fn file_at(root: &Path, commit: &str, path: &str) -> Option<String> {
    let out = std::process::Command::new("git").args(["show", &format!("{commit}:{path}")]).current_dir(root).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// A model reachable only through an agent CLI can read the repository for
/// itself, which is what the author must not do.
fn refuse_an_agent(model: &str) -> Result<(), String> {
    if hexa_infer::wiring::served_by_frontier_cli(model) {
        return Err(format!(
            "'{model}' is served by an agent CLI that can read the repository itself, and the spec author and \
             test writer must see only the ADR and its ports (ADR-2610221540 §2). Name a completion model, or \
             set the tier to one"
        ));
    }
    Ok(())
}

/// The largest fenced block in a reply, or the reply when it has none. The
/// largest, not the first: a short note in its own fence before the code was
/// taken for the code.
fn block(reply: &str) -> String {
    let t = reply.trim();
    // Fences are lines: one opens on its own line and the next line that is
    // only ``` closes it. A ``` inside a string in the body is neither, so a
    // test of a model reply carrying a code fence stays one block, and a note
    // in a fence of its own before the code is a second, smaller one.
    let mut blocks: Vec<String> = Vec::new();
    let mut open: Option<Vec<&str>> = None;
    for line in t.lines() {
        match open.as_mut() {
            Some(body) if line.trim() == "```" => {
                blocks.push(body.join("\n").trim().to_string());
                open = None;
            }
            Some(body) => body.push(line),
            None => {
                let l = line.trim_start();
                // A fence that opens and closes on one line is read below.
                if l.starts_with("```") && !l[3..].contains("```") {
                    open = Some(Vec::new());
                }
            }
        }
    }
    if let Some(body) = open {
        blocks.push(body.join("\n").trim().to_string());
    }
    if let Some(best) = blocks.into_iter().max_by_key(String::len) {
        return best;
    }
    if !t.contains("```") {
        return t.to_string();
    }
    // Segments alternate outside, inside, outside, ...; an unclosed last
    // fence runs to the end of the reply.
    t.split("```")
        .skip(1)
        .step_by(2)
        .map(|inside| match inside.split_once('\n') {
            Some((_tag, rest)) => rest.trim().to_string(),
            // One line, like ```json {...}```: the language tag is the word before the body.
            None => inside.trim_start_matches(|c: char| c.is_ascii_alphanumeric() || "_+-".contains(c)).trim().to_string(),
        })
        .max_by_key(String::len)
        .unwrap_or_default()
}

/// Scenarios per test-writer call. One answer for 29 concrete scenarios kept
/// only the last four (strata's SQLite storage spec, 2026-11-02), so a larger spec is written in
/// batches, each a module of the one file.
const TESTS_PER_CALL: usize = 10;

/// Where a test writer's raw answer is kept, so a rejected one can be read.
fn keep_answer(adr: &str, part: usize, answer: &str) -> Option<PathBuf> {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dir = std::env::temp_dir().join("hexa-spec-tests");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("{adr}-{nanos}-part{part}.txt"));
    std::fs::write(&path, answer).ok()?;
    Some(path)
}

const AUTHOR: &str = "You are the spec author. Turn an architecture decision record into behaviour: \
given / when / then scenarios a person can read and a test can check. You are given the ADR and the \
port and domain signatures it names, and nothing else; you have not seen and must not guess at any \
implementation. Every numbered decision (§n) needs at least one `example` scenario and at least one \
`negative` scenario (what the decision must refuse). Every decision you list as `universal` also needs \
at least one `property` scenario (the rule, stated over all inputs). Cite decisions as \"§n\". Answer with one JSON \
object and nothing else.";

#[derive(Debug)]
pub struct Drafted {
    pub spec: Spec,
    /// What the coverage rule says about the draft. A draft is checked like
    /// anyone's (§3).
    pub findings: Vec<Finding>,
    /// The files the author was given, beside the ADR.
    pub given: Vec<String>,
}

/// `hexa spec draft` (§2): the ADR and its ports, to a tool-less model.
pub async fn draft(
    backends: &dyn hexa_infer::ports::Backends,
    model: &str,
    root: &Path,
    adr_id: &str,
) -> Result<Drafted, String> {
    refuse_an_agent(model)?;
    let text = adr_text(root, adr_id)?;
    let decisions = numbered_decisions(&text);
    let given = given_files(root, &text);
    let user = format!(
        "The ADR, {adr_id}:\n\n{text}\n\nThe port and domain files it names:\n\n{}\n\
         Its numbered decisions: {}.\n\nAnswer with:\n\
         {{\"adr\":\"{adr_id}\",\"feature\":\"<short name>\",\"description\":\"<one line>\",\
         \"universal\":[\"§n\" for each decision that states a rule over all inputs],\
         \"specs\":[{{\"id\":\"S01\",\"decision\":[\"§1\"],\"kind\":\"example|negative|property\",\
         \"description\":\"…\",\"given\":\"…\",\"when\":\"…\",\"then\":\"…\"}}]}}",
        given_block(root, &given),
        decisions.join(", ")
    );
    let reply = hexa_infer::complete::complete_text_fresh_with(backends, model, AUTHOR, &user, 8000).await?;
    let mut spec: Spec = parse_draft(&block(&reply), adr_id).map_err(|e| format!("the author's reply is not a spec: {e}"))?;
    // A draft claims nothing that only happens after it: no acceptance, no
    // seal, no tests, no red commit.
    spec.adr = adr_id.to_string();
    spec = spec.accepted_content();
    let findings = coverage(adr_id, &decisions, &spec);
    Ok(Drafted { spec, findings, given })
}

/// The author's reply as a spec: a whole spec object, or the bare list of
/// scenarios it was asked for, which is given the ADR's id as its feature.
fn parse_draft(text: &str, adr_id: &str) -> Result<Spec, serde_json::Error> {
    match serde_json::from_str::<serde_json::Value>(text)? {
        serde_json::Value::Array(specs) => {
            serde_json::from_value(serde_json::json!({ "adr": adr_id, "feature": adr_id, "specs": specs }))
        }
        other => serde_json::from_value(other),
    }
}

// ── tests, red before the code (§5) ─────────────────────────────────────────

const WRITER: &str = "You write Rust integration tests from accepted behaviour scenarios. Write one #[test] \
function per scenario. Put a doc comment line `/// <scenario id>: <description>` directly above each \
test. Use only the crate's public API shown in the signatures you are given; you have not seen the \
implementation. Each test must check the scenario's `then`, so it fails until the decision is built. \
Answer with the whole file in one ```rust block and nothing else.";

#[derive(Debug)]
pub struct Generated {
    /// The test file, repo-relative.
    pub file: String,
    /// Scenario id → `<file>::<fn>`.
    pub refs: BTreeMap<String, String>,
    /// The commit that holds the tests, while every one of them fails.
    pub red: String,
}

/// The crate's library name, from its manifest: `[lib] name`, else the
/// package name.
fn lib_name(manifest: &str) -> Option<String> {
    let toml: toml::Value = toml::from_str(manifest).ok()?;
    let name = |table: &str| toml.get(table)?.get("name")?.as_str().map(str::to_string);
    name("lib").or_else(|| name("package")).map(|v| v.replace('-', "_"))
}

/// The first line of every generated test file: it is sealed, so style lints
/// on it can only block a gate, never be fixed.
const GENERATED_HEADER: &str = "#![allow(unused, clippy::all)]";

/// Format `path` with rustfmt for `edition`, and return the formatted text.
/// `None` when rustfmt is missing or fails (the compile check that follows
/// then reports the file as it is).
fn rustfmt_in_place(path: &Path, edition: Option<&str>, path_env: &str) -> Option<String> {
    let mut cmd = std::process::Command::new("rustfmt");
    if let Some(e) = edition {
        cmd.args(["--edition", e]);
    }
    if !path_env.is_empty() {
        cmd.env("PATH", path_env);
    }
    let ok = cmd.arg(path).output().ok()?.status.success();
    if !ok {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// The crate's Rust edition: inline, or inherited from the workspace root's
/// `[workspace.package]`. A writer that does not know it writes 2021 code in a
/// 2024 crate (`gen` is reserved there).
fn crate_edition(root: &Path, manifest: &str) -> Option<String> {
    let toml: toml::Value = toml::from_str(manifest).ok()?;
    match toml.get("package")?.get("edition")? {
        toml::Value::String(e) => Some(e.clone()),
        toml::Value::Table(t) if t.get("workspace").and_then(|w| w.as_bool()) == Some(true) => {
            let root_toml: toml::Value = toml::from_str(&std::fs::read_to_string(root.join("Cargo.toml")).ok()?).ok()?;
            root_toml.get("workspace")?.get("package")?.get("edition")?.as_str().map(str::to_string)
        }
        _ => None,
    }
}

/// The crates a test in this crate can name: its own library, its normal and
/// dev dependencies (as `use` paths spell them, `-` as `_`, renames by their
/// key), in order, without repeats.
fn test_crates(lib: &str, manifest: &str) -> Vec<String> {
    let mut out = vec![lib.to_string()];
    if let Ok(toml) = toml::from_str::<toml::Value>(manifest) {
        for table in ["dependencies", "dev-dependencies"] {
            if let Some(deps) = toml.get(table).and_then(|t| t.as_table()) {
                for name in deps.keys() {
                    let n = name.replace('-', "_");
                    if !out.contains(&n) {
                        out.push(n);
                    }
                }
            }
        }
    }
    out
}

/// The crate's `[[bin]]` names, from its manifest.
fn bin_names(manifest: &str) -> Vec<String> {
    let Ok(toml) = toml::from_str::<toml::Value>(manifest) else { return Vec::new() };
    toml.get("bin")
        .and_then(|b| b.as_array())
        .map(|bins| bins.iter().filter_map(|b| b.get("name")?.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

/// The Rust path a given file is reached by, e.g. `hexa_core::domain::spec`
/// for `hexa-core/src/domain/spec.rs`: the library of the crate that holds
/// it, then its module path under `src/`. A given file may live in another
/// crate than the tests do, and the writer must not guess which.
fn module_of(root: &Path, file: &str) -> Option<String> {
    let abs = root.join(file);
    let krate = crate_of(root, &abs)?;
    let lib = lib_name(&std::fs::read_to_string(krate.join("Cargo.toml")).ok()?)?;
    let rel = abs.strip_prefix(krate.join("src")).ok()?.with_extension("");
    let mut parts: Vec<String> = rel.iter().map(|s| s.to_string_lossy().to_string()).collect();
    if matches!(parts.last().map(String::as_str), Some("lib" | "mod" | "main")) {
        parts.pop();
    }
    Some(std::iter::once(lib).chain(parts).collect::<Vec<_>>().join("::"))
}

/// Scenario id → test fn, from `/// <id>` doc lines directly above a test
/// `fn`. Only doc lines, attributes and blank lines may stand between them,
/// and one attribute must mark the fn a test; anything else unbinds the id.
fn tests_by_scenario(body: &str, ids: &[&str]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    // The id waiting for its fn, and whether a test attribute has been seen.
    let mut pending: Option<(String, bool)> = None;
    for line in body.lines() {
        let t = line.trim();
        if let Some(doc) = t.strip_prefix("///") {
            let first = doc.split_whitespace().next().unwrap_or("").trim_end_matches(':');
            if ids.contains(&first) {
                pending = Some((first.to_string(), false));
            }
            continue;
        }
        let Some((sid, is_test)) = pending.as_mut() else { continue };
        if t.is_empty() {
            continue;
        }
        if let Some(attr) = t.strip_prefix("#[") {
            let path: String = attr.chars().take_while(|c| !matches!(c, '(' | ']')).collect();
            let path = path.trim();
            *is_test |= path == "test" || path.ends_with("::test");
            continue;
        }
        let words: Vec<&str> = t.split_whitespace().collect();
        let qualifier = |w: &&str| matches!(*w, "pub" | "async" | "unsafe" | "const" | "extern") || w.starts_with("pub(");
        if let Some(at) = words.iter().position(|w| *w == "fn").filter(|&at| words[..at].iter().all(qualifier)) {
            let rest = words.get(at + 1).copied().unwrap_or("");
            let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            if *is_test && !name.is_empty() {
                out.insert(sid.clone(), name);
            }
        } else if let Some(at) = words.iter().position(|w| *w == "mod").filter(|&at| words[..at].iter().all(qualifier)) {
            // A scenario that is a suite: every test in the module is its
            // test. The trailing `::` marks the ref as a module.
            let rest = words.get(at + 1).copied().unwrap_or("");
            let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            if !name.is_empty() {
                out.insert(sid.clone(), format!("{name}::"));
            }
        }
        pending = None;
    }
    out
}

async fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = tokio::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Every test in `refs` run alone in a checkout of `head` plus `file` holding
/// `body`: the tree the `red` commit holds, which `git commit -- <file>`
/// builds without the working tree's other edits. Returns why each rejected
/// test is not red there.
async fn red_at(
    root: &Path,
    path_env: &str,
    head: &str,
    file: &str,
    body: &str,
    refs: &BTreeMap<String, String>,
) -> Result<RedCheck, String> {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let wt = std::env::temp_dir().join(format!("hexa-spec-red-{}-{nanos}", std::process::id()));
    let wt_s = wt.to_string_lossy().to_string();
    git(root, &["worktree", "add", "-q", "--detach", &wt_s, head]).await?;
    // A build of its own: an older tree built into the shared target/ leaves
    // artifacts newer than HEAD's sources, and cargo then runs HEAD's tests
    // against the old code.
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"))
        .join("hexa-red");
    let written = std::fs::create_dir_all(wt.join(file).parent().unwrap_or(&wt)).and_then(|_| std::fs::write(wt.join(file), body));
    if let Err(e) = written {
        let _ = git(root, &["worktree", "remove", "--force", &wt_s]).await;
        return Err(format!("{file} could not be checked at {head}: {e}"));
    }
    // The file must build before any one test is run: one compile, whose
    // messages a follow-up can carry (ADR-2610281516 §5).
    if let Some(out) = compile_errors(&wt, path_env, &target, file).await {
        let _ = git(root, &["worktree", "remove", "--force", &wt_s]).await;
        return Ok(RedCheck::DoesNotCompile(out));
    }
    let mut rejected = Vec::new();
    for (sid, r) in refs {
        match run_test_alone(&wt, path_env, r, Some(&target)).await {
            TestRun::Failed(_) => {}
            TestRun::Broken { tail, .. } => rejected.push(format!("{sid} never ran:\n{tail}")),
            TestRun::Passed | TestRun::Ambiguous(_) => {
                rejected.push(format!("{sid} ({r}) passes before the code exists, so it tests nothing"))
            }
            TestRun::NotFound(why) => rejected.push(format!("{sid}: {why}")),
            TestRun::Vacuous => rejected.push(format!("{sid} ran 0 tests")),
            TestRun::NotProvenHere(why) => rejected.push(format!("{sid} is not proven here: {why}")),
        }
    }
    let _ = git(root, &["worktree", "remove", "--force", &wt_s]).await;
    Ok(RedCheck::Rejected(rejected))
}

/// What the red check found.
#[derive(Debug)]
enum RedCheck {
    /// The test file does not compile; cargo's output.
    DoesNotCompile(String),
    /// Why each rejected test is not red; empty when every one is.
    Rejected(Vec<String>),
}

impl RedCheck {
    /// Why each test is not red, a compile failure counting as one reason.
    #[cfg(test)]
    fn into_rejected(self) -> Vec<String> {
        match self {
            RedCheck::DoesNotCompile(out) => vec![format!("the test file does not compile:\n{out}")],
            RedCheck::Rejected(r) => r,
        }
    }
}

enum Rejection {
    /// Something a follow-up can carry back to the writer.
    Mendable(String),
    Final(Vec<String>),
}

/// cargo's output when the test file at `file` does not build, else `None`.
async fn compile_errors(wt: &Path, path_env: &str, target: &Path, file: &str) -> Option<String> {
    let abs = wt.join(file);
    let krate = crate_of(wt, &abs)?;
    let stem = abs.file_stem()?.to_string_lossy().to_string();
    let manifest = krate.join("Cargo.toml").display().to_string();
    let shell = format!(
        "export PATH={path_env:?} HEXA_NO_SKIP=1 CARGO_TARGET_DIR={:?}; cargo test --manifest-path {manifest:?} --test {stem:?} --no-run",
        target.display().to_string()
    );
    let (ok, out) = crate::direct_exec::run_evidence(&shell, wt).await;
    (!ok && (out.contains("error[E") || out.contains("could not compile"))).then_some(out)
}

/// Why any test of a file that does not build at `commit` is not red there,
/// checked one test at a time: the file with every other test removed. A
/// test that does not build alone was written before its API; one that
/// builds must fail (ADR-2610281628 §3).
async fn written_first(
    root: &Path,
    path_env: &str,
    commit: &str,
    file: &str,
    body: &str,
    refs: &BTreeMap<String, String>,
) -> Result<Vec<String>, String> {
    let names: Vec<String> = refs.values().filter_map(|r| r.rsplit("::").next().map(str::to_string)).collect();
    let mut rejected = Vec::new();
    for (sid, r) in refs {
        let me = r.rsplit("::").next().unwrap_or(r);
        let others: Vec<String> = names.iter().filter(|n| *n != me).cloned().collect();
        let alone = hexa_core::domain::spec::without_tests(body, &others);
        let one = BTreeMap::from([(sid.clone(), r.clone())]);
        match red_at(root, path_env, commit, file, &alone, &one).await? {
            RedCheck::DoesNotCompile(_) => {}
            RedCheck::Rejected(r) => rejected.extend(r),
        }
    }
    Ok(rejected)
}

/// cargo's output when `file` holding `body` does not build on `commit`'s
/// tree, else `None`.
async fn builds_at(root: &Path, path_env: &str, commit: &str, file: &str, body: &str) -> Result<Option<String>, String> {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let wt = std::env::temp_dir().join(format!("hexa-spec-head-{}-{nanos}", std::process::id()));
    let wt_s = wt.to_string_lossy().to_string();
    git(root, &["worktree", "add", "-q", "--detach", &wt_s, commit]).await?;
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"))
        .join("hexa-red");
    let written = std::fs::create_dir_all(wt.join(file).parent().unwrap_or(&wt)).and_then(|_| std::fs::write(wt.join(file), body));
    let out = match written {
        Ok(()) => Ok(compile_errors(&wt, path_env, &target, file).await),
        Err(e) => Err(format!("{file} could not be checked at {commit}: {e}")),
    };
    let _ = git(root, &["worktree", "remove", "--force", &wt_s]).await;
    out
}

/// The compiler's messages about the test file alone: every `error` line,
/// and the source excerpts that point into `file`. An excerpt from any other
/// file — a project source a type error quotes — is dropped, so a follow-up
/// never carries code the writer must not see (ADR-2610281516 §5).
pub(crate) fn test_file_diagnostics(output: &str, file: &str) -> String {
    let name = Path::new(file).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let mut out: Vec<&str> = Vec::new();
    let mut excerpt = false;
    let mut in_warning = false;
    for line in output.lines() {
        let t = line.trim_start();
        if t.starts_with("error") {
            in_warning = false;
            excerpt = false;
            out.push(line);
            continue;
        }
        if t.starts_with("warning") {
            in_warning = true;
            excerpt = false;
            continue;
        }
        if in_warning {
            continue;
        }
        if let Some(loc) = t.strip_prefix("--> ").or_else(|| t.strip_prefix("::: ")) {
            excerpt = loc.contains(file) || (!name.is_empty() && loc.contains(&name));
            if excerpt {
                out.push(line);
            }
            continue;
        }
        let is_excerpt = t.starts_with('|')
            || t.split_once('|').is_some_and(|(n, _)| !n.trim().is_empty() && n.trim().chars().all(|c| c.is_ascii_digit()))
            || t == "...";
        if is_excerpt {
            if excerpt {
                out.push(line);
            }
            continue;
        }
        if t.starts_with("= ") || t.starts_with("note") || t.starts_with("help") {
            out.push(line);
        }
    }
    out.join("\n")
}

/// `hexa spec tests` (§5): one test per scenario of an accepted spec, from
/// the scenarios and the ports alone. Kept only if every test fails now.
pub async fn generate_tests(
    backends: &dyn hexa_infer::ports::Backends,
    model: &str,
    root: &Path,
    spec: &Spec,
    krate: &Path,
    path_env: &str,
) -> Result<Generated, String> {
    generate_tests_with(backends, model, root, spec, krate, path_env, &TestOptions::default()).await
}

/// [`generate_tests`], with red proved at an earlier commit or a sealed
/// file replaced (ADR-2610281442 §3–§4).
pub async fn generate_tests_with(
    backends: &dyn hexa_infer::ports::Backends,
    model: &str,
    root: &Path,
    spec: &Spec,
    krate: &Path,
    path_env: &str,
    options: &TestOptions,
) -> Result<Generated, String> {
    acceptance_holds(spec).map_err(|why| format!("{}: {why}. Tests are generated from an accepted spec", spec.adr))?;
    if spec.specs.is_empty() {
        // No scenario, no test to fail: a `red` commit would prove nothing.
        return Err(format!("{}: the spec has no scenarios, so there is no test to fail before the code", spec.adr));
    }
    refuse_an_agent(model)?;
    let manifest = std::fs::read_to_string(root.join(krate).join("Cargo.toml")).map_err(|_| {
        format!(
            "{}: no Cargo.toml. Generated tests are Rust for now; TypeScript and Go are not generated yet",
            root.join(krate).display()
        )
    })?;
    let lib = lib_name(&manifest).ok_or("the crate's manifest names no package")?;
    let text = adr_text(root, &spec.adr)?;

    // Where red is proved: HEAD, or an earlier commit the code postdates (§3).
    // Re-proving named scenarios always builds its red commit, from HEAD's
    // tree unless told otherwise (ADR-2610281628 §2).
    let red_at_or_head = options.red_at.clone().or_else(|| options.only.as_ref().map(|_| "HEAD".to_string()));
    let base = match &red_at_or_head {
        None => None,
        Some(c) => {
            let c = git(root, &["rev-parse", "--verify", &format!("{c}^{{commit}}")]).await.map_err(|_| format!("{c} is not a commit"))?;
            let ancestor = tokio::process::Command::new("git")
                .args(["merge-base", "--is-ancestor", &c, "HEAD"])
                .current_dir(root)
                .status()
                .await
                .map_err(|e| e.to_string())?;
            if !ancestor.success() {
                return Err(format!("{c} is not an ancestor of HEAD: red must be proved at a commit this branch grew from"));
            }
            Some(c)
        }
    };
    // What the writer sees comes from the tree red is proved at: the API that
    // existed before the code, and never the code (ADR-2610221540 §2).
    // A file that did not exist there yet is read from the working tree: a
    // test that leans on it cannot build at the red commit and is refused.
    let read = |p: &str| match &base {
        Some(c) => file_at(root, c, p).or_else(|| std::fs::read_to_string(root.join(p)).ok()),
        None => std::fs::read_to_string(root.join(p)).ok(),
    };
    let exists = |p: &str| read(p).is_some();
    let given = given_files_in(&exists, &text);
    let targets: Vec<hexa_core::ports::spec::Scenario> = spec
        .accepted_content()
        .specs
        .into_iter()
        .filter(|s| options.only.as_ref().is_none_or(|only| only.contains(&s.id)))
        .collect();
    let scenarios = serde_json::to_string_pretty(&targets).map_err(|e| e.to_string())?;
    let crates: Vec<String> = test_crates(&lib, &manifest).iter().map(|c| format!("`{c}`")).collect();
    let crates = format!("The tests may use only these crates: {}, std.\n", crates.join(", "));
    let crates = match crate_edition(root, &manifest) {
        Some(e) => format!("The crate is Rust edition {e}; write code valid in that edition.\n{crates}"),
        None => crates,
    };
    let paths: String = given
        .iter()
        .filter_map(|g| module_of(root, g).map(|m| format!("`{g}` is reached as `{m}`.\n")))
        .collect();
    let bins: String = bin_names(&manifest)
        .iter()
        .map(|b| format!("The crate's `{b}` binary is `env!(\"CARGO_BIN_EXE_{b}\")`.\n"))
        .collect();
    // Signatures alone show no code, so they come from the working tree: the
    // API as it stands, which an amended scenario may rely on. Only what is
    // shown whole comes from the red commit (ADR-2610281628).
    let here = |p: &str| std::fs::read_to_string(root.join(p)).ok();
    let signatures = signature_files(&|p: &str| here(p).is_some(), &text);
    let sig_paths: String = signatures
        .iter()
        .filter_map(|g| module_of(root, g).map(|m| format!("`{g}` is reached as `{m}`.\n")))
        .collect();
    // Re-proving beside sealed tests: those tests, less the ones being
    // replaced, show the imports and helpers that build. They were written
    // without the code too, so they show none of it (ADR-2610281628 §2).
    let siblings: String = match &options.only {
        None => String::new(),
        Some(only) => {
            let mut out = String::new();
            for f in spec.test_files() {
                let Ok(src) = std::fs::read_to_string(root.join(&f)) else { continue };
                let replaced: Vec<String> = tests_by_scenario(&src, &only.iter().map(String::as_str).collect::<Vec<_>>()).into_values().collect();
                let kept = hexa_core::domain::spec::without_tests(&src, &replaced);
                out.push_str(&format!("--- {f} (sealed; the new file sits beside it) ---\n{kept}\n"));
            }
            let mut out = if out.is_empty() {
                out
            } else {
                format!("The tests already sealed for this spec; reuse their imports and helpers, and name no function the same:\n\n{out}\n")
            };
            if let Some(why) = &options.because {
                out.push_str(&format!(
                    "These scenarios are re-proved because: {why}\nThe tests they replace failed for that reason; write the new ones so that they do not.\n\n"
                ));
            }
            out
        }
    };
    let prompt = |scenarios: &str, batch: &str| {
        format!(
            "The crate is `{lib}`; tests reach it as `{lib}::…`.\n{crates}{paths}{sig_paths}{bins}\n{siblings}The ADR:\n\n{text}\n\n\
             The port and domain signatures:\n\n{}\nThe public API of the other files the ADR names:\n\n{}\n\
             {batch}The accepted scenarios:\n\n{scenarios}\n",
            given_block_from(&read, &given),
            signature_block(&here, &signatures)
        )
    };
    let user = prompt(&scenarios, "");
    let ids: Vec<&str> = targets.iter().map(|s| s.id.as_str()).collect();
    // A spec of many scenarios is written in batches, each a module of the one
    // file: one answer for 29 scenarios kept only the last four.
    let parts: Vec<&[hexa_core::ports::spec::Scenario]> = targets.chunks(TESTS_PER_CALL).collect();
    let batched = parts.len() > 1;
    let edition = crate_edition(root, &manifest);
    let stem = format!("{}_spec", spec.adr.to_lowercase().replace('-', "_"));
    let file = match options.only {
        // A new file beside the sealed ones: the first free `<stem>_<n>.rs`.
        Some(_) => (2..).map(|n| krate.join("tests").join(format!("{stem}_{n}.rs"))).find(|f| !root.join(f).exists()).unwrap_or_default(),
        None => krate.join("tests").join(format!("{stem}.rs")),
    };
    let file = file.strip_prefix(".").unwrap_or(&file).to_string_lossy().to_string();
    let abs = root.join(&file);
    match &options.replace {
        None if abs.exists() && options.only.is_none() => {
            return Err(format!(
                "{file} exists: a regeneration is a new seal; replace it with --replace --because \"<reason>\""
            ))
        }
        Some(r) if !root.join(&r.old_file).exists() => return Err(format!("{} is not there to replace", r.old_file)),
        _ => {}
    }
    let head = git(root, &["rev-parse", "HEAD"]).await?;

    // One generation, then at most two follow-ups when the file is rejected
    // for something the writer can mend: a scenario with no test, a site
    // that cannot pass, or the file not compiling (ADR-2610281516 §5). Each
    // follow-up is a fresh completion carrying the scenarios, the writer's
    // last reply and the error about it, never source from outside the test
    // file; each repaired file passes every check again from the start.
    const REPAIRS: usize = 2;
    let mut ask = user.clone();
    let mut repairs = 0;
    let (body, names, red_commit) = loop {
        let (reply, body, names, mendable) = if batched {
            let (body, names) = write_batches(backends, model, &spec.adr, &parts, &prompt, REPAIRS).await?;
            (String::new(), body, names, None)
        } else {
            let reply = hexa_infer::complete::complete_text_fresh_with(backends, model, WRITER, &ask, 8000).await?;
            let body = block(&reply) + "\n";
            let names = tests_by_scenario(&body, &ids);
            let mendable = unmendable_reason(&body, &ids, &names);
            (reply, body, names, mendable)
        };
        // Sealed, so written formatted and lint-exempt: `cargo fmt` is then a
        // no-op on it, and a style lint can never fail a gate on it.
        let body = sealed_form(&body, &spec.adr, edition.as_deref(), path_env);
        let outcome = match mendable {
            Some(why) => Err(Rejection::Mendable(why)),
            None => {
                let refs: BTreeMap<String, String> =
                    names.iter().map(|(sid, f)| (sid.clone(), format!("{file}::{f}"))).collect();
                let red_commit = match &base {
                    None => None,
                    Some(c) => Some(red_commit_at(root, c, &file, &body, &spec.adr).await?),
                };
                let check_at = red_commit.clone().unwrap_or_else(|| head.clone());
                match red_at(root, path_env, &check_at, &file, &body, &refs).await {
                    Err(e) => Err(Rejection::Final(vec![e])),
                    // Written before its API: red at that commit when the
                    // file builds at HEAD, as `--prove` counts it
                    // (ADR-2610281628 §3).
                    Ok(RedCheck::DoesNotCompile(_)) if base.as_deref().is_some_and(|b| b != head) => {
                        match builds_at(root, path_env, &head, &file, &body).await {
                            // Each test on its own: one that builds and
                            // passes there is no gate, however its file fared.
                            Ok(None) => match written_first(root, path_env, base.as_deref().unwrap_or(&head), &file, &body, &refs).await {
                                Ok(r) if r.is_empty() => Ok(base.clone()),
                                Ok(r) => Err(Rejection::Final(r)),
                                Err(e) => Err(Rejection::Final(vec![e])),
                            },
                            Ok(Some(at_head)) => Err(Rejection::Mendable(format!(
                                "The test file does not build at HEAD:\n\n{}",
                                test_file_diagnostics(&at_head, &file)
                            ))),
                            Err(e) => Err(Rejection::Final(vec![e])),
                        }
                    }
                    Ok(RedCheck::DoesNotCompile(out)) => Err(Rejection::Mendable(format!(
                        "The test file does not compile (it does not build at HEAD):\n\n{}",
                        test_file_diagnostics(&out, &file)
                    ))),
                    Ok(RedCheck::Rejected(r)) if !r.is_empty() => Err(Rejection::Final(r)),
                    Ok(RedCheck::Rejected(_)) => Ok(red_commit),
                }
            }
        };
        match outcome {
            Ok(red_commit) => break (body, names, red_commit),
            Err(Rejection::Mendable(why)) if repairs < REPAIRS && !batched => {
                repairs += 1;
                ask = format!(
                    "The crate is `{lib}`; tests reach it as `{lib}::…`.\n{crates}{bins}\nThe accepted scenarios:\n\n{scenarios}\n\n\
                     Your previous reply:\n\n{reply}\n\nIt was rejected. {why}\n\n\
                     Reply with the whole corrected test file, one fenced block."
                );
            }
            Err(Rejection::Mendable(why)) => {
                let kept = keep_answer(&spec.adr, 1, &reply).map(|p| format!("\n  raw answer: {}", p.display())).unwrap_or_default();
                return Err(format!("generation rejected after {repairs} repair(s), nothing kept: {why}{kept}"));
            }
            Err(Rejection::Final(r)) => return Err(format!("generation rejected, nothing kept:\n  {}", r.join("\n  "))),
        }
    };
    if git(root, &["rev-parse", "HEAD"]).await? != head {
        return Err("generation rejected, nothing kept: HEAD moved while the tests ran, so their red is not the commit's".into());
    }
    let refs: BTreeMap<String, String> = names.iter().map(|(sid, f)| (sid.clone(), format!("{file}::{f}"))).collect();

    if let Some(parent) = abs.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    if options.only.is_some() {
        // The caller commits the new file with the old one trimmed and the
        // spec rebound, as one commit (ADR-2610281628 §2).
        std::fs::write(&abs, &body).map_err(|e| e.to_string())?;
        let red = red_commit.ok_or("re-proving needs a red commit")?;
        git(root, &["update-ref", &format!("refs/hexa/red/{}", spec.adr), &red]).await?;
        let red = git(root, &["rev-parse", "--short", &red]).await?;
        return Ok(Generated { file, refs, red });
    }
    let previous = options.replace.as_ref().and_then(|_| std::fs::read(&abs).ok());
    std::fs::write(&abs, &body).map_err(|e| e.to_string())?;
    let unsealed = Unsealed { root: root.to_path_buf(), file: file.clone(), previous };
    let message = match &options.replace {
        Some(r) => format!(
            "test(spec): {} tests replaced: {} regenerated from the accepted spec\n\n\
             Removed {}: {}\n\nThe acceptance is unchanged; only the tests are new (ADR-2610281442 §4).{}",
            spec.adr,
            file,
            r.old_file,
            r.because,
            red_commit.as_deref().map(|c| format!(" Every test fails at {c}.")).unwrap_or_default()
        ),
        None => format!(
            "test(spec): {} scenarios, generated from the accepted spec (failing)\n\n\
             Written by hexa's test writer from the scenarios and the ports alone (ADR-2610221540 §5).\n\
             Every test fails {}, before the decision is built.",
            spec.adr,
            red_commit.as_deref().map(|c| format!("at {c}")).unwrap_or_else(|| "here".into())
        ),
    };
    let old = options.replace.as_ref().map(|r| r.old_file.clone()).filter(|o| *o != file);
    let committed = seal(unsealed, &body, &message, old.as_deref()).await?;
    let red = match red_commit {
        Some(c) => {
            git(root, &["update-ref", &format!("refs/hexa/red/{}", spec.adr), &c]).await?;
            git(root, &["rev-parse", "--short", &c]).await?
        }
        None => committed,
    };
    Ok(Generated { file, refs, red })
}

/// Why a reply's tests cannot be kept as they are, when the writer can mend it:
/// a scenario with no test, or a test that cannot pass (ADR-2610281442 §2).
fn unmendable_reason(body: &str, ids: &[&str], names: &BTreeMap<String, String>) -> Option<String> {
    let missing: Vec<&str> = ids.iter().copied().filter(|i| !names.contains_key(*i)).collect();
    let sites = hexa_core::domain::spec::unpassable_sites(body);
    if !missing.is_empty() {
        // The line is there but binds nothing: it stands inside a function or
        // below the attribute. Say so, and where it goes.
        let misplaced: Vec<&str> = missing
            .iter()
            .copied()
            .filter(|id| {
                body.lines().any(|l| {
                    l.trim().strip_prefix("///").and_then(|d| d.split_whitespace().next()).is_some_and(|w| w.trim_end_matches(':') == *id)
                })
            })
            .collect();
        let placement = if misplaced.is_empty() {
            String::new()
        } else {
            format!(
                " The `/// <id>: <description>` line for {} is there but inside a function or below its attribute; it must stand above `#[test]`, outside the function body.",
                misplaced.join(", ")
            )
        };
        Some(format!(
            "The tests name no test for {}: every scenario gets one, under a `/// <id>: <description>` line.{placement}",
            missing.join(", ")
        ))
    } else if !sites.is_empty() {
        let lines: Vec<&str> = body.lines().collect();
        let named: Vec<String> = sites
            .iter()
            .map(|(l, t)| format!("line {l}: {t} — `{}`", lines.get(l - 1).map(|x| x.trim()).unwrap_or("")))
            .collect();
        Some(format!("A test that cannot pass is no gate; the file holds {}.", named.join("; ")))
    } else {
        None
    }
}

/// The file as it is sealed: the lint-exempt header, then rustfmt for the
/// crate's edition. Unformatted when rustfmt is missing or fails.
fn sealed_form(body: &str, adr: &str, edition: Option<&str>, path_env: &str) -> String {
    let body = format!("{GENERATED_HEADER}\n{body}\n");
    // Unique per call: generations run side by side in one process.
    static CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = std::env::temp_dir().join(format!("hexa-fmt-{}-{n}-{adr}.rs", std::process::id()));
    if std::fs::write(&tmp, &body).is_err() {
        return body;
    }
    let formatted = rustfmt_in_place(&tmp, edition, path_env).unwrap_or(body);
    let _ = std::fs::remove_file(&tmp);
    formatted
}

/// The tests of a spec too large for one answer: each batch written alone, a
/// module `part_N` of the one file, with up to `repairs` follow-ups for a
/// batch whose tests miss a scenario or cannot pass. Returns the file's text
/// and the test path of every scenario.
async fn write_batches(
    backends: &dyn hexa_infer::ports::Backends,
    model: &str,
    adr: &str,
    parts: &[&[hexa_core::ports::spec::Scenario]],
    prompt: &dyn Fn(&str, &str) -> String,
    repairs: usize,
) -> Result<(String, BTreeMap<String, String>), String> {
    let mut bodies = Vec::new();
    let mut names: BTreeMap<String, String> = BTreeMap::new();
    for (i, part) in parts.iter().enumerate() {
        let n = i + 1;
        let scenarios = serde_json::to_string_pretty(part).map_err(|e| e.to_string())?;
        let note = format!(
            "These are batch {n} of {} of the scenarios. Write a self-contained file for them only: it becomes \
             `mod part_{n}` of the test file, so reach the crate by absolute paths and use no `#![...]` attribute \
             other than `allow`.\n\n",
            parts.len()
        );
        let mut ask = prompt(&scenarios, &note);
        let ids: Vec<&str> = part.iter().map(|s| s.id.as_str()).collect();
        let mut tried = 0;
        let (body, found) = loop {
            let reply = hexa_infer::complete::complete_text_fresh_with(backends, model, WRITER, &ask, 8000).await?;
            let kept = keep_answer(adr, n, &reply).map(|p| format!("\n  raw answer: {}", p.display())).unwrap_or_default();
            let body = block(&reply) + "\n";
            let found = tests_by_scenario(&body, &ids);
            match unmendable_reason(&body, &ids, &found) {
                None => break (body, found),
                Some(why) if tried < repairs => {
                    tried += 1;
                    ask = format!(
                        "{}\n\nYour previous reply:\n\n{reply}\n\nIt was rejected. {why}\n\n\
                         Reply with the whole corrected test file, one fenced block.",
                        prompt(&scenarios, &note)
                    );
                }
                Some(why) => return Err(format!("generation rejected after {tried} repair(s), nothing kept: {why}{kept}")),
            }
        };
        for (sid, f) in found {
            names.insert(sid, format!("part_{n}::{f}"));
        }
        bodies.push(body);
    }
    let body = bodies.iter().enumerate().map(|(i, b)| format!("mod part_{} {{\n{b}\n}}\n", i + 1)).collect::<Vec<_>>().join("\n");
    Ok((body, names))
}

/// How `hexa spec tests` proves red and whether it replaces a sealed file.
#[derive(Debug, Default, Clone)]
pub struct TestOptions {
    /// Prove red at this commit, an ancestor of HEAD (ADR-2610281442 §3).
    pub red_at: Option<String>,
    /// Replace a sealed file, for a stated reason (§4).
    pub replace: Option<Replace>,
    /// Re-prove only these scenarios, into a new file, and leave the commit
    /// to the caller (ADR-2610281628 §2).
    pub only: Option<Vec<String>>,
    /// Why they are re-proved; the writer is told.
    pub because: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Replace {
    pub old_file: String,
    pub because: String,
}

/// A commit whose tree is `base`'s plus `file` holding `body`, with `base`
/// as its parent: the tests as they stood before the code. Built in a
/// private index; HEAD and the working tree are untouched.
async fn red_commit_at(root: &Path, base: &str, file: &str, body: &str, adr: &str) -> Result<String, String> {
    let index = std::env::temp_dir().join(format!("hexa-red-index-{}-{}", std::process::id(), adr));
    let run = |args: Vec<String>, input: Option<String>| {
        let index = index.clone();
        async move {
            let mut cmd = tokio::process::Command::new("git");
            cmd.args(&args).current_dir(root).env("GIT_INDEX_FILE", &index);
            cmd.stdin(if input.is_some() { std::process::Stdio::piped() } else { std::process::Stdio::null() });
            cmd.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
            let mut child = cmd.spawn().map_err(|e| e.to_string())?;
            if let (Some(text), Some(mut stdin)) = (input, child.stdin.take()) {
                use tokio::io::AsyncWriteExt;
                stdin.write_all(text.as_bytes()).await.map_err(|e| e.to_string())?;
            }
            let out = child.wait_with_output().await.map_err(|e| e.to_string())?;
            if !out.status.success() {
                return Err(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()));
            }
            Ok::<String, String>(String::from_utf8_lossy(&out.stdout).trim().to_string())
        }
    };
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let result = async {
        run(s(&["read-tree", base]), None).await?;
        let blob = run(s(&["hash-object", "-w", "--stdin"]), Some(body.to_string())).await?;
        run(s(&["update-index", "--add", "--cacheinfo", &format!("100644,{blob},{file}")]), None).await?;
        let tree = run(s(&["write-tree"]), None).await?;
        let msg = format!("test(spec): {adr} tests, red before the code (ADR-2610281442 §3)");
        run(
            s(&["-c", "user.name=hexa-spec", "-c", "user.email=spec@hexa.local", "commit-tree", &tree, "-p", base, "-m", &msg]),
            None,
        )
        .await
    }
    .await;
    let _ = std::fs::remove_file(&index);
    result
}

/// A generated test file written but not yet in its `red` commit. Dropped
/// on any error, or with the future, it takes the file back out of the index
/// and the working tree: an unsealed file left behind would block every
/// later generation for its ADR.
struct Unsealed {
    root: PathBuf,
    file: String,
    /// A replaced file's sealed content, put back if the seal fails.
    previous: Option<Vec<u8>>,
}

impl Drop for Unsealed {
    fn drop(&mut self) {
        if let Some(previous) = &self.previous {
            let _ = std::fs::write(self.root.join(&self.file), previous);
            let _ = std::process::Command::new("git")
                .args(["checkout", "-q", "HEAD", "--", &self.file])
                .current_dir(&self.root)
                .output();
            return;
        }
        let _ = std::process::Command::new("git")
            .args(["rm", "--cached", "-q", "--ignore-unmatch", "--", &self.file])
            .current_dir(&self.root)
            .output();
        let _ = std::fs::remove_file(self.root.join(&self.file));
    }
}

/// Commit `body` as the `red` commit of `adr`'s tests, and return its sha.
/// Until the commit is made, a failure leaves nothing behind.
async fn seal(unsealed: Unsealed, body: &str, msg: &str, removed: Option<&str>) -> Result<String, String> {
    let (root, file) = (unsealed.root.clone(), unsealed.file.clone());
    // What was checked red, whatever touched the file while the tests ran.
    std::fs::write(root.join(&file), body).map_err(|e| e.to_string())?;
    git(&root, &["add", "--", &file]).await?;
    let mut paths = vec![file.clone()];
    if let Some(old) = removed {
        git(&root, &["rm", "-q", "--", old]).await?;
        paths.push(old.to_string());
    }
    let mut args = vec!["-c", "user.name=hexa-spec", "-c", "user.email=spec@hexa.local", "commit", "-q", "-m", msg, "--"];
    args.extend(paths.iter().map(String::as_str));
    git(&root, &args).await?;
    // Committed: the file is sealed now, and stays whatever follows.
    std::mem::forget(unsealed);
    git(&root, &["rev-parse", "--short", "HEAD"])
        .await
        .map_err(|e| format!("{file} is committed as red, but the commit's sha could not be read: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An inherited edition is read from the workspace root's
    /// `[workspace.package]`; an inline one from the crate; none is `None`.
    #[test]
    fn the_edition_is_read_inline_or_from_the_workspace() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("Cargo.toml"), "[workspace]\nmembers = [\"c\"]\n[workspace.package]\nedition = \"2024\"\n").unwrap();
        assert_eq!(crate_edition(d.path(), "[package]\nname = \"c\"\nedition.workspace = true\n").as_deref(), Some("2024"));
        assert_eq!(crate_edition(d.path(), "[package]\nname = \"c\"\nedition = \"2021\"\n").as_deref(), Some("2021"));
        assert_eq!(crate_edition(d.path(), "[package]\nname = \"c\"\n").as_deref(), None);
    }

    #[test]
    fn only_port_and_domain_files_are_contracts() {
        assert!(is_contract("src/ports.rs"));
        assert!(is_contract("hexa-core/src/domain/spec.rs"));
        assert!(is_contract("src/ports/http.rs"));
        assert!(!is_contract("src/usecase.rs"));
        assert!(!is_contract("src/adapters/primary/cli.rs"));
        assert!(!is_contract("src/report.rs"), "a substring is not a component");
    }

    #[test]
    fn an_unpaired_backtick_does_not_drop_later_contract_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src/ports")).unwrap();
        std::fs::create_dir_all(root.join("src/domain")).unwrap();
        std::fs::write(root.join("src/ports/a.rs"), "").unwrap();
        std::fs::write(root.join("src/domain/b.rs"), "").unwrap();
        std::fs::write(root.join("src/ports.rs"), "").unwrap();
        let adr = "1. A stray ` in prose.\n2. Change `src/ports/a.rs` and `src/domain/b.rs`.\n";
        assert_eq!(given_files(root, adr), ["src/ports/a.rs", "src/domain/b.rs"]);
        let adr = "````md\nx\n````\n\nSee `src/ports.rs` then `src/ports/a.rs`.\n";
        assert_eq!(given_files(root, adr), ["src/ports.rs", "src/ports/a.rs"]);
        // Prose between spans, and text outside any backticks, is not a path.
        let adr = "src/ports.rs `x` src/ports/a.rs `y` src/domain/b.rs";
        assert!(given_files(root, adr).is_empty());
    }

    #[tokio::test]
    async fn red_is_checked_at_the_commit_not_the_working_tree() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let sh = |args: &[&str]| assert!(std::process::Command::new("git").args(args).current_dir(root).status().unwrap().success());
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"redcheck\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n").unwrap();
        std::fs::write(root.join("src/lib.rs"), "pub fn f() -> u32 { 1 }\n").unwrap();
        sh(&["init", "-q"]);
        sh(&["add", "."]);
        sh(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "init"]);
        // Uncommitted: `git commit -- <test file>` leaves this edit out of red.
        std::fs::write(root.join("src/lib.rs"), "pub fn f() -> u32 { 2 }\n").unwrap();
        let head = git(root, &["rev-parse", "HEAD"]).await.unwrap();
        let path = std::env::var("PATH").unwrap_or_default();
        let refs = |f: &str| BTreeMap::from([("S1".to_string(), format!("tests/t.rs::{f}"))]);

        let body = "#[test]\nfn one() { assert_eq!(redcheck::f(), 1); }\n";
        let rejected = red_at(root, &path, &head, "tests/t.rs", body, &refs("one")).await.unwrap().into_rejected();
        assert_eq!(rejected.len(), 1, "fails only in the dirty tree, passes at the commit: {rejected:?}");
        assert!(rejected[0].contains("passes before the code exists"), "{rejected:?}");

        let body = "#[test]\nfn two() { assert_eq!(redcheck::f(), 2); }\n";
        let rejected = red_at(root, &path, &head, "tests/t.rs", body, &refs("two")).await.unwrap().into_rejected();
        assert!(rejected.is_empty(), "fails at the commit, so it is red there: {rejected:?}");

        assert_eq!(std::fs::read_to_string(root.join("src/lib.rs")).unwrap(), "pub fn f() -> u32 { 2 }\n");
        assert!(!root.join("tests/t.rs").exists(), "the check writes nothing into the working tree");
        let list = git(root, &["worktree", "list"]).await.unwrap();
        assert_eq!(list.lines().count(), 1, "the checkout is removed: {list}");
    }

    #[tokio::test]
    async fn a_failed_seal_leaves_no_test_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let sh = |args: &[&str]| assert!(std::process::Command::new("git").args(args).current_dir(root).status().unwrap().success());
        sh(&["init", "-q"]);
        std::fs::write(root.join("README"), "x\n").unwrap();
        sh(&["add", "."]);
        sh(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "init"]);
        let hook = root.join(".git/hooks/pre-commit");
        std::fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let file = "tests/adr_x_spec.rs";
        std::fs::create_dir_all(root.join("tests")).unwrap();
        std::fs::write(root.join(file), "// first\n").unwrap();

        let unsealed = Unsealed { root: root.to_path_buf(), file: file.to_string(), previous: None };
        let err = seal(unsealed, "#[test]\nfn t() {}\n", "ADR-X", None).await.unwrap_err();
        assert!(err.contains("commit"), "{err}");
        assert!(!root.join(file).exists(), "an unsealed test file blocks every later run");
        let staged = git(root, &["diff", "--cached", "--name-only"]).await.unwrap();
        assert!(staged.is_empty(), "nothing stays staged: {staged}");

        // A dropped future drops its guard, and the file with it.
        std::fs::write(root.join(file), "// second\n").unwrap();
        drop(Unsealed { root: root.to_path_buf(), file: file.to_string(), previous: None });
        assert!(!root.join(file).exists());

        // The commit made, the file is sealed and stays.
        std::fs::remove_file(&hook).unwrap();
        std::fs::write(root.join(file), "// third\n").unwrap();
        let unsealed = Unsealed { root: root.to_path_buf(), file: file.to_string(), previous: None };
        let red = seal(unsealed, "#[test]\nfn t() {}\n", "ADR-X", None).await.unwrap();
        assert_eq!(red, git(root, &["rev-parse", "--short", "HEAD"]).await.unwrap());
        assert_eq!(std::fs::read_to_string(root.join(file)).unwrap(), "#[test]\nfn t() {}\n");
    }

    #[test]
    fn a_doc_line_names_the_test_below_it() {
        let body = "/// S1: a\n#[test]\nfn one() {}\n\n/// not an id\n#[test]\nfn two() {}\n/// S2\n#[tokio::test]\nasync fn three() {}\n";
        let m = tests_by_scenario(body, &["S1", "S2"]);
        assert_eq!(m.get("S1").map(String::as_str), Some("one"));
        assert_eq!(m.get("S2").map(String::as_str), Some("three"));
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn a_doc_line_binds_only_the_test_directly_below_it() {
        // An id above a struct, then a helper fn: neither is the test.
        let body = "/// S1: a\nstruct Fixture;\nfn helper() {}\n#[test]\nfn real() {}\n";
        assert!(tests_by_scenario(body, &["S1"]).is_empty());
        // An id above a non-test fn binds nothing, and nothing in its body.
        let body = "/// S1: a\nfn helper() {\n    // see fn other\n    let s = \" fn fake\";\n}\n";
        assert!(tests_by_scenario(body, &["S1"]).is_empty());
        // Doc lines, blank lines and attributes may stand between id and test.
        let body = "/// S1: a\n/// more\n#[test]\n#[ignore]\n\npub async fn kept() {}\n";
        assert_eq!(tests_by_scenario(body, &["S1"]).get("S1").map(String::as_str), Some("kept"));
    }

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
    fn a_binary_test_runs_in_its_binary_target() {
        let dir = tempfile::tempdir().unwrap();
        let k = dir.path();
        std::fs::create_dir_all(k.join("src/bin")).unwrap();
        std::fs::write(k.join("src/lib.rs"), "").unwrap();
        std::fs::write(k.join("src/main.rs"), "").unwrap();
        let tail = |a: Vec<String>, n: usize| a[a.len() - n..].to_vec();
        let a = cargo_args(k, &k.join("src/main.rs"), "f").unwrap();
        assert_eq!(tail(a, 4), ["--bins", "--", "f", "--exact"], "src/main.rs is the binary, not the library");
        let a = cargo_args(k, &k.join("src/bin/x.rs"), "f").unwrap();
        assert_eq!(tail(a, 5), ["--bin", "x", "--", "f", "--exact"], "not the bogus path bin::x::f");
        let a = cargo_args(k, &k.join("src/bin/x/util.rs"), "f").unwrap();
        assert_eq!(tail(a, 5), ["--bin", "x", "--", "util::f", "--exact"]);
        let a = cargo_args(k, &k.join("src/lib.rs"), "f").unwrap();
        assert_eq!(tail(a, 4), ["--lib", "--", "f", "--exact"]);
        let a = cargo_args(k, &k.join("src/cli/mod.rs"), "f").unwrap();
        assert_eq!(tail(a, 5), ["--lib", "--bins", "--", "cli::f", "--exact"], "a module main.rs may own");
        std::fs::remove_file(k.join("src/lib.rs")).unwrap();
        let a = cargo_args(k, &k.join("src/cli/mod.rs"), "f").unwrap();
        assert_eq!(tail(a, 4), ["--bins", "--", "cli::f", "--exact"], "no library target to ask for");
    }

    #[test]
    fn the_library_name_is_the_package_name_with_underscores() {
        assert_eq!(lib_name("[package]\nname = \"my-crate\"\n").as_deref(), Some("my_crate"));
        assert_eq!(lib_name("[workspace]\n").as_deref(), None);
    }

    #[test]
    fn the_library_name_prefers_a_lib_override_and_reads_only_its_own_section() {
        let m = "[package]\nname = \"my-crate\" # c\n\n[lib]\nname = \"foo\"\n";
        assert_eq!(lib_name(m).as_deref(), Some("foo"), "[lib] name is the path tests use");
        let m = "[package]\nversion = \"0.1.0\"\n\n[dependencies]\nname = \"x\"\n";
        assert_eq!(lib_name(m).as_deref(), None, "a key from a later section is not the package name");
        assert_eq!(lib_name("[package]\nname = \"x\" # c\n").as_deref(), Some("x"));
    }

    /// The writer was told only the tests' crate, so for a domain file in
    /// another crate it wrote `hexa_cli::domain::…` and every test failed
    /// to compile.
    #[test]
    fn a_given_file_is_named_by_its_own_crates_path() {
        let t = tempfile::tempdir().unwrap();
        let core = t.path().join("my-core/src/domain");
        std::fs::create_dir_all(&core).unwrap();
        std::fs::write(t.path().join("my-core/Cargo.toml"), "[package]\nname = \"my-core\"\n").unwrap();
        std::fs::write(core.join("thing.rs"), "").unwrap();
        std::fs::write(core.join("mod.rs"), "").unwrap();
        assert_eq!(module_of(t.path(), "my-core/src/domain/thing.rs").as_deref(), Some("my_core::domain::thing"));
        assert_eq!(module_of(t.path(), "my-core/src/domain/mod.rs").as_deref(), Some("my_core::domain"));
        assert_eq!(module_of(t.path(), "elsewhere/thing.rs"), None, "no crate, no claim");
        let m = "[package]\nname = \"c\"\n\n[[bin]]\nname = \"tool\"\npath = \"src/main.rs\"\n";
        assert_eq!(bin_names(m), ["tool"]);
        assert!(bin_names("[package]\nname = \"c\"\n").is_empty());
    }

    #[test]
    fn a_single_line_fence_keeps_its_body_without_the_language_tag() {
        assert_eq!(block("```json {\"a\": 1}```"), "{\"a\": 1}");
        assert_eq!(block("```{\"a\": 1}```"), "{\"a\": 1}");
        assert_eq!(block("Here:\n```json\n{\"a\": 1}\n```\n"), "{\"a\": 1}");
        assert_eq!(block("{\"a\": 1}"), "{\"a\": 1}");
        // A fence inside the body, as a test of a fenced model reply has, is not the end.
        assert_eq!(block("```rust\nlet r = \"```rust\\nx\\n```\";\nfn t() {}\n```\n"), "let r = \"```rust\\nx\\n```\";\nfn t() {}");
    }

    /// A writer that puts the `/// <id>:` line inside the function is told so,
    /// and where the line goes: the binding rule itself stays as strict.
    #[test]
    fn a_scenario_line_inside_a_function_body_is_named_in_the_repair() {
        let misplaced = "#[test]\nfn s1_a() {\n    /// S1: a thing\n    assert!(true);\n}\n";
        let names = tests_by_scenario(misplaced, &["S1"]);
        assert!(names.is_empty(), "a line inside the body binds nothing: {names:?}");
        let why = unmendable_reason(misplaced, &["S1"], &names).expect("S1 has no test");
        assert!(why.contains("S1") && why.contains("above `#[test]`") && why.contains("inside"), "{why}");

        // No such line anywhere: the plain message, with no claim about placement.
        let absent = "#[test]\nfn s1_a() {\n    assert!(false);\n}\n";
        let why = unmendable_reason(absent, &["S1"], &tests_by_scenario(absent, &["S1"])).expect("S1 has no test");
        assert!(!why.contains("inside"), "{why}");

        // Placed right, nothing to mend.
        let placed = "/// S1: a thing\n#[test]\nfn s1_a() {\n    assert_eq!(thing(), 2);\n}\n";
        assert!(unmendable_reason(placed, &["S1"], &tests_by_scenario(placed, &["S1"])).is_none());
    }
}
