# ADR-2609140844: a routed intent returns a procedure, not a command

**Status:** Accepted
**Date:** 2026-09-14
**Epoch:** hexa
**Drivers:** A comparison against Cursor's `pstack` plugin. `hexa hey` classifies a plain bug-fix request as Unknown, then recommends `hexa brain enqueue`, a subcommand that does not exist. Behind that bug sits a larger gap: hexa has two gates at the end of a task and nothing at the start that fixes the order of the work.

## Context

hexa's front door for natural language is `hexa hey`. It is a dispatcher.
`classify_intent` in `hexa-cli/src/commands/hey.rs:178` is roughly three
hundred lines of keyword arms, and every arm returns one command:
`TaskIntent::HexCommand`, `TaskIntent::Shell`, `TaskIntent::Workplan`, or
`TaskIntent::Unknown`. Text goes in, one command string comes out.

Two defects follow from that shape.

**The miss path names a command that does not exist.** Run `hexa hey "fix a
bug where the scroll drifts"`. The keyword classifier misses. The local model
fallback misses. The command then prints:

```
hexa brain enqueue hexa-command -- "<your-command>"
```

`hexa brain` is not a subcommand. `hexa brain` exits with `error:
unrecognized subcommand 'brain'`. The strings are hardcoded at
`hexa-cli/src/commands/hey.rs:543` and `:548`. This breaks the project's own
standing rule — never recommend a command that is not in `hexa --help` — in
the one command whose entire job is to recommend commands. It is the same
family as ADR-2609122048: a tool answering confidently without having done
the work.

**Even a hit returns the wrong kind of answer.** "Fix this bug" is not one
command. It is an ordered procedure: reproduce it, find the root cause, write
the gate, make the fix, prove the fix, commit. hexa already owns a verb for
every step — `hexa do run`, `hexa verify`, `hexa harden`, `hexa analyze`,
`hexa loop gate`. Nothing binds them into an order, so the order lives in
CLAUDE.md as prose, and prose cannot fail. An agent that skips a step is not
detected until a gate at the very end, if a gate is reached at all.

The gap is visible from outside. Cursor's `pstack` plugin (MIT,
`cursor/plugins`) routes a request to one of twenty-three named playbooks and
copies that playbook's steps **verbatim** into the agent's to-do list before
any work starts. It refuses to paraphrase them, because paraphrase is where
drift enters. `grep -rl playbook hexa-cli/src hexa-exec/src docs/` returns
nothing. hexa has no such concept.

pstack cannot enforce its playbooks. They are markdown, read by a model that
may skip a page. hexa is in the opposite position: every step of a hexa
procedure can end in a command that exits 0. The routing idea is the part
worth taking. The enforcement is the part hexa can add.

## Decision

1. **A classified intent returns a named playbook and its ordered steps, not
   a single command.** `hexa hey` prints the playbook name and the steps. The
   command it would have run becomes step one of several, or the whole
   playbook when the task really is one command.

2. **A playbook is data, not prose.** Playbooks ship as an embedded asset
   family under `hexa-cli/assets/`, beside the existing skills, one file per
   playbook. Each step carries the verb it runs and the check that proves the
   step finished.

3. **The steps are emitted verbatim.** The tool copies them. It does not
   summarise, reorder, or rewrite them to fit the request.

4. **Every playbook ends at the two gates.** A playbook whose final step is
   not an evidence command plus `hexa analyze .` is invalid, and a test
   refuses it. This is what keeps a playbook from becoming the spec problem
   in a new hat: each step names a command, so the playbook is executable
   rather than descriptive.

5. **No match is a result, not an error.** When nothing matches, `hexa hey`
   says which playbooks it considered and exits 0. Absence of a match is an
   answer.

6. **A recommended command must exist.** Every command string the CLI prints
   as a suggestion resolves against the clap command tree, checked by a test.
   No suggestion escapes to a user unless the binary can run it.

The first playbook set is four, not twenty-three: `bug-fix`, `feature`,
`refactor`, `investigation`. Breadth is cheap to add later and expensive to
get wrong now.

## Consequences

The output shape of `hexa hey` changes. Any consumer parsing a single command
line breaks. Nothing in the repository does; the only references are prose in
`hexa-cli/assets/templates/claude-md-hexa-section.md:7` and `:43`.

A playbook file becomes a new thing that can drift away from the verbs it
names. Decisions 4 and 6 are the guard, and they are tests, not review.

`hexa go` and `hexa loop` overlap with this and must stay separate. A
playbook says how to carry out one task. `hexa loop` says where the project
stands across tasks. Merging them would put project state inside a per-task
procedure.

No new port, no new adapter, and no new dependency. This adds an asset family
and a loader, and changes one command's return type.

The `hexa brain` fix lands on its own and improves `hexa hey` immediately,
whether or not the rest of this ADR is accepted.

## Implementation

The gate comes before the code and is not derived from it:

```
hexa hey "fix a bug where the scroll drifts" | grep -q "playbook: bug-fix"
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

A second gate covers decisions 4 and 6, as a workspace test: every step of
every embedded playbook names a verb that exists in the clap tree, and every
playbook's last step is a gate. A playbook set of zero passes that test
vacuously, so the test asserts a non-empty set — `evidence_is_vacuous` in a
new shape.

Phases, in order:

1. Remove the `hexa brain` recommendation at `hexa-cli/src/commands/hey.rs:543`
   and `:548`. Replace it with the playbook-miss message from decision 5.
   Ship alone.
2. Add the test from decision 6: no suggestion string is unknown to clap.
3. Add the playbook asset family and its loader, with the four playbooks.
4. Change `classify_intent` to resolve to a playbook, trying playbook match
   before the model fallback.
5. `cargo check --workspace`, `cargo test --workspace`, `hexa analyze .`.

## References

- `hexa-cli/src/commands/hey.rs:178` — `classify_intent`, the keyword arms.
- `hexa-cli/src/commands/hey.rs:543`, `:548` — the `hexa brain` recommendation.
- `hexa-cli/assets/templates/claude-md-hexa-section.md:7`, `:43` — the only
  consumers of `hexa hey`.
- ADR-2609121400 — hexa is a scaffolding system with two gates.
- ADR-2609122048 — a tool that reports "nothing found" must prove it looked.
  Same family of defect.
- `docs/COMPARISON.md` — where hexa sits against prompting, spec-driven, and
  test-driven approaches.
- `cursor/plugins` `pstack`, MIT — https://github.com/cursor/plugins/tree/main/pstack
- `hexa hey --help` cites ADR-2026-04-14-0000, which predates this ledger's
  epoch and is not in `docs/adrs/`.
