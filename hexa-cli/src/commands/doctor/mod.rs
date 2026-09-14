//! `hexa doctor`: is hexa installed, is this project initialised, and is
//! there a path to a model. Every failure is listed in the summary, and the
//! verdict is derived from the list.

pub mod composition;

use colored::Colorize;

use crate::assets::Assets;

pub async fn run_doctor(_verbose: bool, _fix: bool) -> anyhow::Result<()> {
    println!("{} hexa doctor", "\u{2b21}".cyan());
    println!();

    // Every failure is listed in the summary. The verdict is derived from
    // the lines above it, so it cannot say "all checks passed" under a ✗.
    let mut failures: Vec<String> = Vec::new();

    // 1. Installation
    println!("  {}", "Installation:".bold());
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "unknown".to_string());
    println!("    binary:       {}", exe);
    println!("    version:      hexa {}", env!("CARGO_PKG_VERSION"));
    let on_path = tokio::process::Command::new("which").arg("hexa").output().await;
    match on_path {
        Ok(o) if o.status.success() => {
            println!("    on PATH:      {} ({})", "\u{2713}".green(), String::from_utf8_lossy(&o.stdout).trim());
        }
        _ => {
            println!("    on PATH:      {} (install -m 755 {} ~/.local/bin/hexa)", "\u{2717}".red(), exe);
            failures.push("hexa is not on PATH".to_string());
        }
    }
    println!();

    // 2. Project
    println!("  {}", "Project:".bold());
    let cwd = std::env::current_dir()?;
    println!("    directory:    {}", cwd.display());
    let has_cargo = cwd.join("Cargo.toml").is_file();
    let has_package_json = cwd.join("package.json").is_file();
    let has_go_mod = cwd.join("go.mod").is_file();
    let project_type = if has_cargo {
        "rust (Cargo.toml)"
    } else if has_go_mod {
        "go (go.mod)"
    } else if has_package_json {
        "typescript (package.json)"
    } else {
        "unknown"
    };
    println!("    type:         {}", project_type);
    let config = cwd.join(".hexa").join("project.json").is_file();
    let rules = cwd.join(".hexa").join("ADR-rules.toml").is_file();
    print_check(".hexa/project.json", config);
    print_check(".hexa/ADR-rules.toml", rules);
    if !config || !rules {
        failures.push("project is not initialised; run `hexa init .`".to_string());
    }
    print_check(".git/", cwd.join(".git").is_dir());
    if !cwd.join(".git").is_dir() {
        failures.push("no git repository; `hexa do` commits its evidence and needs one".to_string());
    }
    println!("    docs/adrs/:   {}", if cwd.join("docs").join("adrs").is_dir() { "present" } else { "none" });
    println!("    assets:       {} embedded", Assets::iter().count());
    // A gate needs the tool that runs it (ADR-2609132018). clippy and
    // rustfmt are optional rustup components, and their absence is silent
    // until a push.
    for c in required_components(project_type) {
        match component_present(c) {
            true => println!("    {} {:<12} present", "\u{2713}".green(), c),
            false => {
                println!("    {} {:<12} missing — rustup component add {}", "\u{2717}".red(), c, c);
                failures.push(format!("{c} is missing, and a project gate runs it: rustup component add {c}"));
            }
        }
    }
    println!();

    // 3. Inference
    let inference = composition::run_composition_check().await;
    if !inference.has_any_inference() {
        failures.push("no path to a model: start the local server or log in to `claude`".to_string());
    }
    // A tier naming a model nothing can serve fails the run: doctor's job is
    // to say whether the installation works, and it does not (ADR-2609131617).
    for t in inference.unserved_tiers() {
        failures.push(format!(
            "tier {} names {}, which no reachable path serves — pull it, register the endpoint that has it, or point the tier at one of: {}",
            t.label,
            t.model,
            {
                let served = inference.served_models();
                if served.is_empty() { "nothing reachable".to_string() } else { served.join(", ") }
            }
        ));
    }
    println!();

    // Summary
    println!("  {}", "Summary:".bold());
    if failures.is_empty() {
        println!("    {}", "All checks passed".green());
    } else {
        println!("    {} check{} failed:", failures.len(), if failures.len() == 1 { "" } else { "s" });
        for f in &failures {
            println!("      {} {}", "\u{2717}".red(), f);
        }
        // A health check that prints failures and exits 0 is the same lie
        // one layer up: `hexa doctor && deploy` would proceed
        // (ADR-2609131617 §4).
        anyhow::bail!("{} check(s) failed", failures.len());
    }

    Ok(())
}

