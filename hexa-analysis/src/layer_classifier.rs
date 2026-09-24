//! Layer Classifier — pure functions for hexagonal architecture layer classification.
//!
//! Encodes the allowed dependency direction rules as lookup tables,
//! keeping rule logic testable independently of the analyzer.
//!
//! Ported from `src/core/usecases/layer-classifier.ts`.

use super::domain::HexLayer;

// ── Directory-Based Patterns ─────────────────────────────
//
// Ordered most-specific first. The first match wins.

const LAYER_PATTERNS: &[(&str, HexLayer)] = &[
    // Go conventional directories (more specific first)
    ("/internal/domain/", HexLayer::Domain),
    ("/internal/ports/", HexLayer::Ports),
    ("/internal/usecases/", HexLayer::Usecases),
    ("/internal/", HexLayer::Usecases),            // Go: internal/ catch-all → private business logic
    ("/cmd/", HexLayer::AdaptersPrimary),           // Go: cmd/ is the CLI/HTTP entry point
    ("/pkg/", HexLayer::Ports),                     // Go: pkg/ is the public API
    // Rust conventional directories
    ("/src/bin/", HexLayer::AdaptersPrimary),       // Rust: binary entry points
    ("/src/routes/", HexLayer::AdaptersPrimary),    // Rust: web route handlers
    ("/src/commands/", HexLayer::AdaptersPrimary),  // Rust: CLI subcommand handlers
    ("/src/handlers/", HexLayer::AdaptersPrimary),  // Rust/Go: HTTP handler modules
    ("/src/middleware/", HexLayer::AdaptersPrimary), // Rust/Go: HTTP middleware
    // Go naming conventions
    ("/handlers/", HexLayer::AdaptersPrimary),
    // Hex-standard patterns (generic, checked last)
    ("/domain/", HexLayer::Domain),
    ("/ports/", HexLayer::Ports),
    ("/usecases/", HexLayer::Usecases),
    ("/orchestration/", HexLayer::Usecases),
    ("/adapters/primary/", HexLayer::AdaptersPrimary),
    ("/adapters/secondary/", HexLayer::AdaptersSecondary),
    // A flat `adapters/` directory, checked after the two specific ones.
    // Without it such a file is Unknown, the boundary checker skips the edge,
    // and the grade scores zero for a violation the same command just printed
    // (ADR-2609122048). Primary is the permissive of the two roles, so a flat
    // driving adapter that calls a use case is not flagged as a false
    // positive; the cost is that a flat driven adapter doing the same is
    // missed. Rules 1, 4 and 5 hold either way.
    ("/adapters/", HexLayer::AdaptersPrimary),
    ("/infrastructure/", HexLayer::Infrastructure),
];

// ── Filename-Based Patterns ──────────────────────────────
//
// Checked after directory patterns fail. Returns a special role or a hexa layer.

/// Result of filename pattern matching — either a hexa layer or a special file role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FilenameMatch {
    Layer(HexLayer),
    CompositionRoot,
    EntryPoint,
    Infrastructure,
    BuildConfig,
}

fn match_filename(path: &str) -> Option<FilenameMatch> {
    let basename = path.rsplit('/').next().unwrap_or(path);

    // Rust special files
    if basename == "lib.rs" {
        return Some(FilenameMatch::CompositionRoot);
    }
    if basename == "main.rs" || basename == "main.go" || basename == "main.ts" {
        return Some(FilenameMatch::EntryPoint);
    }
    if basename == "embed.rs" || basename == "daemon.rs" {
        return Some(FilenameMatch::Infrastructure);
    }
    if basename == "build.rs" || basename == "Cargo.toml" {
        return Some(FilenameMatch::BuildConfig);
    }

    // Go special files
    if basename.starts_with("composition-root") {
        return Some(FilenameMatch::CompositionRoot);
    }
    if basename.ends_with("_adapter.go") {
        return Some(FilenameMatch::Layer(HexLayer::AdaptersPrimary));
    }
    if basename.ends_with("_service.go") {
        return Some(FilenameMatch::Layer(HexLayer::Usecases));
    }
    if basename.starts_with("handler_") && basename.ends_with(".go") {
        return Some(FilenameMatch::Layer(HexLayer::AdaptersPrimary));
    }

    None
}

// ── Public API ───────────────────────────────────────────

