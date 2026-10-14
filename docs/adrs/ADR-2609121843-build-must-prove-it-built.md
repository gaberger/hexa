# ADR-2609121843: `hexa build` must prove it built something

**Status:** Accepted
**Date:** 2026-09-12
**Drivers:** a `hexa build` run on an existing green repository reported `✓ 3 designs → 3 critiques → spec 53ch → build GREEN` having changed not one file. The gate passed because it was already passing. Observed on blacksheep (github.com/gaberger — the Diffy re-implementation), phase 2, 2026-09-12.

## Context

`run_build` (hexa-exec/src/adversarial.rs) ends like this:

```rust
if let Err(e) = claude_run_retry(&build_prompt, repo_root, …).await { report.notes.push(…); }
let (ok, _) = crate::direct_exec::run_evidence(gate, repo_root).await;
report.build_ok = ok;
```

`build_ok` is the gate's exit code and nothing else. On a greenfield target that is sound: an
empty directory cannot pass `cargo test`. On an EXISTING repository whose suite is already green
— which is every `build` after the first — the gate answers a question that was already answered.
A build agent that edits nothing, errors out, or writes a file and reverts it all produce
`build GREEN`.

The run that exposed it also shows the second half of the problem. `report.spec_chars = spec.len()`
records a **53-character** synthesized spec and then hands it to the build agent as the spec. Fifty
three characters is one sentence; it cannot describe a public API, a data model and a test plan.
The value is measured, printed, and never judged.

The only reason the failure was visible at all is incidental: `commit_result` found nothing to
commit and logged `git commit failed (non-fatal):` with an empty error. That line is the one true
statement in the run, and it is marked non-fatal.

This matters more than a bad run. hexa's claim is that a gate cannot lie because it has an exit
code (ADR-2609121400). A gate that is evaluated against work that never happened lies exactly the
way a spec does, and for the same reason: nothing connected the artifact to the claim.
`docs/guides/development-workflow.md` already says "A gate that passes having run nothing is
rejected. `hexa do` and `hexa scaffold` both check for a vacuous pass." `build` does not, and the
sentence reads as though it does.

## Decision

1. **A build must change the tree.** `run_build` captures `git rev-parse HEAD` and the working-tree
   status before the build agent and compares after. No change to tracked files ⇒ `build_ok = false`
   and a note naming it: "the build agent changed no files; the gate was already green". The same
   check covers an agent that edits and reverts.
2. **A degenerate spec fails before the build agent runs.** A synthesized spec under
   `MIN_SPEC_CHARS` (600, about a paragraph) is a failed synthesize, not a spec: return the report
   with a note, do not spend a build call on it. The existing `synthesize failed` branch gets a
   sibling.
3. **The summary line reports what happened.** `✓ N designs → N critiques → spec Nch → build GREEN`
   becomes `… → build GREEN (12 files changed)` or `… → build FAILED (no files changed)`. The count
   is the evidence the line currently implies and does not have.
4. **A failed commit is not non-fatal when the build claimed success.** If `build_ok` is true and
   `commit_result` finds nothing to commit, that is the contradiction in 1 arriving by another
   route; it fails the run.

## Consequences

- `hexa build` on an existing repository can no longer report success for doing nothing. This is
  the case that matters, because after the first build every build is that case.
- A wasted build call is saved whenever synthesize degenerates.
- The exit code of `hexa build` becomes usable in CI, which is what the two-gate claim promises.
- One behaviour change for users: a build whose only effect is to satisfy an already-green gate now
  fails. That is the point.

## Implementation

- `hexa-exec/src/adversarial.rs`: `MIN_SPEC_CHARS`; a `tree_fingerprint(repo_root)` helper (HEAD +
  `git status --porcelain` digest) called either side of the build agent; `report.files_changed`;
  the spec-length guard; `build_ok` conjoined with "something changed".
- `hexa-cli/src/commands/build.rs`: print the file count in the summary line.
- Tests: a fixture repo with a passing gate and a build agent stub that writes nothing ⇒
  `build_ok == false`; a stub that writes a file ⇒ true; a 53-character spec ⇒ the build agent is
  never invoked.

## References

- hexa-exec/src/adversarial.rs (`run_build`, phases 3 and 4; `report.spec_chars`)
- docs/guides/development-workflow.md ("A gate that passes having run nothing is rejected")
- ADR-2609121400 (two gates; a document that cannot fail cannot be trusted)
- Observed 2026-09-12 on blacksheep phase 2: spec 53ch, 0 files changed, `build GREEN`.

## Status at acceptance (2026-10-14)

Checked against `main` at 43f5c5c. The decision still stands. No later ADR supersedes or contradicts it, and ADR-2609132059 ("done means the gate ran") points the same way. **None of it is built yet.**

| Item | State | Where |
|---|---|---|
| 1. A build must change the tree | **Not built** | `hexa-exec/src/adversarial.rs:1075-1076`: `build_ok` is still the gate's exit code alone. There is no `tree_fingerprint` and no `files_changed` |
| 2. A spec under `MIN_SPEC_CHARS` fails before the build agent | **Not built** | `adversarial.rs:1059`: `spec_chars` is recorded and never judged. There is no `MIN_SPEC_CHARS` |
| 3. The summary line counts the files changed | **Not built** | `hexa-cli/src/commands/build.rs:138` prints only `GREEN` or `FAILED` |
| 4. A claimed success with nothing to commit fails the run | **Not built** | `adversarial.rs:1078-1096`: `commit_result` still records "nothing new to commit" as a non-fatal note |

No test holds any of the four yet.

**Remaining work**, to queue: everything under Implementation. That is the tree fingerprint around the build agent, `MIN_SPEC_CHARS`, the file count in the summary line, the fatal empty commit, and the three stub tests.

