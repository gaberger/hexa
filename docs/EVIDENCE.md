# Evidence

Every claim on the README maps to a command here. Each command was run before
this page was written, and the expected output is what it printed. A claim with
no command on this page is not yet validated.

## Setup

```bash
cargo build --workspace
```

The hermetic reproducers need nothing else. The live ones need a built `hexa` on
`PATH` and, where noted, a toolchain for the example's language.

## The scaffold is deterministic and runs

**Claim.** `hexa init --scaffold` writes byte-identical output for the same name,
and the result passes its gate with no edits, in Rust, Go and TypeScript.

```bash
cargo test -p hexa-cli --test scaffold_is_executable
```

Expected: `13 passed`. The tests scaffold twice and diff the bytes, run each
language's gate, plant a violation and assert each shipped rule fires, and
assert a clean scaffold trips none of them. The TypeScript gate needs one
`npm install`, so it is skipped unless `HEXA_TEST_NPM=1` is set; CI sets it
(ADR-2609211245).

## The rules that ship with a scaffold fire

**Claim.** `.hexa/ADR-rules.toml` is enforced by `hexa analyze`, not advisory.

```bash
hexa init /tmp/probe --scaffold --lang rust
printf 'pub fn bad() { let _p = "/home/x"; let _m = "qwen3:4b"; }\n' > /tmp/probe/src/offender.rs
hexa analyze /tmp/probe
```

Expected: two violations named with file and line. `Hardcoded absolute path` and
`A model or provider name outside the inference boundary`.

## A vacuous pass is rejected

**Claim.** A test command that exits 0 having run zero tests does not satisfy
the gate.

```bash
cargo test -p hexa-exec --lib vacuous
```

Expected: `5 passed`. One test is the regression that motivated the counter: a
multi-binary cargo run with one empty binary and 39 tests elsewhere is not
vacuous. The old substring check called it vacuous and would have reverted a
correct commit.

## The architecture grade cannot rise as violations are added

**Claim.** The health score is monotonic in the violation count.

```bash
cargo test -p hexa-analysis --lib health_score
```

Expected: `10 passed`, including `the_score_never_climbs_as_violations_are_added`
over 0 to 60 violations. Before the fix, 26 violations scored 96 because the
penalty wrapped in a `u8`.

## hexa passes its own analyzer

```bash
hexa analyze .
```

Expected: `Architecture grade: A+ — score 100/100`, `0 boundary violations`,
`coverage 149/149 files in a layer`, and `score_components` all zero (with
`coverage_ceiling` 100) in `hexa analyze . --json`.

```bash
cargo test -p hexa-cli --test hexa_grades_itself_by_ratchet
```

Expected: `1 passed`. It asserts hexa's boundary violations are exactly the
list in the test — now empty — so a new one fails, and so does a fixed one left
listed.

The scan covers every crate. The analyzer has no built-in knowledge of hexa's
directory names. hexa excludes two paths — `hexa-cli/assets/scaffold`, the
template files compiled into the binary, and `bench/fixtures`, benchmark data —
and declares them in `.hexa/project.json` under `analyze.exclude`, the same way
any project would. Its files' layers are declared there under `analyze.layers`.

`hexa-cli/tests/scaffold_is_executable.rs::a_violation_planted_in_any_crate_is_seen`
copies each crate, plants a domain-imports-adapter violation in it, and asserts
the analyzer names the file and exits 1. This exists because an earlier build of
the analyzer excluded `hexa-core/` and `hexa-cli/` by name, so hexa graded itself
over six of eight crates and reported A+.

## The grade covers every file, and every import

**Claim.** A file the classifier cannot place, or an import it cannot follow,
no longer passes silently. hexa graded itself A+ through each of these holes
before they were closed (ADR-2609241707).

```bash
cargo test -p hexa-cli --test a_grade_cannot_exceed_what_it_classified \
  --test a_project_declares_its_layers --test a_layer_crossing_between_packages_is_seen \
  --test a_flat_module_is_still_checked --test every_rust_path_to_a_module_is_an_edge
```

Expected: five `test result: ok` lines, 28 tests in all. Between them:

- one unclassified file withholds A+ and is named; half unclassified caps the
  score at 50; declaring the layer restores it;
- a layer declared in `analyze.layers` reaches the grade, the inventory and
  `--file`; a misspelt layer stops the run;
- an import between a workspace's own packages is an edge in Rust, Go and
  TypeScript, and a third-party one is not;
- a file directly in `src/` is checked, and a declared file matches the module
  path an import resolves to;
- a Rust file's `super::`, inline, nested and `pub use` paths are edges, and
  test code and type paths (`Vec::new`, `Ordering::Less`) are not.

Each group has controls that must stay clean, and each was shown failing
before its fix. The two poles hold: `examples/stress/game-sokoban-rs-tangled`
grades F 34, `examples/url-shortener-rs` A+ 100.

## Every detector reads all three languages, or says it does not

```bash
for lang in rust go ts; do
  hexa init /tmp/probe-$lang --scaffold --lang $lang
  (cd /tmp/probe-$lang && hexa analyze .)
done
```

Expected, for each language: `Architecture grade: A+ — score 100/100`,
`0 boundary violations`, every file in a layer (`coverage 6/6` for Rust, `5/5`
for Go and TypeScript), `dead layers 0`, `orphans 0`. For Go and
TypeScript, `cohesion`, `duplication` and `god types` print
`n/a (no Rust files; detector is Rust-only)`; they never print a count for a
tree they cannot read. A fresh scaffold has nothing dead, unused, orphaned or
circular by construction, so any other number is a detector defect
(ADR-2609121400). The per-language fixtures, wired and broken, are the files
matching `hexa-analysis/tests/*_per_language.rs`.

## A commit failure does not delete the work

**Claim.** When the gate passes and `git commit` fails, the change is kept in
the working tree and the error names the failed step.

```bash
cargo test -p hexa-exec --lib commit_failure
```

Expected: `2 passed`. Before the fix, a fresh clone with no git identity
reverted every correct repair and reported it as a failed test.

## The shipped documents name only real verbs

**Claim.** Every `hexa …` command in the README, in `ARCHITECTURE.md`, in every
document under `docs/`, and in the `CLAUDE.md` section written into scaffolded
projects resolves against the binary. Every relative link resolves. Every
diagram image exists.

```bash
cargo test -p hexa-cli --test shipped_docs_name_real_verbs
```

Expected: `10 passed`. Planting a dead verb or a dead link fails it.

## A workplan task is done only with evidence

**Claim.** A task marked done without commits in its scope is not accepted.

```bash
cargo test -p hexa-cli --test reconcile_evidence
```

Expected: `3 passed`.

## The provider is named in one place

**Claim.** The local inference server's identity lives in `hexa-infer` only.

```bash
cargo test -p hexa-infer --lib local_provider
```

Expected: `5 passed`, including that an unconfigured project yields no tiers
rather than a default model, and that `OLLAMA_HOST` normalises whether given as
`host:port` or a URL.

## The examples pass their gates

Each was built by `hexa scaffold` or `hexa build` from one sentence.

```bash
(cd examples/linkstore-svc    && cargo test)   # 27, of which 12 end-to-end
(cd examples/url-shortener-rs && cargo test)   # 39
(cd examples/game-life-rs     && cargo test)   # 36
(cd examples/ratelimiter-proof && cargo test)  # 18
(cd examples/game-ttt-go      && go test ./...)
(cd examples/game-2048-ts     && npm install && npm test)   # 88
```

`linkstore-svc` binds a real port and writes a real SQLite file. Breaking its
HTTP status fails 4 tests. Breaking a domain rule fails 3 more.

**What produced them is not recorded, and cannot be recovered.** These six
carry their test counts and nothing else: no model, no provider, no date, no
version of hexa. The runs and the machines that made them are gone, so the
question "which model wrote these?" has no answer on file, and a reconstructed
one would be a guess wearing a date.