/// Classify a project-relative file path into a hexagonal architecture layer.
///
/// Returns `HexLayer::Unknown` for files that don't match any pattern
/// (test files, config files, build scripts, etc.).
pub fn classify_layer(file_path: &str) -> HexLayer {
    // Prefix with / so patterns like /cmd/ match paths starting with cmd/
    let normalized = format!("/{}", file_path);

    // Skip Go test files — they mirror the package they test, not a distinct layer
    if normalized.ends_with("_test.go") {
        return HexLayer::Unknown;
    }

    // Every pattern ends in `/`, because it names a directory. A path that *is*
    // a directory — which is what a Go package import or a Rust module path
    // resolves to — has no trailing slash of its own, so `/internal/ports`
    // could never match `/internal/ports/`. It fell through to the `/internal/`
    // catch-all and was graded a use case, which reported
    // "adapters must not import from usecases" against a correctly layered Go
    // project. Trying both spellings costs one allocation and closes that.
    let as_dir = format!("{normalized}/");

    // Check directory-based patterns first, most specific first.
    for &(pattern, layer) in LAYER_PATTERNS {
        if normalized.contains(pattern) || as_dir.contains(pattern) {
            return layer;
        }
    }

    // A crate or package named for its layer — `okf-domain`, `my_app_ports`,
    // `app-usecases` — is that layer whatever its folders are called. This
    // is the crate-per-layer workspace, where the build tool itself refuses
    // an undeclared import; it was recognised only by a Rust-only display
    // scan with its own rules, and the grade never saw it. Checked before
    // file names, so the crate's `lib.rs` is the crate's layer.
    if let Some(layer) = layer_named_package(&normalized) {
        return layer;
    }

    // Check filename-based patterns
    if let Some(m) = match_filename(&normalized) {
        return match m {
            FilenameMatch::Layer(layer) => layer,
            // composition-root and entry-point are recognized but not hexa layers
            FilenameMatch::CompositionRoot => HexLayer::CompositionRoot,
            FilenameMatch::EntryPoint => HexLayer::EntryPoint,
            FilenameMatch::Infrastructure => HexLayer::Infrastructure,
            FilenameMatch::BuildConfig => HexLayer::Unknown,
        };
    }

    HexLayer::Unknown
}

/// The layer a directory segment is named for: its last `-`/`_`-separated
/// part, when there is a separator. The file name itself is not a package.
fn layer_named_package(normalized: &str) -> Option<HexLayer> {
    let dirs = normalized.rsplit_once('/').map_or("", |(d, _)| d);
    dirs.split('/').find_map(|seg| {
        let (_, last) = seg.rsplit_once(['-', '_'])?;
        match last {
            "domain" => Some(HexLayer::Domain),
            "ports" | "port" => Some(HexLayer::Ports),
            "usecases" | "usecase" | "orchestration" => Some(HexLayer::Usecases),
            _ => None,
        }
    })
}

// ── Project-declared layers ──────────────────────────────

/// Layers a project declares for paths the built-in patterns cannot read,
/// from `.hexa/project.json`:
///
/// ```json
/// { "analyze": { "layers": { "hexa-cli/src/commands": "adapters/primary" } } }
/// ```
///
/// A key is a project-relative path prefix, matched on whole path segments;
/// the longest matching key wins, and a path no key matches falls back to
/// [`classify_layer`]. Layout is the project's business — code organised by
/// crate or by concern is as hexagonal as code organised by folder name —
/// so, as with `analyze.exclude`, the names live in the project's config and
/// not in the analyzer. Without this, an unrecognised file was Unknown and
/// every edge touching it went unchecked.
#[derive(Debug, Clone, Default)]
pub struct LayerMap {
    /// (prefix, layer), longest prefix first.
    entries: Vec<(String, HexLayer)>,
}

impl LayerMap {
    /// Read `analyze.layers` from `<root>/.hexa/project.json`. No file, or no
    /// `layers` key, is an empty map. A layer name that is not one of the
    /// hexagon's is an error that names it: a declaration dropped silently
    /// would leave the grade claiming to cover code it does not check.
    pub fn from_project(root: &std::path::Path) -> Result<Self, String> {
        let Ok(text) = std::fs::read_to_string(root.join(".hexa").join("project.json")) else {
            return Ok(Self::default());
        };
        let v: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| format!(".hexa/project.json is not JSON: {e}"))?;
        let Some(layers) = v.get("analyze").and_then(|a| a.get("layers")) else {
            return Ok(Self::default());
        };
        let obj = layers
            .as_object()
            .ok_or("analyze.layers must be an object of path prefix → layer")?;
        let mut entries = Vec::with_capacity(obj.len());
        for (prefix, name) in obj {
            let name = name.as_str().unwrap_or("");
            let layer = parse_layer(name).ok_or_else(|| {
                format!(
                    "analyze.layers[\"{prefix}\"] = \"{name}\" is not a layer; use one of: {}",
                    LAYER_NAMES.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
                )
            })?;
            entries.push((prefix.trim_matches('/').to_string(), layer));
        }
        entries.sort_by_key(|(p, _)| std::cmp::Reverse(p.len()));
        Ok(Self { entries })
    }

    /// The declared layer for `path`, else the built-in classification.
    ///
    /// A key naming a source file also names its module: an import of
    /// `crate::store::Disk` resolves to `src/store/Disk`, not `src/store.rs`,
    /// and a declaration that missed it left the import unchecked.
    pub fn classify(&self, path: &str) -> HexLayer {
        let under = |p: &str, key: &str| {
            p == key || p.strip_prefix(key).is_some_and(|rest| rest.starts_with('/'))
        };
        for (prefix, layer) in &self.entries {
            let module = [".rs", ".ts", ".tsx", ".go"]
                .iter()
                .find_map(|ext| prefix.strip_suffix(ext));
            if under(path, prefix) || module.is_some_and(|m| under(path, m)) {
                return *layer;
            }
        }
        classify_layer(path)
    }
}

