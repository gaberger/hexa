//! ADR-2611221645: TypeScript path aliases are import edges.
//!
//! Acceptance tests written from the accepted spec (docs/specs/ADR-2611221645.json)
//! alone, black box: each test lays out a TypeScript project in a tempdir, runs
//! the built `hexa` binary, and reads its observable output.
//!
//! How an edge is observed: `hexa analyze . --json` reports every violating
//! edge in full under `analysis.violations[].edge` (from_file, to_file,
//! from_layer, to_layer, import_path, line). So each fixture puts the importing
//! file in the domain layer and every candidate target in a secondary-adapter
//! layer: an edge to any candidate is then a violation, and its to_file is
//! visible. An edge to a file that is not a project file has no layer and is
//! never a violation.
//!
//! S21 to S24 are not covered here.

use std::collections::BTreeSet;
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

/// The violation edges from `from` written with `import_path`.
fn edges_for(v: &Value, from: &str, import_path: &str) -> Vec<Value> {
    violation_edges(v)
        .into_iter()
        .filter(|e| e["from_file"] == from && e["import_path"] == import_path)
        .collect()
}

/// The single edge from `from` written as `import_path`, which must exist.
fn the_edge(v: &Value, from: &str, import_path: &str, all: &str) -> Value {
    let hits = edges_for(v, from, import_path);
    assert_eq!(
        hits.len(),
        1,
        "expected exactly one edge from {from} via {import_path:?}, got {hits:?}\n{all}"
    );
    hits[0].clone()
}

fn to_file(e: &Value) -> String {
    e["to_file"].as_str().unwrap_or_default().to_string()
}

fn no_edge_to(v: &Value, target_prefix: &str, all: &str) {
    let bad: Vec<_> = violation_edges(v)
        .into_iter()
        .filter(|e| to_file(e).starts_with(target_prefix))
        .collect();
    assert!(bad.is_empty(), "no edge may name {target_prefix}, got {bad:?}\n{all}");
}

fn edge_count(v: &Value) -> u64 {
    v["analysis"]["edge_count"].as_u64().expect("analysis.edge_count")
}

const GET: &str = "export function get(): number { return 1; }\n";
const IMPORT_HTTP: &str = "import { get } from \"@/lib/http\";\nexport const a = () => get();\n";

/// S01: An @/ alias resolves to the project file under src, and the edge is judged by that file's layer.
#[test]
fn s01_an_alias_resolves_to_the_project_file_and_is_judged_by_its_layer() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "tsconfig.json", r#"{"compilerOptions": {"paths": {"@/*": ["./src/*"]}}}"#);
    write(
        r,
        "src/domain/order.ts",
        "// an order\n// priced by the api\nimport { get } from \"@/adapters/secondary/http\";\nexport const price = () => get();\n",
    );
    write(r, "src/adapters/secondary/http.ts", GET);
    let (v, all) = analyze(r);
    let e = the_edge(&v, "src/domain/order.ts", "@/adapters/secondary/http", &all);
    assert_eq!(e["to_file"], "src/adapters/secondary/http.ts", "{all}");
    assert_eq!(e["from_layer"], "domain", "{all}");
    assert_eq!(e["to_layer"], "adapters_secondary", "{all}");
    assert_eq!(e["line"], 3, "{all}");
}

/// S02: The resolved alias target tries the same extensions and index files a relative import tries.
#[test]
fn s02_an_alias_target_tries_the_same_extensions_and_index_files() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "tsconfig.json", r#"{"compilerOptions": {"paths": {"@/*": ["./src/*"]}}}"#);
    layers(r, r#"{"src/app": "domain", "src/lib": "adapters/secondary"}"#);
    write(r, "src/lib/http/index.ts", GET);
    write(r, "src/app/main.ts", "import { get } from \"@/lib/http\";\nexport const a = () => get();\n");
    // The relative control is its own file: an edge joins two files, so one
    // file importing the same module twice is one edge, not two.
    write(r, "src/app/other.ts", "import { get } from \"../lib/http\";\nexport const b = () => get();\n");
    let (v, all) = analyze(r);
    let alias = the_edge(&v, "src/app/main.ts", "@/lib/http", &all);
    let rel = the_edge(&v, "src/app/other.ts", "../lib/http", &all);
    assert_eq!(to_file(&rel), "src/lib/http/index.ts", "the relative control:\n{all}");
    assert_eq!(to_file(&alias), "src/lib/http/index.ts", "{all}");
    assert_eq!(to_file(&alias), to_file(&rel), "{all}");
}

