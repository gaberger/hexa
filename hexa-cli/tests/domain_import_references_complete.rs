//! Every reference in the file is read, and every one is judged.
//!
//! ADR-2609221430. ADR-2609211600 made an inline reference count as an import.
//! It did not make *every* inline reference count, and the gaps are not exotic:
//! a path inside `println!`, a second path on a line whose first path was
//! allowed, a crate declared in a nested workspace member's own manifest.
//!
//! Each is the same failure as the one ADR-2609211600 closed — the rules file
//! names a protection the tool does not give — and each looked fine, because a
//! check that finds nothing and a check that never looked print the same thing.
//!
//! Every case here was reproduced against `08e7cda` before the fix was written.
//! The negative controls at the end must pass both before and after: the hard
//! half of this change is *not* flagging local code, and widening the extractor
//! is exactly how that gets broken.

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
fn warnings(out: &str) -> usize {
    counts(out).1
}

const MANIFEST: &str = "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
     [dependencies]\nsqlx = \"0.7\"\ntokio = \"1\"\nserde = \"1\"\n";

/// The shipped rules file, so this gate tests what a scaffolded project gets.
fn shipped_rules() -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/templates/ADR-rules.toml"),
    )
    .expect("read the shipped rules file")
}

fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
    let d = tempfile::tempdir().expect("tempdir");
    let r = d.path();
    std::fs::create_dir_all(r.join(".hexa")).unwrap();
    std::fs::write(r.join(".hexa/ADR-rules.toml"), shipped_rules()).unwrap();
    std::fs::write(r.join("Cargo.toml"), MANIFEST).unwrap();
    for (path, body) in files {
        let full = r.join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, body).unwrap();
    }
    d
}

/// A project with no root manifest of its own, for workspace fixtures.
fn bare_project(files: &[(&str, &str)]) -> tempfile::TempDir {
    let d = tempfile::tempdir().expect("tempdir");
    let r = d.path();
    std::fs::create_dir_all(r.join(".hexa")).unwrap();
    std::fs::write(r.join(".hexa/ADR-rules.toml"), shipped_rules()).unwrap();
    for (path, body) in files {
        let full = r.join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, body).unwrap();
    }
    d
}

fn rust_case(body: &str) -> (tempfile::TempDir, String) {
    let d = project(&[("src/domain/subject.rs", body)]);
    let (_, out) = run(d.path(), &["analyze", "."]);
    (d, out)
}

// ── 1. Macro arguments (Decision 1) ──────────────────────────────────────────
//
// `collect_rust_references` reads `scoped_identifier` nodes. Inside a macro the
// arguments are one `token_tree`, which contains none, so every path written in
// a macro call was invisible.

#[test]
fn a_denied_path_inside_println_is_an_error() {
    let (_d, out) = rust_case("pub fn a() {\n    println!(\"{:?}\", std::fs::read(\"x\"));\n}\n");
    assert_eq!(errors(&out), 1, "a macro argument is still code:\n{out}");
    assert!(out.contains("std::fs"), "and the finding names the path:\n{out}");
    assert!(out.contains("subject.rs:2"), "at its own line:\n{out}");
}

#[test]
fn a_denied_path_inside_vec_is_an_error() {
    let (_d, out) = rust_case("pub fn b() {\n    let _ = vec![std::fs::read(\"x\")];\n}\n");
    assert_eq!(errors(&out), 1, "vec! is a macro too:\n{out}");
}

#[test]
fn a_denied_path_inside_assert_is_an_error() {
    let (_d, out) = rust_case("pub fn c() {\n    assert!(std::env::var(\"X\").is_ok());\n}\n");
    assert_eq!(errors(&out), 1, "std::env is denied by the shipped policy:\n{out}");
}

#[test]
fn a_dependency_named_inside_format_is_an_error() {
    let (_d, out) = rust_case("pub fn d() {\n    let _ = format!(\"{:?}\", sqlx::query(\"x\"));\n}\n");
    assert_eq!(errors(&out), 1, "a crate reached through a macro is still reached:\n{out}");
    assert!(out.contains("sqlx"), "{out}");
}

