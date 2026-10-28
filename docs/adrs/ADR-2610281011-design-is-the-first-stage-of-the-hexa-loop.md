---
id: ADR-2610281011
status: proposed
date: 2026-10-28
---
# ADR-2610281011: design is the first stage of the hexa loop

**Status:** Proposed
**Date:** 2026-10-28
**Drivers:** ADR-2610271736 gave hexa the Elephant-Goldfish Model as five commands: `hexa design brainstorm|draft|review|resolve` and `hexa drift`. Nothing in hexa's own loop knows they exist. `hexa loop` records Decide → Gate → Build → Harden and has no design stage. `hexa go` never suggests a review or a drift check. `hexa hey` cannot route to either verb. The feature-sized prompt hook says "Decide (ADR …)", and the `/hexa-workflow` skill starts at the ADR. An approved design "enters hexa's pipeline as an ADR" (ADR-2610271736 §8), but nothing carries it there: the ADR is written by hand from a document the pipeline never reads again, and nothing checks that the design was reviewed in the form that became the decision. `hexa drift` checks every document every time, which costs one completion per document. That makes it too costly to run on every change, and a check nobody runs is no check.

## Context

The gate-first pipeline (ADR-2610221540) is strong from the ADR onward. A spec author who never saw the code writes the gate, a person accepts it, and sealed tests fail first. Its weak point is the step before that. Nothing tests the ADR's own reasoning. A wrong decision compiles into a correct gate for the wrong thing, and every later step then faithfully proves the mistake. EGM covers that weak point. Goldfish reviewers read the design cold, before anything is decided. EGM also covers the other end: after the change ships, the drift report checks that the docs still describe the code.

For the two to work as one loop, three things must hold. A design must be reviewed in the form that is decided. The decision must cite the design. The drift check must be cheap enough to run on every change.

## Decision

1. **The loop's first stage is design.** The stages are `design`, `decide`, `gate`, `build`, `harden` and `done`, in that order. `loop_stages()` in `hexa-core/src/domain/workflow.rs` gives them, `parse_stage` reads one, and `stage_word` writes one. `hexa loop stage design` records it. `hexa loop design <draft>` records the draft the work is under, which must be a file inside the project, and sets the stage to `design`. Design is optional: an ADR may be written directly, and a loop with no design recorded starts at `decide` as before.
2. **A review records the draft it read.** `hexa design review` writes `Draft sha256: <hex>` into its report: the hash of the draft's bytes as they were read. `reviewed_sha` in `hexa-core/src/domain/design.rs` reads that hash back, and `reviewed_stages` reads which stages the report has a section for. `review_covers(report, draft_sha)` is `Ok` only when the report recorded `draft_sha` and holds sections for stages 5, 6, 7 and 8. Otherwise its `Err` names what is missing: a different hash, a missing hash, or the missing stage numbers.
3. **A reviewed, committed design becomes a proposed ADR, and only such a design does.** `hexa design adr <draft>` refuses a draft that has uncommitted changes, or that git does not track. It also refuses a draft for which no review report in the artifact directory passes `review_covers` against the draft's current bytes. A draft revised by `hexa design resolve` must therefore be reviewed again. Given such a draft, the elephant runs on the ADR schema `hexa adr schema` prints, and on the design. hexa writes `docs/adrs/ADR-<next id>-<slug>.md` with `**Status:** Proposed`, numbered decisions under `## Decision`, and a `**Design:** <draft>` line. It also records the new ADR in the loop and sets the stage to `decide`. It never accepts the ADR and never writes a spec. This amends ADR-2610271736 §8: `hexa design adr` may write one file under `docs/adrs/`, and no other `hexa design` command may.
4. **An ADR names the design it came from, and the name must resolve.** `hexa adr doctor` reports, as an error, an ADR whose `**Design:**` line names a path that is not a file in the project. An ADR with no `**Design:**` line is not an error.
5. **`hexa go` suggests the next design step.** `next_design_action(&DesignState)` in `hexa-core/src/domain/workflow.rs` returns one of four values. `Nothing` means no draft is recorded. `Review(draft)` means no review report covers the draft's current bytes (§2). `Commit(draft)` means a report covers it but the draft has uncommitted changes. `ToAdr(draft)` means it is covered and committed. `hexa go` prints the matching verb: `hexa design review <draft>`, `git commit <draft>` or `hexa design adr <draft>`. It prints nothing when the value is `Nothing`.
6. **`hexa drift --since <ref>` checks what a change touched.** `docs_touched(docs, changed)` in `hexa-core/src/domain/design.rs` takes each candidate document with the paths it names (`referenced_paths`), and the paths `git diff --name-only <ref>` lists. It returns, in the documents' order and each once, every document that is itself changed, or that names a changed path. `hexa drift --since <ref>` runs one goldfish per returned document and no others. When none is returned, it writes the report with the line `No document touched since <ref>` and makes no completion. Without `--since`, `hexa drift` checks every document as before.
7. **`hexa hey` routes to the design verbs.** A request that asks to brainstorm or design an idea routes to `design brainstorm "<idea>"`, with the idea taken from the request. A request to review a design, with a path in it, routes to `design review <path>`; without a path it has no route. A request about drift, or about whether the docs match the code, routes to `drift`. All three are in `hexa_commands()` in `hexa-core/src/domain/intent.rs`. Each is non-destructive except `design brainstorm`, which writes files and so needs confirmation.
8. **The loop's guidance starts at design.** The feature-sized prompt hook names the design step first: ``Design (`hexa design brainstorm "<idea>"`, optional) → Decide → …``. The `/hexa-workflow` skill gains a section, before Decide, covering `hexa design brainstorm`, `draft`, `review`, `resolve` and `adr`. Its ship step runs `hexa drift --since <base>`. The pipeline in hexa's `CLAUDE.md` gains a step 0, Design, that names the same verbs.

## Consequences

- Before this, a design and its decision were linked only in someone's memory. Now the ADR cites the design (§4), and the design was reviewed at the exact bytes the decision came from (§2, §3).
- Re-review after a resolve costs one more review run. That cost is what keeps a review from certifying text it never read.
- `hexa drift --since` runs in proportion to the change, not to the documentation set. It is cheap enough to put in the ship step on every change.
- Small changes are unaffected: design stays optional, and a loop without one starts at `decide`.
- `hexa design adr` writes under `docs/adrs/`, and it is the only design command allowed to. That is ADR-2610271736 §8, narrowed, not removed.

## Implementation

`hexa-core/src/domain/workflow.rs` (new) holds the loop stages and `next_design_action`. `hexa-core/src/domain/design.rs` gains `reviewed_sha`, `reviewed_stages`, `review_covers` and `docs_touched`. `hexa-core/src/domain/intent.rs` gains the three routes. The verbs use these through `hexa-exec/src/egm.rs`. `hexa loop` reads its stages from `workflow.rs`.

## Gate

`hexa spec verify ADR-2610281011`: the spec's sealed tests, each red before the code (ADR-2610221540 §5).

## References

- ADR-2610271736: `hexa design` and `hexa drift`, and the §8 this amends.
- ADR-2610221540: the gate-first pipeline that design now precedes.
- ADR-2610271308: `hexa hey` routes only to verbs hexa has.
- ADR-2610270901: `claude-text`, the tool-less completion route.