/// S03: Targets are tried in order and the first that names a project file wins.
#[test]
fn s03_targets_are_tried_in_order_and_the_first_existing_wins() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "tsconfig.json", r#"{"compilerOptions": {"paths": {"@shared/*": ["./generated/*", "./src/shared/*"]}}}"#);
    layers(r, r#"{"src/app": "domain", "src/shared": "adapters/secondary", "generated": "adapters/secondary"}"#);
    write(r, "src/shared/money.ts", "export const cents = 100;\n");
    write(r, "src/app/a.ts", "import { cents } from \"@shared/money\";\nexport const a = () => cents;\n");
    let (v, all) = analyze(r);
    let e = the_edge(&v, "src/app/a.ts", "@shared/money", &all);
    assert_eq!(to_file(&e), "src/shared/money.ts", "{all}");
    no_edge_to(&v, "generated/", &all);
}

/// S04: When both targets name a project file, the earlier target wins.
#[test]
fn s04_when_both_targets_exist_the_earlier_wins() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "tsconfig.json", r#"{"compilerOptions": {"paths": {"@shared/*": ["./generated/*", "./src/shared/*"]}}}"#);
    layers(r, r#"{"src/app": "domain", "src/shared": "adapters/secondary", "generated": "adapters/secondary"}"#);
    write(r, "src/shared/money.ts", "export const cents = 100;\n");
    write(r, "generated/money.ts", "export const cents = 100;\n");
    write(r, "src/app/a.ts", "import { cents } from \"@shared/money\";\nexport const a = () => cents;\n");
    let (v, all) = analyze(r);
    let e = the_edge(&v, "src/app/a.ts", "@shared/money", &all);
    assert_eq!(to_file(&e), "generated/money.ts", "{all}");
    no_edge_to(&v, "src/shared/money.ts", &all);
}

/// S05: Targets are relative to baseUrl when it is set.
#[test]
fn s05_targets_are_relative_to_base_url() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "tsconfig.json", r#"{"compilerOptions": {"baseUrl": "./web", "paths": {"@/*": ["src/*"]}}}"#);
    layers(r, r#"{"web/src/app": "domain", "web/src/lib": "adapters/secondary"}"#);
    write(r, "web/src/lib/http.ts", GET);
    write(r, "web/src/app/a.ts", IMPORT_HTTP);
    let (v, all) = analyze(r);
    let e = the_edge(&v, "web/src/app/a.ts", "@/lib/http", &all);
    assert_eq!(to_file(&e), "web/src/lib/http.ts", "{all}");
}

/// S06: A pattern with no wildcard matches only the exact specifier.
#[test]
fn s06_a_pattern_without_a_wildcard_matches_the_exact_specifier() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "tsconfig.json", r#"{"compilerOptions": {"paths": {"config": ["./src/config/index.ts"]}}}"#);
    layers(r, r#"{"src/app": "domain", "src/config": "adapters/secondary"}"#);
    write(r, "src/config/index.ts", "export const port = 8080;\n");
    write(r, "src/app/a.ts", "import { port } from \"config\";\nexport const a = () => port;\n");
    let (v, all) = analyze(r);
    let e = the_edge(&v, "src/app/a.ts", "config", &all);
    assert_eq!(to_file(&e), "src/config/index.ts", "{all}");
}

