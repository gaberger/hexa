# ADR-2609140929: playbooks are learned from runs, not guessed

**Status:** Proposed
**Date:** 2026-09-14
**Epoch:** hexa
**Drivers:** ADR-2609140844 shipped four playbooks written by hand in an afternoon. They are a guess about how work goes here. hexa records how work actually went, and reads none of it. Also: that ADR gave playbooks no verb of their own, so they can be routed to but not listed, shown, or edited.

## Context

ADR-2609140844 made `hexa hey` hand back a procedure. The four procedures it
hands back were written from first principles by one author in one sitting.
They are defensible and they are unvalidated. Nothing checks that the
`bug-fix` playbook resembles how a bug has ever actually been fixed in this
repository.

That is the spec problem in miniature, and the ADR that created it should say
so. A hand-written playbook is prose about a process. It is executable prose —
every step names a real verb and ends at a gate, which is more than a spec
managed — but its *ordering* and its *coverage* are still a guess. The tests
in `playbooks_are_executable.rs` prove a playbook is runnable. They cannot
prove it is right.

hexa has the evidence to close that. `~/.hexa/*.jsonl` holds what ran.
`.hexa/loop.json` holds the stages work passed through. The git log holds what
landed and in what order. ADR-2609140928 adds the decision trail, which is the
missing middle. Together those are a record of the actual verb sequences that
produced accepted work.

`pstack`'s `/automate-me` does this against chat transcripts: it mines how you
have actually worked and drafts a mode skill from it. The transcript is the
weakest possible source, because it records what was said. hexa's sources
record what ran and whether it passed.

There is a second, smaller gap in the same place. ADR-2609140844 added four
playbook files and no way to see them. `hexa hey` routes to a playbook;
nothing lists them, prints one, or validates one the operator wrote. The
assets are reachable only by guessing a phrase that routes to them.

## Decision

1. **`hexa playbook list` and `hexa playbook show <name>` exist.** The
   playbooks shipped by ADR-2609140844 become inspectable. This lands first
   and stands alone.

2. **`hexa playbook check <file>` validates a playbook the operator wrote,
   using the same rules the shipped ones are held to** — real verbs, at least
   one proof step, ends at the grade, three steps or more. The rules move out
   of the test file and into the binary, and the test calls them. One
   definition, two callers.

3. **`hexa playbook learn` drafts a playbook from recorded runs.** It reads
   the verb sequences that actually preceded accepted work and proposes the
   order they form.

4. **It drafts. It never installs.** The output is a file in `docs/` that the
   operator reads and moves into place. A tool that learns a procedure from
   history and then silently starts handing it out has closed a loop nobody
   asked it to close.

5. **A draft is validated before it is written, by decision 2's rules.** A
   draft that fails them is reported with the reason, not written. hexa does
   not emit a playbook it would reject from anyone else.

6. **It names its evidence.** The draft carries which runs it came from and
   how many. A draft from three runs and a draft from three hundred are
   different claims and must not look alike.

7. **Too little history is a result, not an error.** Below the threshold it
   says how many runs it found and how many it needs, and exits 0.

## Consequences

Decisions 1 and 2 are useful immediately and depend on nothing. Decision 3
depends on ADR-2609140928; without the decision trail, the learner sees verb
sequences but not why a step was taken, and would draft a procedure that
reproduces the shape of past runs including their mistakes.

Moving the validation rules from `playbooks_are_executable.rs` into the binary
is the risk in this ADR. A rule that lives where it is enforced cannot be
weakened by accident; a rule that lives in a library can be weakened by
changing the library. The test therefore keeps its own vacuity guards and
asserts the rule set is non-empty, so a hollowed-out validator fails the test
rather than passing everything.

A learned playbook will encode this repository's habits, including the bad
ones. That is the honest cost of learning from history and is why decision 4
keeps a person in the loop. The draft is a proposal about the past, not a rule
about the future.

`docs/playbooks/` joins the reader-facing tree, so drafted playbooks fall
under the existing doc tests that require every named verb to exist.

No new port, no new dependency. One verb group over assets hexa already
embeds and state it already writes.

## Implementation

The gate, written before the code, one line per decision that can be gated:

```
hexa playbook list | grep -q 'bug-fix'                 # decision 1
hexa playbook show bug-fix | grep -q 'hexa analyze'    # decision 1
hexa playbook check hexa-cli/assets/playbooks/bug-fix.json   # decision 2, exits 0
hexa playbook check <a playbook with no proof step>          # decision 2, exits non-zero
cd "$(mktemp -d)" && git init -q && hexa playbook learn      # decision 7, exits 0
```

The fourth line is the real gate for decision 2. A validator that accepts
everything passes the third line and fails the fourth, which is the shape the
`evidence_is_vacuous` guard exists to catch.

Phases:

1. `hexa playbook list` and `show`. Ships alone; closes the gap
   ADR-2609140844 left.
2. Move the validation rules into the binary; `playbooks_are_executable.rs`
   calls them and keeps its vacuity guards.
3. `hexa playbook check`.
4. `hexa playbook learn`, after ADR-2609140928 has landed trails to learn
   from.
5. `cargo check --workspace`, `cargo test --workspace`, `hexa analyze .`.

## References

- ADR-2609140844 — a routed intent returns a procedure, not a command. This
  ADR audits the playbooks that one created and gives them a verb.
- ADR-2609140928 — a run leaves a decision trail behind it. Decision 3
  depends on it.
- `hexa-cli/src/playbook.rs` — the loader and matcher.
- `hexa-cli/tests/playbooks_are_executable.rs` — the rules decision 2 moves
  into the binary.
- `hexa-cli/assets/playbooks/` — the four hand-written playbooks under audit.
- `cursor/plugins` `pstack`, `/automate-me`, MIT —
  https://github.com/cursor/plugins/tree/main/pstack
