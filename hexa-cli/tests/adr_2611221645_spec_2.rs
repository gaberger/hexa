//! ADR-2611221645 §5: a relative TypeScript import resolves to the file it names.
//!
//! Scenarios S19, S26, S27 and S28 of docs/specs/ADR-2611221645.json, written
//! from the scenario text alone, black box: each test lays out a TypeScript
//! project in a tempdir, runs the built `hexa` binary's `analyze . --json`, and
//! reads its observable output. The harness is copied from
//! `adr_2611221645_spec.rs` (sealed).
//!
//! How resolved_path is observed: a violating edge is reported in full under
//! `analysis.violations[].edge`, and its `to_file` is the import's resolved
//! path. An edge whose target is not a project file has no layer and is never
//! a violation, so its path is not printed; where a scenario's target is such a
//! file (S27), the test asserts what `analyze` does print (edge_count,
//! violations, file_count).

use std::path::Path;
use std::process::Command;

use serde_json::Value;

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn layers(root: &Path, map: &str) {
    write(root, ".hexa/project.json", &format!("{{\"analyze\":{{\"layers\":{map}}}}}"));
}

fn hexa(root: &Path, args: &[&str]) -> (String, String) {
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(args)
        .current_dir(root)
        .env("HOME", home.path())
        .output()
        .unwrap();
    (String::from_utf8_lossy(&out.stdout).to_string(), String::from_utf8_lossy(&out.stderr).to_string())
}

/// `hexa analyze . --json`: the report, and everything printed (for messages).
fn analyze(root: &Path) -> (Value, String) {
    let (stdout, stderr) = hexa(root, &["analyze", ".", "--json"]);
    let v: Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("analyze --json is not JSON ({e}):\n{stdout}{stderr}"));
    (v, format!("{stdout}{stderr}"))
}

/// The full edge of every reported violation.
fn violation_edges(v: &Value) -> Vec<Value> {
    v["analysis"]["violations"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|x| x["edge"].clone())
        .collect()
}

/// The single violation edge from `from` written as `import_path`, which must exist.
fn the_edge(v: &Value, from: &str, import_path: &str, all: &str) -> Value {
    let hits: Vec<Value> = violation_edges(v)
        .into_iter()
        .filter(|e| e["from_file"] == from && e["import_path"] == import_path)
        .collect();
    assert_eq!(
        hits.len(),
        1,
        "expected exactly one edge from {from} via {import_path:?}, got {hits:?}\n{all}"
    );
    hits[0].clone()
}

fn edge_count(v: &Value) -> u64 {
    v["analysis"]["edge_count"].as_u64().expect("analysis.edge_count")
}

fn health_score(v: &Value) -> u64 {
    v["analysis"]["health_score"].as_u64().expect("analysis.health_score")
}

/// A layer name as `analyze` prints it (`adapters_primary`), or as written in
/// the config (`adapters/primary`).
fn is_layer(got: &Value, want: &str) -> bool {
    got.as_str().is_some_and(|g| g == want || g == want.replace('/', "_"))
}

/// S19: A project with no paths whose relative imports each name an existing
/// .ts file as written grades exactly as before.
///
/// "Before and after this change" cannot run two binaries from one test, so
/// the test asserts the values the pre-change rule implies, derived by hand.
/// The pre-change rule: a relative specifier resolves against the importer
/// with `.ts` added; any other specifier stays as written. Every relative
/// import below names, with `.ts` added, a file that exists, so under that
/// rule each one is an edge to that file:
///
///   1. src/domain/money.ts          -> src/adapters/secondary/db.ts  (domain -> adapters/secondary: VIOLATION)
///   2. src/ports/repo.ts            -> src/domain/order.ts           (ports -> domain: allowed)
///   3. src/usecases/place.ts        -> src/ports/repo.ts             (usecases -> ports: allowed)
///   4. src/usecases/place.ts        -> src/domain/money.ts           (usecases -> domain: allowed)
///   5. src/adapters/secondary/db.ts -> src/ports/repo.ts             (adapters/secondary -> ports: allowed)
///   6. src/adapters/secondary/db.ts -> src/domain/order.ts           (adapters/secondary -> domain: VIOLATION,
///      a secondary adapter imports ports/ only)
///
/// plus one edge per package import, "zod" (place.ts) and "pg" (db.ts),
/// which name no project file: no layer, never a violation. edge_count = 6 + 2 = 8.
/// Violations: exactly edges 1 and 6.
/// Cycles: none (money -> db -> repo -> order never returns to money).
/// Dead exports: Order (used by repo, db), Repo (place, db), cents (place),
/// Db (money) are all imported; `place` is imported by no one: 1 dead export.
/// Unused ports: Repo is implemented by Db: 0. Every file is in a layer, so
/// the coverage ceiling is 100.
/// health_score = 100 - 10*2 (violations) - 15*0 (cycles) - 1 (dead export) = 79.
#[test]
fn s19_a_project_with_no_paths_grades_exactly_as_before() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "tsconfig.json", r#"{"compilerOptions": {"strict": true, "module": "ESNext"}, "include": ["src"]}"#);
    write(r, "src/domain/order.ts", "export interface Order { id: number }\n");
    write(
        r,
        "src/domain/money.ts",
        "import { Db } from \"../adapters/secondary/db\";\nexport const cents = (d: Db) => d.load().id;\n",
    );
    write(
        r,
        "src/ports/repo.ts",
        "import type { Order } from \"../domain/order\";\nexport interface Repo { load(): Order }\n",
    );
    write(
        r,
        "src/usecases/place.ts",
        "import type { Repo } from \"../ports/repo\";\nimport { cents } from \"../domain/money\";\nimport { z } from \"zod\";\nexport const place = (r: Repo) => (z ? 0 : 1) + r.load().id + (cents ? 0 : 1);\n",
    );
    write(
        r,
        "src/adapters/secondary/db.ts",
        "import type { Repo } from \"../../ports/repo\";\nimport type { Order } from \"../../domain/order\";\nimport pg from \"pg\";\nexport class Db implements Repo { load(): Order { return { id: pg ? 1 : 0 }; } }\n",
    );
    let (v, all) = analyze(r);

    let e = the_edge(&v, "src/domain/money.ts", "../adapters/secondary/db", &all);
    assert_eq!(e["to_file"], "src/adapters/secondary/db.ts", "{all}");
    let e6 = the_edge(&v, "src/adapters/secondary/db.ts", "../../domain/order", &all);
    assert_eq!(e6["to_file"], "src/domain/order.ts", "{all}");
    assert_eq!(violation_edges(&v).len(), 2, "exactly the two violations the pre-change rule implies:\n{all}");
    assert_eq!(edge_count(&v), 8, "6 project edges + 2 package edges:\n{all}");
    assert_eq!(health_score(&v), 79, "100 - 20 (two violations) - 1 (one dead export):\n{all}");
}

