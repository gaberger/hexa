//! Architecture health check command.
//!
//! `hexa analyze [path]` — checks hexa layer structure using `hexa_core::rules::boundary`
//! types, and when nexus is running, delegates to the full tree-sitter boundary analysis.

use std::path::{Path, PathBuf};

use colored::Colorize;
use hexa_core::rules::boundary::{self, Layer};


/// Layers shown in the "Hex layers" checklist, in display order. Detection itself is
/// language-agnostic (`hexa_core::rules::boundary::detect_layer`, a path-substring
/// matcher) — this is purely display labeling, not a second hardcoded detection path.
const DISPLAY_LAYERS: &[(Layer, &str)] = &[
    (Layer::Domain, "Domain"),
    (Layer::Ports, "Ports"),
    (Layer::Usecases, "Use Cases"),
    (Layer::AdapterPrimary, "Primary Adapters"),
    (Layer::AdapterSecondary, "Secondary Adapters"),
];

#[allow(clippy::too_many_arguments)]
pub async fn run(
    path: &str,
    strict: bool,
    adr_compliance_only: bool,
    json_output: bool,
    file: Option<&str>,
    quiet: bool,
    violations_only: bool,
    exit_code: bool,
    grade_floor: Option<&str>,
) -> anyhow::Result<()> {
    let root = Path::new(path)
        .canonicalize()
        .unwrap_or_else(|_| Path::new(path).to_path_buf());

    // Single-file mode: analyze just one file
    if let Some(file_path) = file {
        return run_single_file(file_path, &root, quiet, violations_only, exit_code);
    }

    // JSON mode: collect results and emit structured output
    if json_output {
        return run_json(&root, strict, adr_compliance_only).await;
    }

    println!(
        "{} Architecture analysis: {}",
        "\u{2b21}".cyan(),
        root.display()
    );
    println!();

    // Violations collected during boundary analysis (used by violations_only and exit_code)
    let mut local_violations: Vec<boundary::Violation> = Vec::new();
    let mut rust_violations: Vec<RustViolation> = Vec::new();
    let mut all_violation_count = 0usize;
    // The final score, for the --grade floor at the end.
    let mut final_score: Option<u64> = None;

    // If --ADR-compliance flag is set, skip boundary analysis entirely
    if !adr_compliance_only {
        // Check for hexa project markers
        let has_src = root.join("src").is_dir();
        let has_package_json = root.join("package.json").is_file();
        let has_cargo_toml = root.join("Cargo.toml").is_file();
        let has_go_mod = root.join("go.mod").is_file();
        let has_pyproject = root.join("pyproject.toml").is_file()
            || root.join("setup.py").is_file()
            || root.join("requirements.txt").is_file();
        let has_hex_config = root.join(".hexa").is_dir();
        let has_docs_adrs = root.join("docs").join("adrs").is_dir();

        // The language is read from the manifest that is present. A Rust
        // project is not missing package.json and go.mod; it is a Rust
        // project, and the rest of the report is read in that light.
        let language = if has_cargo_toml {
            Some(("rust", "Cargo.toml"))
        } else if has_go_mod {
            Some(("go", "go.mod"))
        } else if has_package_json {
            Some(("typescript", "package.json"))
        } else {
            None
        };
        println!("  {}", "Project:".bold());
        match language {
            Some((lang, manifest)) => println!("    language:     {} ({})", lang, manifest),
            None if has_pyproject => println!(
                "    language:     {} (Python; hexa grades Rust, Go and TypeScript)",
                "unsupported".yellow()
            ),
            None => println!(
                "    language:     {} (no Cargo.toml, go.mod or package.json)",
                "unknown".yellow()
            ),
        }
        print_check(".hexa/ config", has_hex_config);
        println!("    docs/adrs/:   {}", if has_docs_adrs { "present" } else { "none" });

        // Check hexa architecture layers by classifying every source file under src/
        // via hexa_core::rules::boundary::detect_layer — a path-substring matcher that's
        // language-agnostic and tolerant of arbitrary package-name nesting (e.g. both
        // "src/domain/x.ts" and "src/mypkg/core/domain/x.py" resolve to Layer::Domain).
        let mut layer_file_counts: Vec<(&str, usize)> = Vec::new();
        let mut layer_counts: std::collections::HashMap<Layer, usize> = std::collections::HashMap::new();
        // Rust and TypeScript keep layers under src/; Go keeps them at the
        // module root (internal/domain, adapters/secondary).
        let scan_dir = if has_src { root.join("src") } else { root.clone() };
        if has_src || has_go_mod {
            println!();
            println!("  {}", "Hex layers:".bold());

            for file in collect_source_files(&scan_dir) {
                let rel = file
                    .strip_prefix(&root)
                    .unwrap_or(&file)
                    .to_string_lossy()
                    .replace('\\', "/");
                // The patterns are `/adapters/secondary/`; a path relative to
                // the module root, `adapters/secondary/memory.go`, has no
                // leading slash and matched nothing.
                *layer_counts.entry(boundary::detect_layer(&format!("/{rel}"))).or_insert(0) += 1;
            }

            // A layer that is not there is not a failure. A library has no
            // primary adapter and the grade says whether what is there holds.
            for (layer, label) in DISPLAY_LAYERS {
                let count = layer_counts.get(layer).copied().unwrap_or(0);
                if count > 0 {
                    println!("    {} {} ({} files)", "\u{2713}".green(), label, count);
                } else {
                    println!("    {} {} (none)", "\u{00b7}".dimmed(), label.dimmed());
                }
                layer_file_counts.push((label, count));
            }

            // The composition root is the one file that names adapters. Its
            // name is a convention per language: lib.rs or main.rs in a Rust
            // crate, composition-root.ts, composition-root.go or main.go.
            let mut candidates: Vec<String> = [
                "src/composition-root.ts",
                "src/composition_root.rs",
                "src/composition-root.rs",
                "src/lib.rs",
                "src/main.rs",
                "composition-root.go",
                "main.go",
                "src/composition-root.go",
                "src/main.go",
            ]
            .iter()
            .map(|p| p.to_string())
            .collect();
            if let Ok(cmd) = std::fs::read_dir(root.join("cmd")) {
                for e in cmd.flatten() {
                    candidates.push(format!("cmd/{}/main.go", e.file_name().to_string_lossy()));
                }
            }
            let composition_root = candidates
                .into_iter()
                .find(|p| root.join(p).is_file())
                .or_else(|| {
                    (layer_counts.get(&Layer::CompositionRoot).copied().unwrap_or(0) > 0)
                        .then(|| "detected".to_string())
                });
            match composition_root {
                Some(p) => println!("    {} Composition root ({})", "\u{2713}".green(), p),
                None => println!("    {} Composition root", "\u{2717}".red()),
            }
        }

        // Rust workspace layer detection (ADR-2026-03-28-3000)
        let rust_layers = if has_cargo_toml {
            let layers = scan_rust_workspace_layers(&root);
            if !layers.is_empty() {
                println!();
                println!("  {}", "Rust workspace layers:".bold());
                for (label, count) in &layers {
                    let indicator = if *count > 0 { "\u{2713}".green() } else { "\u{2023}".dimmed() };
                    println!("    {} {} ({} files)", indicator, label, count);
                }
            }
            layers
        } else {
            Vec::new()
        };

        // Offline boundary check: scan for obvious violations without nexus
        local_violations = if has_src {
            scan_local_violations(&root)
        } else {
            Vec::new()
        };

        // Rust boundary violations
        rust_violations = if has_cargo_toml {
            scan_rust_boundary_violations(&root)
        } else {
            Vec::new()
        };

        // Go project layer detection
        let go_files_total = if has_go_mod {
            let go_dirs: &[&str] = &["cmd", "internal", "pkg", "api"];
            let mut total = 0usize;
            let has_go_subdirs = go_dirs.iter().any(|d| root.join(d).is_dir());
            if has_go_subdirs {
                println!();
                println!("  {}", "Go project layers:".bold());
                for dir in go_dirs {
                    let layer_path = root.join(dir);
                    if layer_path.is_dir() {
                        let count = collect_source_files(&layer_path).len();
                        total += count;
                        println!("    {} {} ({} files)", "\u{2713}".green(), dir, count);
                    }
                }
            }
            // Also count root-level .go files (flat layout like fizzbuzz)
            let root_go: Vec<_> = std::fs::read_dir(&root)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| {
                    e.path().extension().and_then(|x| x.to_str()) == Some("go")
                        && e.path().is_file()
                })
                .collect();
            if !root_go.is_empty() {
                if !has_go_subdirs {
                    println!();
                    println!("  {}", "Go project (flat layout):".bold());
                }
                println!("    {} root-level .go files ({})", "\u{2023}".dimmed(), root_go.len());
                total += root_go.len();
            }
            total
        } else {
            0
        };

        // Count total source files across the project
        let mut total_files = 0usize;
        if has_src {
            total_files += collect_source_files(&root.join("src")).len();
        }
        // Add Rust workspace file counts
        let rust_total: usize = rust_layers.iter().map(|(_, c)| c).sum();
        total_files += rust_total;
        total_files += go_files_total;

        println!();
        println!("  {}", "Boundary analysis:".bold());
        println!("    {} {} source files scanned", "\u{2023}".dimmed(), total_files);

        all_violation_count = local_violations.len() + rust_violations.len();
        if all_violation_count > 0 {
            println!(
                "    {} {} boundary violation(s) (import scan)",
                "\u{26a0}".yellow(),
                all_violation_count
            );
            for v in &local_violations {
                println!(
                    "      {} {} \u{2192} {} ({})",
                    "\u{2717}".red(),
                    v.source_file,
                    v.imported_path,
                    v.rule,
                );
            }
            for v in &rust_violations {
                println!(
                    "      {} {}:{} — {}",
                    "\u{2717}".red(),
                    v.file,
                    v.line,
                    v.message,
                );
            }
        }

        // Full tree-sitter boundary analysis, in-process (ADR-2608241500 P6.2).
        // This used to ask the daemon, and only if it happened to be running
        // and to have this directory registered as a project — so the
        // authoritative score depended on a background process and a
        // registration step. Same `hexa-analysis` engine either way.
        // (violations, cycles, dead exports, unused ports): the grade is a
        // sum, and a sum without its components gets a story attached.
        let mut score_components: Option<(usize, usize, Vec<String>, Vec<String>)> = None;
        let deep_score: Option<u64> = match deep_analysis(&root).await {
            Ok(result) => {
                // This count used to be printed and then dropped. `--exit-code`
                // summed only the shallow scanners, so a project whose
                // violations lived in a nested source tree printed
                // "Boundary violations: 17" and exited 0. The CI gate was
                // vacuous for exactly the case it exists for.
                all_violation_count += result.violations.len();
                if !result.violations.is_empty() {
                    println!(
                        "    {} Boundary violations: {}",
                        "\u{26a0}".yellow(),
                        result.violations.len().to_string().red()
                    );
                    // Name them. A count tells you something is wrong; only the
                    // file and the rule tell you what to change.
                    for v in result.violations.iter().take(10) {
                        println!(
                            "        {} {} {} {}",
                            v.edge.from_file.dimmed(),
                            "\u{2192}".dimmed(),
                            v.edge.to_file,
                            format!("({})", v.rule).red()
                        );
                    }
                    if result.violations.len() > 10 {
                        println!("        … and {} more", result.violations.len() - 10);
                    }
                }
                println!(
                    "    {} Analysed {} files, {} import edges",
                    "\u{2713}".green(),
                    result.file_count,
                    result.edge_count
                );
                score_components = Some((
                    result.violations.len(),
                    result.circular_deps.len(),
                    result
                        .dead_exports
                        .iter()
                        .map(|d| format!("{}:{} {}", d.file, d.line, d.export_name))
                        .collect::<Vec<_>>(),
                    result.unused_ports.clone(),
                ));
                Some(result.health_score as u64)
            }
            Err(e) => {
                println!("    {} Deep analysis failed: {}", "\u{26a0}".yellow(), e);
                None
            }
        };

        if all_violation_count == 0 {
            println!("    {} 0 boundary violations", "\u{2713}".green());
        }

        // Compute final score and grade. The tree-sitter score wins when the
        // deep pass ran; the offline heuristic is the fallback.
        let score = deep_score.unwrap_or_else(|| {
            let v = all_violation_count as u64;
            if v == 0 { 100 } else { 100u64.saturating_sub(v * 10) }
        });
        final_score = Some(score);
        let letter = grade_letter(score);
        let score_colored = match score {
            95..=100 => format!("{}", score).bright_green().to_string(),
            90..=94 => format!("{}", score).green().to_string(),
            70..=89 => format!("{}", score).yellow().to_string(),
            60..=69 => format!("{}", score).red().to_string(),
            _ => format!("{}", score).bright_red().to_string(),
        };

        println!();
        println!(
            "  {} Architecture grade: {} — score {}/100",
            "\u{2b21}".cyan(),
            letter.bold(),
            score_colored,
        );
        if let Some((violations, cycles, dead, unused)) = &score_components {
            println!(
                "    violations {} · cycles {} · dead exports {} · unused ports {}",
                violations,
                cycles,
                dead.len(),
                unused.len()
            );
            for d in dead.iter().take(8) {
                println!("      dead export   {}", d);
            }
            if dead.len() > 8 {
                println!("      … and {} more dead exports", dead.len() - 8);
            }
            for u in unused.iter().take(8) {
                println!("      unused port   {}", u);
            }
            println!("    {}", SCORE_FORMULA.dimmed());
            println!("    {}", GRADE_BANDS.dimmed());
            if *violations > 0 || *cycles > 0 || !dead.is_empty() || !unused.is_empty() {
                // Said here because it was not done: an agent relayed "B, 87,
                // unchanged" five times and fixed only the two items its own
                // diff had added. The grade is a property of the tree.
                println!(
                    "    {}",
                    "These are deductions to clear, not a status to report. Each item above names its fix; the grade is a property of the tree, not of your diff.".yellow()
                );
            }
        }
    }

    // Architectural-health detectors (ADR-2608241500 P6.5). Folded in from the
    // hexa-analyzer binary, which fed the improver daemon — and which nothing
    // has run since the daemon went. They report; they do not gate.
    if !violations_only && !quiet {
        println!();
        println!("  {}", "Architectural health:".bold());
        println!("    {}", HEALTH_NOTE.dimmed());
        for (label, outcome) in health_findings(&root) {
            match outcome {
                Health::Count(0, _) => println!("    {} {:<18} 0", "\u{2713}".green(), label),
                Health::Count(n, lines) => {
                    println!("    {} {:<18} {}", "\u{2022}".yellow(), label, n);
                    for l in lines.iter().take(5) {
                        println!("        {}", l.dimmed());
                    }
                    if lines.len() > 5 {
                        println!("        {}", format!("… and {} more", lines.len() - 5).dimmed());
                    }
                }
                // Not a pass. The detector did not look.
                Health::NotApplicable(why) => {
                    println!("    {} {:<18} n/a ({})", "\u{25cb}".dimmed(), label, why.dimmed())
                }
                Health::Failed(e) => println!("    {} {:<18} FAILED ({})", "\u{2717}".red(), label, e),
            }
        }
    }

    // ADR compliance check (ADR-045)
    if !violations_only {
        println!();
        println!("  {}", "ADR compliance:".bold());
    }
    let compliance = check_adr_compliance(&root);
    let adr_violations = &compliance.violations;
    let error_count = adr_violations.iter().filter(|v| v.severity == "error").count();
    let warning_count = adr_violations.iter().filter(|v| v.severity == "warning").count();

    if violations_only {
        // Print only violation lines, no summary
        for v in &local_violations {
            println!(
                "VIOLATION {} \u{2192} {} ({})",
                v.source_file, v.imported_path, v.rule,
            );
        }
        for v in &rust_violations {
            println!(
                "VIOLATION {}:{} — {}",
                v.file, v.line, v.message,
            );
        }
        for v in adr_violations {
            println!(
                "VIOLATION [{}] {}:{} — {}",
                v.adr, v.file, v.line, v.message,
            );
        }
    } else if let Some(reason) = &compliance.skipped {
        // Not a pass. Say so in the words of the thing that did not happen.
        println!(
            "    {} ADR rules NOT CHECKED — {}",
            "\u{25cb}".yellow(),
            reason,
        );
    } else if adr_violations.is_empty() {
        println!(
            "    {} All ADR rules satisfied",
            "\u{2713}".green()
        );
    } else {
        println!(
            "    {} {} ADR violation(s): {} error(s), {} warning(s)",
            "\u{26a0}".yellow(),
            adr_violations.len(),
            error_count,
            warning_count,
        );
        // One rule prints once: its id, severity, message, then every site.
        // Four sites of one rule printed the same paragraph four times.
        let mut order: Vec<(String, String)> = Vec::new();
        for v in adr_violations {
            let key = (v.id.clone(), v.adr.clone());
            if !order.contains(&key) {
                order.push(key);
            }
        }
        for (id, adr) in order {
            let sites: Vec<&AdrViolationLocal> =
                adr_violations.iter().filter(|v| v.id == id && v.adr == adr).collect();
            let first = sites[0];
            let icon = if first.severity == "error" {
                "\u{2717}".red()
            } else {
                "\u{26a0}".yellow()
            };
            println!(
                "    {} {} [{}] {} site{}, {}",
                icon,
                id.bold(),
                adr,
                sites.len(),
                if sites.len() == 1 { "" } else { "s" },
                first.severity
            );
            println!("      {}", first.message);
            for v in sites {
                println!("      {}:{}", v.file, v.line);
            }
        }
    }

    // Store compliance results in HexFlo memory (best-effort)

    // Boundary violations and ADR *errors* fail the gate. ADR warnings do not,
    // because that is what `--strict` is for. Counting warnings here made
    // `--strict` redundant and made hexa's own CI gate red on four warnings
    // that its own documentation classifies as advisory.
    let adr_errors = adr_violations.iter().filter(|v| v.severity == "error").count();
    let total_violations = all_violation_count + adr_errors;

    // --exit-code: exit 1 on any boundary violation or ADR error
    if exit_code && total_violations > 0 {
        std::process::exit(1);
    }

    // --grade: the letter is a gate only when asked for. A grade with no
    // floor passes by definition, and a reader who wants B to fail says so.
    if let Some(floor) = grade_floor {
        let floor = floor.to_uppercase();
        if grade_rank(&floor) == 0 && floor != "F" {
            anyhow::bail!("--grade {floor}: not a grade (A+, A, B, C, D, F)");
        }
        match final_score {
            Some(score) => {
                let letter = grade_letter(score);
                if grade_rank(letter) < grade_rank(&floor) {
                    println!();
                    println!(
                        "  {} grade {} is below the floor {} (score {}/100)",
                        "\u{2717}".red(),
                        letter.bold(),
                        floor.bold(),
                        score
                    );
                    std::process::exit(1);
                }
            }
            None => anyhow::bail!("--grade {floor}: no grade was computed"),
        }
    }

    // --strict: exit with code 1 if any violations exist (warnings promoted to errors)
    if strict && !adr_violations.is_empty() {
        if !violations_only {
            println!();
            println!(
                "  {} --strict mode: {} violation(s) found — exiting with code 1",
                "\u{2717}".red(),
                adr_violations.len(),
            );
        }
        std::process::exit(1);
    }

    Ok(())
}

