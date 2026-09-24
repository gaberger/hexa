# Upgrading

Behaviour changes that need a hand when you move between versions. Each entry
names the commit that carried it, what changes for a project that already
exists, and the one command that resolves it.

A change that needs nothing from you is not listed here; it is in the commit
log.

## What the grade leaves out is decided by folder, not substring (`9672371`)

The built-in exclusions matched substrings: `dist` left out
`src/domain/distance.ts`, `examples` anything with that word in its path,
`tests/` a folder named `contests/`. They now match whole path segments
(`dist/`, `examples/`, `tests/`, `target/`, `node_modules/`), and `test/` and
`__tests__/` — where TypeScript keeps its tests — are excluded too.

**What changes for you.** A source file whose name only contained one of those
words is now graded, which can surface violations it always had. Test helpers
under `test/` stop counting as source files with no layer, which could only
raise a grade.

## A grade cannot exceed the code it could classify (`bf1bafc`)

Every file the grade reads must have a layer for the grade to reach A+, and
the score is capped at the share of files that do (ADR-2609241707). Before
this, a file the classifier could not place was skipped silently — every import
into or out of it went unchecked — and the grade said nothing about how much
was skipped.

**What changes for you.** A project organised by crate, by feature, or by any
folder names other than `domain/`, `ports/`, `usecases/`, `adapters/` may grade
lower. The report names each file with no layer. Declare where it belongs:

```json
{ "analyze": { "layers": { "src/billing": "usecases", "crates/store": "adapters/secondary" } } }
```

in `.hexa/project.json`, then `hexa analyze .` again. Keys are path prefixes
matched on whole segments, longest first; a misspelt layer stops the run and
names the entry. A crate or package named for its layer (`app-domain`,
`app_ports`) needs no entry.

## Imports between your own packages are checked (`0f0024f`, `342a903`)

An import of another crate, Go module or npm package *in the same workspace*
is now an edge the grade checks. Before, it was taken for a third-party
dependency and never checked, so a domain crate importing an adapter crate
graded clean.

**What changes for you.** Violations that were always there become visible,
and the grade drops by 10 for each. Each one is listed with its file and
import. hexa's own tree went from A+ to C on this change before its fixes —
the usual fix is the one hexa's scaffold teaches: have the port re-export the
domain value types it speaks, and import them through the port.

## `--json` no longer has `rust_layers` or `rust_violations` (`316787e`)

Both came from a Rust-only scan with its own layer rules, which disagreed
with the grade (it called a flat `adapters/` folder secondary; the grade,
by ADR-2609122048, calls it primary). Every violation is in
`boundary_violations`, from the one classifier.

**What changes for you.** A script that read those fields reads
`boundary_violations` for violations and `layer_inventory` for per-layer
counts (files, interfaces, types, implementations, functions — per language).

## Memory is stored per project (`97dc8f0`)

`hexa memory` used to write to one file per *user*, so every project on the
machine shared one store. It now writes to `.hexa/memory.jsonl` inside the
project, found by walking up from the working directory to the nearest `.hexa/`
(ADR-2609211200).

**What changes for you.** Entries written before this commit are still in
`~/.hexa/memory.jsonl`, and a project will not see them. Nothing is lost, and
nothing moved on its own — copying a lesson into a project that may not own it
is a guess the tool does not make.

```bash
hexa memory --global list      # everything written before the change
```

Re-store the entries that belong to a project from inside that project. Leave
the ones that are genuinely about the machine — model calibration, a lesson
about your own setup — where they are; `--global` reaches them from anywhere.

Outside a project, `hexa memory` still uses `~/.hexa/memory.jsonl`, so running
it in a directory that was never scaffolded behaves as it always did.

## An error-severity rule violation now costs grade (`6be13ef`)

A finding from the project's own `.hexa/ADR-rules.toml` at `severity = "error"`
now subtracts 10 points from the architecture score, the same as a boundary
violation (ADR-2609211430 §1). Before this, `hexa analyze --exit-code` failed on
such a tree while `--grade A` passed and the tool printed A+.

**What changes for you.** A project that already carries error-severity
findings will score lower than it did, and a CI job gating on `--grade A` may
go red on a tree that passed yesterday. The tree did not get worse; the grade
stopped disagreeing with the exit code about it.

```bash
hexa analyze . --json | jq .score_components   # rule_errors is now a term
```

Two ways forward, both one line. Resolve the finding, which is what the rule
asked for. Or decide the rule is advisory in this project and set that rule to
`severity = "warning"` — warnings stay out of the score, which is what
`--strict` is for.

## Scaffolded TypeScript projects require Node 22 (`1abe882`)

