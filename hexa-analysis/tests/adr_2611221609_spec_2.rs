#![allow(unused, clippy::all)]
//! Scenarios for ADR-2611221609: an exported var inside a Go `var ( … )`
//! group is an export, exactly like a spec in a `const ( … )` group.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use hexa_analysis::domain::{ExportDeclaration, ExportKind, Language};
use hexa_analysis::ports::AstPort;
use hexa_analysis::treesitter_adapter::TreeSitterAdapter;

fn go_exports(path: &str, source: &str) -> Vec<ExportDeclaration> {
    TreeSitterAdapter::new()
        .extract_exports(Path::new(path), source, Language::Go)
        .expect("Go source parses")
}

fn go_references(path: &str, source: &str) -> HashMap<String, usize> {
    TreeSitterAdapter::new()
        .extract_references(Path::new(path), source, Language::Go)
        .expect("Go source parses")
}

fn named<'a>(exports: &'a [ExportDeclaration], name: &str) -> Vec<&'a ExportDeclaration> {
    exports.iter().filter(|e| e.name == name).collect()
}

fn only<'a>(exports: &'a [ExportDeclaration], name: &str) -> &'a ExportDeclaration {
    let found = named(exports, name);
    assert_eq!(
        found.len(),
        1,
        "expected exactly one export named {name}, got {exports:?}"
    );
    found[0]
}

/// S01: An exported var inside a var group is an export of kind Value at its spec's line
#[test]
fn s01_grouped_exported_var_is_value_export_at_spec_line() {
    let src = "package pkg\n\nvar (\n\tTimeout = 30\n)\n";
    let exports = go_exports("pkg/config.go", src);
    let e = only(&exports, "Timeout");
    assert_eq!(e.file, "pkg/config.go");
    assert_eq!(e.line, 4);
    assert_eq!(e.kind, ExportKind::Value);
    assert!(!e.hexa_public);
}

/// S02: Every exported spec in a var group is recorded, each at its own line
#[test]
fn s02_every_exported_spec_in_var_group_recorded_at_own_line() {
    let src = "package pkg\n\nvar (\n\tA = 1\n\tB string\n\tC, D = 2, 3\n)\n";
    let exports = go_exports("pkg/vars.go", src);
    for (name, line) in [("A", 4), ("B", 5), ("C", 6), ("D", 6)] {
        let e = only(&exports, name);
        assert_eq!(e.line, line, "{name} line");
        assert_eq!(e.kind, ExportKind::Value, "{name} kind");
    }
}

/// S03: A grouped var and a grouped const with the same shape produce the same export record
#[test]
fn s03_grouped_var_and_grouped_const_agree() {
    let var_src = "package pkg\n\nvar (\n\tLimit = 5\n)\n";
    let const_src = "package pkg\n\nconst (\n\tLimit = 5\n)\n";
    let var_exports = go_exports("pkg/limits.go", var_src);
    let const_exports = go_exports("pkg/limits.go", const_src);
    let v = only(&var_exports, "Limit");
    let c = only(&const_exports, "Limit");
    for e in [v, c] {
        assert_eq!(e.line, 4);
        assert_eq!(e.kind, ExportKind::Value);
        assert!(!e.hexa_public);
    }
    assert_eq!(v.name, c.name);
    assert_eq!(v.line, c.line);
    assert_eq!(v.kind, c.kind);
    assert_eq!(v.hexa_public, c.hexa_public);
}

/// S04: A grouped exported var that nothing uses is reported as a dead export
#[test]
fn s04_unused_grouped_var_is_dead_export() {
    // The dead-export rule (AstPort::extract_references): an export that no
    // other file in the project names is dead.
    let files: Vec<(&str, &str)> = vec![
        (
            "internal/domain/vars.go",
            "package domain\n\nvar (\n\tUnusedFlag = true\n)\n",
        ),
        (
            "cmd/app/main.go",
            "package main\n\nimport \"fmt\"\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n",
        ),
    ];

    let mut dead: Vec<(String, String, usize)> = Vec::new();
    for (file, src) in &files {
        for e in go_exports(file, src) {
            if e.kind == ExportKind::Method || e.kind == ExportKind::Impl {
                continue;
            }
            let named_elsewhere =
                files
                    .iter()
                    .filter(|(other, _)| other != file)
                    .any(|(other, osrc)| {
                        go_references(other, osrc)
                            .get(&e.name)
                            .copied()
                            .unwrap_or(0)
                            > 0
                    });
            if !named_elsewhere {
                dead.push((e.file.clone(), e.name.clone(), e.line));
            }
        }
    }

    let hits: Vec<_> = dead
        .iter()
        .filter(|(f, n, _)| f == "internal/domain/vars.go" && n == "UnusedFlag")
        .collect();
    assert_eq!(hits.len(), 1, "dead exports: {dead:?}");
    assert_eq!(hits[0].2, 4, "line of the UnusedFlag spec");
}