/// Layer names as `HexLayer`'s `Display` writes them, so what a project
/// declares is what every report prints.
const LAYER_NAMES: &[(&str, HexLayer)] = &[
    ("domain", HexLayer::Domain),
    ("ports", HexLayer::Ports),
    ("usecases", HexLayer::Usecases),
    ("adapters/primary", HexLayer::AdaptersPrimary),
    ("adapters/secondary", HexLayer::AdaptersSecondary),
    ("infrastructure", HexLayer::Infrastructure),
    ("composition-root", HexLayer::CompositionRoot),
    ("entry-point", HexLayer::EntryPoint),
];

fn parse_layer(name: &str) -> Option<HexLayer> {
    LAYER_NAMES.iter().find(|(n, _)| *n == name).map(|(_, l)| *l)
}

/// Check whether an import from `from_layer` to `to_layer` is allowed.
///
/// Same-layer imports are always allowed. Cross-layer imports follow
/// the hexagonal dependency direction rules.
fn is_allowed_import(from_layer: HexLayer, to_layer: HexLayer) -> bool {
    if from_layer == to_layer {
        return true;
    }
    allowed_targets(from_layer).contains(&to_layer)
}

/// Return the set of layers that `layer` is allowed to import from.
fn allowed_targets(layer: HexLayer) -> &'static [HexLayer] {
    match layer {
        HexLayer::Domain => &[],
        HexLayer::Ports => &[HexLayer::Domain],
        HexLayer::Usecases => &[HexLayer::Domain, HexLayer::Ports],
        // Primary adapters may drive use cases. This is the only table: a
        // second copy in hexa_core::rules::boundary was kept "in step" by hand
        // until it was deleted, because two tables encoding one rule answer
        // differently depending on which command you happened to run.
        HexLayer::AdaptersPrimary => &[HexLayer::Ports, HexLayer::Usecases],
        // Secondary adapters are NOT granted it: a driven adapter calling back
        // into usecases inverts the dependency.
        HexLayer::AdaptersSecondary => &[HexLayer::Ports],
        HexLayer::Infrastructure => &[HexLayer::Ports],
        // Special files have no restrictions checked
        HexLayer::CompositionRoot
        | HexLayer::EntryPoint
        | HexLayer::Unknown => &[],
    }
}

