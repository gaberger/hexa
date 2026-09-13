# ADR-2609131907: An empty review is a claim that needs calibrating

**Status:** Accepted
**Date:** 2026-09-13
**Amends:** ADR-2609131702, whose reviewer naming stands; this says what an empty answer from one is worth.
**Drivers:** The first working local review returned `0 candidates` from three of three lenses on `hexa-infer/src/discover.rs`, and I could not tell a clean file from a reviewer taking the easy answer. The frontier found 21 candidates on a comparable file the same morning.

## Context

ADR-2609131646 established that an empty findings list is a real answer and a non-answer is not.
That distinction is about whether the reviewer replied. It says nothing about whether the reviewer
looked, and a model that replies `{"findings":[]}` to everything satisfies it perfectly.

The project already knows what to do with an unverified claim, in another repository and about a
different detector. blacksheep's ADR-2609122053: *recall and precision are claims, and a claim needs
a labelled corpus.* A bug finder that reports nothing is measured by handing it something with a
known defect and seeing whether it comes back. Nothing about that argument is specific to
configuration analysis; it applies to any component whose output is "I found nothing".

The frontier needs no such probe, because it has been observed finding real defects in this
codebase all day. A local reviewer has been observed returning empty lists, which is either good
news or no news, and the difference is measurable for the price of one small call.

## Decision

1. **A local reviewer that reports nothing is probed before its silence is believed.** One extra
   call, per pass rather than per lens, carrying a short snippet with one planted defect of a class
   the lenses hunt. Deliberately tiny, so the probe costs a fraction of a review.

2. **The probe's outcome qualifies the result, and never edits it.** Found the planted defect:
   the pass reports `calibrated`. Missed it: the pass reports
   `uncalibrated — the reviewer missed a planted defect, so an empty result is not evidence`. The
   findings themselves are untouched either way; what changes is what the report claims for them.

3. **A probe runs only when it can change the reading** — the local reviewer was used, and it
   returned no findings at all. A pass with findings has already demonstrated the reviewer looks,
   and a frontier-only pass is not in question.

4. **A probe that itself fails to answer is reported as such**, not as a miss. The distinction of
   ADR-2609131646 applies to the probe exactly as it does to a lens.

## Consequences

- `0 candidates` from a local model stops being ambiguous. It becomes either a measured all-clear
  or an explicit "this reviewer could not find a defect placed in front of it".
- One extra call on the passes that return nothing. The snippet is a dozen lines against a target
  of hundreds, so it is the cheapest call the harness makes.
- The probe measures one defect of one class. It refutes a reviewer that finds nothing; it does not
  prove one that finds this. That is the right asymmetry and the ADR says so rather than implying
  more.
- A future probe set, one per lens, would say more. One is what today's evidence justifies.

## Gate

`cargo test -p hexa-exec calibration`: the probe snippet contains the defect its expectation
describes; a reply naming the defect calibrates; a reply finding nothing marks the pass
uncalibrated; a probe that does not answer is reported as unrun rather than as a miss; and no probe
is issued when the pass already has findings or used the frontier.

## Evidence

`cargo test -p hexa-exec -- probe calibration planted issued 2>&1 | grep -E 'answered_tests::|test result: ok'; cargo test -p hexa-infer has_port 2>&1 | grep -E 'has_port_tests::|test result: ok'` at a13c2b6 with uncommitted changes on 2026-09-13 19:25 UTC:

```text
test adversarial::answered_tests::each_calibration_outcome_says_what_it_is ... ok
test adversarial::answered_tests::the_phase_says_how_the_calls_are_issued ... ok
test adversarial::answered_tests::only_naming_the_planted_defect_calibrates ... ok
test adversarial::answered_tests::the_probe_contains_the_defect_it_claims_to ... ok
test adversarial::answered_tests::the_verdict_line_carries_the_calibration ... ok
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 99 filtered out; finished in 0.00s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 4 filtered out; finished in 0.00s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 4 filtered out; finished in 0.00s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out; finished in 0.00s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test discover::has_port_tests::an_ordinary_host_is_judged_by_its_colon ... ok
test discover::has_port_tests::an_ipv6_address_without_a_port_has_no_port ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 96 filtered out; finished in 0.00s
```