From ADR-2609221900 onward it is recorded at the moment of generation:
`hexa build` writes `PROVENANCE.md` into the project it produces, carrying the
date, the hexa version, the challenge, the gate and its result, and the models
the run resolved. The record states its own limit in its own text — those are
the models *available* to the run, not a claim about which one answered a given
call, because hexa resolves a tier at dispatch and does not record the origin
of each answer.

So the six above stay unattributed unless they are regenerated. That needs a
machine with models configured; it is not something this page can close by
looking harder.

## The run feed counts runs, not hook events

**Claim.** `hexa do runs` counts only rows written by `hexa do run`. Subagent
lifecycle rows that the Claude Code hooks append to the same log are not runs
and do not appear as failures.

```bash
cargo test -p hexa-exec --lib runs_feed_tests
```

Expected: `3 passed`. Before the fix the feed reported 8% pass over a log in
which every real run had passed. The session is written up in
[`analysis/2609151730-the-dashboard-lied-case-study.md`](analysis/2609151730-the-dashboard-lied-case-study.md).

## Self-update picks the highest version, not GitHub's "latest"

**Claim.** `hexa self-update` installs the highest published `vX.Y.Z` and never
moves backwards unless `--version` names a tag.

```bash
cargo test -p hexa-cli --lib latest_release_tests
```

Expected: `4 passed`. Two tags pushed together once left the older one marked
latest; the old code would have downgraded every machine. Same case study.

## Every cited ADR id resolves to a file

**Claim.** `hexa adr doctor` fails on an `ADR-…` id cited in code, docs,
`CODEOWNERS` or `.hexa/*.toml` that has no file under `docs/adrs/` or
`docs/adrs/historical/`, and `--stub-orphans` writes a Historical stub for
each orphan that records what the citing lines say.

```bash
cargo test -p hexa-cli --lib adr_citations
hexa adr doctor --strict
```

Expected: `5 passed`, then `No findings`. On 2026-09-15 the first doctor run
with this check reported 539 errors across 114 orphan ids; the stubs under
`docs/adrs/historical/` are what resolved them (ADR-2609151930 §2).

## The gate runner says why a gate did not run

**Claim.** `hexa adr gates` prefixes PATH with `~/.cargo/bin`, reports a cargo
that cannot read the lock file or an unreachable host as "cannot run here"
rather than "gate failed", prints the last ten lines of a real failure, and
treats a gate that exited 0 having run zero tests as failed.

```bash
cargo test -p hexa-cli --lib adr_gates_classify
PATH=/usr/bin:/bin hexa adr gates
```

Expected: `7 passed`, then a suite in which no cargo gate reads "cannot run
here" because the prefix found rustup's cargo. Before this, the same PATH
produced `0 passed · 21 failed` with no reason given (ADR-2609151930 §6).

## The standalone gate

```bash
hexa ci --standalone-gate
```

Expected: three `pass` lines and `Standalone gate passed`. It runs the
inference adapters' tests and the agent loop's tests. Before the fix it ran
tests in a deleted crate and could never pass.

## Memory belongs to the project it was learned in

**Claim.** Two repositories worked on by the same user do not share one memory
store. A note stored in one project is invisible in another, and the shared
per-user store is reached only by naming it.

```bash
cargo test -p hexa-cli --test memory_is_project_scoped
```

Expected: `4 passed`. The tests drive the built binary, because path resolution
is the subject; `HOME` is set per child process, so no test writes this
process's environment (ADR-2609131749).

The reproduction from the bug report, re-run against the fix:

```bash
cd ~/probe && hexa memory store "adr:0007:why" "Postgres over SQLite"
cd ~/proj2 && hexa memory list          # unrelated, never scaffolded
```

Expected: the store prints `Store: …/probe/.hexa/memory.jsonl (project)`, and
the second directory prints `No memory entries yet` above
`Store: …/.hexa/memory.jsonl (no project here — shared)`. Before
`97dc8f0` the second command printed the first command's entry.

`adr:0007:why` names a different decision in every repository that has an ADR 7,
and memory is newest-wins, so the second project to store that key silently
overwrote the first. Nothing errored (ADR-2609211200).

## A scaffolded TypeScript project passes its own gate

