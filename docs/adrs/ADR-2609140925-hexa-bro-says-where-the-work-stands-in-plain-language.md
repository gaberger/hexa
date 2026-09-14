# ADR-2609140925: `hexa bro` says where the work stands, in plain language

**Status:** Accepted
**Date:** 2026-09-14
**Epoch:** hexa
**Drivers:** The operator, in their own words: "I get lost when we go too long in development." Every surface hexa prints is written for someone who already holds the thread. None of them hands it back.

## Context

hexa knows exactly where the work stands. It says so in a register that
assumes you never left.

`hexa loop show` prints one line: `stage harden · ADR ADR-2609140844 · gate
cargo test --workspace && cargo clippy --workspace --all-targets -- -D
warnings && hexa hey "…" | grep -q "playbook: bug-fix" · tasks 13/13`. Every
token in it is correct. It answers "what stage, which ADR, which gate, how
many tasks". It does not answer the question an operator actually returns
with, which is "what are we doing and did it work".

The rest is scattered. `hexa status` has the project. `hexa analyze` has the
grade. `hexa spend` has the cost. `hexa adr list` has the decisions. The git
log has what landed. `.hexa/loop.json` has the current gate. Six surfaces,
six formats, and the operator has to assemble the story themselves — which is
the task they came back unable to do.

This is not a documentation problem. The facts are all recorded and all
current. The gap is that nothing states them as a narrative, and nobody reads
six dashboards to re-enter their own project.

Cursor's `pstack` plugin ships `/bro`, which restates the last message in
plain human language with no jargon. It works on a chat transcript, which
hexa does not have. hexa has something better to work from: recorded state on
disk that cannot drift from what happened, because it *is* what happened.

## Decision

1. **`hexa bro` prints one short plain-language account of where the work
   stands.** What we are doing, why, what happened last, what is next, and
   whether anything is broken. Prose, not a table.

2. **It reads recorded state only.** `.hexa/loop.json`, the ADR that state
   names, the last recorded gate result, recent commits, and the current
   grade if one was recorded. It runs no gate, starts no agent, and spends no
   inference. `hexa bro` is safe to run when you are lost, which is when it
   will be run.

3. **It is honest about what it does not know.** If the gate has never run,
   it says the gate has never run. It never reports a stale grade as current
   without saying when it was taken. ADR-2609122048 governs this: a tool that
   reports a clean result must have looked.

4. **The register is controlled, and the controls are tested.** Sentences of
   at most 25 words. Active voice. Every hexa verb it names is backticked and
   exists. No grade, count or identifier appears without a sentence around
   it saying what it means.

5. **`hexa bro` never fails.** No recorded work is a result — "no work is
   recorded here" — and exits 0. Exit 1 is reserved for a question that was
   asked and answered badly.

The name is the operator's. It is the register that matters, and `bro` says
the register out loud: this is the surface that talks to you like a person.

## Consequences

A seventh surface joins the six. That is the cost, and it is the right one
only because the other six are each correct and each partial. `hexa bro`
adds no new state; it reads what is already there, so it cannot drift
independently.

The sentence-length and verb checks make prose testable, which is unusual and
is the point. It is the first place in hexa where the *writing* has a gate.
That check generalises: the playbook `done_when` lines from
ADR-2609140844 should eventually run under it too.

`hexa bro` becomes the thing an operator runs first. If it is wrong, it is
wrong in the most expensive place. That is why decision 3 is not optional.

No new port, no new adapter, no new dependency. One verb, one reader over
files hexa already writes.

## Implementation

The gate, written before the code and not derived from it:

```
hexa bro                                        # exits 0 in this repo
hexa bro | grep -q 'ADR-2609140844'             # names the live decision
cd "$(mktemp -d)" && hexa bro                   # exits 0 with no .hexa/ at all
cargo test --workspace
```

A second gate covers decision 4, as a workspace test over `hexa bro`'s own
output: no sentence exceeds 25 words, and every backticked `hexa …` chain
resolves against the clap tree — reusing the extractor in
`hexa-cli/tests/suggested_commands_exist.rs`. A run that produces no
sentences fails the test rather than passing it vacuously.

Phases:

1. Add the reader: one function per recorded source, each returning
   `Option<…>` so "not recorded" is distinct from "recorded as zero".
2. Add the narrator: recorded state to sentences. No inference.
3. Add the register test.
4. Wire `hexa bro` into the clap tree and the getting-started screen.
5. `cargo check --workspace`, `cargo test --workspace`, `hexa analyze .`.

## References

- ADR-2609140844 — a routed intent returns a procedure, not a command. The
  same operator-facing surface, one layer up.
- ADR-2609122048 — a tool that reports "nothing found" must prove it looked.
  Decision 3 is that rule applied to a status report.
- `hexa-cli/src/commands/loop_cmd.rs` — the one-line register this replaces
  for the returning operator, and keeps for the working one.
- `hexa-cli/tests/suggested_commands_exist.rs` — the verb extractor the
  register test reuses.
- `cursor/plugins` `pstack`, `/bro`, MIT —
  https://github.com/cursor/plugins/tree/main/pstack
