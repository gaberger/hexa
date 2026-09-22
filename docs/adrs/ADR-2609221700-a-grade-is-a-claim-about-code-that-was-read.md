---
id: ADR-2609221700
status: accepted
date: 2026-09-22
---
# ADR-2609221700: A grade is a claim about code that was read

**Status:** Accepted
**Date:** 2026-09-22
**Drivers:** `hexa analyze /path-that-does-not-exist` printed **A+ — score 100/100** and exited 0. So did `--grade A`, `--strict`, `--exit-code` and `--json`. An empty but existing directory did the same. Found while testing `hexa loop` and `hexa memory` by hand on `main` at `19b3487`.

## Context

`analyze::run` resolved its root with `canonicalize().unwrap_or_else(|_| path)`. A path that does not exist failed to canonicalize, the fallback handed back the literal path, and everything downstream carried on against a directory that was not there. Nothing errored. Zero files were scanned, so zero findings were made, and a score computed from zero findings is a perfect one:

```
⬡ Architecture analysis: /nonexistent-path-xyz
    ‣ 0 source files scanned
  ⬡ Architecture grade: A+ — score 100/100
```

The "0 source files scanned" line was printed directly above the A+, which is the tell — and it did not stop anything. `--grade A` exited 0. A CI job running `hexa analyze . --grade A` from the wrong working directory, or after a checkout that produced nothing, passed with full marks and no way to tell from the exit code.

This is the failure this project names as the worst kind, and names in three places:

- README and `CLAUDE.md`: *a gate that degrades silently is worse than no gate* — recorded about `hexa ci` falling back to `cargo check` when the daemon was down, which quietly turned "no boundary violations" into "it compiles" while still printing a result.
- *A vacuous gate is a failed gate.* `evidence_is_vacuous` already rejects "running 0 tests" for the do-loop's evidence command. The same shape in `analyze` was not guarded.
- ADR-2609122048: a tool that reports "nothing found" must prove it looked.

`analyze` is the grade in the README, the gate in `hexa build`, and the floor the scaffold has to clear. It is the one number this project asks to be judged on, so it is the worst place for a result that is perfect because nothing was examined.

A second, smaller finding from the same session: `main` returned `anyhow::Result<()>`, so Rust's `Termination` printed errors with `Debug`, and anyhow's `Debug` appends a backtrace whenever `RUST_BACKTRACE` is set. hexa's users are Rust developers, who commonly export that globally. An ordinary refusal — "no ADR-X in docs/adrs/; write it first" — arrived with a stack trace under it and looked like a crash. The message and the exit code were always correct; only the presentation was wrong. It is recorded here because it was found the same way and has the same cost: a tool that looks broken while working teaches people to distrust what it says.

## Decision

1. **A path that does not exist is an error.** `analyze` resolves its root before anything else and refuses a path that is absent, naming it. A path that exists but is not a directory is also refused, and points at `--file`.

2. **A scan that read no files produces no grade.** If no source files are found under the root, `analyze` refuses rather than reporting. The message says that there is nothing to grade and why a grade would be wrong. This applies to every surface at once, because the guard sits above the branch into `--json`: human output, `--json`, `--grade`, `--strict` and `--exit-code` all fail together, which is the property whose absence made the original bug survivable.

   `--file` mode is exempt: it analyses the one file it was given and validates it itself.

3. **An expected refusal prints a sentence, not a stack trace.** `main` no longer returns `Result`. It calls an inner `run()`, and on an error prints `Error: {:#}` — the whole cause chain, one line, no backtrace — and exits 1. A genuine panic is a different path and still reports as one.

## Consequences

- **A CI job that was passing may now fail, correctly.** Any job whose working directory is wrong, or whose checkout produced nothing, was passing `--grade A` against an empty scan and now exits 1. That is the bug being fixed, not a regression, and it is the one case where this change is visible.
- **`hexa analyze` on a docs-only or config-only repository now fails.** There is no source to grade there. Reporting A+ for it was never meaningful.
- **Exit code 1 now covers both "the tree failed the check" and "there was no tree to check".** These are distinguishable by the message, not by the code. Splitting them into separate codes would be a second decision with a compatibility cost, and no caller has asked for it.
- **Nothing about the score itself changes.** This ADR does not touch how a grade is computed; it decides when computing one is meaningless.

## Implementation

- `hexa-cli/src/commands/analyze.rs`: `resolve_root` (Decision 1) and `refuse_an_empty_scan` (Decision 2), called from `run` before the `--json` branch and after the `--file` branch.
- `hexa-cli/src/main.rs`: `main` split into a thin wrapper and `run` (Decision 3).
- `docs/UPGRADING.md`.

**Gates**, both written first and shown failing against `19b3487`:

- `cargo test -p hexa-cli --test an_empty_analysis_is_not_an_a_plus` — 10 cases, **8 failed** before. The 2 that passed are the controls: hexa's own tree and a one-file project must still grade, because refusing an empty scan must not start refusing real ones.
- `cargo test -p hexa-cli --test a_refusal_is_not_a_crash` — 4 cases, **2 failed** before, run with `RUST_BACKTRACE=1` forced on, across two different error paths so it pins the shared exit rather than one verb.

## References

- ADR-2609122048: a tool that reports "nothing found" must prove it looked. This is that rule applied to the grade itself.
- ADR-2609211430: the grade's components, unchanged here.
