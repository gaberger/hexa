---
id: ADR-2610301330
status: proposed
date: 2026-10-30
---
# ADR-2610301330: The status bar gets a glance, not a paragraph

**Status:** Proposed
**Date:** 2026-10-30
**Drivers:** A user, on 2026-10-30, about the Claude Code status bar in a hexa project: "the status line looks like a giant runon sentence". The template `hexa init` installs runs `hexa loop 2>/dev/null | head -1`, so the bar shows the first line of the full report. With nothing recorded that line is advice, `⬡ Loop (ccs): nothing recorded. Decide → Gate → Build → Harden. Record with `hexa loop adr <ID>` and `hexa loop gate '<command>'`.`. With a gate recorded it is the gate command verbatim, pipes and all: `⬡ Loop (ccs): stage gate · ADR none · gate cargo test --test domain compose_status 2>&1 | grep -E "test result: ok. [1-9]"`.

## Context

`status_line` (`hexa-cli/src/commands/loop_cmd.rs`) writes one line for two readers. The hooks print it into the agent's context at session start and on a feature-sized prompt, where the full gate command and the "Record with …" advice are what the agent needs. The status bar is redrawn on every turn and read at a glance, beside the model, the directory and the branch; there the same text is a sentence to be read. `head -1` was the only way to get one line, because `hexa loop` has no one-line form.

`loop.json` stores no result of the gate's last run, so the bar cannot say whether the gate passes without running it on every redraw.

## Decision

1. **`hexa loop line` prints one line of state for a status bar.** It reads the same `.hexa/loop.json` entry `hexa loop` reads, prints exactly one line, writes nothing, and exits 0, including when no loop is recorded.
2. **Nothing recorded is `▸ loop · idle`.** No advice and no command names. The hooks still print the advice.
3. **A recorded loop shows its stage as a position.** `▸ loop · decide › gate › build › harden`, the current stage in capitals (`decide › GATE › build › harden`). Stage `done` prints `▸ loop · done` instead of the track. After it, each only when present: the ADR id, the checklist as `done/total` (`2/5`), and a harness run as `running <verb>/<phase> <elapsed>` or `stalled <verb>/<phase>` (no message).
4. **The line never carries text a person wrote.** Not the gate command, not the evidence command, not a task title, not the run's message. Those are in `hexa loop`. The line stays short whatever was recorded.
5. **The template's status bar runs `hexa loop line`.** `hexa-claude-settings.json` sets `statusLine.command` to `hexa loop line 2>/dev/null`, with no `head`. `hexa init` and the asset refresh already overwrite `statusLine` as hexa-managed, so existing projects pick it up on refresh. `hexa loop` and the hook output do not change.

## Consequences

- The bar is one short line in every hexa project; the agent still gets the full line from the hooks.
- A pass/fail mark for the gate is not shown. Showing one needs the gate's last result written to `loop.json`, which is a separate decision.
- A status bar that wraps `hexa loop line` (ccs's `statusline --auto`) gets the short line unchanged.

## Gate

`hexa spec verify ADR-2610301330`: the spec's sealed tests, each red before the code (ADR-2610221540 §5).

## References

- ADR-2609131408: the loop file, one entry per session.
- ADR-2609131427: the running line.
- ADR-2610221540: the gate-first spec pipeline.
