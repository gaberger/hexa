//! Boundary Checker — validates hexagonal dependency direction rules.
//!
//! Given a set of import edges, classifies each endpoint into a hexa layer
//! and checks whether the import direction is allowed.
//!
//! ADR-034 Phase 3.

use super::domain::{DependencyViolation, HexLayer, ImportEdge};
use super::layer_classifier::get_violation_rule;

/// Find all hexagonal boundary violations in a set of import edges.
///
/// Skips edges where either endpoint is `Unknown` — these are files
/// outside the hexa layer structure (tests, config, build scripts).
pub fn find_violations(edges: &[ImportEdge]) -> Vec<DependencyViolation> {
    let mut violations = Vec::new();

    for edge in edges {
        if edge.from_layer == HexLayer::Unknown || edge.to_layer == HexLayer::Unknown {
            continue;
        }

        if declares_own_submodule(&edge.from_file, &edge.to_file) {
            continue;
        }

        if let Some(rule) = get_violation_rule(edge.from_layer, edge.to_layer) {
            violations.push(DependencyViolation {
                edge: edge.clone(),
                rule: rule.to_string(),
            });
        }
    }

    violations
}

/// Is this a module declaring something nested inside itself?
///
/// `adapters/mod.rs` containing `pub mod secondary;` produces an edge to
/// `adapters/secondary`. That is structural nesting, not one adapter reaching
/// for another, and reading it as a violation flags the shipped scaffold
/// (ADR-2609122048). A rule that flags correct code is worse than no rule.
fn declares_own_submodule(from_file: &str, to_file: &str) -> bool {
    let dir = match from_file.rfind('/') {
        Some(i) => &from_file[..i],
        None => return false,
    };
    if dir.is_empty() {
        return false;
    }
    to_file.strip_prefix(dir).is_some_and(|rest| rest.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer_classifier::classify_layer;

    #[test]
    fn a_parent_module_declaring_its_child_is_not_a_violation() {
        let e = edge("src/adapters/mod.rs", "src/adapters/secondary");
        assert!(find_violations(&[e]).is_empty());
    }

    #[test]
    fn one_adapter_reaching_for_another_is_still_a_violation() {
        let e = edge("src/adapters/primary/cli.rs", "src/adapters/secondary/db.rs");
        assert_eq!(find_violations(&[e]).len(), 1);
    }

    #[test]
    fn the_domain_reaching_outward_is_still_a_violation() {
        let e = edge("src/domain/mod.rs", "src/adapters/secondary/db.rs");
        assert_eq!(find_violations(&[e]).len(), 1);
    }

    fn edge(from: &str, to: &str) -> ImportEdge {
        ImportEdge {
            from_file: from.to_string(),
            to_file: to.to_string(),
            from_layer: classify_layer(from),
            to_layer: classify_layer(to),
            import_path: to.to_string(),
            line: 1,
        }
    }

    #[test]
    fn allowed_imports_produce_no_violations() {
        let edges = vec![
            edge("src/ports/state.rs", "src/domain/types.rs"),
            edge("src/usecases/analyze.rs", "src/ports/state.rs"),
            edge("src/adapters/primary/cli.rs", "src/ports/state.rs"),
            edge("src/adapters/secondary/db.rs", "src/ports/state.rs"),
        ];
        assert!(find_violations(&edges).is_empty());
    }

    #[test]
    fn domain_importing_ports_is_violation() {
        let edges = vec![edge("src/domain/entity.rs", "src/ports/state.rs")];
        let v = find_violations(&edges);
        assert_eq!(v.len(), 1);
        assert!(v[0].rule.contains("domain must not import from ports"));
    }

    #[test]
    fn adapter_importing_other_adapter_is_violation() {
        let edges = vec![edge(
            "src/adapters/primary/cli.rs",
            "src/adapters/secondary/db.rs",
        )];
        let v = find_violations(&edges);
        assert_eq!(v.len(), 1);
        assert!(v[0].rule.contains("adapters must not import from other adapters"));
    }

    #[test]
    fn unknown_layers_are_skipped() {
        let edges = vec![edge("Cargo.toml", "src/domain/entity.rs")];
        assert!(find_violations(&edges).is_empty());
    }

    #[test]
    fn usecases_importing_adapter_is_violation() {
        let edges = vec![edge(
            "src/usecases/analyze.rs",
            "src/adapters/secondary/db.rs",
        )];
        let v = find_violations(&edges);
        assert_eq!(v.len(), 1);
    }
}
