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
    args.extend(["--".to_string(), name, "--exact".to_string()]);
    Some(args)
}

/// Run the test `r` (`<path>.rs::<fn>`) alone under `HEXA_NO_SKIP`, from
/// `root`, building into `target_dir` when given.
pub async fn run_test_alone(root: &Path, path_env: &str, r: &str, target_dir: Option<&Path>) -> TestRun {
    let Some((file, f)) = test_ref(r) else { return TestRun::NotFound(format!("{r} is not `<path>.rs::<test_fn>`")) };
    let abs = root.join(file);
    let leaf = f.rsplit("::").next().unwrap_or(f);
    let text = std::fs::read_to_string(&abs).unwrap_or_default();
    if !text.contains(&format!("fn {leaf}(")) {
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
    let dir = root.join("docs/adrs");
    let entries = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name == format!("{adr_id}.md") || name.starts_with(&format!("{adr_id}-")) && name.ends_with(".md") {
            return std::fs::read_to_string(e.path()).map_err(|e| e.to_string());
        }
    }
    Err(format!("{adr_id}: no such ADR in docs/adrs/"))
}

/// A port or domain file: a path component, or the file's stem, names it.
fn is_contract(path: &str) -> bool {
    let p = Path::new(path);
    p.iter().any(|c| c == "ports" || c == "domain")
        || p.file_stem().is_some_and(|s| s == "ports" || s == "domain")
}

/// The files the author is given: each backquoted path in the ADR that
/// exists and is a port or domain file, in order of first mention. Nothing
/// else is ever read for the prompt.
fn given_files(root: &Path, adr_text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for span in adr_text.split('`').skip(1).step_by(2) {
        let span = span.trim();
        let pathlike = span.contains('/') && Path::new(span).extension().is_some() && !span.contains(' ');
        if pathlike && is_contract(span) && root.join(span).is_file() && !out.iter().any(|g| g == span) {
            out.push(span.to_string());
        }
    }
    out
}