// ── Single-file analysis (--file flag) ─────────────────────────────────

/// Analyze a single file for hexa boundary violations.
/// Used by PostToolUse hooks to check one file at a time.
/// Run the six architectural-health detectors, returning `(label, count)`.
///
/// Counts only: the detail is large and belongs in `--json` or a dedicated
/// report, and a wall of findings on every `hexa analyze` trains people to
/// ignore the whole section. A detector that errors reports 0 rather than
/// failing the analysis — these are advisory.
/// What a health detector said, kept distinct so the display cannot round a
/// decline or a failure down to a green zero.
enum Health {
    /// The count and one line per finding, so a number can be acted on.
    Count(usize, Vec<String>),
    NotApplicable(String),
    Failed(String),
}

fn health_findings(root: &Path) -> Vec<(&'static str, Health)> {
    use hexa_analysis::analyzers::*;
    // Every display detector reports either a count, a reason it did not
    // look, or the error that stopped it. A zero is only printed when the
    // detector looked.
    fn health<T>(
        r: anyhow::Result<T>,
        declined: impl Fn(&T) -> Option<String>,
        describe: impl Fn(&T) -> Vec<String>,
    ) -> Health {
        match r {
            Ok(v) => match declined(&v) {
                Some(why) => Health::NotApplicable(why),
                None => {
                    let lines = describe(&v);
                    Health::Count(lines.len(), lines)
                }
            },
            Err(e) => Health::Failed(e.to_string()),
        }
    }
    vec![
        (
            "cohesion",
            health(cohesion::analyze(root), |r| r.not_applicable.clone(), |r| {
                r.findings
                    .iter()
                    .map(|f| format!("{} {}:{} ({} methods in {} clusters)", f.port, f.file, f.line, f.method_count, f.clusters.len()))
                    .collect()
            }),
        ),
        (
            "duplication",
            health(duplication::analyze(root), |r| r.not_applicable.clone(), |r| {
                r.findings
                    .iter()
                    .map(|f| {
                        format!(
                            "{}: {} and {} ({}:{}, {}:{}, {:.0}% alike)",
                            f.port, f.adapter_a, f.adapter_b, f.file_a, f.line_a, f.file_b, f.line_b, f.similarity * 100.0
                        )
                    })
                    .collect()
            }),
        ),
        (
            "god types",
            health(
                god_types::analyze(root, god_types::GodTypeThresholds::from_project_root(root)),
                |r| r.not_applicable.clone(),
                |r| r.findings.iter().map(|f| format!("{} {} ({} lines)", f.type_name, f.file, f.lines)).collect(),
            ),
        ),
        (
            "dead layers",
            health(dead_layer::analyze(root), |r| r.not_applicable.clone(), |r| {
                r.findings.iter().map(|f| format!("{} ({})", f.layer, f.layer_kind)).collect()
            }),
        ),
        (
            "orphans",
            health(
                orphan::analyze(root, orphan::OrphanOptions { orphan_adapters: true, orphan_ports: true }),
                |r| r.not_applicable.clone(),
                |r| {
                    r.findings
                        .iter()
                        .map(|f| format!("{} {} {}:{}", f.kind, f.adapter.clone().unwrap_or_else(|| f.port.clone()), f.file, f.line))
                        .collect()
                },
            ),
        ),
    ]
}

