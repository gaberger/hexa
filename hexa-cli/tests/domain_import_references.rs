//! A reference is an import, whether or not it has an import line.
//!
//! ADR-2609211600. `[[import_policy]]` judged only import *declarations*, so
//! `use std::fs;` in a domain file was an error and `std::fs::read("x")` on the
//! next line was not. A deny entry the code walks past is worse than no entry:
//! the rules file claims a protection the tool does not give.
//!
//! The hard half is not finding paths — tree-sitter already parses them — it is
//! not flagging local code. `O::new()`, `Self::new()` and `Ordering::Less` all
//! have a first segment that the classifier would otherwise call External. The
//! negative controls below are therefore half of this gate, and they pass both
//! before and after the change.

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

/// Counts from the one canonical line, `N ADR violation(s): E error(s), W warning(s)`.
/// No such line means no findings, which is zero rather than a failure.
fn counts(out: &str) -> (usize, usize) {
    let Some(line) = out.lines().find(|l| l.contains("ADR violation(s):")) else {
        return (0, 0);
    };
    let after = line.split("ADR violation(s):").nth(1).unwrap_or("");
    let digits = |s: &str| -> usize {
        s.chars().filter(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0)
    };
    let e = after.find("error(s)").map(|i| digits(&after[..i])).unwrap_or(0);
    let w = after
        .find("warning(s)")
        .and_then(|i| after.find("error(s)").map(|j| digits(&after[j + 8..i])))
        .unwrap_or(0);
    (e, w)
}

fn errors(out: &str) -> usize {
    counts(out).0
}

/// The manifest the fixtures share: two real dependencies, so a path whose
/// first segment is `sqlx` or `tokio` names something outside the project.
const MANIFEST: &str = "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
     [dependencies]\nsqlx = \"0.7\"\ntokio = \"1\"\nserde = \"1\"\nserde_json = \"1\"\n";

/// A project carrying the **shipped** rules file, so this gate tests what a
/// scaffolded project gets rather than a policy written to suit it.
fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
    let d = tempfile::tempdir().expect("tempdir");
    let r = d.path();
    std::fs::create_dir_all(r.join(".hexa")).unwrap();
    let shipped = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/templates/ADR-rules.toml"),
    )
    .expect("read the shipped rules file");
    std::fs::write(r.join(".hexa/ADR-rules.toml"), shipped).unwrap();
    std::fs::write(r.join("Cargo.toml"), MANIFEST).unwrap();
    for (path, body) in files {
        let full = r.join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, body).unwrap();
    }
    d
}

/// One Rust file in the domain, and the finding count it must produce.
fn rust_case(body: &str) -> (tempfile::TempDir, String) {
    let d = project(&[("src/domain/subject.rs", body)]);
    let (_, out) = run(d.path(), &["analyze", "."]);
    (d, out)
}

// ── 1. Rust inline forms ─────────────────────────────────────────────────────

#[test]
fn an_inline_call_into_a_denied_standard_library_module_is_an_error() {
    // The headline case: the shipped policy denies `std::fs`, and this walked
    // straight past it.
    let (_d, out) = rust_case("pub fn c() {\n    let _ = std::fs::read(\"x\");\n}\n");
    assert_eq!(errors(&out), 1, "std::fs::read is a use of std::fs:\n{out}");
    assert!(out.contains("std::fs"), "and the finding names it:\n{out}");
    assert!(out.contains("subject.rs:2"), "at the line it is written on:\n{out}");
}

#[test]
fn a_global_path_is_an_error() {
    let (_d, out) = rust_case("pub fn e() {\n    let _ = ::sqlx::query(\"x\");\n}\n");
    assert_eq!(errors(&out), 1, "a leading :: does not hide the crate:\n{out}");
    assert!(out.contains("sqlx"), "{out}");
}

#[test]
fn a_crate_named_only_in_a_type_position_is_an_error() {
    let (_d, out) = rust_case("pub fn f(_p: sqlx::PgPool) {}\n");
    assert_eq!(errors(&out), 1, "a signature names the dependency too:\n{out}");
}

#[test]
fn a_crate_named_only_in_an_attribute_is_an_error() {
    let (_d, out) = rust_case("#[tokio::main]\nasync fn m() {}\n");
    assert_eq!(errors(&out), 1, "an attribute is a reference:\n{out}");
    assert!(out.contains("tokio"), "{out}");
}

