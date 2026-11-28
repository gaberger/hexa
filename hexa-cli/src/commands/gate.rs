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
    /// True when the module contributes nothing at runtime, so the gate
    /// surviving its deletion is a property of the language rather than a
    /// hole. A TypeScript interface is erased before the program runs; no
    /// gate, however complete, can notice its absence at runtime.
    pub(crate) type_only: bool,
}

/// The module names of every row that is a real hole: the gate survived the
/// deletion, and the module was not erased by the compiler anyway.
pub(crate) fn uncovered(rows: &[Coverage]) -> Vec<String> {
    rows.iter()
        .filter(|r| r.gate_survived && !r.type_only)
        .map(|r| r.module.clone())
        .collect()
}

/// The module names the gate survived only because the module is type-only.
/// Expected, and reported apart from the holes so it does not read as one.
fn erased_by_the_compiler(rows: &[Coverage]) -> Vec<String> {
    rows.iter()
        .filter(|r| r.gate_survived && r.type_only)
        .map(|r| r.module.clone())
        .collect()
}

/// Whether `ext` is a source extension deletion coverage reads.
fn is_measured_extension(ext: &str) -> bool {
    matches!(ext, "ts" | "tsx" | "js" | "rs" | "go" | "py")
}

/// The files worth deleting one at a time. Source only; never the composition
/// root, which is wiring rather than a subject; never a test, which is the gate
/// itself and would make the measurement circular.
pub(crate) fn candidate_modules(files: &[PathBuf], composition_root: &str) -> Vec<String> {
    let mut out = Vec::new();
    for f in files {
        let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !is_measured_extension(ext) {
            continue;
        }
        let rel = f.to_string_lossy().replace('\\', "/");
        if rel == composition_root {
            continue;
        }
        let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("");
        // pytest's discovery names, and its shared fixtures.
        let python_test = ext == "py" && (name.starts_with("test_") || name.ends_with("_test.py") || name == "conftest.py");
        if name.contains(".test.") || name.ends_with("_test.go") || python_test {
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

/// [`candidate_modules`] under `root`, less each Python file with nothing to
/// run. Deleting an empty `__init__.py` leaves the package importable, so no
/// gate could notice it.
fn measurable_modules(root: &Path, files: &[PathBuf], composition_root: &str) -> Vec<String> {
    let mut out = candidate_modules(files, composition_root);
    out.retain(|m| !is_python_wiring_or_tooling(m));
    out.retain(|m| !(m.ends_with(".py") && std::fs::read_to_string(root.join(m)).is_ok_and(|s| is_blank_python(&s))));
    out
}

/// Python files a gate does not import to use: the composition root and the
/// program entry wherever the package holds them (the names the analyzer reads
/// as entry points), packaging and task scripts, and generated migrations.
fn is_python_wiring_or_tooling(module: &str) -> bool {
    let name = module.rsplit('/').next().unwrap_or(module);
    matches!(name, "composition_root.py" | "__main__.py" | "setup.py" | "manage.py" | "noxfile.py")
        || (module.ends_with(".py") && format!("/{module}").contains("/migrations/"))
}

/// Whether `source` holds only blank lines, comments and a module docstring.
fn is_blank_python(source: &str) -> bool {
    let mut rest = source.trim_start_matches('\u{feff}');
    let mut docstring_read = false;
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            return true;
        }
        if rest.starts_with('#') {
            rest = rest.split_once('\n').map_or("", |(_, after)| after);
            continue;
        }
        if docstring_read {
            return false;
        }
        match after_docstring(rest) {
            Some(after) => {
                docstring_read = true;
                rest = after;
            }
            None => return false,
        }
    }
}

/// What follows the string literal `source` starts with, if it starts with one
/// and nothing but a comment shares its last line.
fn after_docstring(source: &str) -> Option<&str> {
    let body = source.strip_prefix(['r', 'R', 'u', 'U']).unwrap_or(source);
    let quote = [r#"""""#, "'''", "\"", "'"].into_iter().find(|q| body.starts_with(q))?;
    let (text, after) = body[quote.len()..].split_once(quote)?;
    if quote.len() == 1 && text.contains('\n') {
        return None;
    }
    let line_end = after.split('\n').next().unwrap_or("").trim();
    (line_end.is_empty() || line_end.starts_with('#')).then_some(after)
}

/// True when every statement in `source` is erased before the program runs:
/// type imports, type aliases, interface declarations and type re-exports.
/// Deleting such a file cannot change behaviour, so a gate that survives its
/// deletion has not missed anything.
///
/// Conservative by construction. The first line that could run anything —
/// a const, a function, a class, a call, an assignment — returns false. An
/// empty file is not type-only; there is nothing in it to be a type.
pub(crate) fn is_type_only(source: &str) -> bool {
    let mut considered = 0usize;
    // Brace depth inside a declaration whose body is entirely type syntax.
    let mut depth = 0i32;
    // Set while a type alias started on an earlier line is still open; every
    // line until it ends is type text, not a statement.
    let mut in_alias = false;

    for raw in source.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        // Comments carry no runtime meaning in any of these languages.
        if line.starts_with("//")
            || line.starts_with("/*")
            || line.starts_with('*')
            || line.starts_with("*/")
        {
            continue;
        }
        considered += 1;

        // Inside an interface or object type body: every line is type syntax.
        if depth > 0 {
            depth += brace_delta(line);
            continue;
        }

        // A type alias with no body can still span lines — a union written one
        // member per line. Those lines are type text; they end the alias at the
        // terminating `;`, or when the next top-level declaration begins.
        if in_alias {
            if starts_a_top_level_declaration(line) {
                in_alias = false;
            } else {
                depth += brace_delta(line);
                if depth == 0 && line.ends_with(';') {
                    in_alias = false;
                }
                continue;
            }
        }

        // A type alias or an interface may open a body that spans lines; the
        // brace depth carries the rest of it.
        let alias = line.starts_with("export type ") || starts_a_type_alias(line);
        let opens_a_type_body = line.starts_with("export type")
            || starts_a_type_alias(line)
            || line.starts_with("interface ")
            || line.starts_with("export interface ");

        if opens_a_type_body {
            depth += brace_delta(line);
            // An alias whose statement does not finish on this line carries on
            // into the lines below it.
            if alias && depth == 0 && !line.ends_with(';') {
                in_alias = true;
            }
        } else if line.starts_with("import type")
            || line.starts_with("export {")
            || line == "}"
            || line == "};"
        {
            // Erased outright, and closes nothing that is still open.
        } else {
            return false;
        }
    }

    considered > 0 && depth == 0
}

/// True when the line opens a new top-level declaration — which ends any type
/// alias still open above it, so a runtime statement after an alias is still
/// seen as runtime.
fn starts_a_top_level_declaration(line: &str) -> bool {
    const KEYWORDS: [&str; 12] = [
        "import", "export", "interface", "type", "const", "let", "var", "function", "class",
        "async", "enum", "declare",
    ];
    let word: String = line
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '$')
        .collect();
    KEYWORDS.contains(&word.as_str())
}