/// Run the full tree-sitter boundary analysis over `root`.
///
/// `hexa-analysis` is the crate that enforces the hexagonal rules this tool
/// sells. Until now only the daemon depended on it, so deleting the daemon
/// would have orphaned it and broken the verb P9.2 is measured on.
pub async fn deep_analysis(
    root: &Path,
) -> Result<hexa_analysis::domain::ArchAnalysisResult, hexa_analysis::ports::AnalysisError> {
    use hexa_analysis::ports::ArchAnalysisPort;
    let ast = std::sync::Arc::new(hexa_analysis::treesitter_adapter::TreeSitterAdapter::new());
    hexa_analysis::analyzer::ArchAnalyzer::new(ast).analyze(root).await
}

fn run_single_file(
    file_path: &str,
    root: &Path,
    quiet: bool,
    violations_only: bool,
    exit_code: bool,
) -> anyhow::Result<()> {
    let path = Path::new(file_path);
    let abs_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_else(|_| root.to_path_buf()).join(path)
    };

    if !abs_path.exists() {
        eprintln!("hexa analyze --file: file not found: {}", file_path);
        std::process::exit(2);
    }

    // Determine relative path from root for layer detection
    let rel = abs_path
        .strip_prefix(root)
        .unwrap_or(&abs_path)
        .to_string_lossy()
        .to_string();

    let ext = abs_path.extension().and_then(|e| e.to_str()).unwrap_or("");

    let mut violations: Vec<String> = Vec::new();

    match ext {
        "rs" => {
            // Rust boundary check on this single file
            let _src_dir = abs_path.parent().unwrap_or(root);
            // Walk up to find the crate's src/ dir
            let crate_src = find_crate_src_for_file(&abs_path);
            let rel_to_src = if let Some(ref cs) = crate_src {
                abs_path.strip_prefix(cs).unwrap_or(&abs_path).to_string_lossy().to_string()
            } else {
                rel.clone()
            };

            let layer = classify_rust_src_layer(&rel_to_src);
            if let Some(layer_name) = layer {
                let file_rel = abs_path.strip_prefix(root).unwrap_or(&abs_path).to_string_lossy().to_string();
                if let Ok(content) = std::fs::read_to_string(&abs_path) {
                    let mut in_test_section = false;
                    for (idx, line) in content.lines().enumerate() {
                        let trimmed = line.trim();
                        if trimmed == "#[cfg(test)]" { in_test_section = true; }
                        if in_test_section { continue; }
                        if !trimmed.starts_with("use ") { continue; }

                        if matches!(layer_name, "Domain" | "Ports")
                            && (trimmed.contains("::adapters")
                                || trimmed.contains("hexa_nexus::")
                                || trimmed.contains("hexa_cli::")
                                || trimmed.contains("hexa_agent::"))
                            {
                                violations.push(format!(
                                    "{}:{} — {} layer must not import from adapters/downstream: {}",
                                    file_rel, idx + 1, layer_name, trimmed.trim_end_matches(';')
                                ));
                            }
                        if layer_name == "Secondary Adapters" {
                            if let Some(rest) = trimmed.strip_prefix("use crate::adapters::") {
                                let import_mod = rest.split("::").next().unwrap_or("").trim_end_matches(';');
                                let current_mod = rel_to_src
                                    .trim_start_matches("adapters/")
                                    .split('/')
                                    .next()
                                    .unwrap_or("")
                                    .trim_end_matches(".rs");
                                if !import_mod.is_empty()
                                    && import_mod != current_mod
                                    && import_mod != "mod"
                                    && import_mod != "super"
                                {
                                    violations.push(format!(
                                        "{}:{} — Secondary adapter imports sibling '{}': {}",
                                        file_rel, idx + 1, import_mod, trimmed.trim_end_matches(';')
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        "ts" | "js" => {
            // TypeScript/JS boundary check using hexa_core
            let source_layer = boundary::detect_layer(&rel);
            if source_layer != Layer::Unknown && source_layer != Layer::CompositionRoot {
                if let Ok(contents) = std::fs::read_to_string(&abs_path) {
                    let imports = extract_import_paths(&contents, &rel);
                    let viols = boundary::validate_imports(&rel, &imports);
                    for v in viols {
                        violations.push(format!(
                            "{} \u{2192} {} ({})",
                            v.source_file, v.imported_path, v.rule
                        ));
                    }
                }
            }
        }
        "go" => {
            // Go boundary check using hexa layer conventions
            if let Ok(content) = std::fs::read_to_string(&abs_path) {
                let file_rel = abs_path
                    .strip_prefix(root)
                    .unwrap_or(&abs_path)
                    .to_string_lossy()
                    .to_string();

                // Detect Go module prefix from go.mod for import resolution
                let go_mod_prefix = find_go_module_prefix(root);

                // Classify this file's layer
                let layer_name = classify_go_layer(&file_rel);

                if let Some(layer) = layer_name {
                    for (idx, line) in content.lines().enumerate() {
                        let trimmed = line.trim();
                        // Match Go import lines: "path" or named imports
                        if !trimmed.starts_with('"') && !trimmed.starts_with("//") {
                            continue;
                        }
                        if !trimmed.starts_with('"') {
                            continue;
                        }

                        let import_path = trimmed.trim_matches('"');

                        // Resolve to project-relative path
                        let resolved = if let Some(ref prefix) = go_mod_prefix {
                            if let Some(rest) = import_path.strip_prefix(prefix.as_str()) {
                                rest.strip_prefix('/').unwrap_or(rest).to_string()
                            } else {
                                continue; // stdlib or external — skip
                            }
                        } else {
                            continue; // Can't resolve without go.mod
                        };

                        let target_layer = classify_go_layer(&resolved);

                        // Enforce hexa rules
                        if let Some(target) = &target_layer {
                            let violation = match layer.as_str() {
                                "domain" => {
                                    if target != "domain" {
                                        Some(format!("domain must not import {}", target))
                                    } else {
                                        None
                                    }
                                }
                                "ports" => {
                                    if target != "domain" && target != "ports" {
                                        Some(format!("ports must not import {}", target))
                                    } else {
                                        None
                                    }
                                }
                                "adapters" => {
                                    // Check cross-adapter imports
                                    if target == "adapters" && resolved != file_rel {
                                        Some("adapters must not import other adapters".to_string())
                                    } else {
                                        None
                                    }
                                }
                                _ => None,
                            };

                            if let Some(rule) = violation {
                                violations.push(format!(
                                    "{}:{} — {} layer violation: {} (imports {})",
                                    file_rel,
                                    idx + 1,
                                    layer,
                                    rule,
                                    import_path,
                                ));
                            }
                        }
                    }
                }
            }
        }
        _ => {
            // Unsupported extension — nothing to check
        }
    }

    if violations.is_empty() {
        if !quiet && !violations_only {
            println!("\u{2713} {}", file_path);
        }
        return Ok(());
    }

    // Print violations
    for v in &violations {
        println!("VIOLATION {}", v);
    }

    // Single-file mode always exits 1 on violations (designed for hook use)
    let _ = exit_code; // acknowledged — single-file always exits 1 with violations
    std::process::exit(1);
}

/// Walk up from a file to find the enclosing crate's `src/` directory.
fn find_crate_src_for_file(file: &Path) -> Option<PathBuf> {
    let mut dir = file.parent()?;
    loop {
        if dir.join("Cargo.toml").is_file() {
            let src = dir.join("src");
            if src.is_dir() {
                return Some(src);
            }
            return None;
        }
        dir = dir.parent()?;
    }
}

// ── Go Layer Classification ─────────────────────────────────────────────

#[allow(dead_code)]
struct GoLayerRule {
    label: &'static str,
    layer: &'static str,
    signals: &'static [&'static str],
    matches: fn(&str) -> bool,
}

fn match_go_domain(s: &str) -> bool { s.contains("internal/domain") }
fn match_go_ports(s: &str) -> bool { s.contains("internal/ports") }
fn match_go_usecases(s: &str) -> bool { s.contains("internal/usecases") }
fn match_go_adapters(s: &str) -> bool {
    s.contains("internal/adapters") || s.contains("cmd/") || s.contains("pkg/")
}
fn match_go_internal_fallback(s: &str) -> bool { s.contains("internal/") }

static GO_LAYER_RULES: &[GoLayerRule] = &[
    GoLayerRule { label: "domain", layer: "domain", signals: &["internal/domain"], matches: match_go_domain },
    GoLayerRule { label: "ports", layer: "ports", signals: &["internal/ports"], matches: match_go_ports },
    GoLayerRule { label: "usecases", layer: "usecases", signals: &["internal/usecases"], matches: match_go_usecases },
    GoLayerRule { label: "adapters", layer: "adapters", signals: &["internal/adapters", "cmd/", "pkg/"], matches: match_go_adapters },
    GoLayerRule { label: "internal_fallback", layer: "usecases", signals: &["internal/"], matches: match_go_internal_fallback },
];

/// Classify a Go file path into its hexagonal layer.
fn classify_go_layer(path: &str) -> Option<String> {
    GO_LAYER_RULES
        .iter()
        .find(|r| (r.matches)(path))
        .map(|r| r.layer.to_string())
}

/// Read go.mod to extract the module path.
fn find_go_module_prefix(root: &Path) -> Option<String> {
    let go_mod = root.join("go.mod");
    if let Ok(content) = std::fs::read_to_string(go_mod) {
        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("module ") {
                return Some(rest.trim().to_string());
            }
        }
    }
    None
}

// ── Rust Workspace Analysis (ADR-2026-03-28-3000) ────────────────────────────

/// A boundary violation found in Rust source.
pub struct RustViolation {
    pub file: String,
    pub line: usize,
    pub message: String,
}

/// Find workspace crate directories up to two levels deep.
///
/// Scans direct subdirectories AND their subdirectories for `Cargo.toml` files,
/// so nested workspaces like `spacetime-modules/hexflo-coordination/` are included.
/// Excludes `target/` directories and git worktrees (`hexa-worktrees*/`).
fn find_workspace_crate_dirs(root: &Path) -> Vec<PathBuf> {
    // Recursive, bounded. The previous scan went exactly two levels deep, which misses the common
    // hexagonal layout that groups adapter crates under a parent directory:
    //
    //   okf-adapters/secondary/yaml-serde/Cargo.toml     <- depth 3, previously invisible
    //
    // A workspace whose adapters are invisible reports no adapter layers and no adapter boundary
    // violations, which reads as a clean result rather than an incomplete scan.
    const MAX_DEPTH: usize = 4;
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        if depth > MAX_DEPTH {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // Never descend into build output, VCS/tooling dirs, or a crate's own sources.
            if name == "target"
                || name == "src"
                || name == "node_modules"
                || name.starts_with("hexa-worktrees")
                || name.starts_with('.')
            {
                continue;
            }
            if path.join("Cargo.toml").is_file() {
                out.push(path.clone());
            }
            // Keep descending regardless: a crate may contain nested member crates.
            walk(&path, depth + 1, out);
        }
    }
    let mut dirs = Vec::new();
    walk(root, 1, &mut dirs);
    dirs
}

#[allow(dead_code)]
struct RustLayerRule {
    label: &'static str,
    layer: &'static str,
    signals: &'static [&'static str],
    matches: fn(&str) -> bool,
}

fn match_rust_primary(s: &str) -> bool {
    s.starts_with("adapters/primary/") || s.starts_with("adapters/primary.rs")
        || s.starts_with("commands/") || s.starts_with("routes/")
}
fn match_rust_secondary(s: &str) -> bool {
    s.starts_with("adapters/secondary/") || s.starts_with("adapters/secondary.rs")
        || s.starts_with("adapters/")
}
fn match_rust_domain(s: &str) -> bool {
    s.starts_with("domain/") || s.starts_with("domain.rs")
}
fn match_rust_ports(s: &str) -> bool {
    s.starts_with("ports/") || s.starts_with("ports.rs")
}
fn match_rust_usecases(s: &str) -> bool {
    s.starts_with("orchestration/") || s.starts_with("usecases/")
}

static RUST_LAYER_RULES: &[RustLayerRule] = &[
    RustLayerRule { label: "primary_adapters", layer: "Primary Adapters", signals: &["adapters/primary/", "commands/", "routes/"], matches: match_rust_primary },
    RustLayerRule { label: "secondary_adapters", layer: "Secondary Adapters", signals: &["adapters/secondary/", "adapters/"], matches: match_rust_secondary },
    RustLayerRule { label: "domain", layer: "Domain", signals: &["domain/", "domain.rs"], matches: match_rust_domain },
    RustLayerRule { label: "ports", layer: "Ports", signals: &["ports/", "ports.rs"], matches: match_rust_ports },
    RustLayerRule { label: "usecases", layer: "Use Cases", signals: &["orchestration/", "usecases/"], matches: match_rust_usecases },
];


/// Classify a hexa layer from the CRATE's path, for workspaces that put one layer per crate.
///
/// Two layouts are idiomatic for hexagonal Rust, and hexa must read both:
///
///   dir-per-layer     `hexa-core/src/domain/tokens.rs`        layer is a subdirectory of src/
///   crate-per-layer   `okf-domain/src/lib.rs`                layer IS the crate
///
/// Only the first was recognised, so a crate-per-layer workspace classified every file as
/// Infrastructure and `analyze` reported score 100 with zero violations — a vacuous pass
/// indistinguishable from a clean one. That is the layout hexa should most approve of: with a crate
/// per layer, Cargo refuses to resolve an undeclared import, so a boundary violation is a compile
/// error rather than something a linter has to notice.
///
/// `crate_rel` is the crate directory relative to the workspace root, so nested adapter crates
/// (`okf-adapters/secondary/yaml-serde`) are classified by their path, and flat layer crates
/// (`okf-domain`) by the last `-`/`_` separated segment of the directory name.
fn classify_rust_crate_layer(crate_rel: &str) -> Option<&'static str> {
    let p = crate_rel.replace('\\', "/");
    // Path form first: an adapter crate says which side it is on by where it lives.
    if p.contains("adapters/primary/") {
        return Some("Primary Adapters");
    }
    if p.contains("adapters/secondary/") {
        return Some("Secondary Adapters");
    }
    // Otherwise the crate NAME carries the layer: okf-domain, my_app_ports, usecases.
    let last = p.rsplit('/').next().unwrap_or(&p);
    let seg = last.rsplit(['-', '_']).next().unwrap_or(last);
    match seg {
        "domain" => Some("Domain"),
        "ports" | "port" => Some("Ports"),
        "usecases" | "usecase" | "orchestration" => Some("Use Cases"),
        _ => None,
    }
}

/// Layer for a file, from the crate it lives in and its path under `src/`.
/// The `src/` path wins so existing dir-per-layer workspaces classify exactly as before.
fn classify_rust_layer(crate_rel: &str, rel_to_src: &str) -> Option<&'static str> {
    classify_rust_src_layer(rel_to_src).or_else(|| classify_rust_crate_layer(crate_rel))
}

/// Classify a path relative to a crate's `src/` directory into a hexa layer label.
/// Returns `None` for infrastructure (unclassified) files.
fn classify_rust_src_layer(rel_to_src: &str) -> Option<&'static str> {
    let p = rel_to_src.replace('\\', "/");
    RUST_LAYER_RULES
        .iter()
        .find(|r| (r.matches)(&p))
        .map(|r| r.layer)
}

/// Scan Rust workspace crates and return layer label → file count aggregated across all crates.
fn scan_rust_workspace_layers(root: &Path) -> Vec<(String, usize)> {
    let crate_dirs = find_workspace_crate_dirs(root);
    let mut counts: std::collections::HashMap<&'static str, usize> = std::collections::HashMap::new();
    let mut infra_count = 0usize;

    for crate_dir in &crate_dirs {
        let src_dir = crate_dir.join("src");
        if !src_dir.is_dir() {
            continue;
        }
        let files = collect_rust_files(&src_dir);
        for file in &files {
            let rel = file
                .strip_prefix(&src_dir)
                .unwrap_or(file)
                .to_string_lossy()
                .to_string();
            let crate_rel = crate_dir.strip_prefix(root).unwrap_or(crate_dir).to_string_lossy().to_string();
            match classify_rust_layer(&crate_rel, &rel) {
                Some(layer) => *counts.entry(layer).or_insert(0) += 1,
                None => infra_count += 1,
            }
        }
    }

    let order = ["Domain", "Ports", "Use Cases", "Primary Adapters", "Secondary Adapters"];
    let mut result: Vec<(String, usize)> = order
        .iter()
        .filter(|&&l| counts.get(l).copied().unwrap_or(0) > 0)
        .map(|&l| (l.to_string(), counts[l]))
        .collect();
    if infra_count > 0 {
        result.push(("Infrastructure".to_string(), infra_count));
    }
    result
}

/// Returns true if the file path is inside a test directory or is a test file.
fn is_test_path(path: &Path) -> bool {
    path.components()
        .any(|c| c.as_os_str() == "tests" || c.as_os_str() == "test")
        || path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.ends_with("_test.rs") || n.ends_with("_tests.rs"))
            .unwrap_or(false)
}

