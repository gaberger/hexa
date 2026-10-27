//! Integration tests for ADR-2610271649: adapter identity and the
//! within-layer cross-adapter rule.

use hexa_analysis::domain::{
    crosses_adapters, AdapterMap, ArchAnalysisResult, DependencyViolation, HexLayer, ImportEdge,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

const RULE: &str = "adapters must not import from other adapters";

const ALL_LAYERS: [HexLayer; 9] = [
    HexLayer::Domain,
    HexLayer::Ports,
    HexLayer::Usecases,
    HexLayer::AdaptersPrimary,
    HexLayer::AdaptersSecondary,
    HexLayer::Infrastructure,
    HexLayer::CompositionRoot,
    HexLayer::EntryPoint,
    HexLayer::Unknown,
];

// ── helpers: building maps and edges ─────────────────────

fn is_adapter_layer(l: HexLayer) -> bool {
    matches!(l, HexLayer::AdaptersPrimary | HexLayer::AdaptersSecondary)
}

fn secondary(_: &str) -> HexLayer {
    HexLayer::AdaptersSecondary
}

fn primary(_: &str) -> HexLayer {
    HexLayer::AdaptersPrimary
}

fn build(
    keys: &[&str],
    groups: Vec<(&str, Vec<&str>)>,
    layer_of: &dyn Fn(&str) -> HexLayer,
) -> Result<AdapterMap, String> {
    AdapterMap::new(
        keys.iter().map(|k| k.to_string()).collect(),
        groups
            .into_iter()
            .map(|(n, ps)| (n.to_string(), ps.into_iter().map(|p| p.to_string()).collect()))
            .collect(),
        layer_of,
    )
}

fn edge(from: &str, from_layer: HexLayer, to: &str, to_layer: HexLayer) -> ImportEdge {
    ImportEdge {
        from_file: from.to_string(),
        to_file: to.to_string(),
        from_layer,
        to_layer,
        import_path: String::new(),
        line: 1,
    }
}

fn swapped(e: &ImportEdge) -> ImportEdge {
    ImportEdge {
        from_file: e.to_file.clone(),
        to_file: e.from_file.clone(),
        from_layer: e.to_layer,
        to_layer: e.from_layer,
        import_path: e.import_path.clone(),
        line: e.line,
    }
}

// ── helpers: fixture projects ────────────────────────────

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn fresh_dir(name: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "hexa-adapters-{}-{}-{}",
        std::process::id(),
        name,
        n
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create fixture dir");
    dir
}

fn write(dir: &Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(p, content).expect("write fixture file");
}

/// A Rust crate fixture. Every `src/**.rs` file is declared by its parent
/// module, so the module tree is complete.
fn rust_project(name: &str, files: &[(&str, &str)], project_json: Option<&str>) -> PathBuf {
    let dir = fresh_dir(name);
    let mut contents: BTreeMap<String, String> = files
        .iter()
        .map(|(p, c)| (p.to_string(), c.to_string()))
        .collect();
    let mut decls: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for path in contents.keys() {
        let Some(rel) = path.strip_prefix("src/") else { continue };
        if !rel.ends_with(".rs") {
            continue;
        }
        let parts: Vec<&str> = rel.split('/').collect();
        for i in 0..parts.len() {
            let comp = parts[i];
            let last = i == parts.len() - 1;
            let module = if last {
                if comp == "mod.rs" || comp == "lib.rs" || comp == "main.rs" {
                    continue;
                }
                comp.trim_end_matches(".rs")
            } else {
                comp
            };
            let parent = if i == 0 {
                "src/lib.rs".to_string()
            } else {
                format!("src/{}/mod.rs", parts[..i].join("/"))
            };
            decls.entry(parent).or_default().insert(module.to_string());
        }
    }
    for (file, mods) in decls {
        let header: String = mods.iter().map(|m| format!("pub mod {m};\n")).collect();
        let existing = contents.remove(&file).unwrap_or_default();
        contents.insert(file, format!("{header}{existing}"));
    }
    write(
        &dir,
        "Cargo.toml",
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\npath = \"src/lib.rs\"\n",
    );
    for (p, c) in &contents {
        write(&dir, p, c);
    }
    if let Some(json) = project_json {
        write(&dir, ".hexa/project.json", json);
    }
    dir
}

// ── helpers: running hexa analyze ────────────────────────

struct Analysis {
    result: ArchAnalysisResult,
    raw: String,
}

fn find_result(v: &serde_json::Value) -> Option<ArchAnalysisResult> {
    match v {
        serde_json::Value::Object(map) => {
            if map.contains_key("violations") && map.contains_key("health_score") {
                if let Ok(r) = serde_json::from_value::<ArchAnalysisResult>(v.clone()) {
                    return Some(r);
                }
            }
            map.values().find_map(find_result)
        }
        serde_json::Value::Array(items) => items.iter().find_map(find_result),
        _ => None,
    }
}

fn parse_result(s: &str) -> Option<ArchAnalysisResult> {
    for (i, _) in s.match_indices('{').take(50) {
        let mut it = serde_json::Deserializer::from_str(&s[i..]).into_iter::<serde_json::Value>();
        if let Some(Ok(v)) = it.next() {
            if let Some(r) = find_result(&v) {
                return Some(r);
            }
        }
    }
    None
}

fn analyze(dir: &Path) -> Analysis {
    let variants: [&[&str]; 4] = [
        &["analyze", ".", "--json"],
        &["analyze", "--json", "."],
        &["analyze", ".", "--format", "json"],
        &["analyze", ".", "--format=json"],
    ];
    let mut tried = String::new();
    for args in variants {
        let out = Command::new(env!("CARGO_BIN_EXE_hexa"))
            .args(args)
            .current_dir(dir)
            .output()
            .expect("run hexa");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        if let Some(result) = parse_result(&stdout) {
            return Analysis {
                result,
                raw: format!("{stdout}\n{stderr}"),
            };
        }
        tried.push_str(&format!(
            "--- hexa {args:?}\nstdout:\n{stdout}\nstderr:\n{stderr}\n"
        ));
    }
    panic!("hexa analyze produced no analysis result in {}:\n{tried}", dir.display());
}

fn cross_violations(r: &ArchAnalysisResult) -> Vec<&DependencyViolation> {
    r.violations.iter().filter(|v| v.rule == RULE).collect()
}

// ── helpers: hexa's own repository ───────────────────────

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn skip_dir(name: &str) -> bool {
    matches!(name, "target" | ".git" | "node_modules")
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if !skip_dir(&name) {
                walk(root, &path, out);
            }
        } else if ft.is_file() {
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            out.push(rel);
        }
    }
}