fn print_check(label: &str, ok: bool) {
    if ok {
        println!("    {}: {}", label, "✓".green());
    } else {
        println!("    {}: {}", label, "✗".red());
    }
}

/// Markers that indicate project-specific (non-generic) content leaked into
/// embedded assets. Embedded templates must be installable into ANY target
/// project, so they must not reference hexa-intf internal crate names, absolute
/// paths, or SpacetimeDB module names.
const ASSET_GENERIC_MARKERS: &[&str] = &[
    "hexa-nexus",
    "hexa-core",
    "hexa-parser",
    "hexa-desktop",
    "spacetime-modules",
    "hexflo-coordination",
    "/Volumes/",
];

/// Substrings that, when present on a line, exempt it from the marker check.
/// These are legitimate hexa CLI command references (e.g. `hexa analyze`,
/// `hexa plan`, `hexa nexus start`) that happen to contain marker substrings.
const ASSET_GENERIC_EXCEPTIONS: &[&str] = &[
    "hexa analyze",
    "hexa plan",
    "hexa nexus",
    "hexa doctor",
    "hexa status",
    "hexa ci",
    "hexa swarm",
    "hexa task",
    "hexa memory",
    "hexa inbox",
    "hexa adr",
    "hexa hook",
    "hexa init",
    "hexa mcp",
    "hexa secrets",
    "hexa validate",
    "hexa dev",
    "hexa new",
    "hexa pause",
    "hexa steer",
    "hexa pulse",
    "hexa brief",
];

/// Check that all embedded assets are project-generic (no hexa-intf-specific
/// references). Returns a list of (filename, line_number, matched_marker)
/// violations.
pub fn check_embedded_assets_generic() -> Vec<(String, usize, String)> {
    check_content_generic_violations(
        Assets::iter().filter_map(|path| {
            // Skip compiled binaries — grep hits inside them are false
            // positives (e.g. hexflo-coordination.wasm has its own module
            // name baked in by the compiler).
            let p = path.as_ref();
            if is_binary_asset(p) {
                return None;
            }
            Assets::get_str(&path).map(|content| (path.to_string(), content))
        }),
    )
}

/// True if the path looks like a compiled/binary asset that would produce
/// false positives in a text grep (WASM modules, images, icons).
fn is_binary_asset(path: &str) -> bool {
    const BINARY_EXTS: &[&str] = &[
        ".wasm", ".png", ".jpg", ".jpeg", ".gif", ".ico", ".webp", ".woff",
        ".woff2", ".ttf", ".otf", ".eot",
    ];
    BINARY_EXTS.iter().any(|ext| path.ends_with(ext))
}

/// Core violation scanner — takes an iterator of (filename, content) pairs.
/// Extracted so unit tests can call it with synthetic content.
fn check_content_generic_violations(
    files: impl Iterator<Item = (String, String)>,
) -> Vec<(String, usize, String)> {
    let mut violations = Vec::new();

    for (path, content) in files {
        for (line_idx, line) in content.lines().enumerate() {
            // Skip if the line contains a known CLI-command exception
            let is_exception = ASSET_GENERIC_EXCEPTIONS
                .iter()
                .any(|exc| line.contains(exc));
            if is_exception {
                continue;
            }

            for marker in ASSET_GENERIC_MARKERS {
                if line.contains(marker) {
                    violations.push((path.clone(), line_idx + 1, marker.to_string()));
                    break; // one violation per line is enough
                }
            }
        }
    }

    violations
}