/// Scan Rust workspace files for hexa boundary violations via `use` statement analysis.
fn scan_rust_boundary_violations(root: &Path) -> Vec<RustViolation> {
    let crate_dirs = find_workspace_crate_dirs(root);
    let mut violations = Vec::new();

    for crate_dir in &crate_dirs {
        let src_dir = crate_dir.join("src");
        if !src_dir.is_dir() {
            continue;
        }
        let files = collect_rust_files(&src_dir);
        for file_path in &files {
            if is_test_path(file_path) {
                continue;
            }
            let rel_to_src = file_path
                .strip_prefix(&src_dir)
                .unwrap_or(file_path)
                .to_string_lossy()
                .to_string();
            let crate_rel = crate_dir.strip_prefix(root).unwrap_or(crate_dir).to_string_lossy().to_string();
            let Some(layer) = classify_rust_layer(&crate_rel, &rel_to_src) else {
                continue;
            };
            let file_rel = file_path
                .strip_prefix(root)
                .unwrap_or(file_path)
                .to_string_lossy()
                .to_string();

            let Ok(content) = std::fs::read_to_string(file_path) else {
                continue;
            };

            // Once we see #[cfg(test)] we're in the test section at end of file
            let mut in_test_section = false;
            for (idx, line) in content.lines().enumerate() {
                let trimmed = line.trim();
                if trimmed == "#[cfg(test)]" {
                    in_test_section = true;
                }
                if in_test_section {
                    continue;
                }
                if !trimmed.starts_with("use ") {
                    continue;
                }

                // Rule 1: Domain and Ports must not import from adapters or downstream crates
                if matches!(layer, "Domain" | "Ports")
                    && (trimmed.contains("::adapters")
                        || trimmed.contains("hexa_nexus::")
                        || trimmed.contains("hexa_cli::")
                        || trimmed.contains("hexa_agent::"))
                    {
                        violations.push(RustViolation {
                            file: file_rel.clone(),
                            line: idx + 1,
                            message: format!(
                                "{} layer must not import from adapters/downstream crates: {}",
                                layer,
                                trimmed.trim_end_matches(';')
                            ),
                        });
                    }

                // Rule 2: Secondary adapters must not import sibling secondary adapters
                if layer == "Secondary Adapters" {
                    // use crate::adapters::<sibling>::
                    if let Some(rest) = trimmed.strip_prefix("use crate::adapters::") {
                        let import_mod = rest.split("::").next().unwrap_or("").trim_end_matches(';');
                        // Derive the current file's module name (e.g. "adapters/foo.rs" → "foo")
                        let current_mod = rel_to_src
                            .trim_start_matches("adapters/")
                            .split('/')
                            .next()
                            .unwrap_or("")
                            .trim_end_matches(".rs");
                        if !import_mod.is_empty()
                            && import_mod != current_mod
                            && import_mod != "mod"
                            && import_mod != "super"
                        {
                            violations.push(RustViolation {
                                file: file_rel.clone(),
                                line: idx + 1,
                                message: format!(
                                    "Secondary adapter imports sibling adapter '{}': {}",
                                    import_mod,
                                    trimmed.trim_end_matches(';')
                                ),
                            });
                        }
                    }
                }
            }
        }
    }

    violations
}