/// S07: The portal reproducer reports every real violation.
///
/// The fixture is a local fixture, ts-path-aliases-portal, at b046c9f,
/// embedded here so the test is hermetic: its tsconfig.json, its source files,
/// and the entries of the portal's layer and adapter maps that name those files.
#[test]
fn s07_the_portal_reproducer_reports_every_real_violation() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(
        r,
        "tsconfig.json",
        r#"{
	"compilerOptions": {
		"paths": { "@/*": ["./src/*"] },
		"moduleResolution": "Bundler",
		"module": "ESNext",
		"strict": true,
		"noEmit": true
	},
	"include": ["src"]
}
"#,
    );
    write(
        r,
        ".hexa/project.json",
        r#"{
	"name": "portal",
	"lifecycle_enforcement": "advisory",
	"analyze": {
		"layers": {
			"src/ports": "ports",
			"src/lib/openapi.gen.ts": "ports",
			"src/lib/http.ts": "adapters/secondary",
			"src/lib/sse.ts": "adapters/secondary",
			"src/lib/auth-popup.ts": "adapters/secondary",
			"src/lib/api-client-copilot.ts": "adapters/secondary",
			"src/lib/api-client.ts": "adapters/secondary",
			"src/lib": "usecases",
			"src/hooks": "usecases",
			"src/components": "adapters/primary",
			"src/routes": "adapters/primary",
			"src/router.tsx": "composition-root",
			"src/main.tsx": "composition-root",
			"src/composition.ts": "composition-root"
		},
		"exclude": [
			"*src/test",
			"*src/vite-env.d.ts",
			"*src/routeTree.gen.ts"
		],
		"adapters": {
			"ui": ["src/components", "src/routes"],
			"api": ["src/lib/http.ts", "src/lib/sse.ts", "src/lib/api-client-copilot.ts", "src/lib/api-client.ts"],
			"auth-popup": ["src/lib/auth-popup.ts"]
		}
	}
}
"#,
    );
    write(
        r,
        "src/components/ai-workbench-page.tsx",
        "// ui -> api through the \"@/\" alias.\nimport { ApiError } from \"@/lib/http\";\nexport const isApiError = (e: unknown) => e instanceof ApiError;\n",
    );
    write(
        r,
        "src/components/ask-deep-link-card.tsx",
        "// ui -> api, type-only, through the \"@/\" alias.\nimport type { CopilotStep } from \"@/lib/api-client-copilot\";\nexport const stepId = (s: CopilotStep) => s.id;\n",
    );
    write(
        r,
        "src/components/relative-control.tsx",
        "// The same ui -> api edge written relative.\nimport { ApiError } from \"../lib/http\";\nexport const isApiError = (e: unknown) => e instanceof ApiError;\n",
    );
    write(
        r,
        "src/hooks/lab-designer-utils.ts",
        "// usecases -> adapters/secondary through the \"@/\" alias.\nimport { listCopilotSteps } from \"@/lib/api-client\";\nexport const steps = () => listCopilotSteps();\n",
    );
    write(
        r,
        "src/hooks/relative-control.ts",
        "// The same edge written relative.\nimport { listCopilotSteps } from \"../lib/api-client\";\nexport const steps = () => listCopilotSteps();\n",
    );
    write(
        r,
        "src/lib/api-client-copilot.ts",
        "import { apiFetch } from \"./http\";\nexport type CopilotStep = { id: string };\nexport const listCopilotSteps = () => apiFetch<CopilotStep[]>(\"/api/copilot/steps\");\n",
    );
    write(r, "src/lib/api-client.ts", "export * from \"./api-client-copilot\";\n");
    write(
        r,
        "src/lib/http.ts",
        "export class ApiError extends Error {}\nexport async function apiFetch<T>(path: string): Promise<T> {\n\tconst r = await fetch(path);\n\treturn (await r.json()) as T;\n}\n",
    );
    let (v, all) = analyze(r);
    let edges = violation_edges(&v);
    assert_eq!(edges.len(), 5, "5 violations, not the 2 relative ones:\n{all}");
    let aliased: Vec<_> = edges
        .iter()
        .filter(|e| e["import_path"].as_str().unwrap_or_default().starts_with("@/"))
        .collect();
    assert_eq!(aliased.len(), 3, "the 3 alias violations:\n{all}");
    let targets: BTreeSet<(String, String)> = aliased
        .iter()
        .map(|e| (e["from_file"].as_str().unwrap_or_default().to_string(), to_file(e)))
        .collect();
    let want: BTreeSet<(String, String)> = [
        ("src/components/ai-workbench-page.tsx", "src/lib/http.ts"),
        ("src/components/ask-deep-link-card.tsx", "src/lib/api-client-copilot.ts"),
        ("src/hooks/lab-designer-utils.ts", "src/lib/api-client.ts"),
    ]
    .into_iter()
    .map(|(a, b)| (a.to_string(), b.to_string()))
    .collect();
    assert_eq!(targets, want, "{all}");
}

/// S08: A relative path in baseUrl is resolved against the declaring tsconfig, not against the importing file.
#[test]
fn s08_a_target_is_resolved_against_the_declaring_tsconfig_not_the_importer() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "tsconfig.json", r#"{"compilerOptions": {"paths": {"@/*": ["./src/*"]}}}"#);
    layers(r, r#"{"src/app": "domain", "src/lib": "adapters/secondary", "src/app/src/lib": "adapters/secondary"}"#);
    write(r, "src/lib/http.ts", GET);
    write(r, "src/app/src/lib/http.ts", GET);
    write(r, "src/app/a.ts", IMPORT_HTTP);
    let (v, all) = analyze(r);
    let e = the_edge(&v, "src/app/a.ts", "@/lib/http", &all);
    assert_eq!(to_file(&e), "src/lib/http.ts", "{all}");
    no_edge_to(&v, "src/app/src/lib/http.ts", &all);
}