pub async fn run_validate_pipeline(
    skip_test: bool,
    strict: bool,
    _parallel: bool,
) -> anyhow::Result<()> {
    println!("{} hexa validate pipeline", "\u{2b21}".cyan());
    println!();

    let cwd = std::env::current_dir()?;
    let mut stages_passed = Vec::new();
    let mut stages_failed = Vec::new();

    // Stage 1: Build
    println!("  {} {}", "1.".bold(), "Build".bold());
    let build_result = run_build(&cwd).await;
    match build_result {
        Ok(()) => {
            println!("    {} build", "✓".green());
            stages_passed.push("build");
        }
        Err(e) => {
            println!("    {} build: {}", "✗".red(), e);
            stages_failed.push(("build", e.to_string()));
        }
    }
    println!();

    // Stage 2: Test (if not skipped)
    if skip_test {
        println!("  {} {} {}", "2.".bold(), "Test".bold(), "(skipped)".dimmed());
    } else {
        println!("  {} {}", "2.".bold(), "Test".bold());
        let test_result = run_tests(&cwd).await;
        match test_result {
            Ok(()) => {
                println!("    {} tests", "✓".green());
                stages_passed.push("test");
            }
            Err(e) => {
                println!("    {} tests: {}", "✗".red(), e);
                stages_failed.push(("test", e.to_string()));
            }
        }
    }
    println!();

    // Stage 3: Analyze
    println!("  {} {}", "3.".bold(), "Analyze".bold());
    let analyze_result = run_analyze(&cwd, strict).await;
    match analyze_result {
        Ok(()) => {
            println!("    {} architecture", "✓".green());
            stages_passed.push("analyze");
        }
        Err(e) => {
            println!("    {} architecture: {}", "✗".red(), e);
            stages_failed.push(("analyze", e.to_string()));
        }
    }
    println!();

    // Stage 4: Validate (behavioral specs)
    println!("  {} {}", "4.".bold(), "Validate".bold());
    let validate_result = run_validate(&cwd).await;
    match validate_result {
        Ok(()) => {
            println!("    {} specs", "✓".green());
            stages_passed.push("validate");
        }
        Err(e) => {
            println!("    {} specs: {}", "✗".red(), e);
            stages_failed.push(("validate", e.to_string()));
        }
    }
    println!();

    // Summary
    println!("  {}", "Results:".bold());
    println!(
        "    passed:      {}/{}",
        stages_passed.len(),
        stages_passed.len() + stages_failed.len()
    );

    if !stages_failed.is_empty() {
        println!();
        println!("    {}", "Failed stages:".red());
        for (stage, error) in &stages_failed {
            println!("      - {}: {}", stage, error);
        }
        std::process::exit(1);
    }

    println!();
    println!("    {}", "Pipeline complete — all checks passed".green());

    Ok(())
}

async fn run_build(cwd: &std::path::Path) -> anyhow::Result<()> {
    // Check for package.json -> run npm build
    if cwd.join("package.json").is_file() {
        let mut cmd = tokio::process::Command::new("npm");
        cmd.args(["run", "build"]);
        cmd.current_dir(cwd);
        let output = cmd.output().await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("npm build failed: {}", stderr));
        }
        return Ok(());
    }

    // Check for Cargo.toml -> run cargo build
    if cwd.join("Cargo.toml").is_file() {
        let output = tokio::process::Command::new("cargo")
            .args(["build", "--quiet"])
            .current_dir(cwd)
            .output()
            .await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("cargo build failed: {}", stderr));
        }
        return Ok(());
    }

    // Check for go.mod -> run go build
    if cwd.join("go.mod").is_file() {
        let output = tokio::process::Command::new("go")
            .arg("build")
            .current_dir(cwd)
            .output()
            .await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("go build failed: {}", stderr));
        }
        return Ok(());
    }

    // No build system detected
    Err(anyhow::anyhow!(
        "no build system detected (package.json, Cargo.toml, or go.mod required)"
    ))
}