/// Collect only `.rs` files recursively under a directory.
fn collect_rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_rust_files_recursive(dir, &mut files);
    files
}

fn collect_rust_files_recursive(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files_recursive(&path, out);
        } else if path.extension().and_then(|x| x.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// Scan source files for boundary violations using `hexa_core::rules::boundary`.
///
/// This performs a lightweight offline check by inspecting Rust `use` and
/// TypeScript `import` statements without needing tree-sitter.
fn scan_local_violations(root: &Path) -> Vec<boundary::Violation> {
    let src = root.join("src");
    let mut all_violations = Vec::new();

    let files = collect_source_files(&src);

    for path in &files {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        // Test code is exempt. A use case under test constructing the in-memory
        // adapter is the pattern ports-and-adapters exists to enable, not a
        // boundary breach — the dependency is compiled out of the shipped
        // artifact. Counting it turns a real finding into noise: measured on the
        // homelab project, 8 of 12 reported violations were test doubles, which
        // is how the 4 genuine ones stayed invisible behind a grade of F.
        if is_test_file(&rel) {
            continue;
        }

        let source_layer = boundary::detect_layer(&rel);
        if source_layer == Layer::Unknown || source_layer == Layer::CompositionRoot {
            continue;
        }

        // Read file and extract import-like paths (best-effort, not a full parser)
        if let Ok(contents) = std::fs::read_to_string(path) {
            let imports = extract_import_paths(&contents, &rel);
            let violations = boundary::validate_imports(&rel, &imports);
            all_violations.extend(violations);
        }
    }

    all_violations
}

/// Whether a path is test code, by the naming convention of its language.
///
/// Deliberately applied at the violation scan rather than in
/// [`collect_source_files`], which also feeds the per-layer file counts —
/// excluding tests there would silently shrink the reported size of the
/// project.
fn is_test_file(rel: &str) -> bool {
    let p = rel.replace('\\', "/").to_lowercase();
    let name = p.rsplit('/').next().unwrap_or(&p);

    // Go and Rust integration tests carry it in the filename or the directory.
    name.ends_with("_test.go")
        || name.ends_with("_test.rs")
        || p.contains("/tests/")
        || p.starts_with("tests/")
        // TS/JS colocated tests: foo.test.ts, foo.spec.tsx, …
        || name.contains(".test.")
        || name.contains(".spec.")
}

/// Recursively collect `.rs`, `.ts`, `.js`, `.go`, and `.py` source files under a directory.
fn collect_source_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_source_files_recursive(dir, &mut files);
    files
}

fn collect_source_files_recursive(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if matches!(name.as_ref(), "target" | "node_modules" | "vendor" | ".git" | "dist") {
                continue;
            }
            collect_source_files_recursive(&path, out);
        } else if let Some(ext) = path.extension().and_then(|x| x.to_str()) {
            if matches!(ext, "rs" | "ts" | "js" | "go" | "py") {
                out.push(path);
            }
        }
    }
}

/// Extract import/use paths from source text, resolving relative paths against
/// the source file's directory so layer detection works correctly.
///
/// `source_rel` is the source file path relative to the project root,
/// e.g. `src/core/ports/app-context.ts`.
fn extract_import_paths(source: &str, source_rel: &str) -> Vec<String> {
    let mut paths = Vec::new();

    // Directory of the source file (e.g. "src/core/ports" for "src/core/ports/app-context.ts")
    let source_dir = Path::new(source_rel)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    // Rust puts its unit tests *inside* the file it tests, behind `#[cfg(test)]`.
    // Those imports never reach the shipped binary, so an adapter imported there
    // is a test double rather than a boundary breach. Tracked by brace depth:
    // the attribute arms the skip, the next line that opens a block starts it,
    // and returning to the depth it started at ends it.
    let mut cfg_test_armed = false;
    let mut test_block_depth: Option<i32> = None;
    let mut depth: i32 = 0;

    for line in source.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("#[cfg(test)]") {
            cfg_test_armed = true;
        }

        let opens = i32::try_from(line.matches('{').count()).unwrap_or(i32::MAX);
        let closes = i32::try_from(line.matches('}').count()).unwrap_or(i32::MAX);

        if cfg_test_armed && opens > 0 {
            test_block_depth = Some(depth);
            cfg_test_armed = false;
        }

        let in_test_block = test_block_depth.is_some();
        depth += opens - closes;
        if let Some(started_at) = test_block_depth {
            if depth <= started_at {
                test_block_depth = None;
            }
        }

        if in_test_block {
            continue;
        }

        // Rust: use crate::adapters::...
        if let Some(rest) = trimmed.strip_prefix("use crate::") {
            if let Some(path_part) = rest.split(';').next() {
                // Convert module path to a directory-like path for layer detection
                let as_path = format!("src/{}", path_part.split("::").collect::<Vec<_>>().join("/"));
                paths.push(as_path);
            }
        }
        // TypeScript: import ... from './adapters/...'  or  from '../adapters/...'
        // Only match lines that start with import/export to avoid false positives
        // from string literals containing import-like text (e.g. template content).
        if trimmed.contains("from")
            && (trimmed.starts_with("import ") || trimmed.starts_with("export "))
        {
            if let Some(start) = trimmed.find('\'').or_else(|| trimmed.find('"')) {
                let rest = &trimmed[start + 1..];
                if let Some(end) = rest.find('\'').or_else(|| rest.find('"')) {
                    let import_path = &rest[..end];
                    if import_path.starts_with('.') {
                        // Resolve relative path against source file's directory
                        let resolved = resolve_relative_path(&source_dir, import_path);
                        paths.push(resolved);
                    }
                }
            }
        }
    }
    paths
}

/// Resolve a relative import path (e.g. `../../core/ports/swarm.js`)
/// against a source directory (e.g. `src/core/ports`).
///
/// Returns a normalized path like `src/core/ports/swarm.js`.
fn resolve_relative_path(source_dir: &str, import_path: &str) -> String {
    let mut parts: Vec<&str> = source_dir.split('/').filter(|s| !s.is_empty()).collect();

    for segment in import_path.split('/') {
        match segment {
            "." | "" => {} // current dir — skip
            ".." => { parts.pop(); } // go up one level
            other => parts.push(other),
        }
    }

    parts.join("/")
}

// ── ADR Compliance (ADR-045) ────────────────────────────
// Rules are loaded from the project's `.hexa/ADR-rules.toml` — hexa ships no
// project-specific rules. The engine is the framework; the rules are the project's.

struct AdrViolationLocal {
    adr: String,
    id: String,
    file: String,
    line: usize,
    message: String,
    severity: String,
}

#[derive(serde::Deserialize)]
struct AdrRulesFile {
    /// ADR compliance rules — TOML key `[[adr_rules]]`
    /// (distinct from `[rules]` which is the enforce.rs forbidden-paths table)
    #[serde(default)]
    adr_rules: Vec<AdrRuleConfig>,
}