/// S26: A relative import of a .tsx file or a folder's index.ts resolves to that file.
#[test]
fn s26_a_relative_import_of_a_tsx_file_or_a_folder_index_resolves_to_that_file() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "tsconfig.json", r#"{"compilerOptions": {"jsx": "react-jsx"}}"#);
    layers(r, r#"{"src/domain": "domain", "src/ui": "adapters/primary"}"#);
    write(r, "src/ui/Button.tsx", "export const Button = () => null;\n");
    write(r, "src/ui/index.ts", "export const ui = 1;\n");
    write(r, "src/domain/a.ts", "import { Button } from \"../ui/Button\";\nexport const a = () => Button;\n");
    write(r, "src/domain/b.ts", "import { ui } from \"../ui\";\nexport const b = () => ui;\n");
    assert!(!r.join("src/ui/Button.ts").exists());
    let (v, all) = analyze(r);

    let ea = the_edge(&v, "src/domain/a.ts", "../ui/Button", &all);
    assert_eq!(ea["to_file"], "src/ui/Button.tsx", "{all}");
    let eb = the_edge(&v, "src/domain/b.ts", "../ui", &all);
    assert_eq!(eb["to_file"], "src/ui/index.ts", "{all}");
    for e in [&ea, &eb] {
        assert!(is_layer(&e["from_layer"], "domain"), "from_layer {}:\n{all}", e["from_layer"]);
        assert!(is_layer(&e["to_layer"], "adapters/primary"), "to_layer {}:\n{all}", e["to_layer"]);
    }
}

/// S27: A relative import that names no project file is named as before.
///
/// Its target is no project file, so the edge has no layer and `analyze` does
/// not print its path; the test asserts what is printed: the import is still
/// one edge, it is not a violation, and no candidate file was invented.
#[test]
fn s27_a_relative_import_that_names_no_project_file_is_named_as_before() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "tsconfig.json", "{}");
    write(r, "src/domain/a.ts", "import { t } from \"../gone/thing\";\nexport const a = () => t;\n");
    for f in [
        "src/gone/thing",
        "src/gone/thing.ts",
        "src/gone/thing.tsx",
        "src/gone/thing/index.ts",
        "src/gone/thing/index.tsx",
    ] {
        assert!(!r.join(f).exists());
    }
    let (v, all) = analyze(r);

    assert_eq!(edge_count(&v), 1, "the import is still an edge (to src/gone/thing.ts):\n{all}");
    assert!(violation_edges(&v).is_empty(), "the import is not a violation:\n{all}");
    assert_eq!(v["analysis"]["file_count"], 1, "no file was invented:\n{all}");
}

/// S28: A NodeNext .js specifier resolves to the .ts file of the same stem.
///
/// The given names no layers. An edge is printed in full only when it is a
/// violation, so the test adds an analyze.layers map (files and imports exactly
/// as given) that puts src/domain/b.ts in adapters/secondary: the edge's
/// to_file is then visible, and it is the import's resolved_path.
#[test]
fn s28_a_nodenext_js_specifier_resolves_to_the_ts_file_of_the_same_stem() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "tsconfig.json", r#"{"compilerOptions": {"module": "NodeNext", "moduleResolution": "NodeNext"}}"#);
    layers(r, r#"{"src/domain/a.ts": "domain", "src/domain/b.ts": "adapters/secondary"}"#);
    write(r, "src/domain/b.ts", "export const fromB = 1;\n");
    write(r, "src/domain/a.ts", "import { fromB } from \"./b.js\";\nexport const a = () => fromB;\n");
    assert!(!r.join("src/domain/b.js").exists());
    let (v, all) = analyze(r);

    let e = the_edge(&v, "src/domain/a.ts", "./b.js", &all);
    assert_eq!(e["to_file"], "src/domain/b.ts", "{all}");
}