#[test]
fn a_crate_named_only_in_a_macro_invocation_is_an_error() {
    let (_d, out) = rust_case("pub fn q() {\n    let _ = sqlx::query!(\"x\");\n}\n");
    assert_eq!(errors(&out), 1, "a macro path is a reference:\n{out}");
}

#[test]
fn extern_crate_is_an_error() {
    // Listed as a known limit in the README and in ADR-2609211430's
    // deviations. This ADR supersedes that entry.
    let (_d, out) = rust_case("extern crate sqlx;\n");
    assert_eq!(errors(&out), 1, "extern crate names a dependency:\n{out}");
}

// ── 2. TypeScript forms ──────────────────────────────────────────────────────

fn ts_case(body: &str) -> (tempfile::TempDir, String) {
    let d = project(&[
        ("package.json", "{ \"name\": \"demo\", \"type\": \"module\" }\n"),
        ("src/domain/subject.ts", body),
    ]);
    let (_, out) = run(d.path(), &["analyze", "."]);
    (d, out)
}

#[test]
fn a_require_call_is_an_import() {
    let (_d, out) = ts_case("const fs = require(\"node:fs\");\nexport const x = fs;\n");
    assert_eq!(errors(&out), 1, "require loads a module:\n{out}");
    assert!(out.contains("node:fs"), "{out}");
}

#[test]
fn a_dynamic_import_expression_is_an_import() {
    let (_d, out) =
        ts_case("export async function go() {\n  return await import(\"pg\");\n}\n");
    assert_eq!(errors(&out), 1, "a dynamic import is still an import:\n{out}");
    assert!(out.contains("pg"), "{out}");
}

#[test]
fn an_import_in_a_type_position_is_an_import() {
    let (_d, out) = ts_case("export type P = import(\"pg\").Pool;\nexport const y = 1;\n");
    assert_eq!(errors(&out), 1, "a type-position import names the package:\n{out}");
}

// ── 3. What cannot be read is reported, not skipped ──────────────────────────

#[test]
fn a_module_loaded_by_a_computed_name_is_a_warning_that_does_not_move_the_grade() {
    let clean = project(&[
        ("package.json", "{ \"name\": \"demo\", \"type\": \"module\" }\n"),
        ("src/domain/subject.ts", "export const x = 1;\n"),
    ]);
    let computed = project(&[
        ("package.json", "{ \"name\": \"demo\", \"type\": \"module\" }\n"),
        (
            "src/domain/subject.ts",
            "const name = \"p\" + \"g\";\nconst mod = require(name);\nexport const x = mod;\n",
        ),
    ]);

    let (_, out) = run(computed.path(), &["analyze", "."]);
    let (errs, warns) = counts(&out);
    assert_eq!(errs, 0, "a load hexa cannot read is not an error:\n{out}");
    assert_eq!(warns, 1, "but it is not silence either:\n{out}");
    assert!(
        out.to_lowercase().contains("computed"),
        "and it says what it could not check:\n{out}"
    );

    assert_eq!(
        score(computed.path()),
        score(clean.path()),
        "a warning does not move the grade (ADR-2609211430 §1)"
    );
    let (strict, out) = run(computed.path(), &["analyze", ".", "--strict"]);
    assert_eq!(strict, 1, "--strict still fails on it:\n{out}");
}

// ── 4. Negative controls: these must stay clean ──────────────────────────────

#[test]
fn local_code_is_never_a_reference() {
    // Every one of these has a first segment that the classifier would call
    // External if it were handed the path blindly. This is the test that stops
    // the feature reporting a project's own code as an outside dependency.
    let d = project(&[(
        "src/domain/subject.rs",
        "use std::cmp::Ordering;\n\
         \n\
         pub mod util {\n\
         \x20   pub fn f() {}\n\
         }\n\
         \n\
         pub struct O;\n\
         \n\
         impl O {\n\
         \x20   pub fn new() -> Self {\n\
         \x20       Self::make()\n\
         \x20   }\n\
         \x20   fn make() -> Self {\n\
         \x20       O\n\
         \x20   }\n\
         }\n\
         \n\
         pub enum Colour {\n\
         \x20   Red,\n\
         }\n\
         \n\
         pub fn g() {\n\
         \x20   let _ = O::new();\n\
         \x20   let _ = Colour::Red;\n\
         \x20   let _ = Ordering::Less;\n\
         \x20   util::f();\n\
         }\n",
    )]);
    let (_, out) = run(d.path(), &["analyze", "."]);
    assert_eq!(
        errors(&out),
        0,
        "a local type, Self, an enum variant, a local module and a name already \
         imported by `use` are not outside dependencies:\n{out}"
    );
}