#[derive(serde::Deserialize)]
struct AdrRuleConfig {
    adr: String,
    id: String,
    message: String,
    #[serde(default = "default_severity")]
    severity: String,
    /// A line matching any of these is not a violation of this rule, even
    /// when it matches a violation pattern. This is how a text rule admits
    /// what it cannot tell apart: `x.trunc() as u32` is a float cast, and
    /// the float is visible on the line.
    #[serde(default)]
    allow_line_patterns: Vec<String>,
    #[serde(default)]
    file_patterns: Vec<String>,
    #[serde(default)]
    exclude_patterns: Vec<String>,
    #[serde(default)]
    violation_patterns: Vec<String>,
}

fn default_severity() -> String { "warning".to_string() }

/// The result of an ADR compliance run, which is *not* the same thing as a
/// list of violations.
///
/// An empty list means one of two opposite things: every rule passed, or no
/// rule ran. Collapsing them is the silent-fallback failure this codebase has
/// already made once — `hexa ci`'s boundary check fell back to `cargo check`
/// when its analyzer was unreachable and still printed a result, quietly
/// turning "no boundary violations" into "it compiles". Reporting a green tick
/// for a check that did not happen is the same bug in a smaller hat, and until
/// this type existed `hexa analyze` printed exactly that: "skipping compliance
/// check" immediately followed by "✓ All ADR rules satisfied".
struct AdrCompliance {
    /// `None` when the rules ran. `Some(reason)` when they did not.
    skipped: Option<String>,
    violations: Vec<AdrViolationLocal>,
}

impl AdrCompliance {
    fn skipped(reason: impl Into<String>) -> Self {
        Self { skipped: Some(reason.into()), violations: Vec::new() }
    }
    fn ran(violations: Vec<AdrViolationLocal>) -> Self {
        Self { skipped: None, violations }
    }
}

/// Line numbers (0-based) that sit inside a `#[cfg(test)]` module.
///
/// Rust does not put its unit tests in a separate file, so a rule's
/// path-based `exclude_patterns` cannot reach them: `worktree.rs` is not a
/// test path, but the bottom third of it is nothing but tests. Without this,
/// every fixture string in the workspace is reported as production code — a
/// test asserting `detect_from_model_name("qwen3:32b")` was flagged as naming
/// a model outside the inference boundary, which is the rule firing on the
/// code that proves the rule's own subject works. A rule that flags correct
/// code is worse than no rule: it teaches people to skim past the output.
///
/// **Indentation, not brace counting.** The first version of this counted
/// braces and was immediately fooled by a `{` inside a string literal — it
/// swallowed the remaining 300 lines of `init.rs` and silently switched every
/// rule off for that file. Nothing reported an error; the violation count just
/// went down, which looks exactly like progress. That is the silent-fallback
/// failure this rule set exists to name, committed by the rule set's own
/// engine.
///
/// `#[cfg(test)] mod tests { … }` closes with a `}` at the attribute's own
/// indentation, which no string literal can imitate, because rustfmt owns the
/// left margin.
///
/// If no matching close is found, this skips **nothing** for that attribute.
/// A missed exclusion shows up as noise, which someone reads; an over-broad
/// one shows up as silence, which nobody does.
fn cfg_test_lines(content: &str) -> std::collections::HashSet<usize> {
    let mut skip = std::collections::HashSet::new();
    let lines: Vec<&str> = content.lines().collect();
    let indent_of = |l: &str| l.len() - l.trim_start().len();

    let mut i = 0;
    while i < lines.len() {
        if !lines[i].trim_start().starts_with("#[cfg(test)]") {
            i += 1;
            continue;
        }
        let want = indent_of(lines[i]);
        let close = (i + 1..lines.len()).find(|&k| {
            let t = lines[k].trim_start();
            t.starts_with('}') && indent_of(lines[k]) == want
        });
        match close {
            Some(k) => {
                skip.extend(i..=k);
                i = k + 1;
            }
            None => i += 1,
        }
    }
    skip
}

#[cfg(test)]
mod cfg_test_lines_tests {
    use super::cfg_test_lines;

    #[test]
    fn it_skips_a_top_level_test_module_and_nothing_after_it() {
        let src = "fn a() {}\n#[cfg(test)]\nmod t {\n    fn b() {}\n}\nfn c() {}\n";
        let skip = cfg_test_lines(src);
        assert!(!skip.contains(&0), "production code before the module");
        for n in 1..=4 {
            assert!(skip.contains(&n), "line {n} is inside the test module");
        }
        assert!(!skip.contains(&5), "production code after the module");
    }

    /// The regression that motivated the rewrite: a brace inside a string
    /// literal must not extend the module to the end of the file.
    #[test]
    fn a_brace_in_a_string_does_not_swallow_the_rest_of_the_file() {
        let src = "#[cfg(test)]\nmod t {\n    let s = \"{unclosed\";\n}\nfn after() {}\n";
        let skip = cfg_test_lines(src);
        assert!(skip.contains(&2), "the string line is inside the module");
        assert!(!skip.contains(&4), "`fn after` is production code and must be scanned");
    }

    /// Loud over quiet: an unterminated module excludes nothing rather than
    /// silently disabling every rule for the rest of the file.
    #[test]
    fn an_unterminated_module_skips_nothing() {
        let src = "#[cfg(test)]\nmod t {\n    let _ = 1;\n";
        assert!(cfg_test_lines(src).is_empty());
    }
}

fn check_adr_compliance(root: &Path) -> AdrCompliance {
    // Load rules from project's .hexa/ADR-rules.toml
    let rules_path = root.join(".hexa").join("ADR-rules.toml");
    let rules = if rules_path.is_file() {
        match std::fs::read_to_string(&rules_path) {
            Ok(content) => match toml::from_str::<AdrRulesFile>(&content) {
                Ok(parsed) => {
                    eprintln!(
                        "    {} Loaded {} rule(s) from {}",
                        "\u{2713}".green(),
                        parsed.adr_rules.len(),
                        rules_path.strip_prefix(root).unwrap_or(&rules_path).display(),
                    );
                    parsed.adr_rules
                }
                Err(e) => {
                    return AdrCompliance::skipped(format!(
                        "{} is present but does not parse: {e}",
                        rules_path.strip_prefix(root).unwrap_or(&rules_path).display()
                    ));
                }
            },
            Err(e) => {
                return AdrCompliance::skipped(format!(
                    "{} could not be read: {e}",
                    rules_path.strip_prefix(root).unwrap_or(&rules_path).display()
                ));
            }
        }
    } else {
        return AdrCompliance::skipped(
            "no .hexa/ADR-rules.toml — run `hexa init` to write the shipped rule set",
        );
    };

    let active_rules: Vec<&AdrRuleConfig> = rules
        .iter()
        .filter(|r| !r.violation_patterns.is_empty())
        .collect();

    let mut violations = Vec::new();
    // Every `src/` in the project: the root one, plus one per crate or package
    // in a workspace. This used to be a hardcoded list of two directory names,
    // one of which named a crate that no longer exists — so in an eight-crate
    // workspace the rules were checked against one crate and reported as
    // though they had been checked against all of them.
    let mut all_files = collect_source_files(&root.join("src"));
    let mut members: Vec<PathBuf> = std::fs::read_dir(root)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path().join("src"))
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default();
    members.sort();
    for sub in members {
        all_files.extend(collect_source_files(&sub));
    }

    for path in &all_files {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        // Rust keeps its unit tests inline, so path exclusions miss them.
        let inline_tests = if rel.ends_with(".rs") {
            cfg_test_lines(&content)
        } else {
            std::collections::HashSet::new()
        };

        for rule in &active_rules {
            if !rule.file_patterns.is_empty()
                && !rule.file_patterns.iter().any(|p| rel.ends_with(p.as_str()))
            {
                continue;
            }
            if rule.exclude_patterns.iter().any(|p| rel.contains(p.as_str())) {
                continue;
            }

            for (line_num, line) in content.lines().enumerate() {
                if inline_tests.contains(&line_num) {
                    continue;
                }
                let trimmed = line.trim();
                if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
                    continue;
                }
                if rule.allow_line_patterns.iter().any(|p| line.contains(p.as_str())) {
                    continue;
                }
                for pattern in &rule.violation_patterns {
                    if line.contains(pattern.as_str()) {
                        violations.push(AdrViolationLocal {
                            adr: rule.adr.to_string(),
                            id: rule.id.clone(),
                            file: rel.clone(),
                            line: line_num + 1,
                            message: rule.message.to_string(),
                            severity: rule.severity.clone(),
                        });
                        break;
                    }
                }
            }
        }
    }

    AdrCompliance::ran(violations)
}

/// How the score is made, in one line. Printed under every grade and
/// carried in `--json` under `explain`, so a person and a model read the
/// same sentence.
const SCORE_FORMULA: &str =
    "score = 100 − 10·violations − 15·cycles − dead exports (max 20) − unused ports (max 10)";
const GRADE_BANDS: &str = "A+ 95–100 · A 90–94 · B 80–89 · C 70–79 · D 60–69 · F below 60";
const HEALTH_NOTE: &str = "read next to the grade; none of these move the score";