/// `type X = ...` — a type alias, which is erased. Not `typeof`, and not an
/// identifier that merely begins with the letters.
fn starts_a_type_alias(line: &str) -> bool {
    line.starts_with("type ") && line.contains('=')
}

/// Braces opened minus braces closed on one line.
fn brace_delta(line: &str) -> i32 {
    let count = |b: char| i32::try_from(line.chars().filter(|c| *c == b).count()).unwrap_or(i32::MAX);
    count('{') - count('}')
}

/// Restores a deleted file when it goes out of scope, however it goes out of
/// scope. A wrong answer is recoverable; a source file left deleted on disk
/// because the gate panicked is not.
struct Restore {
    path: PathBuf,
    bytes: Vec<u8>,
    /// What stood at `path`: its mode, and where it pointed if it was a link.
    /// Writing the bytes alone made a script no longer executable and a
    /// symlink a copy.
    permissions: std::fs::Permissions,
    link: Option<PathBuf>,
}

impl Restore {
    /// What it takes to put `path`, which holds `bytes`, back.
    fn of(path: PathBuf, bytes: Vec<u8>) -> std::io::Result<Self> {
        let link = std::fs::read_link(&path).ok();
        let permissions = std::fs::metadata(&path)?.permissions();
        Ok(Self { path, bytes, permissions, link })
    }
}

impl Drop for Restore {
    fn drop(&mut self) {
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        #[cfg(unix)]
        if let Some(target) = &self.link {
            // Deleting removed the link, not what it points at.
            let _ = std::fs::remove_file(&self.path);
            if std::os::unix::fs::symlink(target, &self.path).is_ok() {
                return;
            }
        }
        if std::fs::write(&self.path, &self.bytes).is_ok() {
            let _ = std::fs::set_permissions(&self.path, self.permissions.clone());
        }
    }
}