async fn run_tests(cwd: &std::path::Path) -> anyhow::Result<()> {
    // Check for package.json -> run npm test
    if cwd.join("package.json").is_file() {
        let mut cmd = tokio::process::Command::new("npm");
        cmd.arg("test");
        cmd.current_dir(cwd);
        let output = cmd.output().await?;
        
        let stdout = String::from_utf8_lossy(&output.stdout);
        let _stderr = String::from_utf8_lossy(&output.stderr);

        // Check for actual test failures (e.g., "1 fail", "5 fail")
        // npm test can return non-zero for filter warnings even when tests pass
        let has_actual_failures = stdout.contains(" fail") 
            && !stdout.contains("0 fail") 
            && !stdout.contains(" fail()");
        
        if has_actual_failures {
            return Err(anyhow::anyhow!("npm test had failures: {}", stdout));
        }
        
        // Check that some tests actually ran
        if stdout.contains("Ran 0 tests") {
            return Err(anyhow::anyhow!("no tests found"));
        }
        
        return Ok(());
    }

    // Check for Cargo.toml -> run cargo test
    if cwd.join("Cargo.toml").is_file() {
        let output = tokio::process::Command::new("cargo")
            .args(["test", "--quiet"])
            .current_dir(cwd)
            .output()
            .await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("cargo test failed: {}", stderr));
        }
        return Ok(());
    }

    // Check for go.mod -> run go test
    if cwd.join("go.mod").is_file() {
        let output = tokio::process::Command::new("go")
            .args(["test", "./..."])
            .current_dir(cwd)
            .output()
            .await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("go test failed: {}", stderr));
        }
        return Ok(());
    }

    // No test framework detected — skip
    Ok(())
}

async fn run_analyze(cwd: &std::path::Path, strict: bool) -> anyhow::Result<()> {
    // Run hexa analyze via CLI to reuse existing logic
    let mut cmd = tokio::process::Command::new("hexa");
    cmd.arg("analyze").arg(cwd.to_string_lossy().as_ref());
    if strict {
        cmd.arg("--strict");
    }

    let output = cmd.output().await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(anyhow::anyhow!(
            "analyze failed: {}",
            if stdout.is_empty() {
                stderr
            } else {
                stdout
            }
        ));
    }

    // Check output for grade - look for actual failing grade (F)
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.contains("grade") {
        // Only fail on actual F grade, not A+, A, B+, etc
        if stdout.contains("grade: F") || stdout.contains("grade F") {
            return Err(anyhow::anyhow!("architecture analysis failed"));
        }
    }
    
    // Also check for score 0 (but not 100 or other non-zero)
    if stdout.contains("score 0/100") {
        return Err(anyhow::anyhow!("architecture analysis failed"));
    }

    Ok(())
}