/// S09: A wildcard matches at most once, and a pattern's prefix must match literally.
///
/// A control file imports `@/lib/http`, which does match, so the test cannot
/// pass on a build that resolves no alias at all.
#[test]
fn s09_a_pattern_prefix_must_match_literally() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "tsconfig.json", r#"{"compilerOptions": {"paths": {"@/*": ["./src/*"]}}}"#);
    layers(r, r#"{"src/app": "domain", "src/lib": "adapters/secondary"}"#);
    write(r, "src/lib/http.ts", GET);
    write(r, "src/app/a.ts", "import { get } from \"@lib/http\";\nexport const a = () => get();\n");
    write(r, "src/app/b.ts", IMPORT_HTTP);
    let (v, all) = analyze(r);
    assert!(
        edges_for(&v, "src/app/a.ts", "@lib/http").is_empty(),
        "\"@lib/http\" does not match \"@/*\" and must make no edge to src/lib/http.ts:\n{all}"
    );
    let bad: Vec<_> = violation_edges(&v)
        .into_iter()
        .filter(|e| e["from_file"] == "src/app/a.ts")
        .collect();
    assert!(bad.is_empty(), "{bad:?}\n{all}");
    let control = the_edge(&v, "src/app/b.ts", "@/lib/http", &all);
    assert_eq!(to_file(&control), "src/lib/http.ts", "the matching control:\n{all}");
}

/// S10: A pattern's target that names no file is skipped, and is never taken as the edge's target.
#[test]
fn s10_a_target_that_names_no_file_is_skipped() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "tsconfig.json", r#"{"compilerOptions": {"paths": {"@/*": ["./missing/*", "./src/*"]}}}"#);
    layers(r, r#"{"src/app": "domain", "src/lib": "adapters/secondary", "missing": "adapters/secondary"}"#);
    write(r, "src/lib/http.ts", GET);
    write(r, "src/app/a.ts", IMPORT_HTTP);
    let (v, all) = analyze(r);
    let e = the_edge(&v, "src/app/a.ts", "@/lib/http", &all);
    assert_eq!(to_file(&e), "src/lib/http.ts", "{all}");
    no_edge_to(&v, "missing/", &all);
}

/// One case of the S11 property: a tsconfig, a layer map, the target file T,
/// the importing file F, an alias specifier S and a relative specifier R, both
/// naming T from F.
struct Case {
    tsconfig: &'static str,
    layers: Option<&'static str>,
    target: &'static str,
    importer: &'static str,
    alias: &'static str,
    relative: &'static str,
}

fn project_importing(c: &Case, spec: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "tsconfig.json", c.tsconfig);
    if let Some(l) = c.layers {
        layers(r, l);
    }
    write(r, c.target, "export const t = 1;\n");
    write(r, c.importer, &format!("// importer\nimport {{ t }} from \"{spec}\";\nexport const f = () => t;\n"));
    d
}

/// The edges from `from`, without import_path, sorted.
fn edges_without_path(v: &Value, from: &str) -> Vec<String> {
    let mut out: Vec<String> = violation_edges(v)
        .into_iter()
        .filter(|e| e["from_file"] == from)
        .map(|mut e| {
            e.as_object_mut().unwrap().remove("import_path");
            e.to_string()
        })
        .collect();
    out.sort();
    out
}