**Claim.** `hexa init --scaffold --lang ts` produces a project whose stated
gate passes with no edits, and CI runs that gate on every change.

```bash
hexa init /tmp/ts-probe --scaffold --lang ts
cd /tmp/ts-probe && npm install && npm test
```

Expected: `# pass 4` and exit 0, from
`tsc && node --test "dist/**/*.test.js"`. Before `1abe882` the script read
`node --test dist/`, which on Node 22 tries to load the directory as a module
and exits 1 — so every scaffolded TypeScript project failed its first gate. The
covering test existed and asserted the right thing; nothing ran it, because it
is skipped unless `HEXA_TEST_NPM=1` and no workflow set it (ADR-2609211245).

Requires Node 22, which the scaffold declares in `engines`.

The same gate deletes every test from a fresh scaffold and asserts `npm test`
then **fails**. `node --test` over a glob exits 0 when the glob matches
nothing, so a project with its tests removed reported success; the scaffold's
test script now counts the compiled test files first and exits 1 with
`No compiled test files under dist/` when there are none.

## An error-severity rule violation costs grade

**Claim.** A finding the project's own rules file marks `error` moves the
letter hexa prints, the floor `hexa scaffold --grade` enforces, and
`--exit-code`, in agreement.

```bash
cargo test -p hexa-cli --test rule_errors_cost_grade
```

Expected: `7 passed`. Three of them failed against `1abe882`: the grade floor
passed where `--exit-code` failed, the ten-point drop was absent, and
`score_components` reconstructed a number ten points from the score.

## The domain imports only what it is allowed

**Claim.** `[[import_policy]]` in `.hexa/ADR-rules.toml` checks what a layer
imports from outside the project — which layer-to-layer edges cannot see,
because such an import has no edge to violate.

```bash
cargo test -p hexa-cli --test domain_import_policy
```

Expected: `11 passed` — the nine cases of ADR-2609211430 plus one that every
scaffolded language satisfies the policy it ships with, and one that an `allow`
of `serde` does not quietly cover `serde_json`. Two of the eleven pass before
and after the change by design: an adapter importing `sqlx` is not a finding,
and a warning-severity policy reports without moving the grade.

The reproduction from the ADR:

```bash
# a tree whose domain imports sqlx, with the shipped policy
hexa analyze .              # Architecture grade: B — score 89/100
hexa analyze . --grade A    # exit 1
hexa analyze . --exit-code  # exit 1
```

Expected: `B — score 89/100`, one error naming
`` `sqlx::PgPool` — not in this policy's `allow` ``, and exit 1 from both gates.
Before `2e24276` the same tree printed `A+ — score 99/100` and `--grade A`
exited 0 while `--exit-code` exited 1.

## A reference is an import, whether or not it has an import line

**Claim.** `[[import_policy]]` judges a crate named inline — with no `use`
line — exactly as it judges the import of it. Local code is not flagged.

```bash
cargo test -p hexa-cli --test domain_import_references
```

Expected: `21 passed`. The fixtures carry the **shipped** rules file, so this
tests what a scaffolded project gets rather than a policy written to suit the
test. Six Rust forms (an inline call into a denied `std` module, a global
`::path`, a type position, an attribute, a macro invocation, `extern crate`),
five TypeScript (`require`, a dynamic `import()`, a type-position `import()`,
a relative path, a computed name), and three Go.

Against `996628f`, 14 of the first 18 failed. The four that passed are the
negative controls, and they are half of what this gate is for: `O::new()` on a
local type, `Self::new()`, an enum variant after `use std::cmp::Ordering;`, a
local `mod util` then `util::f()`, and `crate::domain::x()` all have the shape
of a crate path and are this project's own code.

The Go cases assert §6 of the ADR rather than assuming it: Go cannot name a
package without importing it, so `os.Getenv` after `import "os"` must report
**one** site and not two. `plugin` is denied by the shipped policy, and that
case was confirmed to fail with the entry removed.

A module loaded by a computed name — `require(name)` — is a **warning**: it
does not move the grade, and `--strict` fails on it. Saying nothing would be
the silent skip this check exists to stop.