/// The `explain` block of `--json`: what each number means, what moves the
/// score by how much, what each gate does, and what fixes each finding. A
/// model that reads the JSON gets this with it, so the report can be acted
/// on without a person to interpret it.
fn explain_json() -> serde_json::Value {
    serde_json::json!({
        "score": {
            "formula": "100 - 10*violations - 15*circular_deps - min(dead_exports, 20) - min(unused_ports, 10)",
            "grade_bands": { "A+": "95-100", "A": "90-94", "B": "80-89", "C": "70-79", "D": "60-69", "F": "0-59" },
            "in_score": ["violations", "circular_deps", "dead_exports", "unused_ports"],
            "not_in_score": ["cohesion", "duplication", "god_types", "dead_layers", "orphans"]
        },
        "components": {
            "violations": {
                "weight": 10,
                "meaning": "an import that crosses a hexagonal boundary the wrong way (rules 1 to 6)",
                "fix": "import through the port; if the port does not re-export the type, add the re-export to the port first"
            },
            "circular_deps": {
                "weight": 15,
                "meaning": "modules that import each other, at module level: files in TypeScript, packages in Go, the module under src/ in Rust",
                "fix": "move the shared piece to the lower layer so the edge points one way"
            },
            "dead_exports": {
                "weight": 1, "cap": 20,
                "meaning": "an export that no other file names; a type its own file names again is not dead",
                "fix": "delete it, or make it private when its own file uses it; run `hexa graph consumers <path>` first"
            },
            "unused_ports": {
                "weight": 1, "cap": 10,
                "meaning": "a port that no adapter or use case names",
                "fix": "wire an adapter to it, or delete the port"
            }
        },
        "health": {
            "in_score": false,
            "meaning": "read next to the grade; a nonzero count is worth a look and is not a failure",
            "detectors": {
                "cohesion":    { "languages": ["rust"], "meaning": "a port whose methods fall into unrelated clusters; it may be two ports" },
                "duplication": { "languages": ["rust"], "meaning": "two adapters of one port whose bodies are mostly alike; the shared part may belong in one place" },
                "god_types":   { "languages": ["rust"], "meaning": "a domain type over the size thresholds in .hexa/project.json; it may be several types" },
                "dead_layers": { "languages": ["rust", "go", "typescript"], "meaning": "a layer directory nothing outside it imports" },
                "orphans":     { "languages": ["rust", "go", "typescript"], "meaning": "a port with no adapter behind it, or an adapter nothing wires" }
            },
            "n_a": "a detector that cannot read the project's language reports not_applicable, never zero"
        },
        "gates": {
            "--exit-code": "exit 1 on any boundary violation or rule error",
            "--grade <LETTER>": "exit 1 when the grade is below the letter",
            "--strict": "rule warnings count as errors"
        },
        "rules": "adr_compliance lists violations of .hexa/ADR-rules.toml with the rule's message; severity error fails --exit-code, warning fails --strict"
    })
}

/// The health detectors as JSON: count, findings, or the reason a
/// detector did not look.
fn health_json(root: &Path) -> serde_json::Value {
    let mut out = serde_json::Map::new();
    for (label, outcome) in health_findings(root) {
        let key = label.replace(' ', "_");
        let v = match outcome {
            Health::Count(n, lines) => serde_json::json!({ "count": n, "findings": lines }),
            Health::NotApplicable(why) => serde_json::json!({ "not_applicable": why }),
            Health::Failed(e) => serde_json::json!({ "failed": e }),
        };
        out.insert(key, v);
    }
    serde_json::Value::Object(out)
}

/// The letter for a 0..100 architecture score.
///
/// One table. `hexa scaffold` gates on this and `hexa analyze` prints it, and a
/// verb that gates on a different table than the one the user is shown is a
/// gate nobody can check.
pub fn grade_letter(score: u64) -> &'static str {
    match score {
        95..=100 => "A+",
        90..=94 => "A",
        80..=89 => "B",
        70..=79 => "C",
        60..=69 => "D",
        _ => "F",
    }
}

/// Rank a letter so grades can be compared. Higher is better.
pub fn grade_rank(letter: &str) -> u8 {
    match letter {
        "A+" => 5,
        "A" => 4,
        "B" => 3,
        "C" => 2,
        "D" => 1,
        _ => 0,
    }
}

#[cfg(test)]
mod grade_tests {
    use super::{grade_letter, grade_rank};

    #[test]
    fn every_band_maps_to_a_letter_and_the_letters_are_ordered() {
        let mut previous = 0u8;
        for score in 0..=100u64 {
            let rank = grade_rank(grade_letter(score));
            assert!(rank >= previous, "grade fell at score {score}");
            previous = rank;
        }
        assert_eq!(grade_letter(100), "A+");
        assert_eq!(grade_letter(94), "A");
        assert_eq!(grade_letter(0), "F");
    }

    #[test]
    fn an_unknown_letter_ranks_lowest_rather_than_passing_a_gate() {
        assert_eq!(grade_rank("Z"), 0);
        assert_eq!(grade_rank(""), 0);
    }
}

/// JSON output mode for `hexa analyze --json`.
async fn run_json(root: &Path, strict: bool, adr_compliance_only: bool) -> anyhow::Result<()> {
    let mut result = serde_json::json!({});

    if !adr_compliance_only {
        // Local boundary violations (TypeScript)
        let has_src = root.join("src").is_dir();
        let violations: Vec<serde_json::Value> = if has_src {
            scan_local_violations(root)
                .iter()
                .map(|v| {
                    serde_json::json!({
                        "source_file": v.source_file,
                        "imported_path": v.imported_path,
                        "rule": v.rule,
                    })
                })
                .collect()
        } else {
            Vec::new()
        };

        // Rust workspace layers and violations (ADR-2026-03-28-3000)
        let has_cargo_toml = root.join("Cargo.toml").is_file();
        let rust_layers_data: Vec<serde_json::Value> = if has_cargo_toml {
            scan_rust_workspace_layers(root)
                .iter()
                .map(|(label, count)| serde_json::json!({"layer": label, "file_count": count}))
                .collect()
        } else {
            Vec::new()
        };
        let rust_violations_data: Vec<serde_json::Value> = if has_cargo_toml {
            scan_rust_boundary_violations(root)
                .iter()
                .map(|v| serde_json::json!({"file": v.file, "line": v.line, "message": v.message}))
                .collect()
        } else {
            Vec::new()
        };

        // Full tree-sitter analysis, in-process (ADR-2608241500 P6.2).
        let mut score: Option<u64> = None;
        let mut boundary_errors: Vec<serde_json::Value> = Vec::new();
        if let Ok(deep) = deep_analysis(root).await {
            score = Some(deep.health_score as u64);
            // What the grade was computed over. A number that can move four
            // points depending on what is lying about in the working tree must
            // say how many files it read (ADR-2609141030, decision 4).
            // "A+ 96 over 150 files" is a claim; "A+ 96" is a rumour.
            result["files_analysed"] = serde_json::json!(deep.file_count);
            result["import_edges"] = serde_json::json!(deep.edge_count);
            if !deep.violations.is_empty() {
                boundary_errors.push(serde_json::json!({"count": deep.violations.len()}));
            }
            // The score's inputs, so a reader can see where the points went.
            // Without this a score of 78 with zero violations was
            // unexplainable from the output, and got misattributed.
            result["score_components"] = serde_json::json!({
                "violations": deep.violations.len(),
                "circular_deps": deep.circular_deps.len(),
                "dead_exports": deep.dead_exports.len(),
                "unused_ports": deep.unused_ports.len(),
            });
            // And the items themselves, so a count can be checked.
            result["unused_ports"] = serde_json::json!(deep.unused_ports);
            result["dead_exports"] = serde_json::json!(deep.dead_exports);
        }

        // Compute local score if nexus didn't provide one
        let total_violations = violations.len() + rust_violations_data.len();
        let final_score = score.unwrap_or_else(|| {
            let v = total_violations as u64;
            if v == 0 { 100 } else { 100u64.saturating_sub(v * 10) }
        });
        result["score"] = serde_json::json!(final_score);
        result["grade"] = serde_json::json!(grade_letter(final_score));
        result["health"] = health_json(root);
        result["explain"] = explain_json();
        result["violations"] = serde_json::Value::Array(violations);
        result["boundary_errors"] = serde_json::Value::Array(boundary_errors);
        result["rust_layers"] = serde_json::Value::Array(rust_layers_data);
        result["rust_violations"] = serde_json::Value::Array(rust_violations_data);
    }

    // ADR compliance
    let compliance = check_adr_compliance(root);
    let adr_violations = &compliance.violations;
    let error_count = adr_violations.iter().filter(|v| v.severity == "error").count();
    let warning_count = adr_violations.iter().filter(|v| v.severity == "warning").count();

    let adr_details: Vec<serde_json::Value> = adr_violations
        .iter()
        .map(|v| {
            serde_json::json!({
                "adr": v.adr,
                "file": v.file,
                "line": v.line,
                "message": v.message,
                "severity": v.severity,
            })
        })
        .collect();

    // `checked` is the field that keeps a consumer from reading
    // violation_count: 0 as a pass when nothing ran.
    result["adr_compliance"] = serde_json::json!({
        "checked": compliance.skipped.is_none(),
        "skipped_reason": compliance.skipped,
        "violation_count": adr_violations.len(),
        "error_count": error_count,
        "warning_count": warning_count,
        "violations": adr_details,
    });

    // Best-effort store in HexFlo

    println!("{}", serde_json::to_string_pretty(&result)?);

    if strict && !adr_violations.is_empty() {
        std::process::exit(1);
    }

    Ok(())
}

