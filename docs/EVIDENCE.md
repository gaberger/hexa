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

Expected: `8 passed`. The tests scaffold twice and diff the bytes, run each
language's gate, plant a violation and assert each shipped rule fires, and
assert a clean scaffold trips none of them.

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

Expected: `8 passed`, including `the_score_never_climbs_as_violations_are_added`
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

Expected: `9 passed`. Planting a dead verb or a dead link fails it.

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

## Not yet validated

- **Localisation.** Given only a failing test name, hexa found the right file in
  1 of 10 trials. There is no command for it. The trial is in
  [`analysis/2609120300-brownfield-trial.md`](analysis/2609120300-brownfield-trial.md).
- **Third-party imports in `domain/`.** The analyzer does not check them. A
  project can import a runtime into its domain and score A+.
- **Repair beyond single-token bugs.** The brownfield trial injected one wrong
  token per file. Multi-file changes are untested.
