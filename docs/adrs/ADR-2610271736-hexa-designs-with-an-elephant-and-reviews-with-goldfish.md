---
id: ADR-2610271736
status: proposed
date: 2026-10-27
---
# ADR-2610271736: hexa designs with an elephant and reviews with goldfish

**Status:** Proposed
**Date:** 2026-10-27
**Drivers:** On 2026-10-27 the Elephant-Goldfish Model (EGM, <https://github.com/google/elephant-goldfish-model>, Google LLC, Apache-2.0) was installed as a Claude Code plugin and its drift report run against hexa. It found 39 divergences between hexa's docs and its code, among them a grader blind spot that had let hexa grade itself A+ over 48 boundary violations (ADR-2610271649). EGM runs only inside a coding agent's session: its reviewers are subagents, so the isolation that makes a reviewer a *goldfish* depends on the host honouring it, and EGM's own docs say a host that cannot create fresh subagents must stop. hexa already has a call that is isolated by construction: `claude-text` (ADR-2610270901), the `claude` CLI with no tools, no settings, no MCP, no session, run in an empty directory. A goldfish is exactly that call.

## Context

EGM's method: an *elephant* holds the project's context and drafts; *goldfish* reviewers read only the document and the source it references, without the conversation that produced it; a person resolves the trade-offs and approves. Its review runs five stages in order: 5 zoom out, 6 comprehension, 7 critic (design critic, structural, style), 8 intermediate zoom out, 9 implementation readiness (implementation readiness, codebase details, style). A synthesizer deduplicates each stage's findings. Its drift report compares checked-in documents with the code and reports only the differences.

hexa's own gate-first pipeline starts at an ADR. Nothing before it stress-tests an idea, and nothing after it checks that the docs still describe the code. EGM covers both ends.

## Decision

1. **EGM's prompts are vendored, pinned and unmodified.** The reviewer personas (`agents/*.md`), the document standards (`resources/document_standards/*.md`), `resources/execution_instructions.md` and the five skills hexa runs (`egm-brainstorm`, `egm-draft`, `egm-review`, `egm-resolve`, `egm-drift-report`) are copied byte for byte into `hexa-cli/assets/egm/` with EGM's `LICENSE`, at upstream commit `a8380ab5f0aba392c97e31c79f9a6666b3f09f14`. `hexa-cli/assets/egm/UPSTREAM.json` records the repository, the commit and every file's sha256. `Pin` and `verify_pin` in `hexa-core/src/domain/design.rs` check the copy: a file whose hash differs, a recorded file that is missing, or a file present but unrecorded is an error naming it. Updating EGM is a new pin, never an edit.
2. **EGM is attributed wherever hexa uses it.** `docs/ATTRIBUTIONS.md` names EGM, Google LLC, the Apache-2.0 licence, the repository, the pinned commit, and Dave Rensin's essay the method comes from. README links it. `hexa design --help` and `hexa drift --help` name EGM, and every report hexa writes from an EGM prompt says which persona and which pinned commit produced it.
3. **A goldfish is one tool-less completion with nothing but its inputs.** `goldfish_prompt` in `hexa-core/src/domain/design.rs` builds a reviewer's whole input from the persona, the standard it applies (if any), the document under review, and the source files the document references, and from nothing else: no conversation, no earlier review, no other reviewer's findings. It runs on the model passed as `--model`, else `inference.tier_models.t2.5`, through the same rule as `hexa spec draft`: an agent route, which could read the tree, is refused. `claude-text` is the intended route.
4. **The source a goldfish sees is the source the document names.** A tool-less reviewer cannot search, so hexa supplies the code. `referenced_paths` returns every path the document names in backquotes that exists in the project, in order and without duplicates; their contents go into the prompt, each file capped and the cap stated in the prompt. A path outside the project, or one that does not exist, is not read, and the report lists it as unresolved.
5. **`hexa design review <draft>` runs EGM's stages, in order.** `stages_for(Preset)` gives the stages: `Full` is 5, 6, 7, 8, 9; `FeatureSpecOnly` is 5, 6, 7, 8; `ImplementationPlanOnly` is 9. `stage_reviewers(Stage)` gives each stage's personas. Reviewers within a stage run concurrently; a stage starts only after every reviewer of the previous one has returned. Each stage's findings are deduplicated by the synthesizer persona, also a goldfish, and written to the review report, one section per stage, each finding numbered `S<stage>-<n>`. `--stage N` runs one stage. The review never edits the draft.
6. **The elephant drafts; a person decides.** `hexa design brainstorm "<idea>"` and `hexa design draft [<brief>]` run the elephant, the frontier agent with the project's context, on EGM's brainstorm and draft prompts, and write `concepts_brief.md` and `design_draft.md`. Brainstorm then runs the critic goldfish on the brief. `hexa design resolve <report> --accept <ids>` hands the elephant the draft and only the findings named, and it revises the draft. No command marks a design approved; a person does that by committing it under `docs/designs/`. Artifacts go to the directory named by an `EGM artifact directory: <path>` line in `CLAUDE.md`, else `docs/designs/`.
7. **`hexa drift [<dir>]` reports where the docs and the code disagree.** For each Markdown document under `<dir>` (default: `README.md`, `ARCHITECTURE.md` and `docs/*.md`), one goldfish with EGM's drift-report prompt, the document and its referenced source returns the divergences only. hexa writes `docs/analysis/<YYYY-MM-DD>_DRIFT_REPORT.md` with one section per document, `No drift detected` where none was found. ADRs are history and are not checked. `hexa drift` edits nothing else, and exits 0 whether or not drift was found unless `--fail-on-drift`.
8. **EGM never writes code here.** As EGM's execution instructions require, no `hexa design` or `hexa drift` command writes outside its artifact directory and `docs/analysis/`. A design that is approved enters hexa's pipeline as an ADR, like any other decision.

## Consequences

- hexa gets a step before the ADR (brainstorm, draft, review) and a check after the docs (drift), both run from the CLI, with no host plugin needed.
- A goldfish's isolation is a property of the call, not of a host's subagent support.
- EGM's prompts change in hexa only by a new pin; a reviewer's wording is never edited in place.
- Every goldfish run is one inference call and is recorded in the spend log like any other.

## Gate

`hexa spec verify ADR-2610271736`: the spec's sealed tests, each red before the code (ADR-2610221540 §5).

## References

- <https://github.com/google/elephant-goldfish-model>, commit `a8380ab5f0aba392c97e31c79f9a6666b3f09f14`; Apache-2.0, Google LLC.
- Dave Rensin, "Elephants, Goldfish, and the New Golden Age of Software Engineering".
- ADR-2610270901: `claude-text`, the tool-less completion route.
- ADR-2610221540: the gate-first pipeline an approved design enters.
- `docs/analysis/2026-10-27_DRIFT_REPORT.md`: the first EGM run against hexa.