fn print_check(label: &str, present: bool) {
    let indicator = if present {
        "\u{2713}".green()
    } else {
        "\u{2717}".red()
    };
    println!("    {} {}", indicator, label);
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_file(base: &std::path::Path, rel: &str, content: &str) {
        let p = base.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, content).unwrap();
    }

    // ── Test code is not a boundary violation ───────────────────────────────

    /// The defect this guards: a use case importing the in-memory adapter to
    /// build a test double was reported as `usecases/ may only import from
    /// domain/ and ports/`. The import is inside `#[cfg(test)]` and never
    /// reaches the binary.
    #[test]
    fn imports_inside_cfg_test_are_not_extracted() {
        let src = r#"
use crate::ports::Store;

pub struct Maintenance;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::secondary::memory_store::MemoryStore;

    #[test]
    fn sweeps() {
        let s = MemoryStore::new();
    }
}
"#;
        let imports = extract_import_paths(src, "src/usecases/maintain.rs");
        assert!(
            imports.iter().any(|i| i.contains("ports")),
            "production import was dropped: {imports:?}"
        );
        assert!(
            !imports.iter().any(|i| i.contains("adapters")),
            "test-only import leaked into production imports: {imports:?}"
        );
    }

    /// The block must END. A file that closes its test module and then declares
    /// more production code must have that code scanned, or the fix would hide
    /// real violations below the tests.
    #[test]
    fn extraction_resumes_after_the_test_module_closes() {
        let src = r#"
#[cfg(test)]
mod tests {
    use crate::adapters::secondary::memory_store::MemoryStore;
}

use crate::adapters::secondary::broadcast_bus::BroadcastBus;
"#;
        let imports = extract_import_paths(src, "src/adapters/primary/http.rs");
        assert!(
            imports.iter().any(|i| i.contains("broadcast_bus")),
            "production import after the test module was skipped: {imports:?}"
        );
        assert!(
            !imports.iter().any(|i| i.contains("memory_store")),
            "test-only import leaked: {imports:?}"
        );
    }

    /// Nested braces inside the test module must not end it early.
    #[test]
    fn nested_blocks_do_not_terminate_the_test_module() {
        let src = r#"
#[cfg(test)]
mod tests {
    fn helper() {
        if true { let _ = 1; }
    }
    use crate::adapters::secondary::memory_store::MemoryStore;
}
"#;
        let imports = extract_import_paths(src, "src/usecases/triage.rs");
        assert!(
            imports.is_empty(),
            "nested braces ended the test block early: {imports:?}"
        );
    }

    /// A file with no test module at all must be unaffected.
    #[test]
    fn production_only_files_are_unchanged() {
        let src = "use crate::adapters::secondary::usage::UsageLog;\n";
        let imports = extract_import_paths(src, "src/adapters/primary/gateway.rs");
        assert_eq!(imports.len(), 1, "production import lost: {imports:?}");
    }

    #[test]
    fn test_files_are_recognised_by_convention() {
        for p in [
            "src/domain/search.test.ts",
            "src/domain/twin.spec.tsx",
            "internal/svc/handler_test.go",
            "tests/integration.rs",
            "src/adapters/tests/helper.rs",
        ] {
            assert!(is_test_file(p), "should be treated as test code: {p}");
        }
    }

    #[test]
    fn production_files_are_not_mistaken_for_tests() {
        for p in [
            "src/usecases/maintain.rs",
            "src/adapters/primary/http.rs",
            // "latest" contains "test" — a substring match would misfire here.
            "src/domain/latest_reading.rs",
            "src/domain/contest.ts",
        ] {
            assert!(!is_test_file(p), "wrongly treated as test code: {p}");
        }
    }

    // ── P5.1: scan_rust_workspace_layers ────────────────────────────────

    #[test]
    fn rust_workspace_detects_domain_and_secondary_adapter() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // Simulate hexa-core with domain + ports
        write_file(root, "hexa-core/Cargo.toml", "[package]\nname=\"hexa-core\"");
        write_file(root, "hexa-core/src/domain/mod.rs", "// domain");
        write_file(root, "hexa-core/src/domain/tokens.rs", "// tokens");
        write_file(root, "hexa-core/src/ports/mod.rs", "// ports");

        // Simulate hexa-nexus with adapters
        write_file(root, "hexa-nexus/Cargo.toml", "[package]\nname=\"hexa-nexus\"");
        write_file(root, "hexa-nexus/src/adapters/spacetime.rs", "// adapter");
        write_file(root, "hexa-nexus/src/adapters/mod.rs", "// mod");

        let layers = scan_rust_workspace_layers(root);
        let map: std::collections::HashMap<&str, usize> =
            layers.iter().map(|(l, c)| (l.as_str(), *c)).collect();

        assert_eq!(*map.get("Domain").unwrap_or(&0), 2, "expected 2 domain files");
        assert_eq!(*map.get("Ports").unwrap_or(&0), 1, "expected 1 ports file");
        assert_eq!(*map.get("Secondary Adapters").unwrap_or(&0), 2, "expected 2 adapter files");
    }

    #[test]
    fn rust_workspace_infra_crate_no_recognized_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        write_file(root, "hexa-parser/Cargo.toml", "[package]\nname=\"hexa-parser\"");
        write_file(root, "hexa-parser/src/lib.rs", "// parser");
        write_file(root, "hexa-parser/src/utils.rs", "// utils");

        let layers = scan_rust_workspace_layers(root);
        let map: std::collections::HashMap<&str, usize> =
            layers.iter().map(|(l, c)| (l.as_str(), *c)).collect();

        assert_eq!(*map.get("Domain").unwrap_or(&0), 0);
        assert_eq!(*map.get("Infrastructure").unwrap_or(&0), 2, "parser files should be infrastructure");
    }

    #[test]
    fn rust_workspace_empty_root_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let layers = scan_rust_workspace_layers(tmp.path());
        assert!(layers.is_empty());
    }

    #[test]
    fn rust_workspace_primary_adapter_commands_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        write_file(root, "hexa-cli/Cargo.toml", "[package]\nname=\"hexa-cli\"");
        write_file(root, "hexa-cli/src/commands/analyze.rs", "// analyze");
        write_file(root, "hexa-cli/src/commands/plan.rs", "// plan");

        let layers = scan_rust_workspace_layers(root);
        let map: std::collections::HashMap<&str, usize> =
            layers.iter().map(|(l, c)| (l.as_str(), *c)).collect();

        assert_eq!(*map.get("Primary Adapters").unwrap_or(&0), 2);
    }

    // ── P5.2: scan_rust_boundary_violations ─────────────────────────────

    #[test]
    fn rust_boundary_domain_importing_adapters_is_violation() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        write_file(root, "my-crate/Cargo.toml", "[package]\nname=\"my-crate\"");
        write_file(
            root,
            "my-crate/src/domain/bad.rs",
            "use hexa_nexus::adapters::spacetime;\npub fn foo() {}",
        );

        let violations = scan_rust_boundary_violations(root);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("Domain layer must not import"));
    }

    #[test]
    fn rust_boundary_secondary_adapter_importing_sibling_is_violation() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        write_file(root, "my-crate/Cargo.toml", "[package]\nname=\"my-crate\"");
        write_file(
            root,
            "my-crate/src/adapters/foo.rs",
            "use crate::adapters::bar::BarClient;\npub fn run() {}",
        );

        let violations = scan_rust_boundary_violations(root);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("sibling adapter"));
    }

    #[test]
    fn rust_boundary_use_in_cfg_test_is_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        write_file(root, "my-crate/Cargo.toml", "[package]\nname=\"my-crate\"");
        write_file(
            root,
            "my-crate/src/domain/clean.rs",
            "pub fn foo() {}\n\n#[cfg(test)]\nmod tests {\n    use hexa_nexus::adapters::mock;\n}",
        );

        let violations = scan_rust_boundary_violations(root);
        assert!(violations.is_empty(), "test-section imports must not be flagged");
    }

    #[test]
    fn rust_boundary_clean_file_produces_no_violations() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        write_file(root, "my-crate/Cargo.toml", "[package]\nname=\"my-crate\"");
        write_file(
            root,
            "my-crate/src/domain/clean.rs",
            "use std::collections::HashMap;\npub struct Foo { pub x: u32 }",
        );
        write_file(
            root,
            "my-crate/src/adapters/clean.rs",
            "use hexa_core::ports::IFooPort;\npub struct FooAdapter;",
        );

        let violations = scan_rust_boundary_violations(root);
        assert!(violations.is_empty());
    }

    // ── P5.3: classify_rust_src_layer ───────────────────────────────────

    #[test]
    fn classify_layer_maps_known_paths() {
        assert_eq!(classify_rust_src_layer("domain/tokens.rs"), Some("Domain"));
        assert_eq!(classify_rust_src_layer("ports/inference.rs"), Some("Ports"));
        assert_eq!(classify_rust_src_layer("adapters/spacetime.rs"), Some("Secondary Adapters"));
        assert_eq!(classify_rust_src_layer("adapters/primary/cli.rs"), Some("Primary Adapters"));
        assert_eq!(classify_rust_src_layer("commands/analyze.rs"), Some("Primary Adapters"));
        assert_eq!(classify_rust_src_layer("routes/chat.rs"), Some("Primary Adapters"));
        assert_eq!(classify_rust_src_layer("orchestration/agent_manager.rs"), Some("Use Cases"));
        assert_eq!(classify_rust_src_layer("lib.rs"), None);
        assert_eq!(classify_rust_src_layer("main.rs"), None);
    }

    // ── P5.3: smoke test — zero violations on clean hexa-intf ────────────

    #[test]
    fn rust_boundary_zero_violations_on_hex_intf() {
        // Find the repo root (two levels up from hexa-cli/src/commands/)
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")); // hexa-cli/
        let root = manifest.parent().unwrap(); // hexa-intf/

        let violations = scan_rust_boundary_violations(root);
        if !violations.is_empty() {
            for v in &violations {
                eprintln!("VIOLATION {}:{} — {}", v.file, v.line, v.message);
            }
        }
        assert!(
            violations.is_empty(),
            "{} Rust boundary violation(s) found in hexa-intf — these are real bugs",
            violations.len()
        );
    }

    #[test]
    fn go_layer_rule_table_invariants() {
        assert_eq!(GO_LAYER_RULES.len(), 5, "expected 5 Go layer rules");
        for rule in GO_LAYER_RULES {
            assert!(!rule.label.is_empty());
            assert!(!rule.signals.is_empty(), "rule {:?} has no signals", rule.label);
        }
        let domain_idx = GO_LAYER_RULES.iter().position(|r| r.label == "domain").unwrap();
        let fallback_idx = GO_LAYER_RULES.iter().position(|r| r.label == "internal_fallback").unwrap();
        assert!(domain_idx < fallback_idx,
            "specific internal/ rules must precede internal_fallback");
    }

    #[test]
    fn rust_layer_rule_table_invariants() {
        assert_eq!(RUST_LAYER_RULES.len(), 5, "expected 5 Rust layer rules");
        for rule in RUST_LAYER_RULES {
            assert!(!rule.label.is_empty());
            assert!(!rule.signals.is_empty(), "rule {:?} has no signals", rule.label);
        }
        let primary_idx = RUST_LAYER_RULES.iter().position(|r| r.label == "primary_adapters").unwrap();
        let secondary_idx = RUST_LAYER_RULES.iter().position(|r| r.label == "secondary_adapters").unwrap();
        assert!(primary_idx < secondary_idx,
            "primary_adapters must precede secondary_adapters (adapters/ fallback)");
    }
}