fn repo_files() -> &'static Vec<String> {
    static FILES: OnceLock<Vec<String>> = OnceLock::new();
    FILES.get_or_init(|| {
        let root = repo_root();
        let mut out = Vec::new();
        walk(&root, &root, &mut out);
        out.sort();
        out
    })
}

fn rs_files_under(prefix: &str) -> Vec<String> {
    let p = format!("{}/", prefix.trim_end_matches('/'));
    repo_files()
        .iter()
        .filter(|f| f.starts_with(&p) && f.ends_with(".rs"))
        .cloned()
        .collect()
}

fn files_matching(pred: impl Fn(&str) -> bool) -> Vec<String> {
    repo_files()
        .iter()
        .filter(|f| f.ends_with(".rs") && pred(f))
        .cloned()
        .collect()
}

/// Files that are the module `name`: `…/name.rs` or `…/name/**.rs`.
fn module_files(crate_prefix: &str, name: &str) -> Vec<String> {
    let file = format!("/{name}.rs");
    let dir = format!("/{name}/");
    files_matching(|f| f.starts_with(crate_prefix) && (f.ends_with(&file) || f.contains(&dir)))
}

fn hexa_project_json() -> serde_json::Value {
    let p = repo_root().join(".hexa/project.json");
    let text = fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
    serde_json::from_str(&text).expect("hexa's project.json parses")
}

fn hexa_layer_keys(v: &serde_json::Value) -> Vec<String> {
    v.get("analyze")
        .and_then(|a| a.get("layers"))
        .and_then(|l| l.as_object())
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default()
}

