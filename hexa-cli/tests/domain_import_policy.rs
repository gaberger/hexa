//! The domain imports only what it is allowed.
//!
//! ADR-2609211430 §2. The dependency rule is checked edge by edge between
//! layers, so an import that leaves the project entirely had no edge to
//! violate: a domain file importing `sqlx` scored A+. The headline rule —
//! "domain imports only domain" — was stricter than anything enforced.
//!
//! These are the ADR's nine gate cases. Numbers 2 and 9 pass before the change
//! as well as after; they pin behaviour that had to survive it, and a gate
//! where every case flips is a gate that has not been checked for
//! false positives.

use std::path::Path;
use std::process::Command;

fn hexa() -> std::path::PathBuf {
    let mut p = std::env::current_exe().expect("test exe");
    p.pop();
    p.pop();
    p.push("hexa");
    p
}

fn run(root: &Path, args: &[&str]) -> (i32, String) {
    let out = Command::new(hexa()).args(args).current_dir(root).output().expect("run hexa");
    (
        out.status.code().unwrap_or(-1),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

fn score(root: &Path) -> u64 {
    let out = Command::new(hexa())
        .args(["analyze", ".", "--json"])
        .current_dir(root)
        .output()
        .expect("run hexa");
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let v: serde_json::Value = serde_json::from_str(text.trim())
        .unwrap_or_else(|e| panic!("analyze --json is not JSON ({e}):\n{text}"));
    v["score"].as_u64().expect("score")
}

/// `policy` is the `[[import_policy]]` body, minus the header line.
fn project(files: &[(&str, &str)], policy: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().expect("tempdir");
    let r = d.path();
    std::fs::create_dir_all(r.join(".hexa")).unwrap();
    std::fs::write(
        r.join(".hexa/ADR-rules.toml"),
        format!(
            "[[import_policy]]\n\
             adr = \"ADR-2609211430\"\n\
             id = \"domain-imports-only-what-it-is-allowed\"\n\
             message = \"The domain imported something from outside the project that it is not allowed to know about.\"\n\
             severity = \"error\"\n\
             {policy}\n"
        ),
    )
    .unwrap();
    for (path, body) in files {
        let full = r.join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, body).unwrap();
    }
    d
}

const RUST_MANIFEST: &str = "[package]\nname=\"demo\"\nversion=\"0.1.0\"\nedition=\"2021\"\n";
const RUST_POLICY: &str = "layer = \"/domain/\"\nallow = [\"serde\"]\ndeny = [\"std::fs\", \"std::net\", \"std::process\", \"std::env\", \"std::io\"]";

/// How many error-severity findings the run reported.
///
/// Read from the one canonical line, `N ADR violation(s): M error(s), …`.
/// A run with no findings prints no such line at all, and that is zero rather
/// than a test failure — the clean cases are half of what this gate checks.
fn errors(out: &str) -> usize {
    let Some(line) = out.lines().find(|l| l.contains("ADR violation(s):")) else {
        return 0;
    };
    let after = line.split("ADR violation(s):").nth(1).unwrap_or("");
    let idx = after.find("error(s)").unwrap_or_else(|| panic!("no error count in:\n{out}"));
    // The count is the last number before `error(s)`, and the terminal may
    // have wrapped it in colour codes, so take the digits rather than the word.
    after[..idx]
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .unwrap_or_else(|_| panic!("no error count in:\n{out}"))
}

// ── 1 ────────────────────────────────────────────────────────────────────────

#[test]
fn a_domain_file_importing_sqlx_is_an_error_that_costs_the_grade() {
    let clean = project(
        &[("Cargo.toml", RUST_MANIFEST), ("src/domain/order.rs", "pub struct Order;\n")],
        RUST_POLICY,
    );
    let dirty = project(
        &[
            ("Cargo.toml", RUST_MANIFEST),
            ("src/domain/order.rs", "use sqlx::PgPool;\npub struct Order;\n"),
        ],
        RUST_POLICY,
    );

    let (_, out) = run(dirty.path(), &["analyze", "."]);
    assert_eq!(errors(&out), 1, "one finding, naming the import:\n{out}");
    assert!(out.contains("sqlx"), "the finding must name what was imported:\n{out}");

    assert_eq!(
        score(dirty.path()),
        score(clean.path()) - 10,
        "an error-severity finding costs ten points"
    );
    let (graded, out) = run(dirty.path(), &["analyze", ".", "--grade", "A+"]);
    assert_eq!(graded, 1, "and the floor must reject it:\n{out}");
}

// ── 2 ────────────────────────────────────────────────────────────────────────

#[test]
fn the_same_import_in_an_adapter_is_not_a_finding() {
    let d = project(
        &[
            ("Cargo.toml", RUST_MANIFEST),
            ("src/adapters/secondary/pg.rs", "use sqlx::PgPool;\npub struct PgStore;\n"),
        ],
        RUST_POLICY,
    );
    let (_, out) = run(d.path(), &["analyze", "."]);
    assert_eq!(errors(&out), 0, "an adapter is where sqlx belongs:\n{out}");
}

// ── 3 ────────────────────────────────────────────────────────────────────────

#[test]
fn an_allowlisted_crate_passes() {
    let d = project(
        &[
            ("Cargo.toml", RUST_MANIFEST),
            ("src/domain/order.rs", "use serde::Deserialize;\npub struct Order;\n"),
        ],
        RUST_POLICY,
    );
    let (_, out) = run(d.path(), &["analyze", "."]);
    assert_eq!(errors(&out), 0, "serde was allowed deliberately:\n{out}");
}

#[test]
fn the_allowlist_does_not_cover_a_crate_that_merely_starts_the_same() {
    let d = project(
        &[
            ("Cargo.toml", RUST_MANIFEST),
            ("src/domain/order.rs", "use serde_json::Value;\npub struct Order;\n"),
        ],
        RUST_POLICY,
    );
    let (_, out) = run(d.path(), &["analyze", "."]);
    assert_eq!(errors(&out), 1, "serde_json is a different dependency:\n{out}");
}

// ── 4 ────────────────────────────────────────────────────────────────────────

#[test]
fn a_denied_standard_library_module_is_an_error_even_though_std_is_allowed() {
    let d = project(
        &[
            ("Cargo.toml", RUST_MANIFEST),
            (
                "src/domain/order.rs",
                "use std::fs::File;\nuse std::cmp::Ordering;\npub struct Order;\n",
            ),
        ],
        RUST_POLICY,
    );
    let (_, out) = run(d.path(), &["analyze", "."]);
    assert_eq!(errors(&out), 1, "std::fs is denied, std::cmp is not:\n{out}");
    assert!(out.contains("std::fs"), "and the denied one is named:\n{out}");
}

// ── 5 ────────────────────────────────────────────────────────────────────────

#[test]
fn a_grouped_import_split_over_lines_is_caught() {
    // The case a substring rule cannot see, and the reason this reads parsed
    // imports rather than lines.
    let d = project(
        &[
            ("Cargo.toml", RUST_MANIFEST),
            (
                "src/domain/order.rs",
                "use sqlx::{\n    self,\n    PgPool,\n};\npub struct Order;\n",
            ),
        ],
        RUST_POLICY,
    );
    let (_, out) = run(d.path(), &["analyze", "."]);
    assert!(errors(&out) >= 1, "a multi-line group is still an import:\n{out}");
    assert!(out.contains("sqlx"), "{out}");
}

// ── 6 ────────────────────────────────────────────────────────────────────────

#[test]
fn typescript_catches_a_bare_package_and_a_node_builtin_but_not_a_relative_import() {
    let policy = "layer = \"/domain/\"\nallow = []\ndeny = [\"node:fs\", \"fs\", \"node:net\", \"net\", \"node:http\", \"http\", \"node:child_process\", \"child_process\", \"node:process\", \"process\"]";
    let d = project(
        &[
            ("package.json", "{ \"name\": \"demo\", \"type\": \"module\" }\n"),
            ("src/domain/count.ts", "export const zero = 0;\n"),
            (
                "src/domain/order.ts",
                "import type { Pool } from \"pg\";\n\
                 import fs from \"node:fs\";\n\
                 import { zero } from \"./count.js\";\n\
                 export const order = zero;\n",
            ),
        ],
        policy,
    );
    let (_, out) = run(d.path(), &["analyze", "."]);
    assert_eq!(errors(&out), 2, "pg and node:fs, not ./count.js:\n{out}");
    assert!(out.contains("pg"), "{out}");
    assert!(out.contains("node:fs"), "{out}");
}

// ── 7 ────────────────────────────────────────────────────────────────────────

#[test]
fn go_catches_the_standard_library_and_a_third_party_module_but_not_its_own() {
    let policy = "layer = \"/domain/\"\nallow = []\ndeny = [\"os\", \"net\", \"net/http\", \"database/sql\", \"os/exec\", \"io/ioutil\"]";
    let d = project(
        &[
            ("go.mod", "module github.com/acme/app\n\ngo 1.22\n"),
            ("internal/domain/count.go", "package domain\n\ntype Count int\n"),
            (
                "internal/domain/order.go",
                "package domain\n\n\
                 import (\n\
                 \t\"os\"\n\
                 \t\"github.com/jackc/pgx/v5\"\n\
                 \t\"github.com/acme/app/internal/domain\"\n\
                 )\n\n\
                 var _ = os.Getenv\n",
            ),
        ],
        policy,
    );
    let (_, out) = run(d.path(), &["analyze", "."]);
    assert_eq!(errors(&out), 2, "os and pgx, not the project's own package:\n{out}");
    assert!(out.contains("pgx"), "{out}");
}

// ── 8 ────────────────────────────────────────────────────────────────────────

#[test]
fn the_score_the_scaffold_floor_reads_carries_the_finding() {
    // `hexa scaffold --grade` reads its floor from the same `deep_analysis`
    // score this asserts. Running the verb itself would need an agent and a
    // model; the number it gates on is checkable here, which is the part
    // that was wrong.
    let dirty = project(
        &[
            ("Cargo.toml", RUST_MANIFEST),
            ("src/domain/order.rs", "use std::process::Command;\npub struct Order;\n"),
        ],
        RUST_POLICY,
    );
    let (exit, out) = run(dirty.path(), &["analyze", ".", "--grade", "A"]);
    assert_eq!(exit, 1, "a denied import must put the tree under an A floor:\n{out}");
}

// ── 9 ────────────────────────────────────────────────────────────────────────

#[test]
fn a_warning_severity_policy_reports_without_moving_the_grade() {
    let clean = project(
        &[("Cargo.toml", RUST_MANIFEST), ("src/domain/order.rs", "pub struct Order;\n")],
        RUST_POLICY,
    );
    let warn = project(
        &[
            ("Cargo.toml", RUST_MANIFEST),
            ("src/domain/order.rs", "use sqlx::PgPool;\npub struct Order;\n"),
        ],
        RUST_POLICY,
    );
    // Rewrite just the severity, leaving everything else identical.
    let rules = warn.path().join(".hexa/ADR-rules.toml");
    let text = std::fs::read_to_string(&rules).unwrap().replace("\"error\"", "\"warning\"");
    std::fs::write(&rules, text).unwrap();

    assert_eq!(
        score(warn.path()),
        score(clean.path()),
        "a warning is advisory — that is what --strict is for"
    );
    let (strict, out) = run(warn.path(), &["analyze", ".", "--strict"]);
    assert_eq!(strict, 1, "--strict still catches it:\n{out}");
}

// ── §3: the scaffold ships a policy its own domain passes ────────────────────

/// A shipped rule that the shipped code violates teaches people to skim past
/// the output. Every scaffold writes this policy into `.hexa/ADR-rules.toml`,
/// so every scaffold's own domain has to satisfy it on the first run, with no
/// edits (ADR-2609211430 §3).
#[test]
fn every_scaffolded_language_passes_the_policy_it_ships_with() {
    for lang in ["rust", "go", "ts"] {
        let d = tempfile::tempdir().expect("tempdir");
        let target = d.path().join("demo-app");
        std::fs::create_dir_all(&target).unwrap();
        let out = Command::new(hexa())
            .args([
                "init",
                target.to_str().unwrap(),
                "--scaffold",
                "--lang",
                lang,
                "--no-claude-md",
            ])
            .output()
            .expect("run hexa init");
        assert!(out.status.success(), "hexa init --lang {lang} failed");

        let rules = std::fs::read_to_string(target.join(".hexa/ADR-rules.toml")).unwrap();
        assert!(
            rules.contains("[[import_policy]]"),
            "{lang}: the scaffold must ship the policy, not just be checked by it"
        );

        let (_, report) = run(&target, &["analyze", "."]);
        assert_eq!(
            errors(&report),
            0,
            "{lang}: the scaffold's own domain must satisfy the policy it ships:\n{report}"
        );
    }
}
