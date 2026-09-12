---
name: hexa-sdlc-loop
description: Map any request onto the six-stage AI-native SDLC loop (Plan, Design, Build, Test, Deploy, Maintain) and route it to the right stage skill. Use when the user asks "which stage is this", "ai-native sdlc", "agentic sdlc", "what artifact do I write next", "how do we run this end to end", or when a request arrives with no artifact behind it.
---

# Hex SDLC Loop — the artifact chain

The SDLC is a loop, not a line. Every stage **ends by committing one artifact**,
and that commit is what starts the next stage. The chain of commits is the audit
trail: who asked for what, what the agent produced, who approved it.

Humans stay accountable for every decision that needs judgment. Their attention
moves to the **gates** — reviewing what the agent flagged — instead of starting
each stage from scratch.

## The chain

| Stage | Skill | Artifact committed | What the commit triggers |
|---|---|---|---|
| 1 Plan | `hexa-intent` | `docs/intent/<slug>.md` | the design pass |
| 2 Design | `hexa-spec-design` | `docs/specs/<slug>.md` | plan drafting |
| 3 Build | `hexa-plan-mode` | `docs/workplans/<slug>.json` + the diff | implementation, then review |
| 4 Test | `hexa-feedback-loop` | tests, eval cases, verification output | a PR that already passed its own checks |
| 5 Deploy | `hexa-review-gate` | the PR with its review findings | merge, then the pipeline |
| 6 Maintain | `hexa-close-loop` | the incident record → a new intent | Stage 1, with nobody in the invocation path |

## Routing a request

1. **Ask what artifact already exists.** No intent → start at Stage 1. Intent but
   no spec → Stage 2. Spec but no workplan → Stage 3. Never skip forward past a
   missing artifact; the skipped stage is where the rework comes from.
2. **Size the request.** A one-line fix with an obvious blast radius goes straight
   to Stage 3 with the intent recorded in the commit message. Anything
   cross-boundary, cross-team, or policy-touching starts at Stage 1.
3. **Name the human at each gate before starting.** Product owner accepts intent
   and spec; an engineer accepts the plan; a code owner approves the PR; a release
   manager authorizes production.

## Adoption order

The stages are non-linear and can be adopted independently, but the arrows point
one way:

```
CLAUDE.md ─┬─► hexa-intent ──► hexa-spec-design ──► hexa-plan-mode ──► hexa-review-gate ──► hexa-close-loop
           └─► hexa-feedback-loop ──────────────────────────────────┘
```

`CLAUDE.md` and `hexa-feedback-loop` have no prerequisites — start with either.
`hexa-close-loop` comes last: it fires the whole chain with no human in the
invocation path, so every gate upstream of it must already hold.

## Where hexa fits

| Playbook concept | hexa surface |
|---|---|
| Institutional knowledge the agent reads | `CLAUDE.md` + skills in `.claude/skills/` |
| Deterministic guardrail | `.claude/settings.json` hooks → `hexa hook` handlers |
| The plan artifact | workplan JSON (`hexa plan draft` → `hexa plan drafts approve`) |
| Independent oracle for the build | behavioral specs (`hexa spec list`), `hexa validate` |
| Architectural decision record | `hexa adr` (Proposed → Accepted → Completed) |
| Repeated mistake, written down once | `hexa memory store lesson:<topic>` |
| Adversarial check on a claim | `hexa verify "<claim>"` |

## Anti-patterns

- **Starting at Build.** If nobody wrote down what was wanted, review has nothing
  to check the diff against and "done" is a matter of opinion.
- **Artifacts in two places with no owner.** Name one system as the source of
  truth per artifact; everything else holds a link or the commit SHA.
- **A gate that is only a skill.** A skill makes a policy likely; a hook makes it
  near-certain. Any policy that must always hold needs the hook behind it
  (`hexa-review-gate`).
- **Human review of every line.** That control was sized for human-written code.
  Move the human to intent and risk; let the passes in `REVIEW.md` read the lines.
