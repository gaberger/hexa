# ADR-2609131617: A tier that names a model must name one something can serve

**Status:** Accepted
**Date:** 2026-09-13
**Epoch:** hexa
**Drivers:** `hexa doctor` on this machine prints `tier T1 qwen3:4b`, `tier T2 gemma4-12b`, `tier T2.5 devstral-small-2:24b`, prints `✗ local Ollama (not reachable)` four lines above them, and then prints `All checks passed`. Those three models exist on no reachable backend. The only one reachable serves `openai/gpt-oss-120b`.

## Context

ADR-2609122048 is already on the books: *a tool that reports "nothing found" must prove it looked.*
Doctor's inference section breaks the same rule from the other end. It proves two things and claims
a third:

- it probes each discovered backend and reports reachable or not — true, and checked;
- it reads `inference.tier_models` from `.hexa/project.json` and prints what it read — true, and
  not checked at all;
- it then says every check passed, which asserts the tier map is usable. Nothing established that.

The gap is not cosmetic. A tier map that resolves to nothing fails at the first `hexa hey`, the
first `verify`, the first T1 routing decision — and doctor, the command whose entire job is to say
whether the installation works, said it was fine. The harness runs that worked today worked because
`hexa harden` calls the frontier `claude` binary directly and never consults the tier map, so the
false green was hidden behind a code path that does not use the thing being reported on.

`registry::serving` already answers "which endpoint serves this model", exactly, never by
substring. Nothing asked it about a tier.

## Decision

1. **A discovered path carries the models it is known to serve.** `Found` gains `models`, filled
   from the registry entry's list for a registered endpoint, and from `HEXA_INFERENCE_MODEL` or
   `HEXA_VLLM_MODEL` for one configured by environment. Empty means *not enumerated*, never *none*.

2. **A reachable local runtime is enumerated.** Doctor asks it for its model list — `/api/tags`,
   falling back to `/v1/models` — with a short timeout. This is the common configuration, and the
   difference between an answer and a shrug.

3. **Every configured tier is resolved against the reachable paths, and the answer is one of three.**
   *Served*, naming the backend. *Not served*, when every reachable path was enumerated and none
   lists it. *Unverified*, when some reachable path could not be enumerated — printed as such, never
   counted as either. A model a frontier CLI serves counts as served by it.

4. **A tier that is not served fails the run.** `All checks passed` is not printed while a tier
   names a model nothing can answer for; the failure line says which tier, which model, and what
   the reachable paths do serve. Unverified warns and does not fail: doctor may say "I could not
   check", and may not say "fine" in its place.

5. **A failing doctor exits non-zero.** It printed `3 checks failed` and exited 0, so
   `hexa doctor && deploy` proceeded on a broken installation — the same lie one layer up from the
   one this ADR is about. Nothing in the installer or CI depended on the old code.

## Consequences

- On this machine doctor now fails with three named tiers and the one model actually served, which
  is the whole diagnosis in one line.
- Starting Ollama with those three models pulled turns the same run green, on evidence.
- A backend that answers neither model endpoint reads Unverified forever. That is the honest
  result, and it is a prompt to register the endpoint's models rather than a reason to assume.
- Enumeration costs one HTTP call per reachable local runtime, bounded by a short timeout, on a
  command a person runs by hand.

## Gate

`cargo test -p hexa-infer serves && cargo test -p hexa-cli tier_coverage` — as two invocations: a model listed by
a reachable path is served by it and names it; the same model on an unreachable path is not served;
a model on no list, with every reachable path enumerated, is not served; an unenumerated reachable
path makes it unverified rather than either; a claude model is served by a reachable frontier; and
doctor's summary fails on a not-served tier and passes with a warning on an unverified one.

## Evidence

`(cargo test -p hexa-infer serves && cargo test -p hexa-cli tier_coverage) 2>&1 | grep -E 'serves_tests::|tier_coverage::|test result: ok\. [45]'` at 173bf93 with uncommitted changes on 2026-09-13 16:22 UTC:

```text
test discover::serves_tests::an_unenumerated_reachable_path_is_unverified_not_either_answer ... ok
test discover::serves_tests::a_claude_model_is_served_by_a_reachable_frontier ... ok
test discover::serves_tests::both_model_endpoints_are_parsed ... ok
test discover::serves_tests::served_models_lists_what_the_reachable_paths_offer ... ok
test discover::serves_tests::serves_is_answered_only_from_reachable_enumerated_paths ... ok
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 81 filtered out; finished in 0.00s
test commands::doctor::composition::tier_coverage::no_tiers_configured_is_not_an_unserved_tier ... ok
test commands::doctor::composition::tier_coverage::a_tier_a_reachable_path_lists_is_served_by_it ... ok
test commands::doctor::composition::tier_coverage::an_unverified_tier_warns_and_does_not_fail ... ok
test commands::doctor::composition::tier_coverage::a_tier_nothing_serves_is_a_failure_and_the_served_models_are_nameable ... ok
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 264 filtered out; finished in 0.00s
```
