# bench — a repeatable defect bench that tests its own rubric

One command, deterministic output, no speculation:

```bash
./bench/score.sh <dir> [dir...]          # the full scorecard
```

That runs, in order: the self-test, the functional gate, every probe, the
architecture grade, and — with `BASE=<rev>` set and the target a git
repository — the blast radius of the change since that revision.

```bash
BASE=67d3aa6 ./bench/score.sh ../build-trial/hexa
```

Two narrower entry points remain:

```bash
./bench/selftest.sh                      # is the rubric fit to judge anything?
./bench/run.sh <dir> [dir...]            # the probe matrix across several targets
```

Both `score.sh` and `run.sh` refuse to produce anything until `selftest.sh`
passes.

## Rerunning the trial

`CHALLENGE.md` is the build brief and `CHANGE-01.md` the change request, both
exactly as the three arms received them, with one correction noted at the foot
of `CHALLENGE.md`: its layering rule 4 originally contradicted `hexa analyze`
and voided the first run's architecture comparison. It now agrees.

To rerun: give each method a clean directory containing `CHALLENGE.md` and a
copy of `gate.sh`, let it build, commit, then hand it `CHANGE-01.md`. Score
with `score.sh`, passing `BASE=<the pre-change commit>` for the second round.

## What it judges

Implementations of the URL-shortener contract in `bench/CHALLENGE.md`: an HTTP
API and a CLI over a persistent store. Each probe in `probes/` takes a target
directory, runs the program from the outside, and exits 0 (clean), 1 (defect)
or 2 (could not run). Probes never read the subject's source.

## How the rubric is tested

This is the part that matters. A probe is only worth running if it can fail.

`selftest.sh` runs every probe against two reference implementations kept in
`fixtures/`:

- `fixtures/sound` — correct. Every probe must report clean.
- `fixtures/unsound` — carries every defect the probes test, each tagged
  `DEFECT-Pn` in the source. Every probe must report a defect.

A probe that passes both is **VACUOUS**: it cannot fail, so its green means
nothing. A probe that fails the sound fixture is **BROKEN**. Either way
`selftest.sh` exits non-zero and `run.sh` will not run.

This is `evidence_is_vacuous` applied to the measuring instrument instead of to
a gate. It is not decoration. On its first execution it caught two of six
probes as vacuous:

- `p2-chunked-body-cap` sent a 200 KB URL. Both fixtures returned 400, one for
  the body size and one for the URL length, and the probe could not tell them
  apart. Rewritten to send a large body containing a short valid URL, it now
  discriminates — and immediately found a real defect in an implementation the
  broken version had cleared.
- `p3-whitespace-location` tested an invariant the runtime enforces below the
  application, so the defect is unreachable on Bun. Retired to
  `probes/retired/` with the evidence.

## The gate is deliberately green on both fixtures

`selftest.sh` also runs the behavioural gate (`gate.sh`) against both
fixtures and expects **PASS on both**. Every defect the probes catch hides
behind a passing functional gate. That is the bench's reason to exist: a gate
answers "does it do what I asked", and these probes answer "what else does it
do".

## Adding a probe

1. Write the probe in `probes/pN-<name>.sh`, sourcing `probes/lib.sh`.
2. Add the defect to `fixtures/unsound/src/main.ts`, tagged `DEFECT-PN`.
3. Run `./selftest.sh`. If the new probe is not "ok — discriminates", it is not
   finished.

A probe added without a matching defect in the unsound fixture will be reported
VACUOUS and will block the whole bench. That is intended.