The generated `package.json` declares `engines.node >= 22` and its test script
uses a glob rather than a directory, because `node --test dist/` stopped
discovering tests and started trying to load the directory as a module
(ADR-2609211245).

**What changes for you.** Only new scaffolds. A project generated before this
commit keeps its old script; if it fails with `Cannot find module …/dist`, the
fix is that one line:

```json
"test": "tsc && node --test \"dist/**/*.test.js\""
```

## Inline references now count as imports (`ADR-2609211600`)

`[[import_policy]]` used to judge only import declarations, so a domain file
could call `std::fs::read("x")` with no `use` line and pass a policy that
denies `std::fs`.

**What changes for you.** A project whose domain reaches outside inline will
report findings it did not before, and at `severity = "error"` each site costs
10 points. The forms now read: an inline path (`std::fs::read`, `::crate::f`,
a crate in a signature, `#[attr::macro]`, `macro!` paths), `extern crate`,
`require("x")`, and `import("x")` in expression or type position. A module
loaded by a computed name is reported as a **warning**.

```bash
hexa analyze . --json | jq .score_components
```

The fix is the same as for any other finding: put the capability behind a
port, or add the dependency to that policy's `allow` with a reason. Local code
is unaffected — `O::new()`, `Self::x()`, an enum variant and a local module
are not references, because a path is only judged when its first segment names
a declared dependency or the standard library.

## Every reference is read, and every one is judged (`ADR-2609221430`)

`[[import_policy]]` read most inline references but not all of them, and what
counted as an "external" name came from the root manifest only. Both are
widened (ADR-2609221430).

**What changes for you.** A project may report findings it did not before, and
at `severity = "error"` each site costs 10 points. The new forms:

- a path written inside a macro call — `println!("{:?}", std::fs::read("x"))`,
  `vec![…]`, `assert!(…)`, `format!(…)`, nested macros included;
- a second reference on a line whose first reference was permitted — a
  permitted path no longer suppresses a denied one beside it;
- a crate declared by a nested workspace member's own `Cargo.toml`, or by a
  `[target.'cfg(…)'.dependencies]` table;
- a qualified path, `<sqlx::PgPool as Default>::default`;
- TypeScript `import x = require("m")`, and `` require(`m`) `` or ``
  import(`m`) `` with nothing interpolated.

```bash
hexa analyze . --json | jq .score_components
```

Two changes may *remove* findings. A policy's `layer` now matches whole path
segments, so `/domain/` no longer matches `src/adapters/domain_helpers/`; and
files under a top-level `tests/`, `benches/` or `examples/` directory are out
of scope, so a fixture that imports a driver on purpose is no longer a
violation.

One new **warning**: a file inside a policy's layer that could not be read or
parsed is now reported instead of silently skipped. It does not move the grade,
but `--strict` fails on it. If you see one, the file is usually not UTF-8.

**If you use `.mcp.json`:** `hexa assets sync --force` used to write a `hexa`
MCP server whose command was the hexa binary with an `mcp` subcommand — one the
binary does not have, so the server exited immediately. It now writes no such entry and removes that one
if it wrote it before, matching on the exact command and args so a `hexa` entry
you re-pointed yourself is left alone. Nothing else in the file is touched. If
you have a hand-written entry that starts the hexa binary with an `mcp`
subcommand, delete it.

## `hexa analyze` refuses to grade a scan that read nothing (`ADR-2609221700`)

`hexa analyze /path-that-does-not-exist` printed **A+ — score 100/100** and
exited 0, and so did `--grade A`, `--strict`, `--exit-code` and `--json`. An
empty but existing directory did the same: zero files scanned means zero
findings, and a score computed from zero findings is a perfect one.

`analyze` now refuses a path that does not exist, and refuses to grade a
directory holding no source files. Every surface fails together, because the
guard sits above the branch into `--json`.

**What changes for you.** A CI job that was passing may now fail — correctly.
If a job runs `hexa analyze . --grade A` with the wrong working directory, or
after a checkout that produced nothing, it was passing against an empty scan
and now exits 1 with a message saying so. That is the bug being fixed rather
than a regression; the tree did not get worse, the check stopped reporting a
result it had not earned.

```bash
hexa analyze .        # from the directory you mean to grade
```

Analysing a docs-only or config-only repository now fails for the same reason.
To analyse a single file rather than a tree, pass it with `--file`.

**Also:** an expected refusal now prints one sentence instead of a stack trace.
`main` returned a `Result`, so errors printed with `Debug`, and a backtrace was
appended whenever `RUST_BACKTRACE` was set — which many Rust developers export
globally. The message and the exit code were always right; only the
presentation was wrong. Nothing to do on your side.