#[test]
fn a_reference_to_the_projects_own_crate_is_not_a_finding() {
    let d = project(&[(
        "src/domain/subject.rs",
        "pub fn g() {\n    let _ = crate::domain::other();\n}\npub fn other() {}\n",
    )]);
    let (_, out) = run(d.path(), &["analyze", "."]);
    assert_eq!(errors(&out), 0, "crate:: is this project:\n{out}");
}

#[test]
fn a_relative_typescript_import_is_not_a_finding() {
    let (_d, out) = ts_case("import { z } from \"./other.js\";\nexport const x = z;\n");
    assert_eq!(errors(&out), 0, "a relative path resolves inside the project:\n{out}");
}

// ── 5. The allowlist applies to references exactly as it does to imports ─────

#[test]
fn an_allowlisted_crate_referenced_inline_passes_and_its_neighbour_does_not() {
    // The shipped policy has an empty `allow`, so this writes its own to test
    // the boundary — the one thing that must not change when references start
    // being judged.
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    std::fs::create_dir_all(r.join(".hexa")).unwrap();
    std::fs::write(
        r.join(".hexa/ADR-rules.toml"),
        "[[import_policy]]\n\
         adr = \"ADR-2609211600\"\n\
         id = \"domain-imports-only-what-it-is-allowed\"\n\
         message = \"The domain reached outside.\"\n\
         severity = \"error\"\n\
         layer = \"/domain/\"\n\
         allow = [\"serde\"]\n\
         deny = []\n",
    )
    .unwrap();
    std::fs::write(r.join("Cargo.toml"), MANIFEST).unwrap();
    std::fs::create_dir_all(r.join("src/domain")).unwrap();
    std::fs::write(
        r.join("src/domain/subject.rs"),
        "pub fn a() {\n    let _ = serde::de::IgnoredAny;\n}\n",
    )
    .unwrap();
    let (_, out) = run(r, &["analyze", "."]);
    assert_eq!(errors(&out), 0, "serde is allowed, inline or not:\n{out}");

    std::fs::write(
        r.join("src/domain/subject.rs"),
        "pub fn a() {\n    let _ = serde_json::to_string(&1);\n}\n",
    )
    .unwrap();
    let (_, out) = run(r, &["analyze", "."]);
    assert_eq!(errors(&out), 1, "serde_json is a different dependency:\n{out}");
}

// ── 6. A renamed dependency is judged by the name code writes ────────────────

#[test]
fn a_renamed_dependency_is_judged_by_its_key() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    std::fs::create_dir_all(r.join(".hexa")).unwrap();
    let shipped = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/templates/ADR-rules.toml"),
    )
    .unwrap();
    std::fs::write(r.join(".hexa/ADR-rules.toml"), shipped).unwrap();
    std::fs::write(
        r.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
         [dependencies]\npg = { package = \"tokio-postgres\", version = \"0.7\" }\n",
    )
    .unwrap();
    std::fs::create_dir_all(r.join("src/domain")).unwrap();
    std::fs::write(
        r.join("src/domain/subject.rs"),
        "pub fn a() {\n    let _ = pg::connect();\n}\n",
    )
    .unwrap();
    let (_, out) = run(r, &["analyze", "."]);
    assert_eq!(errors(&out), 1, "the key is the name code writes:\n{out}");
    assert!(out.contains("pg"), "{out}");
}

// ── 7. One finding per site ──────────────────────────────────────────────────

#[test]
fn a_declaration_and_a_reference_are_two_sites_but_one_line_is_one() {
    let (_d, out) = rust_case(
        "use sqlx::PgPool;\n\
         pub fn a(_p: PgPool) {\n\
         \x20   let _ = sqlx::query(\"x\");\n\
         }\n",
    );
    assert_eq!(errors(&out), 2, "the use line and the call line:\n{out}");

    let (_d, out) = rust_case(
        "pub fn a() {\n    let _ = (sqlx::query(\"x\"), sqlx::query(\"y\"));\n}\n",
    );
    assert_eq!(errors(&out), 1, "twice on one line is one site:\n{out}");
}

// ── 8. The grade moves by exactly one violation ──────────────────────────────

