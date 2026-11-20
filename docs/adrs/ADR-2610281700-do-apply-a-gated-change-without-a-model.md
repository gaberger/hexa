---
id: ADR-2610281700
status: proposed
date: 2026-10-28
---
# ADR-2610281700: `hexa do apply`, a gated change without a model

**Status:** Proposed
**Date:** 2026-10-28
**Drivers:** A project that routes every source change through hexa (parley's D1) pays for a model on edits that need no judgment. Moving a function above a test module, adding a type alias, or labelling a file in `.hexa/project.json`: the change is already known exactly, yet `hexa do run` spends a Claude delegate and many minutes writing it. `--fast` is no cheaper in practice: its single-shot edit appends, and on a change in the middle of a file it appended the whole file to itself (gary/hexa#18). The discipline people want from hexa is the gate, the commit only on green and the revert on red, not the model.

## Context

`hexa do run` (hexa-exec `direct_exec.rs`) has three parts. An agent writes an edit. `run_evidence` runs the command. `commit_with_also` commits the listed files on exit 0, or restores them on failure, and `record_run` logs the run. Only the first part needs a model.

## Decision

1. **`hexa do apply "<description>" --file F [--also G ...] --evidence "<cmd>" [--patch P] [--json]`** applies a unified diff, read from `--patch P` or else from stdin, with no model and no inference of any kind. It runs the evidence and commits only on exit 0, exactly as `do run` gates its edits.
2. **Only the listed files.** Every path the diff touches, created, modified or deleted, must be `--file` or an `--also`. Otherwise it is refused before anything changes. It exits with status 2 and names the path.
3. **It applies whole or not at all.** `git apply --check` must pass first. A diff that does not apply is refused with exit 2: nothing changes and no evidence runs. A listed file with uncommitted changes is refused the same way, so a user's work is never mixed into, or reverted by, the run.
4. **Revert on red, exactly.** When the evidence fails, every touched file is restored to its committed content, and every file the diff created is deleted. The tree is as it was. The exit status is 1.
5. **Commit on green.** The listed files are committed with the message `feat(apply): <description>`, using the same identity and path-limited commit as `do run` (`commit_with_also`). The exit status is 0. The run is recorded like a `do run` (`record_run`, with model `none`), so `hexa do runs` lists it.
6. **`--json`** prints the same one result object as `hexa do run --json`: `{"ok", "commit", "attempts": 1, "code", "message"}`. `code` is `gate_failed` on red and `config` on a refusal.

## Consequences

- A known, mechanical change costs a `git apply` and the evidence, not a model call, and still gets hexa's gate, commit and record.
- `do run` stays for changes that need judgment.
- The revert of §4 is the behaviour gary/hexa#18 asks of `do run`; `apply` has it from the start.

## Gate

Written first and shown failing before the code:

- `cargo test -q -p hexa-cli --test adr_2610281700_spec` — `hexa do apply` against a fresh repository: green commits, red restores and deletes, refusals change nothing, no model is ever called.