fn hexa_groups(v: &serde_json::Value) -> Vec<(String, Vec<String>)> {
    v.get("analyze")
        .and_then(|a| a.get("adapters"))
        .and_then(|l| l.as_object())
        .map(|o| {
            o.iter()
                .map(|(name, paths)| {
                    let ps = paths
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|p| p.as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default();
                    (name.clone(), ps)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn hexa_map() -> AdapterMap {
    let v = hexa_project_json();
    AdapterMap::new(hexa_layer_keys(&v), hexa_groups(&v), &secondary)
        .expect("hexa's adapters configuration is valid")
}

fn word_contains(line: &str, word: &str) -> bool {
    let bytes = line.as_bytes();
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    line.match_indices(word).any(|(i, _)| {
        let before = i == 0 || !is_ident(bytes[i - 1]);
        let end = i + word.len();
        let after = end >= bytes.len() || !is_ident(bytes[end]);
        before && after
    })
}

fn is_use_line(t: &str) -> bool {
    t.starts_with("use ") || t.starts_with("pub use ") || t.starts_with("pub(crate) use ")
}

/// Does `text` import the crate-local module `m` (by `crate::`/`super::` path
/// or in a `use` line)?
fn imports_local_module(text: &str, m: &str) -> bool {
    text.lines().any(|line| {
        let t = line.trim();
        if t.starts_with("//") {
            return false;
        }
        t.contains(&format!("crate::{m}"))
            || t.contains(&format!("super::{m}"))
            || (is_use_line(t) && t.contains("crate::") && word_contains(t, m))
    })
}

fn copy_tree(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).expect("create copy dir");
    for entry in fs::read_dir(src).expect("read dir").flatten() {
        let name = entry.file_name();
        let Ok(ft) = entry.file_type() else { continue };
        let from = entry.path();
        let to = dst.join(&name);
        if ft.is_dir() {
            if !skip_dir(&name.to_string_lossy()) {
                copy_tree(&from, &to);
            }
        } else if ft.is_file() {
            fs::copy(&from, &to).expect("copy file");
        }
    }
}

// ── sample data for the properties ───────────────────────

const SAMPLE_PATHS: [&str; 10] = [
    "src/adapters/secondary/sqlite/repo.rs",
    "src/adapters/secondary/http/client.rs",
    "src/adapters/primary/web/routes.rs",
    "hexa-cli/src/fmt.rs",
    "hexa-cli/src/commands/analyze.rs",
    "crates/x/src/adapters/store/disk.rs",
    "crates/x/src/adapters/mail.rs",
    "src/store.rs",
    "tools/local_store.rs",
    "",
];

fn sample_maps() -> Vec<AdapterMap> {
    vec![
        AdapterMap::default(),
        build(&[], vec![], &secondary).unwrap(),
        build(
            &["crates/x/src/adapters", "crates/x/src/adapters/store", "src/store.rs"],
            vec![("cli", vec!["hexa-cli/src/commands", "hexa-cli/src/fmt.rs"])],
            &primary,
        )
        .unwrap(),
    ]
}

// ── §1 adapter_of ────────────────────────────────────────

/// S01: A file listed by an analyze.adapters group belongs to that group's adapter.
#[test]
fn s01_a_grouped_file_belongs_to_its_group() {
    let map = build(
        &[],
        vec![("cli", vec!["hexa-cli/src/commands", "hexa-cli/src/fmt.rs"])],
        &primary,
    )
    .expect("valid groups");
    assert_eq!(
        map.adapter_of("hexa-cli/src/fmt.rs", HexLayer::AdaptersPrimary),
        Some("cli".to_string())
    );
    assert_eq!(
        map.adapter_of("hexa-cli/src/commands/analyze.rs", HexLayer::AdaptersPrimary),
        Some("cli".to_string())
    );
}

/// S02: With no group, the longest analyze.layers key the file is under names its adapter.
#[test]
fn s02_longest_layer_key_names_the_adapter() {
    let map = build(
        &["crates/x/src/adapters", "crates/x/src/adapters/store"],
        vec![],
        &secondary,
    )
    .expect("valid map");
    let disk = map.adapter_of("crates/x/src/adapters/store/disk.rs", HexLayer::AdaptersSecondary);
    let mem = map.adapter_of("crates/x/src/adapters/store/mem.rs", HexLayer::AdaptersSecondary);
    let mail = map.adapter_of("crates/x/src/adapters/mail.rs", HexLayer::AdaptersSecondary);
    assert!(disk.is_some());
    assert_eq!(disk, mem, "the two store files share the store key's adapter");
    assert!(mail.is_some());
    assert_ne!(mail, disk, "mail is under the shorter key, not the store key");
}

/// S03: With no group and no layer key, the first path segment below adapters/secondary/ or adapters/primary/ names the adapter.
#[test]
fn s03_first_segment_below_adapters_folder_names_the_adapter() {
    let map = build(&[], vec![], &secondary).expect("valid map");
    let repo = map.adapter_of("src/adapters/secondary/sqlite/repo.rs", HexLayer::AdaptersSecondary);
    let schema =
        map.adapter_of("src/adapters/secondary/sqlite/schema.rs", HexLayer::AdaptersSecondary);
    let http = map.adapter_of("src/adapters/secondary/http/client.rs", HexLayer::AdaptersSecondary);
    let web = map.adapter_of("src/adapters/primary/web/routes.rs", HexLayer::AdaptersPrimary);
    assert!(repo.is_some());
    assert_eq!(repo, schema);
    assert!(http.is_some());
    assert!(web.is_some());
    assert_ne!(http, repo);
    assert_ne!(web, repo);
    assert_ne!(http, web);
}

/// S04: A file matched by no group, no layer key and no adapters/ folder segment is its own adapter.
#[test]
fn s04_an_unmatched_file_is_its_own_adapter() {
    let map = build(&[], vec![], &secondary).expect("valid map");
    let a = map.adapter_of("tools/local_store.rs", HexLayer::AdaptersSecondary);
    let b = map.adapter_of("tools/telegram_notifier.rs", HexLayer::AdaptersSecondary);
    assert!(a.is_some());
    assert!(b.is_some());
    assert_ne!(a, b);
}

/// S05: A layer key that names a source file also covers that file's module directory.
#[test]
fn s05_a_file_key_covers_its_module_directory() {
    let map = build(&["src/store.rs"], vec![], &secondary).expect("valid map");
    let file = map.adapter_of("src/store.rs", HexLayer::AdaptersSecondary);
    let child = map.adapter_of("src/store/disk.rs", HexLayer::AdaptersSecondary);
    assert!(file.is_some());
    assert_eq!(file, child);
}

/// S06: A file outside the adapter layers has no adapter, even when a group lists it.
#[test]
fn s06_non_adapter_layers_have_no_adapter() {
    let map = build(&[], vec![("cli", vec!["hexa-cli/src"])], &primary).expect("valid map");
    for layer in [
        HexLayer::Domain,
        HexLayer::Ports,
        HexLayer::Usecases,
        HexLayer::Infrastructure,
        HexLayer::CompositionRoot,
        HexLayer::EntryPoint,
        HexLayer::Unknown,
    ] {
        assert_eq!(
            map.adapter_of("hexa-cli/src/domain.rs", layer),
            None,
            "layer {layer} must have no adapter"
        );
    }
}

/// S07: A key covers whole path segments only. A sibling whose name merely begins with the key's text is not under the key.
#[test]
fn s07_keys_cover_whole_segments_only() {
    let map = build(&["src/store"], vec![], &secondary).expect("valid map");
    let disk = map.adapter_of("src/store/disk.rs", HexLayer::AdaptersSecondary);
    let front = map.adapter_of("src/storefront.rs", HexLayer::AdaptersSecondary);
    assert_ne!(disk, front);
}

/// S08: A group takes precedence over a layer key that also covers the file, so the file does not get the key's adapter.
#[test]
fn s08_a_group_beats_a_layer_key() {
    let map = build(
        &["svc/adapters"],
        vec![("mailer", vec!["svc/adapters/mail.rs"])],
        &secondary,
    )
    .expect("valid map");
    assert_eq!(
        map.adapter_of("svc/adapters/mail.rs", HexLayer::AdaptersSecondary),
        Some("mailer".to_string())
    );
    assert_ne!(
        map.adapter_of("svc/adapters/db.rs", HexLayer::AdaptersSecondary),
        Some("mailer".to_string())
    );
}

/// S09: Every file in an adapter layer has exactly one adapter, and the answer is deterministic. No file outside an adapter layer has an adapter.
#[test]
fn s09_adapter_of_is_total_on_adapter_layers_and_deterministic() {
    for map in sample_maps() {
        for path in SAMPLE_PATHS {
            for layer in ALL_LAYERS {
                let first = map.adapter_of(path, layer);
                let second = map.adapter_of(path, layer);
                assert_eq!(first, second, "non-deterministic for {path:?} {layer}");
                assert_eq!(
                    first.is_some(),
                    is_adapter_layer(layer),
                    "{path:?} at {layer} gave {first:?}"
                );
            }
        }
    }
}

// ── §2 crosses_adapters and the rule ─────────────────────

/// S10: An import between two different secondary adapters is reported as a violation.
#[test]
fn s10_import_between_two_secondary_adapters_is_a_violation() {
    let dir = rust_project(
        "s10",
        &[
            (
                "src/adapters/secondary/tools/run.rs",
                "use crate::adapters::secondary::local_store::Store;\n\npub fn run(s: &Store) -> usize {\n    s.len()\n}\n",
            ),
            (
                "src/adapters/secondary/local_store/mod.rs",
                "pub struct Store;\n\nimpl Store {\n    pub fn len(&self) -> usize {\n        0\n    }\n}\n",
            ),
        ],
        None,
    );
    let a = analyze(&dir);
    let hits: Vec<_> = cross_violations(&a.result)
        .into_iter()
        .filter(|v| {
            v.edge.from_file.ends_with("src/adapters/secondary/tools/run.rs")
                && v.edge.to_file.contains("src/adapters/secondary/local_store")
        })
        .collect();
    assert_eq!(hits.len(), 1, "violations: {:#?}", a.result.violations);

    let e = edge(
        "src/adapters/secondary/tools/run.rs",
        HexLayer::AdaptersSecondary,
        "src/adapters/secondary/local_store/mod.rs",
        HexLayer::AdaptersSecondary,
    );
    assert!(crosses_adapters(&e, &AdapterMap::default()));
}

/// S11: An import from a primary adapter to a different secondary adapter is still reported.
#[test]
fn s11_primary_to_other_secondary_adapter_is_reported() {
    let e = edge(
        "src/adapters/primary/web/h.rs",
        HexLayer::AdaptersPrimary,
        "src/adapters/secondary/db/repo.rs",
        HexLayer::AdaptersSecondary,
    );
    assert!(crosses_adapters(&e, &AdapterMap::default()));

    let dir = rust_project(
        "s11",
        &[
            (
                "src/adapters/primary/web/h.rs",
                "use crate::adapters::secondary::db::repo::Repo;\n\npub fn h(r: &Repo) -> usize {\n    r.count()\n}\n",
            ),
            (
                "src/adapters/secondary/db/repo.rs",
                "pub struct Repo;\n\nimpl Repo {\n    pub fn count(&self) -> usize {\n        0\n    }\n}\n",
            ),
        ],
        None,
    );
    let a = analyze(&dir);
    assert!(
        cross_violations(&a.result).iter().any(|v| {
            v.edge.from_file.ends_with("src/adapters/primary/web/h.rs")
                && v.edge.to_file.ends_with("src/adapters/secondary/db/repo.rs")
        }),
        "violations: {:#?}",
        a.result.violations
    );
}

/// S12: An import inside one adapter is not a violation.
#[test]
fn s12_an_import_inside_one_adapter_is_not_a_violation() {
    let e = edge(
        "src/adapters/secondary/sqlite/repo.rs",
        HexLayer::AdaptersSecondary,
        "src/adapters/secondary/sqlite/schema.rs",
        HexLayer::AdaptersSecondary,
    );
    assert!(!crosses_adapters(&e, &AdapterMap::default()));

    let dir = rust_project(
        "s12",
        &[
            (
                "src/adapters/secondary/sqlite/repo.rs",
                "use crate::adapters::secondary::sqlite::schema::Schema;\n\npub fn open(s: &Schema) -> usize {\n    s.version()\n}\n",
            ),
            (
                "src/adapters/secondary/sqlite/schema.rs",
                "pub struct Schema;\n\nimpl Schema {\n    pub fn version(&self) -> usize {\n        1\n    }\n}\n",
            ),
        ],
        None,
    );
    let a = analyze(&dir);
    assert!(
        !a.result.violations.iter().any(|v| {
            v.edge.from_file.ends_with("sqlite/repo.rs")
                && v.edge.to_file.ends_with("sqlite/schema.rs")
        }),
        "violations: {:#?}",
        a.result.violations
    );
}

/// S13: A module declaring its own submodule stays exempt.
#[test]
fn s13_a_module_declaring_its_submodule_is_exempt() {
    let dir = rust_project(
        "s13",
        &[
            (
                "src/adapters/secondary/tools/run.rs",
                "use crate::adapters::secondary::local_store::Store;\n\npub fn run(s: &Store) -> usize {\n    s.len()\n}\n",
            ),
            (
                "src/adapters/secondary/local_store/mod.rs",
                "pub struct Store;\n\nimpl Store {\n    pub fn len(&self) -> usize {\n        0\n    }\n}\n",
            ),
        ],
        None,
    );
    let a = analyze(&dir);
    let cross = cross_violations(&a.result);
    // Control: the rule is active in this project.
    assert!(
        cross.iter().any(|v| v.edge.from_file.ends_with("tools/run.rs")),
        "the cross-adapter rule is not active: {:#?}",
        a.result.violations
    );
    assert!(
        !cross.iter().any(|v| v.edge.from_file == "src/adapters/mod.rs"
            || (v.edge.from_file.ends_with("mod.rs")
                && v.edge.to_file.starts_with(
                    v.edge.from_file.trim_end_matches("mod.rs")
                )
                && v.edge.to_file != v.edge.from_file
                && !v.edge.from_file.ends_with("tools/run.rs"))),
        "a submodule declaration was reported: {cross:#?}"
    );
}

/// S14: An edge with an end outside the adapter layers does not cross adapters.
#[test]
fn s14_an_edge_leaving_the_adapter_layers_does_not_cross() {
    let map = AdapterMap::default();
    let a = edge(
        "src/adapters/secondary/db/repo.rs",
        HexLayer::AdaptersSecondary,
        "src/ports/store.rs",
        HexLayer::Ports,
    );
    let b = edge(
        "src/usecases/run.rs",
        HexLayer::Usecases,
        "src/ports/store.rs",
        HexLayer::Ports,
    );
    assert!(!crosses_adapters(&a, &map));
    assert!(!crosses_adapters(&b, &map));
}

/// S15: An edge crosses adapters exactly when both ends are adapters with different adapter_of values, and the rule is the same in either direction.
#[test]
fn s15_crosses_adapters_matches_adapter_of_and_is_symmetric() {
    for map in sample_maps() {
        for from in SAMPLE_PATHS {
            for to in SAMPLE_PATHS {
                for fl in ALL_LAYERS {
                    for tl in ALL_LAYERS {
                        let e = edge(from, fl, to, tl);
                        let expected = is_adapter_layer(fl)
                            && is_adapter_layer(tl)
                            && map.adapter_of(from, fl) != map.adapter_of(to, tl);
                        let got = crosses_adapters(&e, &map);
                        assert_eq!(got, expected, "{from:?}({fl}) -> {to:?}({tl})");
                        assert_eq!(
                            crosses_adapters(&swapped(&e), &map),
                            got,
                            "asymmetric for {from:?}({fl}) -> {to:?}({tl})"
                        );
                    }
                }
            }
        }
    }
}

// ── §3 analyze.adapters groups ───────────────────────────

/// S16: A declared group turns imports between its members into imports inside one adapter.
#[test]
fn s16_a_group_makes_member_imports_internal() {
    assert!(build(
        &[],
        vec![("cli", vec!["hexa-cli/src/commands", "hexa-cli/src/fmt.rs"])],
        &primary,
    )
    .is_ok());

    let files: [(&str, &str); 2] = [
        (
            "src/adapters/primary/cli/analyze.rs",
            "use crate::adapters::primary::fmt::render;\n\npub fn analyze() -> String {\n    render()\n}\n",
        ),
        (
            "src/adapters/primary/fmt.rs",
            "pub fn render() -> String {\n    String::new()\n}\n",
        ),
    ];
    let is_edge = |v: &&DependencyViolation| {
        v.edge.from_file.ends_with("src/adapters/primary/cli/analyze.rs")
            && v.edge.to_file.ends_with("src/adapters/primary/fmt.rs")
    };

    // Control: without the group, the two are different adapters.
    let plain = analyze(&rust_project("s16-plain", &files, None));
    assert!(
        cross_violations(&plain.result).iter().any(is_edge),
        "without a group the edge should cross adapters: {:#?}",
        plain.result.violations
    );

    let grouped = analyze(&rust_project(
        "s16-grouped",
        &files,
        Some(
            r#"{"analyze": {"adapters": {"cli": ["src/adapters/primary/cli", "src/adapters/primary/fmt.rs"]}}}"#,
        ),
    ));
    assert!(
        !cross_violations(&grouped.result).iter().any(is_edge),
        "violations: {:#?}",
        grouped.result.violations
    );
}

/// S17: A path listed in two groups is rejected with an error naming it.
#[test]
fn s17_a_path_in_two_groups_is_rejected() {
    let err = build(
        &[],
        vec![
            ("a", vec!["src/adapters/secondary/x.rs"]),
            ("b", vec!["src/adapters/secondary/x.rs"]),
        ],
        &secondary,
    )
    .expect_err("a path in two groups must be rejected");
    assert!(err.contains("src/adapters/secondary/x.rs"), "message: {err}");
}

/// S18: A group whose paths are in different layers is rejected with an error naming a path.
#[test]
fn s18_a_group_spanning_layers_is_rejected() {
    let layer_of = |p: &str| {
        if p.starts_with("src/web") {
            HexLayer::AdaptersPrimary
        } else if p.starts_with("src/db") {
            HexLayer::AdaptersSecondary
        } else {
            HexLayer::Unknown
        }
    };
    let err = build(&[], vec![("mixed", vec!["src/web", "src/db"])], &layer_of)
        .expect_err("a mixed-layer group must be rejected");
    assert!(
        err.contains("src/web") || err.contains("src/db"),
        "message: {err}"
    );
}

/// S19: hexa analyze does not ignore an invalid adapters configuration.
#[test]
fn s19_analyze_reports_an_invalid_adapters_configuration() {
    let files: [(&str, &str); 2] = [
        (
            "src/adapters/secondary/x.rs",
            "pub fn x() -> usize {\n    1\n}\n",
        ),
        (
            "src/adapters/secondary/y.rs",
            "pub fn y() -> usize {\n    2\n}\n",
        ),
    ];
    let path = "src/adapters/secondary/x.rs";
    let baseline = analyze(&rust_project("s19-baseline", &files, None));
    let invalid = analyze(&rust_project(
        "s19-invalid",
        &files,
        Some(&format!(
            r#"{{"analyze": {{"adapters": {{"a": ["{path}"], "b": ["{path}"]}}}}}}"#
        )),
    ));
    assert!(
        invalid.raw.matches(path).count() > baseline.raw.matches(path).count(),
        "the run does not report an error naming {path}:\n{}",
        invalid.raw
    );
    assert_eq!(
        invalid.result.health_score,
        baseline.result.health_score.saturating_sub(10),
        "the configuration error must cost a rule error's 10 points"
    );
}

/// S20: Any configuration that repeats a path across groups is rejected.
#[test]
fn s20_any_repeated_path_across_groups_is_rejected() {
    let pool = [
        "src/adapters/secondary/a.rs",
        "src/adapters/secondary/b",
        "src/adapters/primary/web",
        "lib/x/store.rs",
        "svc/mail",
        "tools/run.rs",
        "deep/nested/dir/file.rs",
        "pkg/q",
    ];
    let key_sets: [&[&str]; 3] = [&[], &["src/adapters"], &["svc", "lib/x"]];
    let n = pool.len();
    for t in 0..n * 3 {
        let repeated = pool[t % n];
        let g = 2 + t % 3;
        let mut groups: Vec<(String, Vec<String>)> =
            (0..g).map(|i| (format!("g{i}"), Vec::new())).collect();
        for (j, p) in pool.iter().filter(|p| **p != repeated).enumerate() {
            if (j + t) % 2 == 0 {
                groups[(j + t) % g].1.push(p.to_string());
            }
        }
        let first = t % g;
        let second = (first + 1) % g;
        groups[first].1.push(repeated.to_string());
        groups[second].1.insert(0, repeated.to_string());
        for keys in key_sets {
            let result = AdapterMap::new(
                keys.iter().map(|k| k.to_string()).collect(),
                groups.clone(),
                &secondary,
            );
            match result {
                Ok(_) => panic!("accepted {repeated:?} repeated in {groups:?}"),
                Err(msg) => assert!(
                    msg.contains(repeated),
                    "message {msg:?} does not name {repeated:?}"
                ),
            }
        }
    }
}

// ── §4 hexa's own configuration ──────────────────────────

/// S21: hexa's own configuration groups its split adapters.
#[test]
fn s21_hexas_configuration_groups_its_split_adapters() {
    let v = hexa_project_json();
    let names: BTreeSet<String> = hexa_groups(&v).into_iter().map(|(n, _)| n).collect();
    for want in ["cli", "treesitter", "inference-http", "git"] {
        assert!(names.contains(want), "no group {want:?} in {names:?}");
    }
    let map = hexa_map();

    let same = |label: &str, files: &[String], layer: HexLayer| {
        assert!(files.len() >= 2, "{label}: too few files found: {files:?}");
        let first = map.adapter_of(&files[0], layer);
        assert!(first.is_some(), "{label}: {} has no adapter", files[0]);
        for f in files {
            assert_eq!(
                map.adapter_of(f, layer),
                first,
                "{label}: {f} is not in {}'s adapter",
                files[0]
            );
        }
    };

    let mut cli = vec![
        "hexa-cli/src/fmt.rs".to_string(),
        "hexa-cli/src/assets.rs".to_string(),
    ];
    let commands = rs_files_under("hexa-cli/src/commands");
    assert!(!commands.is_empty(), "no commands files");
    cli.extend(commands);
    same("cli", &cli, HexLayer::AdaptersPrimary);

    let mut ts = files_matching(|f| f.ends_with("/treesitter_adapter.rs"));
    ts.extend(files_matching(|f| f.ends_with("/treesitter_api.rs")));
    same("treesitter", &ts, HexLayer::AdaptersSecondary);

    let mut infer = files_matching(|f| f.ends_with("/vec_stream.rs") || f.contains("/vec_stream/"));
    assert!(!infer.is_empty(), "no vec_stream file");
    let secondary_mods = rs_files_under("hexa-infer/src/adapters/secondary");
    assert!(!secondary_mods.is_empty(), "no hexa-infer secondary modules");
    infer.extend(secondary_mods);
    same("inference-http", &infer, HexLayer::AdaptersSecondary);

    let mut git = Vec::new();
    for (krate, file) in [
        ("hexa-git/", "/history.rs"),
        ("hexa-git/", "/worktree.rs"),
        ("hexa-exec/", "/git_history.rs"),
        ("hexa-exec/", "/git_worktrees.rs"),
    ] {
        let found = files_matching(|f| f.starts_with(krate) && f.ends_with(file));
        assert!(!found.is_empty(), "no {krate}…{file}");
        git.extend(found);
    }
    same("git", &git, HexLayer::AdaptersSecondary);
}

/// S22: hexa's groups do not hide the real couplings.
#[test]
fn s22_hexas_groups_do_not_hide_real_couplings() {
    let map = hexa_map();
    let layer = HexLayer::AdaptersSecondary;
    let tools = rs_files_under("hexa-exec/src/tools");
    assert!(!tools.is_empty(), "no hexa-exec tools files");
    let store = module_files("hexa-exec/", "local_store");
    let notifier = module_files("hexa-exec/", "telegram_notifier");
    assert!(!store.is_empty(), "no local_store");
    assert!(!notifier.is_empty(), "no telegram_notifier");

    let adapters = |files: &[String]| -> BTreeSet<Option<String>> {
        files.iter().map(|f| map.adapter_of(f, layer)).collect()
    };
    let tool_adapters = adapters(&tools);
    let store_adapters = adapters(&store);
    let notifier_adapters = adapters(&notifier);
    assert!(
        tool_adapters.is_disjoint(&store_adapters),
        "a tools file shares local_store's adapter"
    );
    assert!(
        tool_adapters.is_disjoint(&notifier_adapters),
        "a tools file shares telegram_notifier's adapter"
    );
    assert!(
        store_adapters.is_disjoint(&notifier_adapters),
        "local_store and telegram_notifier are the same adapter"
    );
}

// ── §5 the couplings removed ─────────────────────────────

/// S23: The hexa-exec tools reach the store and the notifier through ports that default_deps wires.
#[test]
fn s23_tools_reach_store_and_notifier_through_ports() {
    let root = repo_root();
    let tools = rs_files_under("hexa-exec/src/tools");
    assert!(!tools.is_empty(), "no hexa-exec tools files");
    for f in &tools {
        let text = fs::read_to_string(root.join(f)).expect("read tools file");
        for m in ["local_store", "telegram_notifier"] {
            assert!(
                !imports_local_module(&text, m),
                "{f} imports crate::{m}"
            );
        }
    }
    let defines_default_deps = rs_files_under("hexa-exec/src").iter().any(|f| {
        fs::read_to_string(root.join(f))
            .map(|t| t.contains("fn default_deps"))
            .unwrap_or(false)
    });
    assert!(defines_default_deps, "hexa_exec has no default_deps()");
}

/// S24: No direct adapter-to-adapter edge remains for the couplings named in the ADR.
#[test]
fn s24_no_named_adapter_to_adapter_edge_remains() {
    let root = repo_root();
    let read = |f: &str| fs::read_to_string(root.join(f)).expect("read source");

    for f in rs_files_under("hexa-exec/src/tools") {
        let text = read(&f);
        assert!(!imports_local_module(&text, "local_store"), "{f} -> local_store");
        assert!(
            !imports_local_module(&text, "telegram_notifier"),
            "{f} -> telegram_notifier"
        );
    }

    let frontier = files_matching(|f| f.ends_with("/frontier.rs"));
    assert!(!frontier.is_empty(), "no frontier.rs");
    for f in &frontier {
        let text = read(f);
        let hit = text.lines().any(|l| {
            let t = l.trim();
            !t.starts_with("//") && t.contains("hexa_infer") && word_contains(t, "spend")
                && !t.contains("ports")
        });
        assert!(!hit, "{f} imports hexa-infer's spend");
    }

    for m in ["discover", "local_provider", "spend"] {
        let files = module_files("hexa-infer/", m);
        assert!(!files.is_empty(), "no hexa-infer {m}");
        for f in &files {
            let text = read(f);
            for target in ["tiers", "registry"] {
                let hit = text.lines().any(|l| {
                    let t = l.trim();
                    !t.starts_with("//")
                        && !t.contains("ports")
                        && (t.contains(&format!("crate::{target}"))
                            || t.contains(&format!("super::{target}"))
                            || (is_use_line(t) && word_contains(t, target)))
                });
                assert!(!hit, "{f} imports {target}");
            }
        }
    }
}

// ── §6 the grade ─────────────────────────────────────────

/// S25: hexa grades itself A+ with zero boundary violations under the corrected rule.
#[test]
fn s25_hexa_grades_itself_a_plus_under_the_corrected_rule() {
    let map = hexa_map();
    let a = analyze(&repo_root());
    for v in &a.result.violations {
        assert!(
            !crosses_adapters(&v.edge, &map),
            "cross-adapter edge {} -> {}",
            v.edge.from_file,
            v.edge.to_file
        );
    }
    assert!(
        a.result.violations.is_empty(),
        "boundary violations: {:#?}",
        a.result.violations
    );
    assert!(
        a.result.health_score >= 95,
        "grade is not A+: {}",
        a.result.health_score
    );
}

/// S26: No coupling is excused, and a new cross-adapter import breaks the ratchet.
#[test]
fn s26_no_exception_and_a_new_cross_adapter_import_is_caught() {
    let v = hexa_project_json();
    let analyze_cfg = v.get("analyze").cloned().unwrap_or_default();
    if let Some(obj) = analyze_cfg.as_object() {
        for key in obj.keys() {
            let k = key.to_lowercase();
            assert!(
                !(k.contains("allow") || k.contains("exception") || k.contains("exempt")),
                "analyze has an exception entry {key:?}"
            );
        }
    }
    if let Some(groups) = analyze_cfg.get("adapters").and_then(|g| g.as_object()) {
        for (name, paths) in groups {
            assert!(paths.is_array(), "group {name:?} is not a plain path list");
        }
    }

    let root = repo_root();
    let original = analyze(&root);

    let copy = fresh_dir("s26-copy");
    copy_tree(&root, &copy);
    let mut tools = rs_files_under("hexa-exec/src/tools");
    tools.sort_by_key(|f| f.ends_with("mod.rs"));
    let target = tools.first().expect("a hexa-exec tools file").clone();
    let path = copy.join(&target);
    let mut text = fs::read_to_string(&path).expect("read tools file");
    text.push_str("\n#[allow(unused_imports)]\nuse crate::local_store;\n");
    fs::write(&path, text).expect("write tools file");

    let modified = analyze(&copy);
    let before = cross_violations(&original.result).len();
    let after: Vec<_> = cross_violations(&modified.result);
    assert_eq!(
        after.len(),
        before + 1,
        "violations: {:#?}",
        modified.result.violations
    );
    assert!(
        after.iter().any(|v| v.edge.from_file == target
            && v.edge.to_file.contains("local_store")),
        "the added import is not reported: {after:#?}"
    );
    assert_eq!(
        modified.result.health_score,
        original.result.health_score.saturating_sub(10),
        "the new violation must cost 10 points"
    );
    let _ = fs::remove_dir_all(&copy);
}
