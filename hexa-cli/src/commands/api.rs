//! `hexa api`: the API contract a tagged driving port declares (ADR-2610092245).
//!
//! The tag is the source and the OpenAPI document is generated output, so
//! the gate is `check`: the committed document must be what the tags produce
//! now. Exit codes are the gate's vocabulary: 0 the contract holds, 1 it is
//! broken or has drifted, 2 there is no contract — no tagged operation — and
//! an empty contract is a vacuous gate, which is a failed gate.

use clap::{Subcommand, ValueEnum};
use colored::Colorize;
use std::path::{Path, PathBuf};

use hexa_analysis::api_conformance;
use hexa_analysis::api_contract::{self, ApiBuild};
use hexa_analysis::ports::Verdict;
use hexa_analysis::openapi;

#[derive(Subcommand, Debug)]
pub enum ApiAction {
    /// Write the OpenAPI 3.1 document the project's @hexa:api ports declare
    Spec {
        /// Project root
        #[arg(default_value = ".")]
        path: String,
        /// Where to write it; `-` for stdout. Default: openapi.json (or
        /// openapi.yaml) at the project root
        #[arg(long)]
        out: Option<String>,
        #[arg(long, value_enum, default_value_t = Format::Json)]
        format: Format,
    },
    /// Exit 0 when the committed document is what the tags produce now, 1 when it has drifted
    Check {
        /// Project root
        #[arg(default_value = ".")]
        path: String,
        /// The committed document. Default: openapi.json, else openapi.yaml, at the project root
        #[arg(long)]
        spec: Option<String>,
    },
    /// Prove the contract against a running server: every operation sent, every answer judged.
    /// It creates and deletes data — point it at a test instance
    Test {
        /// Project root
        #[arg(default_value = ".")]
        path: String,
        /// Where the server listens, e.g. http://127.0.0.1:8080
        #[arg(long = "base-url")]
        base_url: String,
        /// A JSON object mapping a wire name to the value to send for it
        #[arg(long)]
        examples: Option<String>,
    },
    /// One line per operation: method, path, port method, file:line
    List {
        /// Project root
        #[arg(default_value = ".")]
        path: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Format {
    Json,
    Yaml,
}

const NO_CONTRACT: i32 = 2;

pub async fn run(action: ApiAction) -> anyhow::Result<()> {
    let code = match action {
        ApiAction::Spec { path, out, format } => spec(&root_of(&path)?, out.as_deref(), format)?,
        ApiAction::Check { path, spec } => check(&root_of(&path)?, spec.as_deref())?,
        ApiAction::List { path } => list(&root_of(&path)?)?,
        ApiAction::Test { path, base_url, examples } => test(&root_of(&path)?, &base_url, examples.as_deref()).await?,
    };
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}

fn root_of(path: &str) -> anyhow::Result<PathBuf> {
    let p = PathBuf::from(path);
    anyhow::ensure!(p.is_dir(), "{path} is not a directory");
    Ok(p)
}

fn build(root: &Path) -> anyhow::Result<ApiBuild> {
    let ast = hexa_analysis::default_ast();
    let files = hexa_analysis::analyzer::source_files_sync(root);
    Ok(api_contract::from_project(root, &files, ast.as_ref())?)
}

/// Build the contract, or say why there is none and return the exit code.
fn contract(root: &Path) -> anyhow::Result<Result<ApiBuild, i32>> {
    let built = build(root)?;
    if !built.errors.is_empty() {
        eprintln!("{} the API contract has {} error(s):", "✗".red(), built.errors.len());
        for e in &built.errors {
            eprintln!("  {e}");
        }
        return Ok(Err(1));
    }
    if built.contract.operations.is_empty() {
        eprintln!(
            "{} no tagged operations under {}. Tag a driving port in ports/ with `@hexa:api` and each exposed method with `@hexa:api <METHOD> <path>`.",
            "✗".red(),
            root.display()
        );
        return Ok(Err(NO_CONTRACT));
    }
    Ok(Ok(built))
}

fn generator() -> String {
    format!("hexa {} (hexa api spec)", env!("CARGO_PKG_VERSION"))
}

fn spec(root: &Path, out: Option<&str>, format: Format) -> anyhow::Result<i32> {
    let built = match contract(root)? {
        Ok(b) => b,
        Err(code) => return Ok(code),
    };
    let doc = openapi::render(&built.contract, &generator());
    let text = match format {
        Format::Json => format!("{}\n", serde_json::to_string_pretty(&doc)?),
        Format::Yaml => serde_yaml::to_string(&doc)?,
    };
    if out == Some("-") {
        print!("{text}");
        return Ok(0);
    }
    let dest = match out {
        Some(o) => PathBuf::from(o),
        None => root.join(match format {
            Format::Json => "openapi.json",
            Format::Yaml => "openapi.yaml",
        }),
    };
    std::fs::write(&dest, text)?;
    println!(
        "{} {} operation(s), {} schema(s) → {}",
        "✓".green(),
        built.contract.operations.len(),
        built.contract.schemas.len(),
        dest.display()
    );
    Ok(0)
}

fn read_doc(path: &Path) -> anyhow::Result<serde_json::Value> {
    let text = std::fs::read_to_string(path)?;
    let is_yaml = matches!(path.extension().and_then(|e| e.to_str()), Some("yaml" | "yml"));
    Ok(if is_yaml { serde_yaml::from_str(&text)? } else { serde_json::from_str(&text)? })
}

/// The committed document hexa wrote, if the project has one. A document
/// another tool wrote is not hexa's to check.
fn committed_spec(root: &Path) -> Option<PathBuf> {
    ["openapi.json", "openapi.yaml", "openapi.yml"]
        .iter()
        .map(|n| root.join(n))
        .find(|p| p.is_file())
        .filter(|p| {
            read_doc(p).ok().is_some_and(|d| {
                d["info"]["x-generated-by"].as_str().is_some_and(|g| g.starts_with("hexa"))
            })
        })
}

fn check(root: &Path, spec: Option<&str>) -> anyhow::Result<i32> {
    let built = match contract(root)? {
        Ok(b) => b,
        Err(code) => return Ok(code),
    };
    let committed = match spec {
        Some(s) => PathBuf::from(s),
        None => match ["openapi.json", "openapi.yaml", "openapi.yml"].iter().map(|n| root.join(n)).find(|p| p.is_file()) {
            Some(p) => p,
            None => {
                eprintln!("{} no committed document at {}; run `hexa api spec`", "✗".red(), root.join("openapi.json").display());
                return Ok(1);
            }
        },
    };
    let lines = compare(&built, &committed)?;
    if lines.is_empty() {
        println!(
            "{} {} matches the tags: {} operation(s)",
            "✓".green(),
            committed.display(),
            built.contract.operations.len()
        );
        return Ok(0);
    }
    eprintln!("{} {} has drifted from the tags:", "✗".red(), committed.display());
    for line in lines {
        eprintln!("  {line}");
    }
    eprintln!("  regenerate with `hexa api spec`");
    Ok(1)
}

/// What differs between `committed` and what the tags produce; empty when nothing does.
fn compare(built: &ApiBuild, committed: &Path) -> anyhow::Result<Vec<String>> {
    let want = openapi::content_of(openapi::render(&built.contract, &generator()));
    let have = openapi::content_of(read_doc(committed)?);
    Ok(if want == have { Vec::new() } else { drift(&have, &want) })
}

/// The API gate `hexa ci` runs: `None` when the project declares no API.
/// Otherwise whether the contract reads, and — when hexa wrote a committed
/// document — whether it still matches the tags, with the lines that say why not.
pub fn ci_gate(root: &Path) -> anyhow::Result<Option<(bool, Vec<String>)>> {
    let built = build(root)?;
    if built.ports.is_empty() && built.errors.is_empty() {
        return Ok(None);
    }
    if !built.errors.is_empty() {
        return Ok(Some((false, built.errors.iter().map(ToString::to_string).collect())));
    }
    let Some(committed) = committed_spec(root) else {
        return Ok(Some((true, vec!["no committed document; `hexa api spec` writes one".into()])));
    };
    let lines = compare(&built, &committed)?;
    Ok(Some((lines.is_empty(), lines)))
}

/// What differs, by operation and then by schema.
fn drift(have: &serde_json::Value, want: &serde_json::Value) -> Vec<String> {
    use std::collections::BTreeMap;
    fn ops(doc: &serde_json::Value) -> BTreeMap<String, serde_json::Value> {
        let mut out = BTreeMap::new();
        for (path, item) in doc["paths"].as_object().into_iter().flatten() {
            for (method, op) in item.as_object().into_iter().flatten() {
                out.insert(format!("{} {path}", method.to_uppercase()), op.clone());
            }
        }
        out
    }
    let (h, w) = (ops(have), ops(want));
    let mut lines = Vec::new();
    for (k, v) in &w {
        match h.get(k) {
            None => lines.push(format!("+ {k} (in the tags, not in the document)")),
            Some(old) if old != v => lines.push(format!("~ {k} (changed)")),
            _ => {}
        }
    }
    for k in h.keys().filter(|k| !w.contains_key(*k)) {
        lines.push(format!("- {k} (in the document, no longer in the tags)"));
    }
    let schemas = |d: &serde_json::Value| d["components"]["schemas"].clone();
    for (name, v) in schemas(want).as_object().into_iter().flatten() {
        if schemas(have).get(name) != Some(v) {
            lines.push(format!("~ schema {name}"));
        }
    }
    for name in schemas(have).as_object().into_iter().flatten().map(|(n, _)| n) {
        if schemas(want).get(name).is_none() {
            lines.push(format!("- schema {name}"));
        }
    }
    if lines.is_empty() {
        lines.push("the document's info differs (title or version)".into());
    }
    lines
}

fn list(root: &Path) -> anyhow::Result<i32> {
    let built = match contract(root)? {
        Ok(b) => b,
        Err(code) => return Ok(code),
    };
    for op in &built.contract.operations {
        println!(
            "{:<7} {:<32} {}::{}  {}:{}",
            op.http_method, op.path, op.port, op.method_name, op.file, op.line
        );
    }
    Ok(0)
}

async fn test(root: &Path, base_url: &str, examples: Option<&str>) -> anyhow::Result<i32> {
    let built = match contract(root)? {
        Ok(b) => b,
        Err(code) => return Ok(code),
    };
    let examples: serde_json::Map<String, serde_json::Value> = match examples {
        None => serde_json::Map::new(),
        Some(f) => match serde_json::from_str(&std::fs::read_to_string(f)?)? {
            serde_json::Value::Object(m) => m,
            _ => anyhow::bail!("{f}: --examples is a JSON object mapping a wire name to a value"),
        },
    };
    println!(
        "{} sending {} operation(s) to {base_url}. This creates and deletes data: use a test instance.",
        "⚠".yellow(),
        built.contract.operations.len()
    );
    let probe = crate::default_probe(base_url);
    let report = api_conformance::run(&built.contract, probe.as_ref(), &examples).await;
    for op in &report.operations {
        let (mark, word) = match op.verdict {
            Verdict::Proven => ("✓".green(), "proven".green()),
            Verdict::Violation => ("✗".red(), "violation".red()),
            Verdict::Unproven => ("○".yellow(), "unproven".yellow()),
            Verdict::Unreachable => ("✗".red(), "unreachable".red()),
        };
        let status = op.status.map(|s| s.to_string()).unwrap_or_else(|| "—".into());
        println!("  {mark} {word:<11} {:<32} {status}", op.route);
        for d in &op.details {
            println!("      {d}");
        }
    }
    println!(
        "  {} proven · {} violation(s) · {} unproven · {} unreachable",
        report.count(Verdict::Proven),
        report.count(Verdict::Violation),
        report.count(Verdict::Unproven),
        report.count(Verdict::Unreachable)
    );
    let code = report.exit_code();
    if code == 2 {
        eprintln!("{} nothing was proven: a run that proves nothing is not a pass", "✗".red());
    }
    Ok(code)
}
