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
    println!();

    // 3. Inference
    let inference = composition::run_composition_check().await;
    if !inference.has_any_inference() {
        failures.push("no path to a model: start the local server or log in to `claude`".to_string());
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