/// Collect every file under `root`, skipping the directories that hold build
/// output rather than source.
fn collect_files(root: &Path, out: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    // With Python's environments and caches: installed code is not the
    // project's, and one holding a `.rs` file refused the whole measurement.
    const SKIP: [&str; 10] =
        ["target", "node_modules", ".git", "dist", "build", "graph-out", ".venv", "venv", "site-packages", "__pycache__"];
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
    uncounted: bool,
) -> anyhow::Result<Vec<Coverage>> {
    let mut files = Vec::new();
    collect_files(root, &mut files)?;

    let relative: Vec<PathBuf> = files
        .iter()
        .map(|f| f.strip_prefix(root).unwrap_or(f).to_path_buf())
        .collect();

    let mut rows = Vec::new();
    for module in measurable_modules(root, &relative, composition_root) {
        let path = root.join(&module);
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        // Read the source before it is gone: whether a surviving deletion is
        // a hole depends on whether the file could have run at all.
        let ext = Path::new(&module).extension().and_then(|e| e.to_str()).unwrap_or("");
        let type_only = if matches!(ext, "rs" | "go") {
            // Rust and Go types are not erased; a deleted one is a real hole.
            false
        } else {
            std::str::from_utf8(&bytes).map(is_type_only).unwrap_or(false)
        };

        let guard = Restore::of(path.clone(), bytes)?;
        std::fs::remove_file(&path)?;
        let (ok, out) = hexa_exec::direct_exec::run_evidence(gate, root).await;
        drop(guard);
        // The gate survived the deletion when it still passes by the one
        // reader, not merely exits 0 (ADR-2611081151 §4).
        let survived = hexa_core::ports::evidence::read_evidence(ok, &out, uncounted).passes();
        rows.push(Coverage { module, gate_survived: survived, type_only });
    }
    Ok(rows)
}

/// Why deletion cannot measure these modules, if it cannot: a deleted Rust or
/// Go source is a compile error, so the gate fails for every one of them,
/// tested or not.
pub(crate) fn not_measurable_by_deletion(candidates: &[String]) -> Option<String> {
    for (ext, lang) in [("rs", "rust"), ("go", "go")] {
        let n = candidates.iter().filter(|c| c.ends_with(&format!(".{ext}"))).count();
        if n > 0 {
            return Some(format!(
                "not measurable by deletion for {lang}: {n} .{ext} module(s), and deleting one is a compile error, \
                 so every module would read as needed whether a test reaches it or not"
            ));
        }
    }
    None
}