#[test]
fn a_path_in_a_nested_macro_is_an_error() {
    let (_d, out) =
        rust_case("pub fn e() {\n    println!(\"{:?}\", vec![std::fs::read(\"x\")]);\n}\n");
    assert_eq!(errors(&out), 1, "token trees nest, and so does the walk:\n{out}");
}

// ── 2. Judge first, then de-duplicate (Decision 2) ───────────────────────────
//
// De-duplication keyed on (line, first segment) ran *before* judging, so a
// permitted `std::collections` claimed the key `(1, "std")` and the denied
// `std::fs::read` after it on the same line was dropped without being judged.

#[test]
fn an_allowed_path_does_not_swallow_a_denied_one_on_the_same_line() {
    let (_d, out) = rust_case(
        "pub fn a() { let _m = std::collections::HashMap::<u8,u8>::new(); let _ = std::fs::read(\"x\"); }\n",
    );
    assert_eq!(errors(&out), 1, "the denied path is judged on its own merits:\n{out}");
    assert!(out.contains("std::fs"), "and it is the one reported:\n{out}");
}

#[test]
fn a_use_declaration_does_not_swallow_a_denied_reference_on_the_same_line() {
    let (_d, out) = rust_case(
        "use std::collections::HashMap; pub fn c(){ let _h: HashMap<u8,u8> = HashMap::new(); let _ = std::fs::read(\"x\"); }\n",
    );
    assert_eq!(errors(&out), 1, "a declaration is not a licence for the rest of the line:\n{out}");
    assert!(out.contains("std::fs"), "{out}");
}

#[test]
fn the_reverse_order_still_reports_exactly_one() {
    // This one passed before the fix. It is here so the fix is not a swap of
    // which order works.
    let (_d, out) = rust_case(
        "pub fn a() { let _ = std::fs::read(\"x\"); let _m = std::collections::HashMap::<u8,u8>::new(); }\n",
    );
    assert_eq!(errors(&out), 1, "one reach outside is one finding:\n{out}");
}

#[test]
fn the_same_denied_path_twice_on_one_line_is_one_finding() {
    // De-duplication still has a job: the same name at the same place is one
    // reach outside, not two, and must not charge the grade twice.
    let (_d, out) =
        rust_case("pub fn a() { let _ = (std::fs::read(\"x\"), std::fs::read(\"y\")); }\n");
    assert_eq!(errors(&out), 1, "one name, one line, one finding:\n{out}");
}

#[test]
fn two_different_denied_paths_on_one_line_are_two_findings() {
    let (_d, out) =
        rust_case("pub fn a() { let _ = (std::fs::read(\"x\"), std::env::var(\"Y\")); }\n");
    assert_eq!(errors(&out), 2, "two distinct reaches outside are two findings:\n{out}");
}

// ── 3. Every manifest in the workspace (Decision 3) ──────────────────────────
//
// `project_names` read the root manifest and one directory down, so a
// dependency declared by `crates/core/Cargo.toml` was not a known external
// name and an inline path naming it was never judged. The `use` form *was*
// caught, so one file gave two answers about the same crate.

const MEMBER_MANIFEST: &str =
    "[package]\nname = \"core_x\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
     [dependencies]\nsqlx = \"0.7\"\n";

#[test]
fn a_dependency_of_a_nested_workspace_member_is_external() {
    let d = bare_project(&[
        ("Cargo.toml", "[workspace]\nmembers = [\"crates/core\"]\n"),
        ("crates/core/Cargo.toml", MEMBER_MANIFEST),
        ("crates/core/src/domain/a.rs", "pub fn a() {\n    let _ = sqlx::query(\"x\");\n}\n"),
    ]);
    let (_, out) = run(d.path(), &["analyze", "."]);
    assert_eq!(errors(&out), 1, "the member's own manifest declares sqlx:\n{out}");
    assert!(out.contains("sqlx"), "{out}");
}