/// Get a human-readable violation rule description, or `None` if the import is allowed.
pub fn get_violation_rule(from_layer: HexLayer, to_layer: HexLayer) -> Option<&'static str> {
    if is_allowed_import(from_layer, to_layer) {
        return None;
    }
    // Special files (composition-root, entry-point, unknown) are never violations
    if matches!(
        from_layer,
        HexLayer::CompositionRoot | HexLayer::EntryPoint | HexLayer::Unknown
    ) {
        return None;
    }
    if matches!(
        to_layer,
        HexLayer::CompositionRoot | HexLayer::EntryPoint | HexLayer::Unknown
    ) {
        return None;
    }

    Some(match (from_layer, to_layer) {
        // domain → anything
        (HexLayer::Domain, HexLayer::Ports) => "domain must not import from ports (use domain/value-objects)",
        (HexLayer::Domain, _) => "domain must not import from outside domain",

        // ports → deeper layers
        (HexLayer::Ports, HexLayer::Usecases) => "ports must not import from usecases",
        (HexLayer::Ports, HexLayer::AdaptersPrimary | HexLayer::AdaptersSecondary) => "ports must not import from adapters",
        (HexLayer::Ports, _) => "ports must not import from infrastructure",

        // usecases → adapters or infra
        (HexLayer::Usecases, HexLayer::AdaptersPrimary | HexLayer::AdaptersSecondary) => "usecases may only import from domain and ports",
        (HexLayer::Usecases, _) => "usecases may only import from domain and ports",

        // adapters → wrong direction
        (HexLayer::AdaptersPrimary, HexLayer::Domain) => "adapters must not import from domain directly",
        // No (AdaptersPrimary, Usecases) arm: driving a use case is allowed, so
        // `is_allowed_import` returns before reaching this match. An arm here
        // would be unreachable and would state the opposite of the rule.
        (HexLayer::AdaptersPrimary, HexLayer::AdaptersSecondary) => "adapters must not import from other adapters",
        (HexLayer::AdaptersPrimary, _) => "adapters must not import from infrastructure",

        (HexLayer::AdaptersSecondary, HexLayer::Domain) => "adapters must not import from domain directly",
        (HexLayer::AdaptersSecondary, HexLayer::Usecases) => "adapters must not import from usecases",
        (HexLayer::AdaptersSecondary, HexLayer::AdaptersPrimary) => "adapters must not import from other adapters",
        (HexLayer::AdaptersSecondary, _) => "adapters must not import from infrastructure",

        // infrastructure → wrong direction
        (HexLayer::Infrastructure, HexLayer::Domain) => "infrastructure may import from ports only",
        (HexLayer::Infrastructure, _) => "infrastructure may import from ports only",

        _ => "unexpected layer combination",
    })
}

