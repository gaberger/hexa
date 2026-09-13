# ADR-2609131749: A test never writes the process environment

**Status:** Accepted
**Date:** 2026-09-13
**Epoch:** hexa
**Drivers:** `local_provider::tests::the_host_override_is_normalised_either_way` failed once in a workspace run on 2026-09-13 and passed on every run since — in isolation, three times consecutively, and twice more across the whole workspace. Nothing in the code under test changed between the failure and the passes.

## Context

Seven calls to `std::env::set_var` sit in this crate's tests, all on variables the crate reads:
`OLLAMA_HOST` and `HEXA_PROJECT_ROOT`. Cargo runs tests in one process on parallel threads, and the
environment is process-global, so one test's value is visible to every other test that reads it
while it is set. Each of the three tests carefully saves and restores the previous value, which
makes the window small and does not close it: another thread reading between the set and the
restore sees the wrong value.

A test that passes because of timing is not a test. Worse, this one will fail again at random, on
someone else's change, and point at code that is correct — which costs more than the bug it was
protecting against ever would.

The crate already knows the answer. `base_url_with(p, env)` takes an injected reader, and
`tier_model_in(root, tier)` takes an explicit root, both written so tests need no global state, and
`discover_with` takes env, endpoints, claude path and probe for the same reason. Two of the three
tests could have called those functions and did not.

## Decision

1. **No test in this workspace writes the process environment.** Where a function reads the
   environment, the crate exposes a variant that takes the reading injected, and the test drives
   that. The environment-reading wrapper stays a one-line delegation, which is the part no test
   needs to cover.

2. **The two missing variants are added**: `socket_addr_with(p, env)` beside `base_url_with`, and
   `configured_tiers_in(root)` beside `configured_tiers`. The public functions delegate.

3. **A test guards the rule.** One test reads this crate's own sources and fails if `set_var`
   reappears, because the next person to add one will be following the pattern they see.

## Consequences

- The three tests become deterministic, and they test more than they did: an injected reader can
  present a case the real environment cannot hold, such as both spellings of the override at once.
- A function that reads the environment and has no injectable variant is now visible as a gap.
- The guard is a grep over source text and will flag a legitimate future use. Anything that truly
  needs to set a variable can run in its own process; nothing here does.

## Gate

`cargo test -p hexa-infer` twenty times with no failure, and `set_var` absent from the crate.

## Evidence

`for i in $(seq 1 20); do cargo test -p hexa-infer 2>&1 | grep -q FAILED && echo RUN_$i_FAILED; done; echo '20 consecutive runs: no failures'; cargo test -p hexa-infer 2>&1 | grep -E 'no_test_in_this_crate|host_override|probe_target|no_config_means'` at 53897bc with uncommitted changes on 2026-09-13 17:50 UTC:

```text
20 consecutive runs: no failures
test local_provider::tests::the_host_override_is_normalised_either_way ... ok
test local_provider::tests::the_probe_target_follows_the_host_override ... ok
test local_provider::tests::no_config_means_no_tiers_rather_than_a_default ... ok
test local_provider::tests::no_test_in_this_crate_writes_the_process_environment ... ok
```
