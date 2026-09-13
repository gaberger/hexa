# ADR-2609131341: Evidence is captured, not copied

**Status:** Accepted
**Date:** 2026-09-13
**Epoch:** hexa
**Drivers:** A day of running the loop on blacksheep. Three ADRs there carry a measured table — precision and recall over eight corpora — and every one of those tables was produced by a test, read off a terminal, and typed into the ADR by hand. The commit messages carry the same numbers, typed a second time. One of them was wrong for twenty minutes: a precision of 1.00 that was a convention for zero over zero, copied faithfully because copying does not check.

## Context

The loop has a gate: a command that must exit 0, recorded before the code and run before the
stage is marked done. The gate answers *does it pass*. It says nothing about *what it measured*,
and most decisions worth an ADR rest on a measurement — a precision, a grade, a timing, a count of
false positives by path. Today that measurement reaches the ADR by a person reading stdout and
writing prose, so the ADR records what the author believed the tool said.

`hexa loop stage done` is the moment the work is declared finished and the ADR is the record a
reviewer reads. Nothing at that moment runs anything.

## Decision

1. **`hexa loop evidence '<command>'` records an evidence command** beside the gate. It is any shell
   command; its stdout is the evidence. `cargo test --test instrument -- --ignored --nocapture` is
   one; `hexa analyze . --json | jq .grade` is another.

2. **`hexa loop stage done` runs the evidence command and appends its output to the ADR.** Under a
   `## Evidence` heading, added once if absent: the command, the short commit it ran at (or
   "uncommitted" when the tree is dirty), the UTC time, and the stdout in a fenced block. Every
   `done` appends again, so the section is a history, oldest first.

3. **A failing evidence command refuses `done`.** Evidence comes from a run that succeeded; a
   non-zero exit leaves the ADR untouched and the stage where it was, with the command's stderr in
   the error.

4. **The ADR is the only destination.** Not the commit message, not the loop file. A commit that
   wants the number cites the ADR.

## Consequences

- The number in the decision record is the number the tree produced, byte for byte. Copying
  disappears and so does the class of error it makes.
- An evidence command that prints noise prints noise into the ADR. The command is the place to
  filter — `| sed -n '/^## /,$p'` — and the ADR's section will show whether that was done.
- `done` becomes slower by the length of the evidence run. That is the point: done means measured.
- The loop still does not run the gate at `done`; that is a separate decision about grades and
  floors and is not made here.

## Gate

`cargo test -p hexa-cli evidence`: in a temporary project, a recorded evidence command's stdout
lands under `## Evidence` in the ADR with the command and the time; a second `done` appends without
a second heading; a failing command returns an error and leaves the ADR byte-identical.

## Evidence

`cargo test -p hexa-cli evidence 2>&1 | grep -E 'loop_cmd::evidence_tests|test result: ok. 17'` at 9aa6acd with uncommitted changes on 2026-09-13 13:43 UTC:

```text
test commands::loop_cmd::evidence_tests::evidence_is_appended_to_the_adr_once_per_done_and_never_from_a_failing_run ... ok
test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 226 filtered out; finished in 0.01s
```