#[test]
fn the_declaration_and_the_reference_agree_in_one_file() {
    // The asymmetry that gave this away: before the fix the `use` line was an
    // error and the inline path two lines down was not.
    let d = bare_project(&[
        ("Cargo.toml", "[workspace]\nmembers = [\"crates/core\"]\n"),
        ("crates/core/Cargo.toml", MEMBER_MANIFEST),
        (
            "crates/core/src/domain/a.rs",
            "use sqlx::PgPool;\npub fn a(_p: &PgPool) {\n    let _ = sqlx::query(\"x\");\n}\n",
        ),
    ]);
    let (_, out) = run(d.path(), &["analyze", "."]);
    assert_eq!(errors(&out), 2, "the declaration and the reference are both reported:\n{out}");
}

#[test]
fn a_glob_member_is_expanded() {
    let d = bare_project(&[
        ("Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n"),
        ("crates/core/Cargo.toml", MEMBER_MANIFEST),
        ("crates/core/src/domain/a.rs", "pub fn a() {\n    let _ = sqlx::query(\"x\");\n}\n"),
    ]);
    let (_, out) = run(d.path(), &["analyze", "."]);
    assert_eq!(errors(&out), 1, "members globs are expanded:\n{out}");
}

#[test]
fn an_excluded_manifest_is_not_read() {
    // `rusqlite` is declared only by the excluded member, so it is not a known
    // external name and an inline path naming it is out of scope. The point is
    // that `exclude` is honoured, not that the code is fine.
    let d = bare_project(&[
        ("Cargo.toml", "[workspace]\nmembers = [\"crates/core\"]\nexclude = [\"crates/skip\"]\n"),
        ("crates/core/Cargo.toml", MEMBER_MANIFEST),
        ("crates/core/src/domain/keep.rs", "pub fn k() {}\n"),
        (
            "crates/skip/Cargo.toml",
            "[package]\nname = \"skipped\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
             [dependencies]\nrusqlite = \"0.31\"\n",
        ),
        ("crates/skip/src/domain/a.rs", "pub fn a() {\n    let _ = rusqlite::open(\"x\");\n}\n"),
    ]);
    let (_, out) = run(d.path(), &["analyze", "."]);
    assert_eq!(errors(&out), 0, "an excluded member's manifest is not read:\n{out}");
}

// ── 4. Target-specific dependencies (Decision 3) ─────────────────────────────

#[test]
fn a_target_specific_dependency_is_external() {
    let d = bare_project(&[
        (
            "Cargo.toml",
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
             [target.'cfg(unix)'.dependencies]\nnix = \"0.29\"\n",
        ),
        ("src/domain/n.rs", "pub fn a() {\n    let _ = nix::unistd::getpid();\n}\n"),
    ]);
    let (_, out) = run(d.path(), &["analyze", "."]);
    assert_eq!(errors(&out), 1, "a cfg-gated dependency is still a dependency:\n{out}");
    assert!(out.contains("nix"), "{out}");
}

#[test]
fn a_target_specific_dev_dependency_is_external() {
    let d = bare_project(&[
        (
            "Cargo.toml",
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
             [target.'cfg(windows)'.dev-dependencies]\nwinapi = \"0.3\"\n",
        ),
        ("src/domain/w.rs", "pub fn a() {\n    let _ = winapi::um::winbase::GetTickCount();\n}\n"),
    ]);
    let (_, out) = run(d.path(), &["analyze", "."]);
    assert_eq!(errors(&out), 1, "every target dependency table is read:\n{out}");
}

// ── 5. TypeScript import-equals and template literals (Decision 4) ───────────

