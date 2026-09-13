# ADR-2609131702: The harness reviews with whatever can answer, and says which

**Status:** Accepted
**Date:** 2026-09-13
**Epoch:** hexa
**Drivers:** *"Why can't we drop into localai?"* `hexa harden` consults no endpoint, no tier map and no registry. It shells out to `claude -p`, and when that account hit its monthly limit the pass could not run at all (ADR-2609131646), on a machine with a reachable 120b model and a tier map pointing at it.

## Context

`hexa do` already routes through the tier map: it resolves `tier_model`, reads the target file into
the prompt, and calls `complete_text`, which picks the adapter from the registry entry's provider.
Every piece the harness needs is in the crate it already depends on.

The reason the harness did not use it is real, and it is not "nobody got around to it". `claude -p`
is an **agent**: it opens files, follows a call into another module, and edits the tree. A local
completion is a **single call**: it sees what the prompt contains and returns text. Three phases,
three different answers:

| phase | needs | a local completion can |
|---|---|---|
| hunt | read the target | yes, with the code in the prompt |
| verify | read the cited location | yes, same |
| fix | edit files and add a test | **no** |

A pass that fell back for all three would report fixes that never happened. That is the failure
this ADR must not introduce while removing another.

## Decision

1. **Hunt and verify fall back; fix does not.** Each call asks the frontier first. When the frontier
   does not answer — a spend limit, an expired login, a timeout — hunt and verify retry against the
   tier-mapped model with the target's source in the prompt. The fix phase has no fallback: without
   an agent that can edit, confirmed findings are reported unfixed and the report says why.

2. **The report names the reviewer.** A finding found by `openai/gpt-oss-120b` reading an excerpt is
   not the same claim as one found by an agent that walked the repository, and a reader deciding
   what to trust needs to know which happened. The verdict line carries it, per phase.

3. **What the local reviewer was shown is bounded and stated.** The target is read up to a cap; a
   target that does not fit is truncated and the report says so, naming the bytes shown. A reviewer
   shown half a file is a weaker reviewer and the number is how a reader judges it.

4. **A tier with no model configured is not a fallback.** When the tier map resolves to nothing, or
   the model it names is unreachable, the call is a non-answer like any other and the pass reports
   that nothing reviewed it (ADR-2609131646). Falling back to nothing is the failure that ADR
   exists to name.

## Consequences

- `hexa harden` runs on this machine with the frontier out of budget, at reduced strength, and the
  output says which strength.
- The fix loop becomes frontier-only. On a budget-limited machine `hexa harden` becomes a review
  that reports, which is most of its value and not all of it.
- Two reviewers may disagree. Verify already defaults to refute, and a claim the local reviewer
  raises and the frontier refutes is dropped exactly as before.
- The local reviewer sees no other file. A defect whose evidence is in a caller is out of its reach,
  and §2 makes that visible rather than leaving a reader to assume otherwise.

## Gate

`cargo test -p hexa-exec reviewer`: a frontier answer is used and named; a frontier non-answer falls
back to the tier model and the report names it; a fix phase never falls back; a target over the cap
is truncated with the shown size recorded; and no configured tier makes the call a non-answer rather
than a silent skip.

## Evidence

`cargo test -p hexa-exec answered 2>&1 | grep -E 'answered_tests::|test result: ok. 9'` at 1c5569f with uncommitted changes on 2026-09-13 17:05 UTC:

```text
test adversarial::answered_tests::a_first_line_is_trimmed_not_dropped ... ok
test adversarial::answered_tests::a_gate_that_never_ran_is_not_a_failed_gate ... ok
test adversarial::answered_tests::a_review_with_no_answer_is_not_a_clean_review ... ok
test adversarial::answered_tests::a_partial_review_keeps_its_count_and_names_the_ratio ... ok
test adversarial::answered_tests::reviewers_are_recorded_once_each_and_never_for_a_non_answer ... ok
test adversarial::answered_tests::an_envelope_is_answered_and_prose_is_not ... ok
test adversarial::answered_tests::the_verdict_line_names_the_reviewer_and_any_truncation ... ok
test adversarial::answered_tests::a_target_over_the_cap_is_truncated_and_says_so ... ok
```
