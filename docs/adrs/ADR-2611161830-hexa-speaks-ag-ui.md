# ADR-2611161830: hexa speaks AG-UI

**Status:** Proposed
**Date:** 2026-11-16
**Drivers:** "hexa should follow this specification https://docs.ag-ui.com/spec/draft"
**Supersedes:** none. Builds on ADR-2611161800 (a plan ends on main), whose verbs are the first to speak it.

## Context

- AG-UI (Agent–User Interaction protocol, MIT, github.com/ag-ui-protocol/ag-ui) is an event stream from an agent (the producer) to an interface (the consumer). The draft, version 1.1, has 31 event types in eight families: runs and steps, text messages, tool calls, reasoning, state, activity, subagents, and passthrough. Only the run lifecycle is mandatory for a producer. Every other family is a feature a producer emits when it has something to say with it. Structure is the JSON Schema's (`spec/draft/schema.json`); ordering, lifecycle and error handling are the prose's.
- hexa's machine output today is a different JSON shape per verb: `status --json`, `agent list`, the hook lines, the run ledger. An interface that wants to watch hexa work (Lintel in harness, T3 Code, any AG-UI client) has to learn each shape and poll for it. Nothing streams, and nothing says when a run began, which phase it is in, or how it ended.
- hexa is one binary with no daemon and no HTTP server. The spec's two standard bindings are HTTP (SSE and protobuf). It also allows a custom transport that keeps the event model, the patterns and the processing rules, frames JSON "exactly as the SSE binding does — one event object per frame", and documents its framing, input and termination.

## Decision

hexa is an AG-UI producer. Every verb that does work can report it as an AG-UI 1.1 event stream.

1. **Transport: JSON lines, as a documented custom binding.** `--events` (or `HEXA_EVENTS=1`) makes a working verb write its events to stdout, one event object per line, and nothing else. Without the flag, output is as today, for a person. `--events=<path>` writes the stream to a file and leaves stdout to the person. The binding:
   - **Framing:** one JSON object per `\n`-terminated line.
   - **Input:** the verb's arguments are the run input. A `RunAgentInput` read from stdin comes later and is out of scope here.
   - **Termination:** the stream ends after the terminal event, then the process exits. A process that dies before one leaves a truncated run, which a consumer must not read as success.
   - **Rejected input:** bad arguments are refused before `RUN_STARTED`, on stderr, with a non-zero exit, as the binding contract asks.
2. **A verb's work is one run.** It starts with `RUN_STARTED` (`threadId`, `runId`, `protocolVersion: "1.1"`) and ends with exactly one of these:
   - `RUN_FINISHED` with `result`, when the verb did its job;
   - `RUN_FINISHED` with outcome `cancelled`, when it was stopped on purpose (Ctrl-C);
   - `RUN_ERROR` with `message` and `code`, when it failed. A failing gate is `RUN_ERROR` with code `gate_failed`. A refusal is `RUN_ERROR` with code `refused` and the refusal's text.

   The ids:
   - `threadId` is the loop session (`loop_cmd::session_id`), so one session's runs form one thread.
   - `runId` is new for every run and never reused.
3. **Phases are steps.** Each verb names its phases and opens and closes them as `STEP_STARTED` and `STEP_FINISHED`, balanced, all closed before the run ends. For `do gate` they are `gate` (which gates and commits in one) and `land`; for `loop stage done`, `close-out`; for harden, each hunt and fix.
4. **The loop is the state.** `STATE_SNAPSHOT` at the start of a run carries the loop: `adr`, `gate`, `stage` and `checklist`. Changes during the run are `STATE_DELTA` RFC 6902 patches against it, such as a task checked off or the stage moved.
5. **Progress outside the transcript is activity.** `hexa finish` emits its open-work list as `ACTIVITY_SNAPSHOT` with `activityType: "hexa.open_work"`. Long verbs report their progress as `hexa.*` activities, such as tests counted and files landed.
6. **Agents hexa starts are subagents.** harden's and build's reviewers and candidates are `SUBAGENT_STARTED` (with `name` and `description`), then `SUBAGENT_FINISHED` or `SUBAGENT_ERROR`. Their events carry `subagentRunId`.
7. **Usage is the spend ledger's.** `usage` on the terminal event carries one `TokenUsage` per provider and model, under the spec's accounting: cached and reasoning tokens are parts of the totals, never additions, and an absent count is never sent as zero.
8. **The rules a producer must keep are enforced in one place.** A single emitter type owns the stream, and no verb writes event JSON itself. It refuses an event before `RUN_STARTED` or after the run closes, a step opened twice or closed unopened, and an optional field sent as `null`. When a verb returns early, it closes any open steps and ends the run itself, `RUN_ERROR` on an error path and `RUN_FINISHED` otherwise. The domain holds the event types and the lifecycle rules, with no I/O. A port is the sink. An adapter writes the lines.
9. **The schema is the oracle.** The draft's `schema.json` is vendored, with its source and licence in PROVENANCE.md. Tests validate every event hexa emits against it with the `jsonschema` crate, a dev-dependency only. The tests check hexa against the protocol's own definition, not against our reading of it. When the draft is frozen, the vendored copy and `protocolVersion` move to the frozen version together.

