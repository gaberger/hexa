//! Cycle Detector — DFS-based circular dependency detection.
//!
//! Builds an adjacency list from import edges and finds all cycles
//! using depth-first search with a recursion stack.
//!
//! ADR-034 Phase 3.

use std::collections::{BTreeMap, BTreeSet};

use super::domain::ImportEdge;

/// Detect all circular dependency clusters in the import graph.
///
/// Returns each cluster as a sorted vector of module keys. A cluster
/// `[A, B, C]` means every one of those modules can reach every other, so
/// the three cannot be built, tested or reasoned about apart.
///
/// This is the strongly connected components of the module graph, not a
/// walk that collects back-edges. The difference is not academic
/// (ADR-2609140020): the previous implementation started a DFS from
/// `HashMap::keys()` and walked neighbours out of a `HashSet`, both of
/// which Rust seeds randomly per process. On one fixture eight consecutive
/// runs over unchanged code reported 2, 3, 4 and 5 cycles, and since the
/// grade subtracts 15 points per cycle the same tree scored anywhere from
/// 0 to 19. A grade that is a random variable is not a gate.
///
/// Order is not the only thing that was wrong. A single DFS with one global
/// `visited` set finds back-edges on whichever spanning forest it happens to
/// build, so it cannot enumerate cycles at all — the count depended on entry
/// order by construction, not merely on the hash seed. Components are
/// well defined regardless of where the walk starts.
pub fn detect_cycles(edges: &[ImportEdge]) -> Vec<Vec<String>> {
    // Nodes are modules, keyed per language (see `module_key`). Self-edges
    // are intra-module reuse, not cycles.
    let keyed: Vec<(String, String)> = edges
        .iter()
        .map(|e| (module_key(&e.from_file), module_key(&e.to_file)))
        .filter(|(a, b)| a != b)
        .collect();
    // BTree, not Hash: iteration order is the input to the result, so it
    // has to come from the data rather than from a per-process seed.
    let mut graph: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (from, to) in &keyed {
        graph.entry(from.as_str()).or_default().insert(to.as_str());
        // A node with no outgoing edges still belongs to the graph, or a
        // component ending at it is never closed.
        graph.entry(to.as_str()).or_default();
    }

    let mut state = Tarjan {
        graph: &graph,
        index: BTreeMap::new(),
        low: BTreeMap::new(),
        on_stack: BTreeSet::new(),
        stack: Vec::new(),
        next: 0,
        components: Vec::new(),
    };
    for node in graph.keys() {
        if !state.index.contains_key(*node) {
            state.walk(node);
        }
    }

    let mut components = state.components;
    // A component of one node is a module that merely imports others.
    // Only a mutual reach is a cycle.
    components.retain(|c| c.len() > 1);
    for c in &mut components {
        c.sort();
    }
    components.sort();
    components
}

struct Tarjan<'a> {
    graph: &'a BTreeMap<&'a str, BTreeSet<&'a str>>,
    index: BTreeMap<&'a str, usize>,
    low: BTreeMap<&'a str, usize>,
    on_stack: BTreeSet<&'a str>,
    stack: Vec<&'a str>,
    next: usize,
    components: Vec<Vec<String>>,
}