fn ts_project(files: &[(&str, &str)]) -> tempfile::TempDir {
    let d = tempfile::tempdir().expect("tempdir");
    let r = d.path();
    std::fs::create_dir_all(r.join(".hexa")).unwrap();
    std::fs::write(r.join(".hexa/ADR-rules.toml"), shipped_rules()).unwrap();
    std::fs::write(r.join("package.json"), "{\"name\":\"t\",\"dependencies\":{\"pg\":\"^8\"}}\n")
        .unwrap();
    std::fs::write(r.join("tsconfig.json"), "{\"compilerOptions\":{\"module\":\"NodeNext\"}}\n")
        .unwrap();
    for (path, body) in files {
        let full = r.join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, body).unwrap();
    }
    d
}

#[test]
fn a_typescript_import_equals_is_an_import() {
    let d = ts_project(&[("src/domain/ie.ts", "import pg = require(\"pg\");\nexport const a = pg;\n")]);
    let (_, out) = run(d.path(), &["analyze", "."]);
    assert_eq!(errors(&out), 1, "import-equals is how TypeScript writes require:\n{out}");
    assert!(out.contains("pg"), "{out}");
}

#[test]
fn a_template_literal_with_no_substitution_is_a_literal() {
    let d = ts_project(&[("src/domain/tl.ts", "export const b = require(`pg`);\n")]);
    let (_, out) = run(d.path(), &["analyze", "."]);
    assert_eq!(errors(&out), 1, "backticks around a constant name are still a constant:\n{out}");
    assert_eq!(warnings(&out), 0, "and it is not a computed load:\n{out}");
}

#[test]
fn a_template_literal_with_a_substitution_stays_a_warning() {
    let d = ts_project(&[("src/domain/ts.ts", "export const c = (x: string) => require(`${x}`);\n")]);
    let (_, out) = run(d.path(), &["analyze", "."]);
    assert_eq!(errors(&out), 0, "nothing can be judged about it:\n{out}");
    assert_eq!(warnings(&out), 1, "but it is never skipped in silence:\n{out}");
}

#[test]
fn a_dynamic_import_with_a_plain_template_is_a_literal() {
    let d = ts_project(&[("src/domain/di.ts", "export const d = () => import(`pg`);\n")]);
    let (_, out) = run(d.path(), &["analyze", "."]);
    assert_eq!(errors(&out), 1, "import() reads the same as require():\n{out}");
}

// ── 6. Qualified paths (Decision 5) ──────────────────────────────────────────

#[test]
fn a_qualified_path_is_unwrapped() {
    let (_d, out) = rust_case("pub fn q() {\n    let _ = <sqlx::PgPool as Default>::default;\n}\n");
    assert_eq!(errors(&out), 1, "`<T as Trait>` still names T:\n{out}");
    assert!(out.contains("sqlx"), "and the finding names the crate, not `<sqlx`:\n{out}");
}

#[test]
fn a_qualified_path_over_a_local_type_is_still_clean() {
    let (_d, out) = rust_case(
        "pub struct O;\nimpl Default for O { fn default() -> Self { O } }\n\
         pub fn q() {\n    let _ = <O as Default>::default;\n}\n",
    );
    assert_eq!(errors(&out), 0, "unwrapping must not start flagging local types:\n{out}");
}

// ── 7. Nothing is skipped silently (Decision 6) ──────────────────────────────

/// A file whose bytes are not UTF-8: `read_to_string` fails, which is the
/// `else { continue }` that dropped a file from the check with no finding.
fn unreadable_project() -> tempfile::TempDir {
    let d = project(&[("src/domain/ok.rs", "pub fn a() {}\n")]);
    std::fs::write(d.path().join("src/domain/bad.rs"), [0xff, 0xfe, 0x00, 0x9c]).unwrap();
    d
}

#[test]
fn a_file_that_cannot_be_read_is_reported() {
    let d = unreadable_project();
    let (_, out) = run(d.path(), &["analyze", "."]);
    assert_eq!(warnings(&out), 1, "a file the check could not read is named:\n{out}");
    assert!(out.contains("bad.rs"), "and it says which one:\n{out}");
}

