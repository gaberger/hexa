# ADR-2609131646: An unanswered hunt is not a clean one

**Status:** Accepted
**Date:** 2026-09-13
**Epoch:** hexa
**Drivers:** `hexa harden hexa-infer/src/discover.rs` returned in two seconds with `4 lenses → 0 candidates → ✓ 0 confirmed real → 0 fixed (gate-passed)`. Nothing was reviewed. Every one of the four agents had received `You've hit your monthly spend limit.` and the harness read that as a clean file.

## Context

`claude -p` exits 0 when it declines. A spend limit, an auth expiry, a model refusal — each is a
successful process carrying prose instead of the requested JSON. The hunt phase asks for
`{"findings":[…]}`, runs `extract_json` over the reply, and on `None` adds nothing to the count.
Nothing distinguishes *the reviewer looked and found nothing* from *no reviewer ever ran*, and the
pass prints the first while meaning the second.

The result is the worst output a review tool can produce: a confident all-clear over code nobody
read, ending in a commit. This is ADR-2609122048 — *a tool that reports "nothing found" must prove
it looked* — broken inside the tool that the ADR was written for, and it hid behind exactly the
defect it warns about. It went unnoticed today because the previous run on `loop_cmd.rs` had
budget, answered 21 candidates, and looked healthy.

An empty findings list is a real and common answer. The distinction is not "were there findings"
but "was the question answered".

## Decision

1. **A reply that does not parse as the requested envelope is not an answer.** The hunt counts
   three outcomes per lens: answered with N findings, did not answer, or the task failed. Only the
   first contributes to the count, and the other two are named with the first line of what came
   back, so `You've hit your monthly spend limit.` appears in the output rather than being
   swallowed.

2. **A review where no lens answered did not happen.** `ReviewReport` carries `lenses` and
   `answered`. With `answered == 0` the pass reports `no lens answered — nothing was reviewed`,
   never `0 confirmed real`, and the renderer leads with that rather than a tick.

3. **A review that did not happen commits nothing.** The final commit is conditional on the review
   having run, not only on the gate passing — a gate that passed before the pass began proves
   nothing about a pass that never looked.

4. **A gate that was never reached is reported as `not run`, and a pass that reviewed nothing
   exits non-zero.** The first run of this fix printed `final gate: FAIL` for a gate it had
   returned before reaching, which is the same lie in the adjacent field; and it exited 0, so
   `hexa harden && ship` would have proceeded.

5. **A partial review says so.** With some lenses answered and some not, the count stands and the
   line names how many of how many answered, because a hunt down two of four lenses is a weaker
   claim than a hunt that ran.

## Consequences

- The run that produced this ADR now reads `0 of 4 lenses answered — nothing was reviewed`, with
  the spend-limit text quoted, and exits without committing.
- Every verdict in the verify phase already had a `no verdict` branch; it now carries the same
  reason text, for the same cause.
- `hexa harden` cannot run on this machine until the frontier budget resets or a reachable local
  model is mapped to the harness. That is now visible in one line instead of looking like success.
- The same swallow exists anywhere `extract_json` returning `None` is treated as a value rather
  than an absence. The build path's design and critique phases collect `Ok(Ok(_))` alike; they
  count what they got and never claim emptiness means clean, so they read honestly today, but the
  shape is worth watching.

## Gate

`cargo test -p hexa-exec answered`: an envelope that parses counts as answered, with its findings;
a reply that is prose, a refusal, or empty is not answered and carries its first line as the
reason; a review with no answered lens is marked as not reviewed and its renderer does not claim
clean; a partially answered review keeps its count and reports the ratio; and a gate that was
never reached reads `not run` rather than `FAIL`.

## Evidence

`cargo test -p hexa-exec answered 2>&1 | grep -E 'answered_tests::|test result: ok. 5'` at 2e81c3b with uncommitted changes on 2026-09-13 16:50 UTC:

```text
test adversarial::answered_tests::a_gate_that_never_ran_is_not_a_failed_gate ... ok
test adversarial::answered_tests::a_first_line_is_trimmed_not_dropped ... ok
test adversarial::answered_tests::a_partial_review_keeps_its_count_and_names_the_ratio ... ok
test adversarial::answered_tests::a_review_with_no_answer_is_not_a_clean_review ... ok
test adversarial::answered_tests::an_envelope_is_answered_and_prose_is_not ... ok
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 89 filtered out; finished in 0.00s
```
