# ADR-2609132059: Done means the gate ran, and passed on something

**Status:** Accepted
**Date:** 2026-09-13
**Drivers:** `hexa loop stage done` runs the evidence command and records the stage. It has never run the gate. Nine loops were closed today and every green came from me running the gate by hand; the mechanism the whole pipeline is built around was never consulted at the one moment it exists for.

## Context

The pipeline's promise is that work is done under a gate written before the code. Three of the four
stages enforce it: `hexa loop adr` refuses an ADR that does not exist, `hexa loop gate` records the
command, and the pre-edit hook stops a feature-sized edit with no gate recorded. Then `done` —
the stage that asserts the work is finished — records a string and asks nothing.

So a loop can be marked done on a red gate, and nothing objects. That the nine closed today were
all genuinely green is a fact about me, not about the tool.

The second half is blacksheep's own written lesson, which hexa does not implement: *a vacuous gate
is a failed gate. "0 tests passed" is not a pass. Every new gate shape needs that guard.* A gate of
`cargo test --test typo-in-the-name` exits 0, runs nothing, and would satisfy a `done` that only
checked the exit code. Adding the gate run without this guard would buy a weaker promise than it
appears to.

`run_evidence` in hexa-exec is the right primitive and already carries a scar worth keeping: it
wraps the command in `set -o pipefail`, because a gate like `cargo test | tail` returns tail's zero
and a failing test once got committed through exactly that.

## Decision

1. **`stage done` runs the recorded gate, and a failing gate refuses the stage.** The output's tail
   is printed with the refusal, so the reason is in front of the person who has to fix it. The
   stage stays where it was.

2. **A gate that proved nothing does not pass.** When every test-result line in the output reports
   zero passed, the gate ran and measured nothing, and `done` is refused with that said plainly.

3. **A gate whose output has no test counts is reported, not judged.** Not every gate is a test
   runner — `cargo build`, a script, a grep — and this cannot tell a successful build from a
   vacuous one. It passes, with a line saying the shape was not one the vacuity check can read.
   Three states, because two would have to lie about one of them.

4. **No gate recorded means `done` says so.** The stage is allowed, with a line saying nothing was
   proved. Recording a gate is already enforced where the work happens, and a loop over documents
   is a real thing.

## Consequences

- Closing a loop becomes as slow as its gate. That is the cost of the claim `done` makes, and the
  evidence command already imposed it.
- The nine loops closed today would all still have closed. The tenth, on a red gate, would not.
- The vacuity check reads test-runner output. It will not recognise every runner, which is what §3
  is for: an unrecognised shape is reported as unrecognised rather than assumed good.

## Gate

`cargo test -p hexa-cli gate_at_done`: a passing gate with tests closes the stage; a failing gate
refuses it and the stage does not move; a gate whose every result line says zero passed is refused
as vacuous; output with no test counts passes with the shape reported; and no gate recorded closes
with that said.

## Evidence

`cargo test -p hexa-cli gate_at_done 2>&1 | grep -E 'gate_at_done::|test result: ok. 4'` at e5bec01 with uncommitted changes on 2026-09-13 21:02 UTC:

```text
test commands::loop_cmd::gate_at_done::each_outcome_reads_as_what_it_is ... ok
test commands::loop_cmd::gate_at_done::a_gate_with_no_test_counts_passes_and_says_it_was_not_checked ... ok
test commands::loop_cmd::gate_at_done::empty_targets_beside_a_real_one_still_prove_something ... ok
test commands::loop_cmd::gate_at_done::only_a_gate_that_proved_something_closes_the_stage ... ok
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 275 filtered out; finished in 0.00s
```
