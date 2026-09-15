---
id: ADR-2609151100
status: accepted
date: 2026-09-15
supersedes: []
superseded_by: null
depends_on: []
components: []
modules: []
---

# ADR-2609151100: A lifecycle event is not a run

**Status:** Accepted
**Date:** 2026-09-15
**Drivers:** `hexa do runs` reported 62 runs, 5 passed, 57 failed, 8% pass. Five runs had ever happened, and all five passed.

## Context

`~/.hexa/agent-runs.jsonl` has two writers.

`hexa do run` appends one row per run, carrying `agent`, `instruction`,
`file`, `ok`, `evidence_passed` and `committed`. The Claude Code hook handler
(`hexa hook`, on `SubagentStart` and `SubagentStop`) appends one row per
subagent lifecycle event to the same file, carrying `kind: "subagent"`,
`event`, `agent_id`, `agent_type` and `ts`, and nothing else.

`runs_snapshot` mapped every row into a `DirectRun` field by field, defaulting
each missing field. A hook row has no `ok`, so it became `ok: false`. It has no
`instruction` or `file`, so it printed as a bare `✗ —` line. `runs_summary`
then counted it as a failed run.

The store had held every real run correctly since ADR-2026-06-04-1740. The
count was wrong from the first hook firing on 2026-09-12, and nothing failed,
because a summary over rows it cannot interpret still prints a confident
number. This is the silent-degradation shape named in the key lessons: the
gate kept reporting, and what it reported had stopped meaning anything.

## Decision

1. The mapping from log rows to runs is a named function, `runs_from_rows`,
   and it keeps only rows that carry an `agent` string. A subagent lifecycle
   row does not, and is dropped before display ids are assigned, so ids number
   only runs.
2. The counting is a named function, `summary_of`, over the filtered runs.
3. Both are gated by `runs_feed_tests` in `hexa-exec/src/direct_exec.rs`,
   which feeds a mixed log and asserts that hook rows contribute nothing to
   the list, the summary, or the ids.

The two writers keep sharing the file. Separating them would be cleaner, but
the reader must filter by shape regardless, because the log is append-only and
the hook rows already written do not go away.

## Consequences

- `hexa do runs` reports the runs that happened. On the log that triggered
  this ADR it now reports 5 runs, 5 passed, 100%.
- Any future writer that appends a non-run row to the runs log is invisible
  to the feed rather than counted as a failure. That is the correct default
  for a feed whose whole job is a pass rate.
- The gate is a unit test over rows, not over the live store, so it runs
  under `cargo test --workspace` with no fixture on disk.

## Implementation

- `hexa-exec/src/direct_exec.rs`: `runs_from_rows`, `summary_of`,
  `runs_feed_tests`. Landed at 293af9c via `hexa do run`, gated on
  `cargo test -p hexa-exec --lib runs_feed_tests`, which failed before the
  edit because the functions did not exist.

## References

- ADR-2026-06-04-1740 — the direct executor and its runs feed.
- ADR-2609132122 — a recorded gate is run or it is prose.
- `CLAUDE.md` § Key lessons — "A gate that degrades silently is worse than no gate."