/// The repository `target` sits in, else the working directory: where a
/// repo-relative path such as `src/main.ts` is read from.
fn repo_root_of(target: &Path) -> PathBuf {
    std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(if target.is_dir() { target } else { Path::new(".") })
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// The composition root as a path relative to `target`, the form candidates
/// are compared in. A path is read relative to `target` first, then to the
/// repository root; one outside `target` excludes nothing. Looking only under
/// `target` missed `src/lib.rs` for target `src`, and deleted it.
pub(crate) fn resolve_composition_root(target: &Path, repo_root: &Path, given: Option<&str>) -> anyhow::Result<String> {
    let canon = |p: &Path| std::fs::canonicalize(p).ok();
    let target_abs = canon(target);
    let relative_to_target = |file: &Path| -> Option<String> {
        let file = canon(file)?;
        let rel = file.strip_prefix(target_abs.as_ref()?).ok()?;
        Some(rel.to_string_lossy().replace('\\', "/"))
    };
    let lookup = |c: &str| -> Option<String> {
        let c = c.trim_start_matches("./");
        [target.join(c), repo_root.join(c)].iter().filter(|p| p.is_file()).find_map(|p| relative_to_target(p))
    };
    match given {
        Some(c) => lookup(c).ok_or_else(|| {
            anyhow::anyhow!(
                "--composition-root {c}: no such file under {} or the repository root {}",
                target.display(),
                repo_root.display()
            )
        }),
        // A library crate's `lib.rs` is its composition root, as a binary's
        // `main.rs` is (CLAUDE.md: each crate's lib.rs wires its adapters).
        None => Ok(["src/main.ts", "src/main.rs", "main.go", "src/index.ts", "src/lib.rs", "main.ts", "index.ts", "lib.rs", "main.rs", "src/composition_root.py", "composition_root.py", "src/main.py", "main.py"]
            .iter()
            .find_map(|c| lookup(c))
            .unwrap_or_default()),
    }
}

/// `hexa gate coverage` — name every module the gate does not verify.
pub async fn run(
    target: String,
    gate: String,
    composition_root: Option<String>,
    uncounted: bool,
) -> anyhow::Result<()> {
    use hexa_core::ports::evidence::{read_evidence, Evidence, KNOWN_RUNNERS};
    let root = PathBuf::from(&target);

    let composition_root = resolve_composition_root(&root, &repo_root_of(&root), composition_root.as_deref())?;

    // Deleting a Rust or Go file is a compile error, so every gate "needs"
    // every module and an untested one reads as covered. Measured: an
    // untested util.rs was counted as verified. Refuse rather than report a
    // coverage the method cannot see.
    let mut files = Vec::new();
    collect_files(&root, &mut files)?;
    let relative: Vec<PathBuf> = files.iter().map(|f| f.strip_prefix(&root).unwrap_or(f).to_path_buf()).collect();
    let candidates = measurable_modules(&root, &relative, &composition_root);
    // Coverage of nothing is not coverage: "0 of 0" passed a tree whose
    // language was never read. A tree that is only wiring or tests was read,
    // and has no module to hold to the gate (ADR-2611081151 §7's scenarios).
    let readable = relative.iter().any(|f| f.extension().and_then(|e| e.to_str()).is_some_and(is_measured_extension));
    let nothing = (!readable).then(|| format!("no source to measure under {target}: hexa reads .ts, .tsx, .js, .rs, .go and .py here"));
    if let Some(why) = nothing.or_else(|| not_measurable_by_deletion(&candidates)) {
        println!("{} {}", "deletion coverage of".bold(), gate.cyan());
        println!("  {} {why}", "✗".red());
        println!("  nothing was deleted and no coverage is reported");
        anyhow::bail!("{why}");
    }

    // Coverage is measured from a gate that passes on the unmodified tree
    // (ADR-2611081151 §7): a gate that fails there survives every deletion
    // for the wrong reason.
    let (ok, out) = hexa_exec::direct_exec::run_evidence(&gate, &root).await;
    let baseline = read_evidence(ok, &out, uncounted);
    if !baseline.passes() {
        let why = match &baseline {
            Evidence::Failed { reason } => format!("the gate did not pass on the unmodified tree: {reason}"),
            Evidence::Vacuous => "the gate measured nothing on the unmodified tree: it ran zero tests".to_string(),
            Evidence::Unrecognised => format!(
                "the gate's output on the unmodified tree is unrecognised: no test runner this reader knows wrote it \
                 ({KNOWN_RUNNERS}); declare a gate that runs no tests with --uncounted"
            ),
            Evidence::Passed { .. } | Evidence::Uncounted => String::new(),
        };
        println!("{} {}", "deletion coverage of".bold(), gate.cyan());
        println!("  {} {why}", "✗".red());
        println!("  nothing was deleted and no coverage is reported");
        anyhow::bail!("{why}");
    }

    let rows = measure(&root, &gate, &composition_root, uncounted).await;
    let rows = rows?;

    if uncounted {
        println!(
            "{} {} {}",
            "deletion coverage of".bold(),
            gate.cyan(),
            "(uncounted: its exit status alone decides)".dimmed()
        );
    } else {
        println!("{} {}", "deletion coverage of".bold(), gate.cyan());
    }

    let needed: Vec<&str> =
        rows.iter().filter(|r| !r.gate_survived).map(|r| r.module.as_str()).collect();
    let erased = erased_by_the_compiler(&rows);
    let missed = uncovered(&rows);

    if !needed.is_empty() {
        println!("\n{}", "the gate needs these modules".bold());
        for module in &needed {
            println!("  {} {}", "✓".green(), module);
        }
    }

    if !erased.is_empty() {
        println!(
            "\n{} {}",
            "the gate survives deleting these, and that is expected".bold(),
            "— type-only, erased before the program runs, so not a hole".dimmed()
        );
        for module in &erased {
            println!("  {} {}", "–".dimmed(), module);
        }
    }

    if !missed.is_empty() {
        println!("\n{}", "runtime modules the gate never needs — these are the holes".bold());
        for module in &missed {
            println!(
                "  {} {} {}",
                "✗".red(),
                module,
                "— the gate passes without it".dimmed()
            );
        }
    }

    let runtime_modules = rows.len() - erased.len();
    println!(
        "\n{} of {} runtime modules are verified by nothing",
        missed.len(),
        runtime_modules
    );

    if missed.is_empty() {
        Ok(())
    } else {
        anyhow::bail!("{} runtime modules are verified by nothing", missed.len())
    }
}

#[cfg(test)]
mod gate_coverage {
    //! ADR-2609160300 §1: a gate that survives the deletion of a required
    //! component is a failed gate, by the same rule that rejects a vacuous one.
    use super::{candidate_modules, uncovered, Coverage};
    use std::path::PathBuf;

    fn cov(m: &str, survived: bool) -> Coverage {
        Coverage { module: m.to_string(), gate_survived: survived, type_only: false }
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
    fn a_type_only_module_is_reported_apart_from_a_runtime_one() {
        // TypeScript types are erased before the program runs, so deleting an
        // interface file cannot change behaviour and the gate passes. That is
        // a property of the language, not a hole in the gate. Measured
        // 2026-09-16: both of the Spec Kit arm's ports flagged this way.
        assert!(super::is_type_only("export interface LinkStore {\n  get(): void;\n}\n"));
        assert!(super::is_type_only("import type { X } from \"./x.js\";\nexport type Y = X;\n"));
        assert!(!super::is_type_only("export class Store {\n  get() {}\n}\n"));
        assert!(!super::is_type_only("export const MAX = 8192;\n"));
    }

    #[test]
    fn a_multi_line_type_alias_is_still_type_only() {
        // Found 2026-09-16 by running the verb against the hexa arm: a union
        // type split over several lines was read as runtime code, so a file
        // containing nothing but types was reported as a hole in the gate.
        let src = concat!(
            "export type ShortenResult =\n",
            "  | { readonly ok: true; readonly code: string }\n",
            "  | { readonly ok: false; readonly reason: \"invalid-url\" };\n",
            "\n",
            "export interface LinkService {\n",
            "  shorten(raw: unknown): Promise<ShortenResult>;\n",
            "}\n",
        );
        assert!(super::is_type_only(src), "a multi-line union type is not runtime code");
    }

    #[test]
    fn a_type_alias_does_not_swallow_the_runtime_code_after_it() {
        let src = concat!(
            "export type A =\n",
            "  | { a: 1 }\n",
            "  | { a: 2 };\n",
            "export const LIMIT = 8192;\n",
        );
        assert!(!super::is_type_only(src), "a const after a type alias is still runtime code");
    }

    #[test]
    fn only_runtime_modules_count_as_holes_in_the_gate() {
        let rows = vec![
            Coverage { module: "src/ports/link-store.ts".into(), gate_survived: true, type_only: true },
            Coverage { module: "src/adapters/secondary/cache.ts".into(), gate_survived: true, type_only: false },
            Coverage { module: "src/domain/link.ts".into(), gate_survived: false, type_only: false },
        ];
        assert_eq!(uncovered(&rows), vec!["src/adapters/secondary/cache.ts".to_string()],
            "a type-only module is not a hole; a runtime module the gate never needs is");
    }

    #[test]
    fn rust_and_go_modules_are_refused_as_not_measurable_by_deletion() {
        let why = super::not_measurable_by_deletion(&["src/util.rs".into()]).expect("refused");
        assert!(why.contains("not measurable by deletion for rust"), "{why}");
        let why = super::not_measurable_by_deletion(&["a.ts".into(), "store.go".into()]).expect("refused");
        assert!(why.contains("for go"), "{why}");
        assert!(super::not_measurable_by_deletion(&["a.ts".into(), "b.tsx".into()]).is_none());
    }

    #[test]
    fn the_composition_root_is_found_from_the_repo_root_and_made_target_relative() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("src")).unwrap();
        std::fs::write(d.path().join("src/main.ts"), "").unwrap();
        let target = d.path().join("src");
        assert_eq!(super::resolve_composition_root(&target, d.path(), Some("src/main.ts")).unwrap(), "main.ts");
        assert_eq!(super::resolve_composition_root(&target, d.path(), Some("main.ts")).unwrap(), "main.ts");
        assert_eq!(super::resolve_composition_root(&target, d.path(), None).unwrap(), "main.ts");
        assert_eq!(super::resolve_composition_root(d.path(), d.path(), None).unwrap(), "src/main.ts");
        assert!(super::resolve_composition_root(&target, d.path(), Some("nope.ts")).is_err());
    }

    #[test]
    fn rust_and_go_sources_are_candidates_too() {
        let files: Vec<PathBuf> = ["src/lib.rs", "src/store.go", "src/lib_test.go", "src/main.rs"]
            .iter().map(PathBuf::from).collect();
        let got = candidate_modules(&files, "src/main.rs");
        assert_eq!(got, vec!["src/lib.rs".to_string(), "src/store.go".to_string()]);
    }
}