**The ADR's table, re-run against the build that closes it.** One file per row
under `src/domain/`, with the shipped rules file and a manifest declaring
`sqlx` and `tokio`:

| Written in the domain | Before | After |
|---|---|---|
| `use sqlx::PgPool;` | error | error — ``sqlx::PgPool`` not in `allow` |
| `use std::fs;` | error | error — ``std::fs`` denied |
| TS `import { Pool } from "pg"` | error | error — ``pg`` |
| TS `export { Pool } from "pg"` | error | error — ``pg`` |
| TS `export * from "node:fs"` | error | error — ``node:fs`` denied |
| `std::fs::read("x")` | **none** | error — ``std::fs::read`` denied |
| `::sqlx::query("x")` | **none** | error — ``sqlx::query`` |
| `fn f(_p: sqlx::PgPool)` | **none** | error — ``sqlx::PgPool`` |
| `#[tokio::main]` | **none** | error — ``tokio::main`` |
| `sqlx::query!("x")` | **none** | error — ``sqlx::query`` |
| `extern crate sqlx;` | **none** | error — ``sqlx`` |
| TS `require("node:fs")` | **none** | error — ``node:fs`` denied |
| TS `await import("pg")` | **none** | error — ``pg`` |
| TS `type P = import("pg").Pool` | **none** | error — ``pg`` |
| TS `require(name)`, computed | **none** | **warning** — cannot be checked |
| `R14::new()` on a local type | none | none, **correctly** |

Each finding names the reference and which half of the policy spoke, at the
line it is written on.

## Every reference is read, and every one is judged

**Claim.** The gaps left in the check above are closed: a path written inside a
macro, a second reference on a line whose first was permitted, a crate declared
by a nested workspace member or a `target.<cfg>` table, a qualified path, and
two TypeScript specifier forms. Local code is still not flagged.

```bash
cargo test -p hexa-cli --test domain_import_references_complete
```

Expected: `38 passed`. Against `08e7cda` — the commit this was written on —
**23 of the 38 failed**. The 15 that passed are the negative controls and the
cases that already worked, and they are why the number is not 38: widening an
extractor is exactly how local code starts getting flagged, so half this gate
exists to prove that did not happen.

The ADR's Context table, re-run against the build that closes it. Every row was
reproduced on `08e7cda` first; the middle column is what that build printed.

| Case (a file under `src/domain/`) | On `08e7cda` | Now |
|---|---|---|
| `println!("{:?}", std::fs::read("x"))` | none | error naming `std::fs` |
| `vec![std::fs::read("x")]` | none | error |
| `assert!(std::env::var("X").is_ok())` | none | error |
| `format!("{:?}", sqlx::query("x"))` | none | error naming `sqlx` |
| `println!("{:?}", vec![std::fs::read("x")])` (nested) | none | error |
| `std::collections::…` then `std::fs::read`, one line | none | error naming `std::fs` |
| …same line, order reversed | error | error (unchanged) |
| `std::fs::read` twice on one line | error | error, once |
| `std::fs::read` and `std::env::var` on one line | one error | two errors |
| Nested member `crates/core`, inline `sqlx::query` | none | error |
| …and `use sqlx::PgPool;` in the same file | error | error — both, agreeing |
| Glob member `crates/*` | none | error |
| An `exclude`d member's manifest | not read | not read (unchanged) |
| `[target.'cfg(unix)'.dependencies] nix` → `nix::unistd::getpid()` | none | error |
| `[target.'cfg(windows)'.dev-dependencies] winapi` | none | error |
| TS `import pg = require("pg")` | none | error |
| TS `` require(`pg`) `` | warning | error |
| TS `` require(`${x}`) `` | warning | warning (unchanged) |
| TS `` import(`pg`) `` | warning | error |
| `<sqlx::PgPool as Default>::default` | none | error naming `sqlx` |
| `<O as Default>::default` on a local type | none | none (unchanged) |
| A non-UTF-8 file under `src/domain/` | skipped in silence | one warning naming the file |
| …its effect on the grade | — | none; `--strict` exits 1 |
| `tests/domain/x.rs` importing `sqlx` | error | none |
| `src/adapters/domain_helpers/x.rs` | none | none (unchanged) |
| `src/adapters/domain/x.rs` | error | error (unchanged) |
| `src/domain/x.rs` importing `sqlx` | error | error (unchanged) |