## Consequences

- Any AG-UI consumer can watch hexa work live: Lintel, T3 Code, CopilotKit and the like. No per-verb JSON has to be learned.
- The existing `--json` outputs stay, because they are answers to queries, not runs. A query verb with `--events` is a run whose `result` is that same JSON.
- hexa follows a draft. The draft can change, so the vendored schema test is what says when hexa no longer matches. `protocolVersion` says `1.1`, the draft's version, until a frozen version is published.
- Text messages, tool calls and reasoning are not emitted: hexa does not hold a conversation, and the agent it runs inside reports its own. The families hexa has something to say with are runs and steps, state, activity and subagents.
- One more dev-dependency (`jsonschema`). Nothing is added to the shipped binary's dependencies.

## Implementation

**Gate:** `cargo test -p hexa-core agui_ && cargo test -p hexa-cli --test hexa_speaks_ag_ui`

Tests come first:

1. `agui_` (hexa-core, domain):
   - every event type hexa builds validates against the vendored schema;
   - an optional field left unset is absent, never `null`;
   - an event before `RUN_STARTED`, a duplicate step, an unopened step close and an event after close are each refused;
   - finishing with a step open closes it before `RUN_FINISHED`, and an error does the same before `RUN_ERROR`.
2. `hexa_speaks_ag_ui` (CLI):
   - `hexa finish --events` in a temporary repository prints only JSON lines;
   - the first line is `RUN_STARTED` with `protocolVersion` `1.1` and the last is `RUN_FINISHED`;
   - one `ACTIVITY_SNAPSHOT` of type `hexa.open_work` lists the open items;
   - every line validates against the schema;
   - `hexa do gate --events` with a failing gate ends in `RUN_ERROR` with code `gate_failed`, with steps `gate` opened and closed before it.

Slices, in order:
1. The domain types, the emitter and the JSON-lines adapter, with the `agui_` tests.
2. `finish` and `do gate` speak it.
3. `loop stage done`, harden and build: steps and subagents.
4. Usage from the spend ledger.

## Amendment (2026-11-19): every verb is a run

**Reported by:** the strata lead. In 26.11.13, `HEXA_EVENTS=<path> hexa analyze .` and `--events=<path>` left an empty file, and `--events` printed no lines. `loop`, `spec verify` and `do` were silent too.

**Cause:** slice 2 opened a run only inside `finish` and `do gate`. `main` turned the sink on for every verb, but only those two verbs ever sent `RUN_STARTED`. `close` ends only a run that is open, so every other verb wrote nothing. The CLI tests covered only those two verbs, so nothing failed.

**Now:**

- **Every verb is a run.** `main` opens it as soon as events are on, with one step named for the verb's path (`analyze`, `loop task add`, `spec verify`, `do gate`), closed when the run ends. A verb that opens its run itself (`finish`, `do gate`) joins this one. Its own steps, such as `gate` and `land`, nest inside the verb step.
- **An early exit still ends the run.** A verb that leaves through `process::exit` now goes through `events::exit`. That ends the open run first: `RUN_FINISHED` on status 0, otherwise `RUN_ERROR` with code `exit_status`. Before, `analyze --exit-code` and similar exits skipped `main` and left a truncated stream.
- **`hook` ignores `HEXA_EVENTS`.** Claude Code reads a hook's stdout, and `HEXA_EVENTS=1` exported in the agent's shell used to move that output to stderr. An explicit `--events` on a hook still works.

**Gate:** `cargo test -p hexa-cli --test hexa_speaks_ag_ui`. It runs `analyze`, `analyze --exit-code`, `loop`, `loop task add`, `spec verify`, `do runs` and `do gate` with each of the four ways of turning events on: `--events`, `--events=<path>`, `HEXA_EVENTS=1` and `HEXA_EVENTS=<path>`. For each run it checks:

- every line validates against the vendored schema;
- the run has exactly one `RUN_STARTED` and one terminal event that matches the exit status;
- the verb step is opened and closed inside the run;
- every event carries a timestamp;
- zero events fails the test.

On 96b64b5 it was 8 failed and 2 passed; with the fix, 10 pass.

## References

- AG-UI draft specification: https://docs.ag-ui.com/spec/draft (schema: `/spec/draft/schema.json`; transports: `/spec/draft/basic/transports`; lifecycle: `/spec/draft/events/lifecycle`).
- ADR-2611161800: a plan ends on main. `finish` and `do gate` are this ADR's first producers.