/// S06: A grouped var must not be left out of the exports
#[test]
fn s06_grouped_var_not_left_out() {
    let src = "package pkg\n\nvar (\n\tTimeout = 30\n)\n";
    let exports = go_exports("pkg/config.go", src);
    assert!(!exports.is_empty(), "grouped var produced no exports");
    assert!(exports.iter().any(|e| e.name == "Timeout"), "{exports:?}");
}

/// S07: A grouped var must not be recorded with a kind other than Value or at a line other than its spec's
#[test]
fn s07_grouped_var_kind_and_line() {
    let src = "package pkg\n\nvar (\n\tTimeout = 30\n)\n";
    let exports = go_exports("pkg/config.go", src);
    let e = only(&exports, "Timeout");
    for wrong in [
        ExportKind::Function,
        ExportKind::Type,
        ExportKind::Method,
        ExportKind::Impl,
        ExportKind::Default,
    ] {
        assert_ne!(e.kind, wrong);
    }
    assert_eq!(e.kind, ExportKind::Value);
    assert_ne!(e.line, 3, "line of the var keyword, not the spec");
    assert_eq!(e.line, 4);
}

/// S08: Grouping an exported var does not change its export record
#[test]
fn s08_grouping_does_not_change_record() {
    let names = ["Timeout", "X", "MaxRetries", "Ä"];
    let inits = [
        "30",
        "\"hello\"",
        "[]int{1, 2, 3}",
        "map[string]int{\"a\": 1}",
        "func() int { return 1 }()",
        "true",
    ];
    for name in names {
        for init in inits {
            // Spec on line 4 in both.
            let ungrouped = format!("package pkg\n\n\nvar {name} = {init}\n");
            let grouped = format!("package pkg\n\nvar (\n\t{name} = {init}\n)\n");
            let u = go_exports("pkg/a.go", &ungrouped);
            let g = go_exports("pkg/a.go", &grouped);
            assert_eq!(u.len(), 1, "ungrouped {name} = {init}: {u:?}");
            assert_eq!(g.len(), 1, "grouped {name} = {init}: {g:?}");
            for e in [&u[0], &g[0]] {
                assert_eq!(e.name, name);
                assert_eq!(e.line, 4, "{name} = {init}");
                assert_eq!(e.kind, ExportKind::Value);
            }
            assert_eq!(u[0].hexa_public, g[0].hexa_public);
        }
    }
}

/// S09: Exactly the upper-case names in a var group are recorded, with no duplicates
#[test]
fn s09_exactly_upper_case_names_recorded_once() {
    let cases: Vec<Vec<Vec<&str>>> = vec![
        vec![vec!["Alpha"], vec!["beta"], vec!["Gamma", "delta"]],
        vec![vec!["lower"]],
        vec![vec!["One", "Two", "Three"], vec!["four", "Five"]],
        vec![
            vec!["a"],
            vec!["B"],
            vec!["c", "d"],
            vec!["E", "F"],
            vec!["G"],
        ],
        vec![vec!["Zed", "zed"], vec!["Q"]],
    ];
    for specs in cases {
        let mut src = String::from("package pkg\n\nvar (\n");
        let mut expected: BTreeMap<String, usize> = BTreeMap::new();
        for (i, names) in specs.iter().enumerate() {
            let line = 4 + i;
            let values: Vec<String> = (0..names.len()).map(|k| k.to_string()).collect();
            src.push_str(&format!("\t{} = {}\n", names.join(", "), values.join(", ")));
            for n in names {
                if n.chars().next().is_some_and(|c| c.is_uppercase()) {
                    expected.insert(n.to_string(), line);
                }
            }
        }
        src.push_str(")\n");

        let exports = go_exports("pkg/group.go", &src);
        let got: BTreeSet<String> = exports.iter().map(|e| e.name.clone()).collect();
        let want: BTreeSet<String> = expected.keys().cloned().collect();
        assert_eq!(got, want, "source:\n{src}");
        assert_eq!(exports.len(), expected.len(), "duplicates in {exports:?}");
        for e in &exports {
            assert_eq!(e.kind, ExportKind::Value, "{}", e.name);
            assert_eq!(e.line, expected[&e.name], "{} line", e.name);
        }
    }
}

/// S11: Only the export kind and record of the grouped var are new: a mixed file gains exactly the grouped vars
#[test]
fn s11_mixed_file_gains_exactly_grouped_var() {
    let src = "package pkg\n\nvar Single = 1\n\nfunc Run() {}\n\nvar (\n\tGrouped = 2\n)\n";
    let exports = go_exports("pkg/mixed.go", src);
    let mut names: Vec<String> = exports.iter().map(|e| e.name.clone()).collect();
    names.sort();
    assert_eq!(names, vec!["Grouped", "Run", "Single"], "{exports:?}");
}
