//! `hexa analyze` says what each layer holds — interfaces, types,
//! implementations, functions — for every language it supports, and says
//! which language each count came from.
//!
//! Before this it reported one number per layer: a file count, for Rust only,
//! over a different file set than the grade read. "How many ports are there"
//! had no answer.
//!
//! Every expected number below is counted by hand from the fixture, not
//! derived from the implementation.

use std::path::Path;
use std::process::Command;

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn fixture() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();

    // ── Rust ──
    write(r, "src/domain/order.rs", "\
pub struct Order { pub id: u64 }
pub enum Status { Open, Closed }
pub fn total(o: &Order) -> u64 { o.id }
impl Order { pub fn new() -> Self { Order { id: 0 } } }
#[cfg(test)]
mod tests {
    pub trait Hidden {}
    struct Fake;
    #[test]
    fn t() {}
}
");
    write(r, "src/ports/store.rs", "\
pub trait OrderStorePort { fn save(&self); fn load(&self) -> u64 { 0 } }
pub trait ClockPort { fn now(&self) -> u64; }
");
    write(r, "src/adapters/secondary/mem.rs", "\
pub struct Mem;
impl OrderStorePort for Mem { fn save(&self) {} }
impl ClockPort for Mem { fn now(&self) -> u64 { 0 } }
");

    // ── Go ──
    write(r, "internal/domain/order.go", "\
package domain

type Order struct { ID int }
type Status int

func Total(o Order) int { return o.ID }
func (o Order) Valid() bool { return true }
");
    write(r, "internal/ports/store.go", "\
package ports

type OrderStore interface { Save(o int) error }
");
    // A test file is not the layer it sits in.
    write(r, "internal/domain/order_test.go", "package domain\n\ntype Fake interface{}\n");

    // ── TypeScript ──
    write(r, "src/ports/clock.ts", "\
export interface Clock { now(): number }
export type Millis = number;
");
    write(r, "src/adapters/primary/cli.ts", "\
import type { Clock } from '../../ports/clock.js';
interface Loggable { log(): void }
export class SystemClock implements Clock, Loggable { now() { return 0; } log() {} }
export function main(): void {}
export const run = () => main();
function helper() { const inner = () => 1; return inner(); }
");

    // The grade does not read examples/, so neither does the inventory.
    write(r, "examples/demo/src/domain/x.rs", "pub struct Ignored;\npub trait AlsoIgnored {}\n");
    d
}

fn inventory(root: &Path) -> Vec<serde_json::Value> {
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(["analyze", ".", "--json"])
        .current_dir(root)
        .env("HOME", home.path())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("analyze --json is not JSON ({e}):\n{text}"));
    v["layer_inventory"]
        .as_array()
        .unwrap_or_else(|| panic!("no layer_inventory in:\n{text}"))
        .clone()
}

/// (files, interfaces, types, implementations, functions) for one row.
/// `implementations` is `None` where the language never declares one.
fn row(rows: &[serde_json::Value], lang: &str, layer: &str) -> (u64, u64, u64, Option<u64>, u64) {
    let r = rows
        .iter()
        .find(|r| r["language"] == lang && r["layer"] == layer)
        .unwrap_or_else(|| panic!("no {lang} row for {layer} in {rows:#?}"));
    let n = |k: &str| r[k].as_u64().unwrap_or_else(|| panic!("{lang}/{layer}.{k} missing: {r}"));
    (n("files"), n("interfaces"), n("types"), r["implementations"].as_u64(), n("functions"))
}

#[test]
fn rust_layers_are_counted_by_item() {
    let d = fixture();
    let rows = inventory(d.path());
    // The #[cfg(test)] trait, struct and fn are not the domain's.
    assert_eq!(row(&rows, "rust", "domain"), (1, 0, 2, Some(0), 1), "an inherent impl and a method are not counted");
    assert_eq!(row(&rows, "rust", "ports"), (1, 2, 0, Some(0), 0));
    assert_eq!(row(&rows, "rust", "adapters/secondary"), (1, 0, 1, Some(2), 0));
}

#[test]
fn go_layers_are_counted_by_item() {
    let d = fixture();
    let rows = inventory(d.path());
    // Go never declares that a type implements an interface: not counted, not zero.
    assert_eq!(row(&rows, "go", "domain"), (1, 0, 2, None, 1), "a method is not a function; _test.go is not read");
    assert_eq!(row(&rows, "go", "ports"), (1, 1, 0, None, 0), "an interface type is an interface, not a type");
}

#[test]
fn typescript_layers_are_counted_by_item() {
    let d = fixture();
    let rows = inventory(d.path());
    assert_eq!(row(&rows, "typescript", "ports"), (1, 1, 1, Some(0), 0));
    // Two names in `implements`; three top-level functions, the nested arrow is not one.
    assert_eq!(row(&rows, "typescript", "adapters/primary"), (1, 1, 1, Some(2), 3));
}

#[test]
fn only_what_the_grade_reads_is_counted() {
    let d = fixture();
    let rows = inventory(d.path());
    let files: u64 = rows.iter().map(|r| r["files"].as_u64().unwrap_or(0)).sum();
    assert_eq!(files, 7, "seven source files; examples/ and _test.go excluded: {rows:#?}");
    assert!(
        !rows.iter().any(|r| r["language"] == "typescript" && r["layer"] == "domain"),
        "a layer a language has no files in gets no row"
    );
}
