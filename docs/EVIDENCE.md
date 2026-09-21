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

Expected: `Architecture grade: A+ — score 100/100`, `0 boundary violations`, and
`score_components` all zero in `hexa analyze . --json`.

The scan covers every crate. The analyzer has no built-in knowledge of hexa's
directory names. The one path hexa excludes is `hexa-cli/assets/scaffold`, the
template files compiled into the binary, and it declares that in
`.hexa/project.json` under `analyze.exclude`, the same way any project would.

`hexa-cli/tests/scaffold_is_executable.rs::a_violation_planted_in_any_crate_is_seen`
copies each crate, plants a domain-imports-adapter violation in it, and asserts
the analyzer names the file and exits 1. This exists because an earlier build of
the analyzer excluded `hexa-core/` and `hexa-cli/` by name, so hexa graded itself
over six of eight crates and reported A+.

## Every detector reads all three languages, or says it does not

```bash
for lang in rust go ts; do
  hexa init /tmp/probe-$lang --scaffold --lang $lang
  (cd /tmp/probe-$lang && hexa analyze .)
done
```

Expected, for each language: `Architecture grade: A+ — score 100/100`,
`0 boundary violations`, `dead layers 0`, `orphans 0`. For Go and
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
  checked, whether or not they have an import line (see "A reference is an
  import" above). What remains unseen is code generated by a macro from an
  allowed crate, reflection, and FFI.
- **Repair beyond single-token bugs.** The brownfield trial injected one wrong
  token per file. Multi-file changes are untested.
- **"44 of 110 specs described deleted features."** The figure appears in
  `README.md`, `CLAUDE.md` and `docs/COMPARISON.md`. The corpus it was counted
  over is not in this repository, and no script or analysis here re-derives it,
  so nothing on this page checks it. It needs a source naming the project, the
  commit, and how "already deleted" was determined — or the sentence needs to
  stop carrying a number.