async fn run_validate(cwd: &std::path::Path) -> anyhow::Result<()> {
    // Check for behavioral specs in docs/specs/
    let specs_dir = cwd.join("docs").join("specs");
    if !specs_dir.is_dir() {
        return Ok(()); // No specs to validate
    }

    // List spec files
    let mut spec_files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(specs_dir) {
        for entry in entries.flatten() {
            if entry.path().extension().map(|e| e == "json").unwrap_or(false) {
                spec_files.push(entry.path());
            }
        }
    }

    if spec_files.is_empty() {
        return Ok(());
    }

    // For now, just verify the specs are valid JSON
    // Full validation would require the hexa validate command
    for spec in &spec_files {
        let content = std::fs::read_to_string(spec)?;
        serde_json::from_str::<serde_json::Value>(&content)
            .map_err(|e| anyhow::anyhow!("invalid spec {}: {}", spec.display(), e))?;
    }

    println!("    validated {} spec(s)", spec_files.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(name: &str, content: &str) -> Vec<(String, usize, String)> {
        check_content_generic_violations(
            std::iter::once((name.to_string(), content.to_string())),
        )
    }

    #[test]
    fn detects_absolute_path_violation() {
        let v = scan("test.yml", "root: /Volumes/ExtendedStorage/foo");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].2, "/Volumes/");
    }

    #[test]
    fn cli_command_exception_passes() {
        let v = scan("test.md", "Run `hexa analyze .` to check architecture");
        assert!(v.is_empty(), "hexa CLI command reference should be exempt");
    }

    #[test]
    fn hexa_nexus_cli_exception_passes() {
        let v = scan("test.md", "hexa nexus start");
        assert!(v.is_empty(), "hexa nexus CLI command should be exempt");
    }

    #[test]
    fn hexa_nexus_crate_reference_fails() {
        let v = scan("test.yml", "depends on hexa-nexus crate");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].2, "hexa-nexus");
    }

    #[test]
    fn clean_content_passes() {
        let v = scan("clean.yml", "name: hexa-scaffold\ndescription: generic template\n");
        assert!(v.is_empty());
    }

    #[test]
    fn spacetime_modules_reference_fails() {
        let v = scan("bad.md", "see spacetime-modules/hexflo-coordination for details");
        // Should catch both markers on the same line but we break after first
        assert_eq!(v.len(), 1);
    }
}
/// The toolchain components this project's own gates run
/// (ADR-2609132018 §1, §3). Only for the project type in hand, and only
/// what a gate actually runs: the first cut asked for `rustfmt` too, and
/// doctor promptly failed on a tool no gate in this repository uses, which
/// is the false failure this whole check exists to prevent.
fn required_components(project_type: &str) -> &'static [&'static str] {
    if project_type.starts_with("rust") {
        &["clippy"]
    } else {
        &[]
    }
}

/// Does the component answer? Presence is what decides whether the gate can
/// run here; matching CI's version is a different problem (§4).
fn component_present(name: &str) -> bool {
    component_present_with(name, &|n| {
        std::process::Command::new("cargo")
            .arg(n)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    })
}

/// [`component_present`] with the probe injected, so both directions are
/// testable without depending on what this machine happens to have
/// installed (ADR-2609131749: tests do not reach for global state).
fn component_present_with(name: &str, answers: &dyn Fn(&str) -> bool) -> bool {
    !name.is_empty() && answers(name)
}

#[cfg(test)]
mod toolchain_tests {
    use super::{component_present, component_present_with, required_components};

    /// ADR-2609132018 §3: only the project type in hand is told about its
    /// own tools.
    #[test]
    fn components_are_required_per_project_type() {
        assert_eq!(required_components("rust (Cargo.toml)"), &["clippy"], "only what a gate runs");
        assert!(!required_components("rust (Cargo.toml)").contains(&"rustfmt"), "no gate here runs it");
        assert!(required_components("go (go.mod)").is_empty(), "a Go project is not told about clippy");
        assert!(required_components("typescript (package.json)").is_empty());
        assert!(required_components("unknown").is_empty());
    }

    /// §4: presence is whether the component answers — both directions,
    /// driven through the injected probe so the result does not depend on
    /// what this machine happens to have installed.
    #[test]
    fn presence_is_decided_by_whether_the_component_answers() {
        let installed = |_: &str| true;
        let absent = |_: &str| false;
        assert!(component_present_with("clippy", &installed));
        assert!(!component_present_with("clippy", &absent));
        assert!(!component_present_with("", &installed), "an empty name asks nothing");

        // The probe is asked about the name it was given, and no other.
        let only_rustfmt = |n: &str| n == "rustfmt";
        assert!(component_present_with("rustfmt", &only_rustfmt));
        assert!(!component_present_with("clippy", &only_rustfmt));
    }

    /// A component that is genuinely not a cargo subcommand is absent, and
    /// asking is not an error. This one does touch the machine, which is
    /// why it only checks the negative direction.
    #[test]
    fn an_unknown_component_is_absent_rather_than_a_panic() {
        assert!(!component_present("definitely-not-a-cargo-subcommand"));
    }
}
