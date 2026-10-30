---
id: ADR-2610301500
status: proposed
date: 2026-10-30
---
# ADR-2610301500: A dead end gives a guide, and the guide records the gap

**Status:** Proposed
**Date:** 2026-10-30
**Drivers:** On 2026-10-30, building ADR-2610301330 from a user project (ccs) through hexa's own gate-first pipeline, the agent hit four places where hexa's path did not work and had to go around the code. hexa said how in none of them. The user: "we need a better workflow for these things to make using hexa better. Hexa should offer guides when there are inconsistencies, that have to route around the code but document adding a PR into hexa for fixing".

| # | Where | What hexa said | What was done instead |
|---|---|---|---|
| 1 | `hexa spec draft` on a machine with no completion model registered | `Error: Network error: ollama transport error: builder error`, also with `--model us.anthropic.…` | The spec written by hand from the ADR only, then accepted by the user |
| 2 | `hexa spec tests` needs a model too, and nothing links tests written by hand | — | Each scenario's `test` and the spec's `red` set by editing the JSON in python |
| 3 | `hexa spec accept` after a behaviour change | "its tests, red commit and seal are dropped. Run `hexa spec tests`" | The model-only path again; the refs re-set by hand |
| 4 | A negative scenario guarding behaviour that already exists (S09: `hexa loop` keeps the detail) | `--prove` would call it a mirror | The test reshaped so it fails first |

Each was solved once, in one session, and is now forgotten. The next agent pays for each again.

## Context

hexa's errors say what failed. At a dead end the reader needs three more things: that it is a known gap in hexa and not their mistake, the route around it that keeps the pipeline's guarantees, and where the fix in hexa would go. Without the second, an agent improvises, and an improvised route can drop a guarantee silently (row 2 is one edit away from setting `red` to a commit where the tests do not fail). Without the third, the gap is never fixed: the user in a project is not in hexa's repository when it happens.

`hexa hey` (ADR-2610271308) already keeps a pure table checked against the binary. A table of guides can be checked the same way.

## Decision

1. **A guide is data.** `Guide { id, what, around, fix }` in `hexa-core/src/domain/guide.rs`: `what` names the inconsistency; `around` is the ordered route around it, each step a command or a file edit with what it must preserve; `fix` names the hexa files where the fix belongs. `guides()` is the table.
2. **A known dead end prints its guide.** Where one of the table's dead ends is reached, hexa prints the guide after the error: `what`, then `around` as numbered steps, then `fix`, then `hexa gap pr <id>`. The exit status does not change; a guide never turns a failure into a success.
3. **Printing a guide records the gap.** One line `{id, project, at}` is appended to `~/.hexa/gaps.jsonl`. `hexa gap list` shows each id with its count and the last project it was hit in, the most hit first. Nothing is sent anywhere.
4. **An agent that routes around hexa where no guide exists adds one.** `hexa gap add <id> --what "<…>" --around "<step>"… --fix "<path>"` records a guide in `~/.hexa/gaps.jsonl` as `unlisted`; `hexa gap list` shows it beside the table's. The CLAUDE.md template hexa installs says to do this, in the same place it says "build the verb".
5. **`hexa gap pr <id>` writes the pull request, not the fix.** It prints a PR title and body to stdout: the guide's `what` as the driver with its hit count and projects, `around` as what users do today, `fix` as the files to change, and, for an unlisted gap, the guide entry to add to `guides()`. It touches no network and no repository; the user or agent opens the PR.
6. **Every route around names real commands.** A test checks each `around` step that begins `hexa ` against the binary (`hexa <it> --help` succeeds), as ADR-2610271308 §2 does for routes. A guide whose route uses a removed verb fails the test, not a user.
7. **The first guides are this ADR's four rows.** `no-completion-model` (rows 1, 2): the error names that no completion model is reachable before naming the transport, and the route is to write the spec or the tests from the ADR only and have a person accept. `link-hand-written-tests` (rows 2, 3): set each scenario's `test`, commit the red tests, set `red`, then `hexa spec seal`. `guard-test-is-a-mirror` (row 4): a guard scenario's test also exercises the new behaviour, so it fails first. Each guide's `fix` names the verb that would retire it.

## Consequences

- A gap hit twice is visible as two hits and the projects they came from, so fixes are ranked by what users meet.
- A guide is retired by the fix it names: the PR that builds the missing verb removes the guide, and the test in §6 keeps the remaining ones honest.
- A route around that drops a guarantee is written down where it can be reviewed, instead of improvised in a session.

## Gate

`hexa spec verify ADR-2610301500`: the spec's sealed tests, each red before the code (ADR-2610221540 §5).

## References

- ADR-2610221540: the gate-first spec pipeline.
- ADR-2610271308: a pure routing table checked against the binary.
- ADR-2610301330: the work during which the four rows were found.