// ── Tests ────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_standard_hex_directories() {
        assert_eq!(classify_layer("src/domain/value_objects.rs"), HexLayer::Domain);
        assert_eq!(classify_layer("src/ports/state.rs"), HexLayer::Ports);
        assert_eq!(classify_layer("src/usecases/conversation.rs"), HexLayer::Usecases);
        assert_eq!(classify_layer("src/adapters/primary/cli.rs"), HexLayer::AdaptersPrimary);
        assert_eq!(classify_layer("src/adapters/driver.rs"), HexLayer::AdaptersPrimary);
        assert_eq!(classify_layer("src/adapters/secondary/db.rs"), HexLayer::AdaptersSecondary);
        assert_eq!(classify_layer("src/infrastructure/config.rs"), HexLayer::Infrastructure);
    }

    #[test]
    fn classify_go_conventions() {
        assert_eq!(classify_layer("internal/domain/entity.go"), HexLayer::Domain);
        assert_eq!(classify_layer("cmd/server/main.go"), HexLayer::AdaptersPrimary);
        assert_eq!(classify_layer("pkg/api/types.go"), HexLayer::Ports);
        assert_eq!(classify_layer("internal/service.go"), HexLayer::Usecases);
    }

    #[test]
    fn classify_rust_conventions() {
        assert_eq!(classify_layer("src/bin/hexa-nexus.rs"), HexLayer::AdaptersPrimary);
        assert_eq!(classify_layer("src/routes/swarms.rs"), HexLayer::AdaptersPrimary);
    }

    #[test]
    fn classify_special_files() {
        assert_eq!(classify_layer("src/lib.rs"), HexLayer::CompositionRoot);
        assert_eq!(classify_layer("src/main.rs"), HexLayer::EntryPoint);
    }

    #[test]
    fn skip_go_test_files() {
        assert_eq!(classify_layer("internal/domain/entity_test.go"), HexLayer::Unknown);
    }

    #[test]
    fn allowed_import_same_layer() {
        assert!(is_allowed_import(HexLayer::Domain, HexLayer::Domain));
        assert!(is_allowed_import(HexLayer::Usecases, HexLayer::Usecases));
    }

    #[test]
    fn allowed_import_correct_direction() {
        assert!(is_allowed_import(HexLayer::Ports, HexLayer::Domain));
        assert!(is_allowed_import(HexLayer::Usecases, HexLayer::Ports));
        assert!(is_allowed_import(HexLayer::Usecases, HexLayer::Domain));
        assert!(is_allowed_import(HexLayer::AdaptersPrimary, HexLayer::Ports));
        assert!(is_allowed_import(HexLayer::AdaptersSecondary, HexLayer::Ports));
    }

    /// A driving adapter may invoke the application layer; a driven one may
    /// not.
    #[test]
    fn a_crate_named_for_its_layer_is_that_layer() {
        assert_eq!(classify_layer("okf-domain/src/order.rs"), HexLayer::Domain);
        assert_eq!(classify_layer("okf-domain/src/lib.rs"), HexLayer::Domain);
        assert_eq!(classify_layer("my_app_ports/src/store.rs"), HexLayer::Ports);
        assert_eq!(classify_layer("crates/app-usecases/src/run.rs"), HexLayer::Usecases);
        assert_eq!(classify_layer("okf-port/src/x.rs"), HexLayer::Ports);
        // The segment must *end* in the layer name, after a separator.
        assert_eq!(classify_layer("domainless/src/x.rs"), HexLayer::Unknown);
        assert_eq!(classify_layer("hexa-core/src/x.rs"), HexLayer::Unknown);
        // A file name is not a crate.
        assert_eq!(classify_layer("src/my-domain.rs"), HexLayer::Unknown);
    }

    #[test]
    fn commands_and_orchestration_are_read() {
        assert_eq!(classify_layer("hexa-cli/src/commands/analyze.rs"), HexLayer::AdaptersPrimary);
        assert_eq!(classify_layer("src/orchestration/agent_manager.rs"), HexLayer::Usecases);
    }

    #[test]
    fn primary_adapters_may_drive_usecases_but_secondary_may_not() {
        assert!(is_allowed_import(
            HexLayer::AdaptersPrimary,
            HexLayer::Usecases
        ));
        assert!(get_violation_rule(HexLayer::AdaptersPrimary, HexLayer::Usecases).is_none());

        assert!(!is_allowed_import(
            HexLayer::AdaptersSecondary,
            HexLayer::Usecases
        ));
        assert!(get_violation_rule(HexLayer::AdaptersSecondary, HexLayer::Usecases).is_some());
    }

    /// The allowance must not widen adapter-to-adapter coupling.
    #[test]
    fn adapters_still_cannot_import_each_other() {
        assert!(get_violation_rule(
            HexLayer::AdaptersPrimary,
            HexLayer::AdaptersSecondary
        )
        .is_some());
    }

    #[test]
    fn forbidden_imports() {
        assert!(!is_allowed_import(HexLayer::Domain, HexLayer::Ports));
        assert!(!is_allowed_import(HexLayer::Ports, HexLayer::Usecases));
        assert!(!is_allowed_import(HexLayer::AdaptersPrimary, HexLayer::Domain));
        assert!(!is_allowed_import(HexLayer::AdaptersPrimary, HexLayer::AdaptersSecondary));
        assert!(!is_allowed_import(HexLayer::AdaptersSecondary, HexLayer::AdaptersPrimary));
    }

    #[test]
    fn violation_rules_present() {
        assert!(get_violation_rule(HexLayer::Domain, HexLayer::Ports).is_some());
        assert!(get_violation_rule(HexLayer::AdaptersPrimary, HexLayer::AdaptersSecondary).is_some());
    }

    #[test]
    fn no_violation_for_allowed() {
        assert!(get_violation_rule(HexLayer::Ports, HexLayer::Domain).is_none());
        assert!(get_violation_rule(HexLayer::Usecases, HexLayer::Ports).is_none());
    }

    #[test]
    fn no_violation_for_special_files() {
        assert!(get_violation_rule(HexLayer::CompositionRoot, HexLayer::Domain).is_none());
        assert!(get_violation_rule(HexLayer::EntryPoint, HexLayer::AdaptersPrimary).is_none());
    }

    /// A Go package import resolves to `internal/ports`, with no trailing
    /// slash and no file. Before this was handled it matched the `/internal/`
    /// catch-all and was graded a use case — so a secondary adapter importing
    /// its own port was reported as importing a use case. Found by requiring
    /// hexa's own Go scaffold to grade clean.
    #[test]
    fn a_package_directory_classifies_as_its_layer() {
        assert_eq!(classify_layer("internal/ports"), HexLayer::Ports);
        assert_eq!(classify_layer("internal/domain"), HexLayer::Domain);
        assert_eq!(classify_layer("internal/usecases"), HexLayer::Usecases);
        assert_eq!(classify_layer("adapters/secondary"), HexLayer::AdaptersSecondary);
        assert_eq!(classify_layer("src/core/ports"), HexLayer::Ports);
    }

    /// The catch-all still catches what it should.
    #[test]
    fn an_unrecognised_internal_package_is_still_a_use_case() {
        assert_eq!(classify_layer("internal/scheduler"), HexLayer::Usecases);
    }

    /// Files are unaffected — they matched before and must still match.
    #[test]
    fn a_file_in_a_layer_directory_is_unchanged() {
        assert_eq!(classify_layer("internal/ports/store.go"), HexLayer::Ports);
        assert_eq!(classify_layer("src/core/domain/count.ts"), HexLayer::Domain);
        assert_eq!(
            classify_layer("adapters/secondary/memory.go"),
            HexLayer::AdaptersSecondary
        );
    }

}
