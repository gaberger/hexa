# Build trial: three methods build the same system

**Date:** 2026-09-15
**Task:** Build a URL shortener in TypeScript with two primary adapters (HTTP,
CLI), two secondary adapters behind one port (file store, in-memory cache),
and state that survives a restart. One challenge, one black-box gate, the
seven hexagonal rules stated identically to every arm.
**Arms:** gate-first (hexa verbs), GitHub Spec Kit, BMAD-METHOD.
**Status:** Complete. All three arms finished and were verified by the
operator, not taken on their reports.

## Findings, stated first

1. **All three methods built a working system.** Every gate verified by the
   operator. Spec Kit 13 minutes, BMAD 29, gate-first 72.
2. **The architecture measure found nothing.** All three arms graded below A,
   and every violation in all three is the single rule class my own challenge
   text contradicted. Excluding it, all three are identical: zero violations,
   zero cycles, zero dead exports, 100 out of 100. **A three-way tie.**
3. **What separated the arms was whether an adversarial review step ran, not
   which method ran it.** Probed for six specific defects, drawn evenhandedly
   from both adversarial passes: gate-first 1 defect, BMAD 1, Spec Kit 4. The
   two arms that ran an adversarial step tie. The arm that did not has four
   times their defects.
4. **Each adversarial pass caught what it looked for and missed what the other
   found.** hexa's harden found a URL class that permanently breaks codes, which
   BMAD shipped. BMAD's review found silent store corruption with data loss,
   which hexa shipped. Neither is a superset of the other.
5. **The gate I wrote before the code missed a stated requirement.** Deleting
   the entire cache adapter leaves it green, demonstrated.
6. **Gate-first wrote every test after the implementation.** Reported by the
   arm itself and provable from file mtimes. Its oracle was the adversarial
   hunt, not a test suite.

## Verification, run by the operator

| | Gate-first | Spec Kit | BMAD |
|---|---|---|---|
| Gate | **PASS** 11/11 | **PASS** 11/11 | **PASS** 11/11 |
| Wall clock | 72 min | **13 min** | 29 min |
| Grade as measured | B, 80 | F, 40 | F, 0 |
| Violations, all one disputed class | 2 | 6 | 12 |
| Violations outside that class | **0** | **0** | **0** |
| Grade with that class excluded | **100** | **100** | **100** |
| Source lines | 838 | 606 | 675 |
| Tests passing | 88 | 41 | 59 |
| Tests written before their subject | **none** | 2 of 6 files | all |
| **Defects found by cross-probe** | **1** | **4** | **1** |

## The defect probe

Six defects, three taken from hexa's harden report and three from BMAD's
review report, written into two scripts by the operator after both had
reported, and run identically against all three arms. Sourcing from both
adversarial passes is what makes it symmetric; a probe built only from
harden's findings would have been rigged for hexa.

| Defect | Gate-first | Spec Kit | BMAD |
|---|---|---|---|
| URL above U+00FF: 201 then 500 forever, code permanently dead | ok | **DEFECT** | **DEFECT** |
| Body cap bypassed by chunked encoding | ok | **DEFECT** | ok |
| Whitespace URL corrupts the Location invariant | ok | ok | ok |
| `GET /%` returns 500 instead of 404 | ok | **DEFECT** | ok |
| Corrupt store read as empty, then overwritten: data loss | **DEFECT** | **DEFECT** | ok |
| Empty `STORE_DIR` writes into the working directory | ok | ok | ok |
| **Total** | **1** | **4** | **1** |

Every one of these sat behind a green gate. Spec Kit's four sat behind a green
gate and 41 passing tests.

**Finding 4 is the one worth keeping.** The two arms that ran an adversarial
pass each shipped exactly one defect, and it was the one the other's pass had
caught. hexa's harden reasons from the code and found an input-domain fault
BMAD missed. BMAD's review reasons from the specification and found a
durability fault hexa missed. The evidence here supports running an
adversarial pass, and does not support a preference between these two.

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

- **The probe is six defects, not a census.** It was built from what two
  adversarial passes happened to report. Defects no pass found are invisible
  to it, and all three arms certainly still have some.
- **Gate-first cost 5.5x the fastest arm's wall clock** for one fewer defect
  than Spec Kit and the same count as BMAD. On this task that is a poor trade;
  17 of its first 30 minutes produced no code at all.
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