/// S11: An alias import and the relative import of the same file make the same edge.
#[test]
fn s11_an_alias_import_and_the_relative_import_make_the_same_edge() {
    let cases = [
        Case {
            tsconfig: r#"{"compilerOptions": {"paths": {"@/*": ["./src/*"]}}}"#,
            layers: Some(r#"{"src/app": "domain", "src/lib": "adapters/secondary"}"#),
            target: "src/lib/http.ts",
            importer: "src/app/a.ts",
            alias: "@/lib/http",
            relative: "../lib/http",
        },
        Case {
            tsconfig: r#"{"compilerOptions": {"paths": {"~/*": ["./src/*"]}}}"#,
            layers: None,
            target: "src/adapters/secondary/repo.ts",
            importer: "src/domain/order.ts",
            alias: "~/adapters/secondary/repo",
            relative: "../adapters/secondary/repo",
        },
        Case {
            tsconfig: r##"{"compilerOptions": {"paths": {"#core/*": ["./nothing/*", "./packages/core/src/*"]}}}"##,
            layers: Some(r#"{"apps/web": "domain", "packages/core": "adapters/secondary"}"#),
            target: "packages/core/src/model/index.ts",
            importer: "apps/web/main.ts",
            alias: "#core/model",
            relative: "../../packages/core/src/model",
        },
        Case {
            tsconfig: r#"{"compilerOptions": {"paths": {"db": ["./src/adapters/secondary/db.ts"]}}}"#,
            layers: None,
            target: "src/adapters/secondary/db.ts",
            importer: "src/domain/order.ts",
            alias: "db",
            relative: "../adapters/secondary/db",
        },
        Case {
            tsconfig: r#"{"compilerOptions": {"baseUrl": "./src", "paths": {"@app/*": ["*"]}}}"#,
            layers: None,
            target: "src/adapters/primary/cli.ts",
            importer: "src/domain/x.ts",
            alias: "@app/adapters/primary/cli",
            relative: "../adapters/primary/cli",
        },
        Case {
            tsconfig: r#"{"compilerOptions": {"paths": {"@/*": ["./src/*"]}}}"#,
            layers: None,
            target: "src/ports/repo.ts",
            importer: "src/usecases/place.ts",
            alias: "@/ports/repo",
            relative: "../ports/repo",
        },
    ];
    for c in &cases {
        let via_alias = project_importing(c, c.alias);
        let via_rel = project_importing(c, c.relative);
        let (va, all_a) = analyze(via_alias.path());
        let (vr, all_r) = analyze(via_rel.path());
        assert_eq!(
            edges_without_path(&va, c.importer),
            edges_without_path(&vr, c.importer),
            "{} via {:?} vs {:?}:\n--- alias\n{all_a}\n--- relative\n{all_r}",
            c.importer,
            c.alias,
            c.relative
        );
        assert_eq!(
            violation_edges(&va).len(),
            violation_edges(&vr).len(),
            "one is a violation exactly when the other is ({:?}):\n{all_a}\n{all_r}",
            c.alias
        );
        assert_eq!(edge_count(&va), edge_count(&vr), "{:?}:\n{all_a}\n{all_r}", c.alias);
    }
}

/// S12: paths and baseUrl are inherited through a relative extends, and baseUrl is relative to the file that declares it.
#[test]
fn s12_paths_and_base_url_are_inherited_through_a_relative_extends() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "tsconfig.json", r#"{"extends": "./config/tsconfig.base.json"}"#);
    write(r, "config/tsconfig.base.json", r#"{"compilerOptions": {"baseUrl": "..", "paths": {"@/*": ["src/*"]}}}"#);
    layers(r, r#"{"src/app": "domain", "src/lib": "adapters/secondary"}"#);
    write(r, "src/lib/http.ts", GET);
    write(r, "src/app/a.ts", IMPORT_HTTP);
    let (v, all) = analyze(r);
    let e = the_edge(&v, "src/app/a.ts", "@/lib/http", &all);
    assert_eq!(to_file(&e), "src/lib/http.ts", "{all}");
}

/// S13: Settings in the extending file win over the base.
#[test]
fn s13_settings_in_the_extending_file_win() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "config/tsconfig.base.json", r#"{"compilerOptions": {"paths": {"@/*": ["./legacy/*"]}}}"#);
    write(
        r,
        "tsconfig.json",
        r#"{"extends": "./config/tsconfig.base.json", "compilerOptions": {"paths": {"@/*": ["./src/*"]}}}"#,
    );
    layers(r, r#"{"src/app": "domain", "src/lib": "adapters/secondary", "legacy": "adapters/secondary", "config": "adapters/secondary"}"#);
    write(r, "legacy/lib/http.ts", GET);
    write(r, "config/legacy/lib/http.ts", GET);
    write(r, "src/lib/http.ts", GET);
    write(r, "src/app/a.ts", IMPORT_HTTP);
    let (v, all) = analyze(r);
    let e = the_edge(&v, "src/app/a.ts", "@/lib/http", &all);
    assert_eq!(to_file(&e), "src/lib/http.ts", "{all}");
    no_edge_to(&v, "legacy/", &all);
    no_edge_to(&v, "config/", &all);
}

/// S14: extends is followed through more than one level.
#[test]
fn s14_extends_is_followed_through_more_than_one_level() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "tsconfig.json", r#"{"extends": "./tsconfig.app.json"}"#);
    write(r, "tsconfig.app.json", r#"{"extends": "./tsconfig.base.json", "compilerOptions": {"strict": true}}"#);
    write(r, "tsconfig.base.json", r#"{"compilerOptions": {"paths": {"@/*": ["./src/*"]}}}"#);
    layers(r, r#"{"src/app": "domain", "src/lib": "adapters/secondary"}"#);
    write(r, "src/lib/http.ts", GET);
    write(r, "src/app/a.ts", IMPORT_HTTP);
    let (v, all) = analyze(r);
    let e = the_edge(&v, "src/app/a.ts", "@/lib/http", &all);
    assert_eq!(to_file(&e), "src/lib/http.ts", "{all}");
}

/// S15: An extends that names a package is not followed.
#[test]
fn s15_an_extends_that_names_a_package_is_not_followed() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "tsconfig.json", r#"{"extends": "@tsconfig/strictest/tsconfig.json"}"#);
    write(
        r,
        "node_modules/@tsconfig/strictest/tsconfig.json",
        r#"{"compilerOptions": {"paths": {"@/*": ["./src/*"]}}}"#,
    );
    layers(r, r#"{"src/app": "domain", "src/lib": "adapters/secondary"}"#);
    write(r, "src/lib/http.ts", GET);
    write(r, "src/app/a.ts", IMPORT_HTTP);
    let (v, all) = analyze(r);
    assert!(edges_for(&v, "src/app/a.ts", "@/lib/http").is_empty(), "the package extends must not be followed:\n{all}");
    no_edge_to(&v, "src/lib/http.ts", &all);
}

/// S16: An inherited baseUrl is not rebased onto the extending file's directory.
#[test]
fn s16_an_inherited_base_url_is_not_rebased() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "tsconfig.json", r#"{"extends": "./config/base.json"}"#);
    write(r, "config/base.json", r#"{"compilerOptions": {"baseUrl": "./web", "paths": {"@/*": ["src/*"]}}}"#);
    layers(r, r#"{"src/app": "domain", "config/web/src/lib": "adapters/secondary", "web/src/lib": "adapters/secondary"}"#);
    write(r, "config/web/src/lib/http.ts", GET);
    write(r, "web/src/lib/http.ts", GET);
    write(r, "src/app/a.ts", IMPORT_HTTP);
    let (v, all) = analyze(r);
    let e = the_edge(&v, "src/app/a.ts", "@/lib/http", &all);
    assert_eq!(to_file(&e), "config/web/src/lib/http.ts", "{all}");
    no_edge_to(&v, "web/", &all);
}

/// The S17 sources: several alias imports, a relative one, a package one.
fn s17_sources(r: &Path) {
    layers(r, r#"{"src/app": "domain", "src/lib": "adapters/secondary", "src/core": "domain"}"#);
    write(r, "src/lib/http.ts", GET);
    write(r, "src/lib/store/index.ts", "export const s = 1;\n");
    write(r, "src/core/money.ts", "export const m = 1;\n");
    write(
        r,
        "src/app/a.ts",
        "import { get } from \"@/lib/http\";\nimport { s } from \"@/lib/store\";\nimport { m } from \"@/core/money\";\nimport { get as g } from \"../lib/http\";\nimport React from \"react\";\nexport const a = () => get() + s + m + g() + (React ? 0 : 1);\n",
    );
    write(r, "src/app/b.ts", "import { m } from \"@/core/money\";\nexport const b = () => m;\n");
}

fn edge_set(v: &Value) -> BTreeSet<String> {
    violation_edges(v)
        .into_iter()
        .map(|e| {
            format!(
                "{} -> {} [{} -> {}] line {}",
                e["from_file"], e["to_file"], e["from_layer"], e["to_layer"], e["line"]
            )
        })
        .collect()
}

/// S17: Moving compilerOptions into a relatively extended file, with paths rewritten to the same locations, changes no edge.
#[test]
fn s17_moving_compiler_options_into_an_extended_file_changes_no_edge() {
    let variants: [(&str, &str, &str); 2] = [
        (
            r#"{"compilerOptions": {"baseUrl": "./src", "paths": {"@/*": ["*"]}}}"#,
            "config/base.json",
            r#"{"compilerOptions": {"baseUrl": "../src", "paths": {"@/*": ["*"]}}}"#,
        ),
        (
            r#"{"compilerOptions": {"baseUrl": ".", "paths": {"@/*": ["src/*"]}}}"#,
            "tsconfig.base.json",
            r#"{"compilerOptions": {"baseUrl": ".", "paths": {"@/*": ["src/*"]}}}"#,
        ),
    ];
    for (direct, base_rel, base) in variants {
        let p1 = tempfile::tempdir().unwrap();
        write(p1.path(), "tsconfig.json", direct);
        s17_sources(p1.path());
        let p2 = tempfile::tempdir().unwrap();
        write(p2.path(), "tsconfig.json", &format!("{{\"extends\": \"./{base_rel}\"}}"));
        write(p2.path(), base_rel, base);
        s17_sources(p2.path());
        let (v1, all1) = analyze(p1.path());
        let (v2, all2) = analyze(p2.path());
        // The comparison means something only when P1 resolves its aliases.
        let e = the_edge(&v1, "src/app/a.ts", "@/lib/http", &all1);
        assert_eq!(to_file(&e), "src/lib/http.ts", "P1 resolves the alias:\n{all1}");
        assert_eq!(edge_set(&v1), edge_set(&v2), "same edges, {base_rel}:\n--- P1\n{all1}\n--- P2\n{all2}");
        assert_eq!(edge_count(&v1), edge_count(&v2), "{all1}\n{all2}");
        assert_eq!(v1["analysis"]["violations"], v2["analysis"]["violations"], "same violations:\n{all1}\n{all2}");
    }
}

/// S18: A package import stays external.
#[test]
fn s18_a_package_import_stays_external() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "tsconfig.json", r#"{"compilerOptions": {"paths": {"@/*": ["./src/*"]}}}"#);
    layers(r, r#"{"src/app": "domain", "src/lib": "adapters/secondary", "src/tanstack": "adapters/secondary", "src/react": "adapters/secondary"}"#);
    write(r, "src/lib/http.ts", GET);
    write(r, "src/react/index.ts", "export const r = 1;\n");
    write(r, "src/tanstack/react-query.ts", "export const q = 1;\n");
    write(
        r,
        "src/app/a.ts",
        "import React from \"react\";\nimport { useQuery } from \"@tanstack/react-query\";\nexport const a = () => [React, useQuery];\n",
    );
    let (v, all) = analyze(r);
    for spec in ["react", "@tanstack/react-query"] {
        assert!(edges_for(&v, "src/app/a.ts", spec).is_empty(), "{spec} must produce no violation:\n{all}");
    }
    let from_a: Vec<_> = violation_edges(&v).into_iter().filter(|e| e["from_file"] == "src/app/a.ts").collect();
    assert!(from_a.is_empty(), "no violation from a package import: {from_a:?}\n{all}");
    let (file_out, file_err) = hexa(r, &["analyze", "--file", "src/app/a.ts"]);
    assert!(!file_out.contains("VIOLATION"), "{file_out}{file_err}");
}

/// The S19 project: no paths, relative and package imports, one layer crossing.
fn s19_sources(r: &Path) {
    write(r, "src/adapters/secondary/http.ts", GET);
    write(r, "src/ports/repo.ts", "export interface Repo { get(): number }\n");
    write(r, "src/lib/http.ts", GET);
    write(
        r,
        "src/domain/order.ts",
        "import { get } from \"../adapters/secondary/http\";\nimport { z } from \"zod\";\nimport { g2 } from \"@/lib/http\";\nexport const o = () => get() + (z ? 0 : 1) + (g2 ? 0 : 1);\n",
    );
    write(
        r,
        "src/usecases/place.ts",
        "import type { Repo } from \"../ports/repo\";\nimport React from \"react\";\nexport const p = (r: Repo) => r.get() + (React ? 0 : 1);\n",
    );
}

/// S19: A project with no paths grades exactly as before.
///
/// "Before" is not runnable from a black box, so the reference is the same
/// project with no tsconfig.json at all, plus the expected edges named here.
#[test]
fn s19_a_project_with_no_paths_grades_as_before() {
    let with = tempfile::tempdir().unwrap();
    write(
        with.path(),
        "tsconfig.json",
        r#"{"compilerOptions": {"strict": true, "module": "ESNext", "moduleResolution": "Bundler"}, "include": ["src"]}"#,
    );
    s19_sources(with.path());
    let without = tempfile::tempdir().unwrap();
    s19_sources(without.path());
    let (v1, all1) = analyze(with.path());
    let (v0, all0) = analyze(without.path());
    assert_eq!(v1["analysis"]["violations"], v0["analysis"]["violations"], "{all1}\n{all0}");
    assert_eq!(edge_count(&v1), edge_count(&v0), "{all1}\n{all0}");
    assert_eq!(v1["analysis"]["health_score"], v0["analysis"]["health_score"], "{all1}\n{all0}");
    // The relative crossing is reported, and nothing else: with no paths,
    // "@/lib/http" is a package name and names no project file.
    let e = the_edge(&v1, "src/domain/order.ts", "../adapters/secondary/http", &all1);
    assert_eq!(to_file(&e), "src/adapters/secondary/http.ts", "{all1}");
    assert_eq!(violation_edges(&v1).len(), 1, "{all1}");
    no_edge_to(&v1, "src/lib/", &all1);
}

/// S20: A matched alias whose targets name no project file is not invented into an edge.
///
/// A control file imports `@/lib/http`, which does exist, so the test cannot
/// pass on a build that resolves no alias at all.
#[test]
fn s20_an_alias_whose_targets_name_no_file_makes_no_edge() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "tsconfig.json", r#"{"compilerOptions": {"paths": {"@/*": ["./src/*"]}}}"#);
    layers(r, r#"{"src/domain": "domain", "src/lib": "adapters/secondary"}"#);
    write(r, "src/lib/http.ts", GET);
    write(r, "src/domain/a.ts", "import { ghost } from \"@/lib/ghost\";\nexport const a = () => ghost;\n");
    write(r, "src/domain/b.ts", IMPORT_HTTP);
    let (v, all) = analyze(r);
    assert!(edges_for(&v, "src/domain/a.ts", "@/lib/ghost").is_empty(), "no violation for the ghost import:\n{all}");
    no_edge_to(&v, "src/lib/ghost", &all);
    let control = the_edge(&v, "src/domain/b.ts", "@/lib/http", &all);
    assert_eq!(to_file(&control), "src/lib/http.ts", "the existing control:\n{all}");
}

/// `hexa graph consumers <target> --json` → the files that import it.
fn imported_by(root: &Path, target: &str) -> BTreeSet<String> {
    let (stdout, stderr) = hexa(root, &["graph", "consumers", target, "--json"]);
    let v: Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("graph consumers --json is not JSON ({e}):\n{stdout}{stderr}"));
    v["imported_by"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(str::to_string))
        .collect()
}

/// S25: The graph and analyze agree on every TypeScript import edge.
///
/// Every import in the fixture crosses from the domain into a secondary
/// adapter, so every analyze edge is a reported violation and the whole edge
/// set is visible; edge_count confirms none is hidden.
#[test]
fn s25_the_graph_and_analyze_agree_on_every_import_edge() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "tsconfig.json", r#"{"extends": "./config/base.json"}"#);
    write(
        r,
        "config/base.json",
        r##"{"compilerOptions": {"baseUrl": "..", "paths": {"@/*": ["src/*"], "~shared/*": ["missing/*", "shared/*"], "#legacy/*": ["legacy/*"]}}}"##,
    );
    layers(
        r,
        r#"{"src/domain": "domain", "src/lib": "adapters/secondary", "shared": "adapters/secondary", "legacy": "adapters/secondary"}"#,
    );
    write(r, "src/lib/http.ts", GET);
    write(r, "src/lib/store/index.ts", "export const s = 1;\n");
    write(r, "shared/money.ts", "export const m = 1;\n");
    write(r, "legacy/old.ts", "export const o = 1;\n");
    write(
        r,
        "src/domain/a.ts",
        "import { get } from \"@/lib/http\";\nimport { s } from \"@/lib/store\";\nexport const a = () => get() + s;\n",
    );
    write(r, "src/domain/b/c.ts", "import { get } from \"../../lib/http\";\nexport const c = () => get();\n");
    write(
        r,
        "src/domain/d.ts",
        "import { m } from \"~shared/money\";\nimport { o } from \"#legacy/old\";\nexport const d = () => m + o;\n",
    );
    let (v, all) = analyze(r);
    let edges = violation_edges(&v);
    assert_eq!(edges.len() as u64, edge_count(&v), "every edge must be visible as a violation:\n{all}");
    let (bo, be) = hexa(r, &["graph", "build"]);
    assert!(r.join(".hexa/graph/graph.json").exists(), "graph build failed:\n{bo}{be}");

    let targets = ["src/lib/http.ts", "src/lib/store/index.ts", "shared/money.ts", "legacy/old.ts"];
    let want: [(&str, &[&str]); 4] = [
        ("src/lib/http.ts", &["src/domain/a.ts", "src/domain/b/c.ts"]),
        ("src/lib/store/index.ts", &["src/domain/a.ts"]),
        ("shared/money.ts", &["src/domain/d.ts"]),
        ("legacy/old.ts", &["src/domain/d.ts"]),
    ];
    for t in targets {
        let from_analyze: BTreeSet<String> = edges
            .iter()
            .filter(|e| e["to_file"] == t)
            .filter_map(|e| e["from_file"].as_str().map(str::to_string))
            .collect();
        let from_graph = imported_by(r, t);
        assert_eq!(from_graph, from_analyze, "consumers of {t} vs analyze edges to it:\n{all}");
    }
    for (t, files) in want {
        let expected: BTreeSet<String> = files.iter().map(|s| s.to_string()).collect();
        assert_eq!(imported_by(r, t), expected, "consumers of {t}");
    }
}