impl<'a> Tarjan<'a> {
    fn walk(&mut self, node: &'a str) {
        self.index.insert(node, self.next);
        self.low.insert(node, self.next);
        self.next += 1;
        self.stack.push(node);
        self.on_stack.insert(node);

        if let Some(neighbors) = self.graph.get(node) {
            for &next in neighbors.iter() {
                if !self.index.contains_key(next) {
                    self.walk(next);
                    let child = self.low[next];
                    let mine = self.low[node];
                    self.low.insert(node, mine.min(child));
                } else if self.on_stack.contains(next) {
                    let seen = self.index[next];
                    let mine = self.low[node];
                    self.low.insert(node, mine.min(seen));
                }
            }
        }

        // A node whose lowlink is its own index is the root of a component:
        // everything above it on the stack can reach back to it.
        if self.low[node] == self.index[node] {
            let mut component = Vec::new();
            while let Some(top) = self.stack.pop() {
                self.on_stack.remove(top);
                component.push(top.to_string());
                if top == node {
                    break;
                }
            }
            self.components.push(component);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::HexLayer;

    fn edge(from: &str, to: &str) -> ImportEdge {
        ImportEdge {
            from_file: from.to_string(),
            to_file: to.to_string(),
            from_layer: HexLayer::Unknown,
            to_layer: HexLayer::Unknown,
            import_path: to.to_string(),
            line: 1,
        }
    }

    #[test]
    fn no_cycles_in_dag() {
        let edges = vec![edge("a.rs", "b.rs"), edge("b.rs", "c.rs")];
        assert!(detect_cycles(&edges).is_empty());
    }

    #[test]
    fn simple_cycle() {
        let edges = vec![
            edge("a.rs", "b.rs"),
            edge("b.rs", "c.rs"),
            edge("c.rs", "a.rs"),
        ];
        let cycles = detect_cycles(&edges);
        assert_eq!(cycles.len(), 1);
        assert_eq!(cycles[0].len(), 3);
    }

    #[test]
    fn a_self_edge_is_not_a_cycle() {
        // A module naming itself is intra-module reuse: `mod.rs` and its
        // submodules in Rust, a package's files in Go.
        let edges = vec![edge("src/a.ts", "src/a.ts")];
        assert!(detect_cycles(&edges).is_empty());
    }

    #[test]
    fn two_separate_cycles() {
        let edges = vec![
            edge("a.rs", "b.rs"),
            edge("b.rs", "a.rs"),
            edge("c.rs", "d.rs"),
            edge("d.rs", "c.rs"),
        ];
        let cycles = detect_cycles(&edges);
        assert_eq!(cycles.len(), 2);
    }

    #[test]
    fn empty_graph() {
        assert!(detect_cycles(&[]).is_empty());
    }

    /// Two loops sharing a node are one knot, not two. `a → b → a` and
    /// `b → c → b` means all three modules reach each other, and breaking
    /// one edge does not separate them.
    ///
    /// This is the case the old back-edge walk got wrong in both
    /// directions: it reported two cycles here, and which two depended on
    /// where the walk started (ADR-2609140020).
    #[test]
    fn interlocking_loops_are_one_component() {
        let edges = vec![
            edge("a.ts", "b.ts"),
            edge("b.ts", "a.ts"),
            edge("b.ts", "c.ts"),
            edge("c.ts", "b.ts"),
        ];
        let cycles = detect_cycles(&edges);
        assert_eq!(cycles.len(), 1, "one knot, not two: {cycles:?}");
        assert_eq!(cycles[0], vec!["a.ts", "b.ts", "c.ts"]);
    }

    /// The score subtracts 15 points per cycle, so the answer has to come
    /// from the graph and not from the order the edges arrived in. Every
    /// rotation of the same edge list must give the identical result,
    /// node order included.
    #[test]
    fn the_answer_does_not_depend_on_the_order_of_the_edges() {
        let edges = vec![
            edge("src/domain/a.rs", "src/ports/b.rs"),
            edge("src/ports/b.rs", "src/usecases/c.rs"),
            edge("src/usecases/c.rs", "src/domain/a.rs"),
            edge("src/adapters/d.rs", "src/ports/b.rs"),
            edge("src/usecases/c.rs", "src/adapters/d.rs"),
            edge("src/adapters/d.rs", "src/usecases/c.rs"),
        ];
        let expected = detect_cycles(&edges);
        assert!(!expected.is_empty(), "the fixture must contain a cycle to be worth checking");
        for rotation in 1..edges.len() {
            let mut rotated = edges.clone();
            rotated.rotate_left(rotation);
            assert_eq!(detect_cycles(&rotated), expected, "rotation {rotation} disagreed");
        }
        let mut reversed = edges.clone();
        reversed.reverse();
        assert_eq!(detect_cycles(&reversed), expected, "reversed disagreed");
    }

    /// Two knots that share nothing stay two, and they come back in a
    /// stable order rather than whichever the walk reached first.
    #[test]
    fn separate_knots_stay_separate_and_come_back_sorted() {
        let edges = vec![
            edge("z.ts", "y.ts"),
            edge("y.ts", "z.ts"),
            edge("a.ts", "b.ts"),
            edge("b.ts", "a.ts"),
        ];
        let cycles = detect_cycles(&edges);
        assert_eq!(cycles.len(), 2);
        assert_eq!(cycles[0], vec!["a.ts", "b.ts"]);
        assert_eq!(cycles[1], vec!["y.ts", "z.ts"]);
    }
}

/// The graph node a path belongs to.
///
/// TypeScript imports name files, so a file is the node and a cycle is
/// `a.ts → b.ts → a.ts`. A Go import names a package directory and a Rust
/// `use` names a module path, neither of which is a file name, so file
/// nodes never closed a loop in either language and the detector was
/// TypeScript-only in effect (ADR-2609121400, step 4). Go nodes are
/// package directories. Rust nodes are the module directly under `src/`
/// (`src/domain`, `hexa-core/src/ports`), which is where an architectural
/// cycle lives; `mod.rs` and its submodules importing each other is how
/// Rust modules are built and is not one.
fn module_key(path: &str) -> String {
    if path.ends_with(".go") {
        return match path.rfind('/') {
            Some(i) => path[..i].to_string(),
            None => String::new(),
        };
    }
    let is_rust = path.ends_with(".rs") || path.starts_with("crate/") || path.contains("/src/") || path.starts_with("src/");
    if is_rust && !path.ends_with(".ts") && !path.ends_with(".tsx") {
        let segs: Vec<&str> = path.split('/').collect();
        if let Some(i) = segs.iter().position(|s| *s == "src") {
            if i + 1 < segs.len() {
                return segs[..i + 2].join("/").trim_end_matches(".rs").to_string();
            }
        }
        if path.ends_with(".rs") {
            return match path.rfind('/') {
                Some(i) => path[..i].to_string(),
                None => path.trim_end_matches(".rs").to_string(),
            };
        }
    }
    path.to_string()
}

#[cfg(test)]
mod module_key_tests {
    use super::module_key;

    #[test]
    fn go_nodes_are_package_directories() {
        assert_eq!(module_key("internal/domain/count.go"), "internal/domain");
        assert_eq!(module_key("internal/usecases"), "internal/usecases");
    }

    #[test]
    fn rust_nodes_are_the_module_under_src() {
        assert_eq!(module_key("src/domain/mod.rs"), "src/domain");
        assert_eq!(module_key("src/usecases/increment"), "src/usecases");
        assert_eq!(module_key("src/domain/count/value.rs"), "src/domain");
        assert_eq!(module_key("hexa-core/src/ports/inference.rs"), "hexa-core/src/ports");
        assert_eq!(module_key("src/lib.rs"), "src/lib");
    }

    #[test]
    fn typescript_nodes_are_files() {
        assert_eq!(module_key("src/core/domain/count.ts"), "src/core/domain/count.ts");
    }
}