#[test]
fn an_unreadable_file_does_not_move_the_grade() {
    let d = unreadable_project();
    assert_eq!(score(d.path()), 100, "a warning reports without scoring (ADR-2609211430)");
}

#[test]
fn strict_fails_on_an_unreadable_file() {
    let d = unreadable_project();
    let (code, out) = run(d.path(), &["analyze", ".", "--strict"]);
    assert_eq!(code, 1, "--strict is what makes a warning binding:\n{out}");
}

// ── 8. Layer matching is anchored (Decision 7) ───────────────────────────────

#[test]
fn a_test_directory_is_not_the_domain() {
    let d = project(&[("tests/domain/x.rs", "pub fn a() {\n    let _ = sqlx::query(\"x\");\n}\n")]);
    let (_, out) = run(d.path(), &["analyze", "."]);
    assert_eq!(errors(&out), 0, "a top-level tests/ directory is out of scope:\n{out}");
}

/// A control, not a gap: `/domain/` was never a substring of
/// `/domain_helpers/`, so this passed before the change too. It is here because
/// anchoring is the kind of edit that looks equivalent and is not.
#[test]
fn a_directory_merely_starting_with_domain_is_not_the_domain() {
    let d = project(&[(
        "src/adapters/domain_helpers/x.rs",
        "pub fn a() {\n    let _ = sqlx::query(\"x\");\n}\n",
    )]);
    let (_, out) = run(d.path(), &["analyze", "."]);
    assert_eq!(errors(&out), 0, "`domain_helpers` is not the segment `domain`:\n{out}");
}

/// Also a control. A `domain` directory nested under `adapters` keeps matching:
/// `domain` is a whole segment there, and a rule that disqualified it for its
/// parent is one this ADR did not make.
#[test]
fn a_real_domain_segment_under_adapters_still_matches() {
    let d = project(&[(
        "src/adapters/domain/x.rs",
        "pub fn a() {\n    let _ = sqlx::query(\"x\");\n}\n",
    )]);
    let (_, out) = run(d.path(), &["analyze", "."]);
    assert_eq!(errors(&out), 1, "a whole `domain` segment is the domain:\n{out}");
}

#[test]
fn the_real_domain_is_still_the_domain() {
    // The control for the two above: anchoring must not stop the policy working.
    let (_d, out) = rust_case("pub fn a() {\n    let _ = sqlx::query(\"x\");\n}\n");
    assert_eq!(errors(&out), 1, "src/domain/ is still matched:\n{out}");
}

// ── 9. The dead MCP entry (Decision 8) ───────────────────────────────────────

#[test]
fn hexa_has_no_mcp_subcommand() {
    // The premise of the fix: the entry pointed at a command that does not
    // exist. If an MCP server is ever added, this test says so.
    let d = tempfile::tempdir().unwrap();
    let (code, out) = run(d.path(), &["mcp"]);
    assert_ne!(code, 0, "there is no `hexa mcp`:\n{out}");
}

#[test]
fn assets_sync_writes_no_hexa_mcp_entry() {
    let d = project(&[("src/domain/a.rs", "pub fn a() {}\n")]);
    let (_, out) = run(d.path(), &["assets", "sync", ".", "--force"]);
    let mcp = d.path().join(".mcp.json");
    if mcp.is_file() {
        let text = std::fs::read_to_string(&mcp).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(
            v["mcpServers"].get("hexa").is_none(),
            "a server that exits immediately is worse than no entry:\n{text}\n{out}"
        );
    }
}