fn given_block(root: &Path, given: &[String]) -> String {
    given
        .iter()
        .map(|g| format!("--- {g} ---\n{}\n", std::fs::read_to_string(root.join(g)).unwrap_or_default()))
        .collect()
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

/// The fenced block in a reply, or the reply.
fn block(reply: &str) -> String {
    let t = reply.trim();
    let Some(start) = t.find("```") else { return t.to_string() };
    let body = &t[start + 3..];
    let fenced = body.split("```").next().unwrap_or(body);
    let body = match fenced.split_once('\n') {
        Some(_) => body.split_once('\n').map(|(_, rest)| rest).unwrap_or(body),
        // One line, like ```json {...}```: the language tag is the word before the body.
        None => fenced.trim_start_matches(|c: char| c.is_ascii_alphanumeric() || "_+-".contains(c)),
    };
    body.split("```").next().unwrap_or(body).trim().to_string()
}

const AUTHOR: &str = "You are the spec author. Turn an architecture decision record into behaviour: \
given / when / then scenarios a person can read and a test can check. You are given the ADR and the \
port and domain signatures it names, and nothing else; you have not seen and must not guess at any \
implementation. Every numbered decision (§n) needs at least one `example` scenario and at least one \
`negative` scenario (what the decision must refuse). Cite decisions as \"§n\". Answer with one JSON \
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
    let reply = hexa_infer::complete::complete_text_with(backends, model, AUTHOR, &user, 8000).await?;
    let mut spec: Spec =
        serde_json::from_str(&block(&reply)).map_err(|e| format!("the author's reply is not a spec: {e}"))?;
    // A draft claims nothing that only happens after it: no acceptance, no
    // seal, no tests, no red commit.
    spec.adr = adr_id.to_string();
    spec = spec.accepted_content();
    let findings = coverage(adr_id, &decisions, &spec);
    Ok(Drafted { spec, findings, given })
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

/// Scenario id → test fn, from `/// <id>` doc lines directly above `fn`.
fn tests_by_scenario(body: &str, ids: &[&str]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut pending: Option<String> = None;
    for line in body.lines() {
        let t = line.trim();
        if let Some(doc) = t.strip_prefix("///") {
            let first = doc.split_whitespace().next().unwrap_or("").trim_end_matches(':');
            if ids.contains(&first) {
                pending = Some(first.to_string());
            }
        } else if let Some(sid) = pending.as_ref() {
            if let Some(rest) = t.split("fn ").nth(1).filter(|_| t.starts_with("fn ") || t.contains(" fn ")) {
                let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                if !name.is_empty() {
                    out.insert(sid.clone(), name);
                }
                pending = None;
            }
        }
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
    acceptance_holds(spec).map_err(|why| format!("{}: {why}. Tests are generated from an accepted spec", spec.adr))?;
    refuse_an_agent(model)?;
    let manifest = std::fs::read_to_string(root.join(krate).join("Cargo.toml")).map_err(|_| {
        format!(
            "{}: no Cargo.toml. Generated tests are Rust for now; TypeScript and Go are not generated yet",
            root.join(krate).display()
        )
    })?;
    let lib = lib_name(&manifest).ok_or("the crate's manifest names no package")?;
    let text = adr_text(root, &spec.adr)?;
    let given = given_files(root, &text);
    let scenarios = serde_json::to_string_pretty(&spec.accepted_content().specs).map_err(|e| e.to_string())?;
    let user = format!(
        "The crate is `{lib}`; tests reach it as `{lib}::…`.\n\nThe port and domain signatures:\n\n{}\n\
         The accepted scenarios:\n\n{scenarios}\n",
        given_block(root, &given)
    );
    let reply = hexa_infer::complete::complete_text_with(backends, model, WRITER, &user, 8000).await?;
    let body = block(&reply);

    let ids: Vec<&str> = spec.specs.iter().map(|s| s.id.as_str()).collect();
    let names = tests_by_scenario(&body, &ids);
    let missing: Vec<&str> = ids.iter().copied().filter(|i| !names.contains_key(*i)).collect();
    if !missing.is_empty() {
        return Err(format!("the generated tests name no test for {}: every scenario gets one", missing.join(", ")));
    }

    let stem = format!("{}_spec", spec.adr.to_lowercase().replace('-', "_"));
    let file = krate.join("tests").join(format!("{stem}.rs"));
    let file = file.strip_prefix(".").unwrap_or(&file).to_string_lossy().to_string();
    let abs = root.join(&file);
    if abs.exists() {
        return Err(format!("{file} exists: a regeneration is a new seal, so remove it deliberately first"));
    }
    if let Some(parent) = abs.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&abs, body + "\n").map_err(|e| e.to_string())?;

    let refs: BTreeMap<String, String> = names.iter().map(|(sid, f)| (sid.clone(), format!("{file}::{f}"))).collect();
    let mut rejected = Vec::new();
    for (sid, r) in &refs {
        match run_test_alone(root, path_env, r, None).await {
            TestRun::Failed(_) => {}
            TestRun::Broken { tail, .. } => rejected.push(format!("{sid} never ran:\n{tail}")),
            TestRun::Passed | TestRun::Ambiguous(_) => {
                rejected.push(format!("{sid} passes before the code exists, so it tests nothing"))
            }
            TestRun::NotFound(why) => rejected.push(format!("{sid}: {why}")),
            TestRun::Vacuous => rejected.push(format!("{sid} ran 0 tests")),
            TestRun::NotProvenHere(why) => rejected.push(format!("{sid} is not proven here: {why}")),
        }
    }
    if !rejected.is_empty() {
        let _ = std::fs::remove_file(&abs);
        return Err(format!("generation rejected, nothing kept:\n  {}", rejected.join("\n  ")));
    }

    git(root, &["add", "--", &file]).await?;
    let msg = format!(
        "test(spec): {} scenarios, generated from the accepted spec (failing)\n\n\
         Written by hexa's test writer from the scenarios and the ports alone (ADR-2610221540 §5).\n\
         Every test fails here, before the decision is built.",
        spec.adr
    );
    git(root, &["-c", "user.name=hexa-spec", "-c", "user.email=spec@hexa.local", "commit", "-q", "-m", &msg, "--", &file])
        .await?;
    let red = git(root, &["rev-parse", "--short", "HEAD"]).await?;
    Ok(Generated { file, refs, red })
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn a_doc_line_names_the_test_below_it() {
        let body = "/// S1: a\n#[test]\nfn one() {}\n\n/// not an id\n#[test]\nfn two() {}\n/// S2\n#[tokio::test]\nasync fn three() {}\n";
        let m = tests_by_scenario(body, &["S1", "S2"]);
        assert_eq!(m.get("S1").map(String::as_str), Some("one"));
        assert_eq!(m.get("S2").map(String::as_str), Some("three"));
        assert_eq!(m.len(), 2);
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

    #[test]
    fn a_single_line_fence_keeps_its_body_without_the_language_tag() {
        assert_eq!(block("```json {\"a\": 1}```"), "{\"a\": 1}");
        assert_eq!(block("```{\"a\": 1}```"), "{\"a\": 1}");
        assert_eq!(block("Here:\n```json\n{\"a\": 1}\n```\n"), "{\"a\": 1}");
        assert_eq!(block("{\"a\": 1}"), "{\"a\": 1}");
    }
}
