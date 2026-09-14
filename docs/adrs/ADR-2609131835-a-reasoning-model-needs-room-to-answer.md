# ADR-2609131835: A reasoning model needs room to answer, and an empty reply must say why

**Status:** Accepted
**Date:** 2026-09-13
**Amends:** ADR-2609131702, whose fallback stands; this makes it produce answers.
**Drivers:** The first serialised fallback run: three of four lenses answered `0 candidates` and the fourth returned `NO ANSWER — empty reply`. The gateway's reply, inspected directly, has `content: null`, a populated `reasoning` field, and `finish_reason: length`. The model had spent its entire output budget thinking and never reached the answer.

## Context

`openai/gpt-oss-120b` is a reasoning model. It emits its thinking first and its answer after, into
the same output budget. `complete_text` asked for 4096 tokens, which covered the thinking for a
15KB target and not the reply.

Two layers then hid what happened. The adapter reads `choices[0].message.content`, which is null,
and produces no text block. `complete_text` joins zero text blocks into `""` and returns `Ok("")` —
discarding `stop_reason: MaxTokens`, which the adapter had correctly parsed two functions earlier.
The harness then reported `empty reply`, which is true and useless: it names the symptom and not
the cause, and it is indistinguishable from a model that genuinely answered with nothing.

A measured trivial call to the same gateway consumed all 64 of its 64 permitted tokens on
reasoning, with `finish_reason: length` and `content: null` — the same shape at a scale small
enough to read in full.

## Decision

1. **A truncated generation is an error, not an empty answer.** When a reply has no text and the
   stop reason is the token limit, `complete_text` returns an error naming the model, the budget,
   and why a reasoning model hits it. `Ok("")` was the eighth thing today to report a definite
   result for something that did not happen.

2. **The review budget is sized for thinking plus answer**, 16384 tokens, overridable with
   `HEXA_REVIEW_MAX_TOKENS`. The envelope asked for is a few hundred tokens; the rest is headroom
   for the reasoning in front of it.

3. **The system prompt asks for brief thinking.** A reviewer told to think briefly and then answer
   spends less of the budget before reaching the part that is read.

## Consequences

- A lens that runs out of budget now says so, with the number to raise, instead of `empty reply`.
- 16384 output tokens per lens on a model that takes 25 seconds for a trivial call is slow. That is
  the honest cost of a local review and the thing to measure against, not a reason to shorten the
  budget until the answers disappear again.
- Reading the answer out of `reasoning` when `content` is null was considered and rejected: that
  field is the model's thinking, not its reply, and mining it for JSON would make the harness
  depend on the shape of a model's deliberation.

## Gate

`cargo test -p hexa-infer truncated`: a reply with no text and a max-tokens stop is an error naming
the budget; a reply with no text and an ordinary stop is an empty string, which is a real answer;
and text is returned unchanged in both stop conditions.

## Evidence

`cargo test -p hexa-infer truncated 2>&1 | grep -E 'truncated_tests::|test result: ok. 3'` at 75fc403 with uncommitted changes on 2026-09-13 18:37 UTC:

```text
test complete::truncated_tests::no_text_and_a_max_tokens_stop_is_an_error_that_names_the_budget ... ok
test complete::truncated_tests::no_text_and_an_ordinary_stop_is_an_empty_answer_not_an_error ... ok
test complete::truncated_tests::text_is_returned_under_either_stop ... ok
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 93 filtered out; finished in 0.00s
```