#[test]
fn assets_sync_removes_the_old_entry_and_keeps_the_others() {
    let d = project(&[("src/domain/a.rs", "pub fn a() {}\n")]);
    std::fs::write(
        d.path().join(".mcp.json"),
        "{\"mcpServers\":{\"hexa\":{\"command\":\"hexa\",\"args\":[\"mcp\"]},\
         \"other\":{\"command\":\"other-server\",\"args\":[]}}}\n",
    )
    .unwrap();
    let (_, out) = run(d.path(), &["assets", "sync", ".", "--force"]);
    let text = std::fs::read_to_string(d.path().join(".mcp.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(v["mcpServers"].get("hexa").is_none(), "the dead entry is removed:\n{text}\n{out}");
    assert!(
        v["mcpServers"].get("other").is_some(),
        "and nothing else is touched — this file is the user's:\n{text}"
    );
}

#[test]
fn the_stale_permission_is_gone_from_both_settings_files() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("workspace root");
    for rel in [".claude/settings.json", "hexa-cli/assets/templates/hexa-claude-settings.json"] {
        let p = root.join(rel);
        if p.is_file() {
            let text = std::fs::read_to_string(&p).unwrap();
            assert!(!text.contains("mcp__hex__hex_"), "{rel} still permits the old tool name");
        }
    }
}

// ── 10. Must not regress ─────────────────────────────────────────────────────
//
// Widening the extractor is how local code starts getting flagged. These pass
// before the change and must pass after.

#[test]
fn local_code_with_a_crate_paths_shape_is_still_clean() {
    let (_d, out) = rust_case(
        "pub struct O;\nimpl O { pub fn new() -> Self { O } }\n\
         pub enum E { A }\n\
         pub fn f() {\n    let _ = O::new();\n    let _ = Self_like::nothing;\n}\n\
         mod Self_like { pub const nothing: u8 = 0; }\n",
    );
    assert_eq!(errors(&out), 0, "local paths are not references outside:\n{out}");
}

#[test]
fn comments_doc_comments_and_strings_are_still_clean() {
    let (_d, out) = rust_case(
        "/// A doc comment naming std::fs::read and sqlx::query.\n\
         // A comment naming std::process::exit.\n\
         pub fn f() {\n    let _ = \"std::fs::read(\\\"x\\\")\";\n}\n",
    );
    assert_eq!(errors(&out), 0, "prose is not code:\n{out}");
}

#[test]
fn a_macro_with_no_paths_is_still_clean() {
    let (_d, out) = rust_case("pub fn f() {\n    let _ = vec![1, 2, 3];\n    println!(\"hi\");\n}\n");
    assert_eq!(errors(&out), 0, "reading token trees must not invent references:\n{out}");
}

#[test]
fn a_local_module_path_in_a_macro_is_still_clean() {
    let (_d, out) = rust_case(
        "mod util { pub fn v() -> u8 { 1 } }\n\
         pub fn f() {\n    println!(\"{}\", util::v());\n    println!(\"{}\", crate::util::v());\n}\n",
    );
    assert_eq!(errors(&out), 0, "a local module inside a macro is still local:\n{out}");
}

#[test]
fn an_enum_variant_in_a_macro_is_still_clean() {
    let (_d, out) = rust_case(
        "use std::cmp::Ordering;\n\
         pub fn f(o: Ordering) {\n    assert!(o == Ordering::Less);\n}\n",
    );
    assert_eq!(errors(&out), 0, "Ordering::Less is a variant, not a crate:\n{out}");
}

#[test]
fn hexas_own_domain_still_imports_nothing_it_may_not() {
    // The broadest regression check available: this change reads more of every
    // file in the workspace, and hexa's own domain must stay clean. It used to
    // assert the whole grade was A+ as a stand-in; once the grade read every
    // file and every crate, the grade stopped being about this. The whole
    // tree's violations are ratcheted in hexa_grades_itself_by_ratchet.rs.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("workspace root");
    let (_, out) = run(root, &["analyze", ".", "--json"]);
    // `run` joins stdout and stderr; the report is the first JSON value.
    let v: serde_json::Value = serde_json::Deserializer::from_str(&out)
        .into_iter()
        .next()
        .and_then(Result::ok)
        .unwrap_or_else(|| panic!("no JSON report:\n{out}"));
    assert_eq!(
        v["score_components"]["rule_errors"], 0,
        "hexa's own domain import policy is the floor this change may not lower:\n{out}"
    );
}