The negative controls, all unchanged: comments, doc comments and string
literals naming denied modules; `O::new()` on a local type; an enum variant
after `use std::cmp::Ordering;`; a local `mod util` reached as `util::v()`
inside a macro; `crate::util::v()`; and `vec![1, 2, 3]`, which has no path in
it at all.

Anchoring changed one of these, not three. `domain_helpers` never matched —
`/domain/` is not a substring of `/domain_helpers/` — so that row is a control.
`src/adapters/domain/` still matches, and should: `domain` there is a whole
segment, and deciding that a `domain` directory stops being one because of its
parent is a rule this ADR did not make. What anchoring actually buys is the
`tests/` row, plus the guarantee that a future `domain_x` layout stays out.

**The dead MCP entry.** `hexa assets sync --force` wrote an MCP server whose
command was the hexa binary with an `mcp` subcommand. The binary has no such
subcommand — it answers `error: unrecognized subcommand` — so any client that
loaded `.mcp.json` started a server that exited immediately. The gate pins the
premise as well as the fix: `hexa_has_no_mcp_subcommand` will fail the day hexa
grows one, which is the right moment to revisit this.

It now writes no such entry, and removes that one if it wrote it before —
matched on the exact command and args, so a `hexa` entry pointed somewhere else
is left alone, and every other server in the file is untouched.

## Prose that could not fail, in this repository

**Claim.** hexa's own decision records drifted from its code in silence: ids
were cited that resolved to nothing, and no check said so. `hexa adr doctor`
is the gate that closed it.

On 2026-09-15, an audit found **140 distinct ADR ids cited** across the code and
docs and **113 with no file** — the most-cited decision of all, referenced 39
times, did not exist. The write-up is
[`analysis/2609151900-adrs-as-memory-investigation.md`](analysis/2609151900-adrs-as-memory-investigation.md),
which states the cause plainly: the history before 2026-09-12 was not carried
into this repository, and `hexa adr doctor` reported "registry is consistent"
because it checked the files that exist against each other and never a citation
against the ledger.

Today, on `main`, the number that matters is the one CI enforces:

```bash
hexa adr doctor          # "No findings — registry is consistent"
```

**Zero dangling citations, checked on every push.** That step sits between the
architecture grade and lint in `.github/workflows/ci.yml`, and a dangling
citation fails the build. It is the part of this section that cannot drift,
because nothing has to remember to run it.

| | 2026-09-15 | today |
|---|---|---|
| Decision ids cited | 140 | — |
| Cited ids with no file | **113** | **0**, enforced in CI |

**This section used to print its own count, and that was the mistake.** It said
"153 ids cited, 2 unresolved", from a shell pipeline written for the page. The
pipeline matched a three-digit alternative, which also matches the first three
digits of a four-digit id, so four-digit fixture ids in test code were counted
as citations to ids that appear nowhere as literal text. Corrected to 162 and
7, the figure drifted to 164 and 9 within one commit — because the correction
itself added ids to the repository, and because the count included test
fixtures, which are not citations at all.

A number maintained by hand on a page that promises every number has a command
behind it is the failure this page exists to catch, in the page itself. It is
replaced above by the checker's own result, which a machine re-derives on every
push.

The 2026-09-15 figures were measured with a pattern carrying that same
truncation, so 140 and 113 are close rather than exact. Their magnitude rests
on something no pattern can distort: closing that gap required writing **114
stub files**, one per cited and missing decision. That is the number worth
quoting, because a reader can count the files.

**`hexa adr doctor` does not share that bug, and an earlier version of this
section said it did.** The checker drops a match followed by a digit, so it
never reports a truncated id. What it does instead is quieter and worse: a
four-digit id matches on its first three, the guard sees the fourth digit and
discards the whole match, so a **four-digit citation that dangles is invisible**
— the checker reports "registry is consistent" about a file it could not see.
Reproduced on this build:

```bash
mkdir -p /tmp/adrgap/docs/adrs /tmp/adrgap/src && cd /tmp/adrgap
printf -- '---\nid: ADR-001\nstatus: accepted\ndate: 2026-01-01\n---\n# ADR-001: x\n' \
  > docs/adrs/ADR-001-x.md
printf '// cites ADR-0042, which does not exist\npub fn f() {}\n' > src/lib.rs
hexa adr doctor        # "No findings — registry is consistent"
```

Not fixed here, deliberately. Widening the pattern makes every id visible, and
that surfaces prose that *discusses* an id rather than citing a decision —
including comments inside the checker's own source explaining this very
behaviour. Telling a citation from an example is a design decision that needs
its own ADR and its own gate, not a regex edit. What ships now is the checker
running in CI at all, which is why the gap is written down here instead of
being discovered again later.

This is also why this section describes the fixture ids instead of spelling
them: writing one here makes this page cite it.

The 2026-09-15 figures were measured with the older pattern and carry the same
truncation, so treat 140 and 113 as close rather than exact. The magnitude is
corroborated by something countable that does not depend on any regex: closing
that gap required writing **114 stub files**, one per decision that was cited
and missing.

Two honest qualifications. The gap was closed by writing those 114 stub files
under `docs/adrs/historical/`, each saying what it is — the first is titled
"Decision text not carried into this repository" and its Decision section reads
"Unknown." The citations resolve; the decisions are still gone. And the count
above is a shell pipeline on this page, not a test, so it is checked when
someone runs it. `hexa adr doctor` is the part that runs in CI, and it now
fails on a dangling citation — this session hit that twice, once for an ADR
written but not yet filed, and once for a test fixture that looked like a
citation.

## The refactoring trial, and what of it can be re-run

**Claim.** hexa repaired 17 boundary violations in a project it did not write,
found 5 more nobody had named, edited no logic and no test, and kept all 177
tests green. The README says this; the article repeats it.

The trial is written up, with its measures registered **before** the run:

- [`analysis/2609120500-refactoring-trial-preregistered.md`](analysis/2609120500-refactoring-trial-preregistered.md)
  — the four measures, the task, the gate, written first and not edited after.
- [`analysis/2609120500-refactoring-trial.md`](analysis/2609120500-refactoring-trial.md)
  — the run, including the two measures it **failed** on the pre-registered
  letter, and why the author's first reading of one of them was wrong.

**This one is readable but not re-runnable.** The subject is a project called
`brain`, and nothing here identifies it by URL or commit, so you cannot fetch
the baseline and repeat the run. Every other claim on this page is a command
you can execute. This is the exception, and it is marked as one rather than
left to look like the others.

## Not yet validated

- **Localisation.** Given only a failing test name, hexa found the right file in
  1 of 10 trials. There is no command for it. The trial is in
  [`analysis/2609120300-brownfield-trial.md`](analysis/2609120300-brownfield-trial.md).
- **What no static reading shows.** Third-party dependencies in `domain/` are
  checked whether or not they have an import line, and whether or not they are
  written inside a macro (see the two sections above). What remains unseen is
  code generated by a macro from an allowed crate, reflection, and FFI. A
  module loaded by a computed name, and a file that could not be read, are
  reported as warnings rather than left out.
- **What produced the six example projects.** Not in the repository or its git
  history, and not recoverable — see "The examples pass their gates" above.
  Recorded at generation time from ADR-2609221900 onward; the existing six
  predate it and can only be attributed by regenerating them.
- **Repair beyond single-token bugs.** The brownfield trial injected one wrong
  token per file. Multi-file changes are untested.
- **"44 of 110 specs described deleted features."** *Withdrawn 2026-09-22.* The
  corpus was never in this repository: `docs/specs/` has no commit that ever
  added it, and this repository's history starts 2026-09-13, while the corpus
  belonged to the history before 2026-09-12 that was not carried over. A number
  whose evidence lives in a repository nobody can open is not evidence. The
  three places that carried it now cite the ADR-citation audit below, which is
  the same failure, in this repository, with a command.