#[test]
fn one_inline_reference_costs_exactly_ten_points() {
    // The two fixtures differ in one thing only: the inline reference. An
    // earlier version of this test added a function to the dirty side, which
    // also added a dead export, and the drop was 11 rather than 10 for a
    // reason that had nothing to do with the subject.
    let clean = project(&[(
        "src/domain/subject.rs",
        "pub struct O;\npub fn c() {\n    let _ = 1;\n}\n",
    )]);
    let dirty = project(&[(
        "src/domain/subject.rs",
        "pub struct O;\npub fn c() {\n    let _ = std::fs::read(\"x\");\n}\n",
    )]);
    assert_eq!(
        score(dirty.path()),
        score(clean.path()) - 10,
        "one site, ten points"
    );
    let (graded, out) = run(dirty.path(), &["analyze", ".", "--grade", "A+"]);
    assert_eq!(graded, 1, "and the floor rejects it:\n{out}");
}

// ── 9. hexa's own tree is not exempt ─────────────────────────────────────────

#[test]
fn hexa_still_grades_a_plus_on_its_own_tree() {
    // If this fails, the finding is real and gets fixed in hexa — not
    // exempted. hexa has no `/domain/` directory today, so the shipped policy
    // matches nothing here; that is a fact about hexa's layout, and this test
    // exists to notice when it stops being true.
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("workspace root");
    let (_, out) = run(repo, &["analyze", "."]);
    assert!(
        out.contains("Architecture grade: A+"),
        "hexa must pass the rule it ships:\n{}",
        out.lines().filter(|l| l.contains("grade") || l.contains("ADR violation")).collect::<Vec<_>>().join("\n")
    );
}

// ── 10. Go ───────────────────────────────────────────────────────────────────

/// A Go project carrying the shipped rules file.
fn go_project(files: &[(&str, &str)]) -> tempfile::TempDir {
    let d = tempfile::tempdir().expect("tempdir");
    let r = d.path();
    std::fs::create_dir_all(r.join(".hexa")).unwrap();
    let shipped = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/templates/ADR-rules.toml"),
    )
    .expect("read the shipped rules file");
    std::fs::write(r.join(".hexa/ADR-rules.toml"), shipped).unwrap();
    std::fs::write(r.join("go.mod"), "module github.com/acme/app\n\ngo 1.22\n").unwrap();
    for (path, body) in files {
        let full = r.join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, body).unwrap();
    }
    d
}

/// ADR-2609211600 §6 claims Go needs no change, because Go cannot name a
/// package without importing it. That claim is the reason there is no Go
/// reference extractor, so it is worth a test rather than a sentence: the
/// qualified call `os.Getenv` must produce exactly the one finding its import
/// already produced, not two.
#[test]
fn a_go_package_is_reported_once_by_its_import_not_twice_by_its_use() {
    let d = go_project(&[(
        "internal/domain/order.go",
        "package domain\n\n\
         import (\n\
         \t\"os\"\n\
         )\n\n\
         func Home() string {\n\
         \treturn os.Getenv(\"HOME\")\n\
         }\n",
    )]);
    let (_, out) = run(d.path(), &["analyze", "."]);
    assert_eq!(
        errors(&out),
        1,
        "the import is the one site; the qualified call is not a second:\n{out}"
    );
    assert!(out.contains("os"), "{out}");
}

/// Go's own code is not a finding, the same negative control the Rust and
/// TypeScript sides carry.
#[test]
fn a_go_import_of_the_projects_own_module_is_not_a_finding() {
    let d = go_project(&[
        ("internal/domain/count.go", "package domain\n\ntype Count int\n"),
        (
            "internal/domain/order.go",
            "package domain\n\n\
             import (\n\
             \t\"github.com/acme/app/internal/domain\"\n\
             )\n\n\
             var _ = domain.Count(0)\n",
        ),
    ]);
    let (_, out) = run(d.path(), &["analyze", "."]);
    assert_eq!(errors(&out), 0, "the module's own package is inside the project:\n{out}");
}

/// ADR-2609211600 §6: `plugin` loads code at run time, which is what this
/// policy exists to keep out of a domain. It is new to the shipped deny list,
/// so it needs its own case.
#[test]
fn the_go_plugin_package_is_denied_by_the_shipped_policy() {
    let d = go_project(&[(
        "internal/domain/loader.go",
        "package domain\n\n\
         import (\n\
         \t\"plugin\"\n\
         )\n\n\
         func Load(p string) (*plugin.Plugin, error) {\n\
         \treturn plugin.Open(p)\n\
         }\n",
    )]);
    let (_, out) = run(d.path(), &["analyze", "."]);
    assert_eq!(errors(&out), 1, "loading code at run time is infrastructure:\n{out}");
    assert!(
        out.contains("denied by this policy's `deny`"),
        "and it is the deny list that says so:\n{out}"
    );
}
