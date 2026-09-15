# Setup defect, recorded 2026-09-15 before the hexa and BMAD arms reported

At the time of writing, only the Spec Kit arm has finished. Its gate passes
and `hexa analyze` grades it F with 6 boundary violations, all of the form
"adapters must not import from domain directly".

**The challenge text I wrote contradicts the analyzer.** CHALLENGE.md rule 4
says:

> `src/adapters/primary/` and `src/adapters/secondary/` import only from
> `src/ports/` (and `src/domain/` value types).

The parenthesis is mine and it is wrong. `hexa-analysis/src/layer_classifier.rs`
lines 208 and 215 emit "adapters must not import from domain directly" for
both adapter layers with no value-type exception. hexa's own CLAUDE.md states
the rule correctly as "adapters import `ports/` only".

So an arm that read the challenge carefully and imported domain value types
into an adapter did what the written specification told it to do, and the
executable check disagrees.

**Two readings, both recorded before the remaining results are known:**

1. **A setup defect.** I wrote a challenge that contradicts the measuring
   instrument. Any arm penalised for following it was penalised for my error.
   This is the reading that counts against the trial.
2. **An accidental instantiation of the claim under test.** The written
   specification was wrong. The executable check was right. Only an arm that
   ran the executable check could discover the difference, and only the
   gate-first arm is permitted to run it. If the gate-first arm scores well
   here, that is *why*, and it is not a fair comparison of methods: it is a
   demonstration that a spec nobody runs can be wrong without anyone noticing.

**How this will be scored.** Neither arm is judged on rule 4 alone. The
results document will report the grade twice for every arm: as measured, and
recomputed with the six adapter-to-domain violations of this kind excluded.
If the ordering between arms changes between those two numbers, the trial has
no architecture result and says so.

**Not fixed retroactively.** CHALLENGE.md stays as the arms received it. The
gate is unaffected; it is black box and never inspected source.
