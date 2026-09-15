# Build trial: three methods build the same system

**Date:** 2026-09-15
**Task:** Build a URL shortener in TypeScript with two primary adapters (HTTP,
CLI), two secondary adapters behind one port (file store, in-memory cache),
and state that survives a restart. One challenge, one black-box gate, the
seven hexagonal rules stated identically to every arm.
**Arms:** gate-first (hexa verbs), GitHub Spec Kit, BMAD-METHOD.
**Status:** Spec Kit and BMAD complete and independently verified. The
gate-first arm had not reported when this was written; its section is marked
pending and will be amended, including if it wins.

## Findings, stated first

1. **Both spec-driven methods built a working system quickly.** Spec Kit in
   13 minutes, BMAD in 29. Both gates verified by the operator, not taken on
   the arms' word. Eleven checks each, including restart persistence.
2. **Both scored F on the architecture grade, and the grade is not usable.**
   Every violation in both arms, 6 of 6 and 12 of 12, is the single rule class
   my own challenge text contradicted. Excluding that class, both arms are
   clean: zero violations, zero cycles, zero dead exports. **The architecture
   measure found no difference between the methods except one I created.**
3. **The gate was written before the code, as the method prescribes, and it
   did not check a requirement the challenge stated.** I deleted the entire
   cache adapter from the Spec Kit arm and the gate still reported PASS. BMAD's
   review found this hole by reading the specification. The gate could not have
   found it, because the hole was in the gate.

## Verification, run by the operator

| | Spec Kit | BMAD | Gate-first |
|---|---|---|---|
| Gate (`./gate.sh`) | **PASS**, 11/11 | **PASS**, 11/11 | pending |
| Wall clock | 13 min | 29 min | pending |
| Architecture grade, as measured | F, 40/100 | F, 0/100 | pending |
| Violations, all of one class | 6 | 12 | pending |
| Violations outside that class | **0** | **0** | pending |
| Grade with that class excluded | **A+, 100** | **A+, 100** | pending |
| Cycles / dead exports / unused ports | 0 / 0 / 0 | 0 / 0 / 0 | pending |
| Source | 12 files, 606 lines | 12 files, 675 lines | pending |
| Test files | 6 | 7 | pending |
| Method artifacts | 20 documents | 15 documents | pending |

## Finding 2 in detail: the measure measured my mistake

`CHALLENGE.md` rule 4 said adapters import from ports "(and `src/domain/`
value types)". The parenthesis is mine and it is wrong. The analyzer
(`hexa-analysis/src/layer_classifier.rs:208,215`) forbids adapter-to-domain
imports outright, and hexa's own instructions state the rule correctly.

Both arms imported domain value types into adapters. Both did what the written
specification told them. Both were graded F for it.

This was recorded in `2609152200-build-trial-setup-defect.md` and committed
**before** either the BMAD or gate-first result was known, precisely so the
confound could not be reinterpreted afterwards. That document promised the
grade would be reported twice and that if the ordering changed, the
architecture result would be void.

The ordering does not merely change. It **vanishes**: with the disputed class
excluded, both arms are perfect. So the architecture comparison between the
two spec methods has **no result**, and the trial's primary measure produced
nothing on this task.

Note what this also means. Both spec-driven methods, told the hexagonal rules
in prose and given no analyzer, produced a correct ports-and-adapters layout
on every rule that was stated unambiguously: one composition root, no adapter
importing another adapter, use cases touching only domain and ports, `.js`
extensions throughout. The claim that spec-driven methods cannot deliver
architecture is not supported here.

## Finding 3 in detail: a gate that missed a requirement

The challenge says, explicitly:

> Caching: resolutions are served from an in-memory cache in front of the file
> store. The cache must be a separate implementation behind the same contract
> as the file store, not a field inside it.

I wrote the gate before any arm started. It checks shortening, redirecting,
404, 400, both CLI verbs, the unknown-code exit status, and persistence across
restart. It never checks that a cache exists, because a cache in front of a
correct store is behaviourally invisible.

Demonstrated, not argued. In a copy of the Spec Kit arm I unwired the cache
and deleted `src/adapters/secondary/caching-link-store.ts` outright:

```
--- gate with the cache adapter deleted:
GATE: PASS
```

The Spec Kit arm had itself written, in `src/main.ts`, the comment
"Drop the CachingLinkStore wrapper and everything else still works." It
noticed. Nothing in its method required it to act on that.

BMAD's `bmad-build` step-04 review caught the same thing and called it its
sharpest finding: deleting the cache would have left all tests and the gate
green. It then closed the gap with `tests/main-wiring.test.ts`, a test that
asserts the composition root actually wires the cache.

**This is a real point against gate-first as I practised it.** The method says
write the gate first and let it decide. It does not say check that the gate
covers the requirements, and this project has no verb that does. A gate is
only as strong as its coverage, and coverage of a written requirement is
exactly the thing a gate cannot check about itself. The existing guard,
`evidence_is_vacuous`, catches a gate that runs zero tests. It cannot catch a
gate that runs eleven good checks and misses a twelfth requirement.

## What BMAD's review found that the gate did not

Recorded because it is evidence about methods, not about this task. BMAD's
three-layer review produced findings its arm then verified and fixed:

- An unreadable `links.json` read as an empty store, which the next save would
  have truncated. Permanent data loss.
- `GET /%` returning 500 rather than 404.
- `process.exit` immediately after a `stdout.write`, truncating a piped write.
  The gate pipes CLI stdout, so this was a live flake the passing gate hid.
- `STORE_DIR=""` writing into the repository root.
- Control characters passing URL validation into a `Location` header.
- Modulo bias in code generation.

My gate passed a system with several of those defects present.

## What this trial does not show

- **The gate-first arm has not reported.** Every comparative statement here is
  between two spec-driven methods. The headline claim is untested until that
  arm lands.
- **One task, one operator, no blind review.** I wrote the challenge, the gate
  and the analysis, and I am not blind to any arm.
- **The architecture measure is unproven, not disproven.** It returned no
  signal here because of my error and because the task, while it has real
  boundaries, is small enough that all three methods can hold them. Five
  successive change requests, the type A design in the pre-registration, is
  where architecture erosion would actually show. That has not been run.
- **Test provenance did not separate the methods again.** BMAD reports every
  test written before its subject, with the post-review additions observed red
  first. Spec Kit wrote two of six files before and four after. Neither matches
  the assumption that spec-driven methods write tests only afterwards.

## What to change

1. **The challenge text must be generated from the analyzer's rules, not
   written by hand.** This failure is exactly the mirror-test failure in a new
   hat: a human-written description of a machine-checked rule drifted from the
   rule, and the drift was invisible until something ran.
2. **A gate needs a coverage check against the stated requirements.** Proposed
   verb behaviour: given a challenge and a gate, list requirements the gate
   cannot fail on. That is a real gap in this tool, found by a competing
   method.
3. **Run type A.** The build alone does not discriminate. The five change
   requests are the measure that might.
