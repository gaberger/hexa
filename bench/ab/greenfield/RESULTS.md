# hexa A/B, part 2: a blank slate

**The question:** given only what the software must do, does a model build
better software through hexa than on its own?

**The answer, from 30 builds and 30 changes on three models:**

- Both arms build software that works from the outside.
- Built through hexa, it has far fewer hidden defects: 0.53 a build against
  1.87, and 8 of 15 builds with none against 0 of 15.
- hexa costs about 10 times as much and takes about 15 minutes instead of one.
- hexa's design did not make the follow-up change cheaper.

## The comparison

- **Arm A, no hexa at all.** `claude -p` gets [SPEC.md](SPEC.md) and
  `gate.sh` in an empty folder and designs as it likes. Nothing it reads names
  an architecture: SPEC.md is `bench/CHALLENGE.md` with every word about
  ports, adapters, layers and contracts removed.
- **Arm B, with hexa.** The same SPEC.md, built through
  `hexa scaffold --lang ts --grade A`.
- **Round 2 is the same in both arms.** The agent alone gets
  [CHANGE.md](CHANGE.md): swap file storage for SQLite, with no word about
  architecture. This tests the design each arm produced, not the tool.

The models were Opus 5.5, Sonnet 5.5 and Haiku 5.5. Each ran 5 builds per arm,
each followed by the change, and each was pinned on every call. None was void.
Both arms are judged only by instruments that ignore design:

- the black-box `gate.sh`;
- the 6 hidden defect probes in `bench/probes`, which no arm sees;
- tokens and dollars;
- lines of code and cyclomatic complexity;
- the change's footprint.

hexa's grade is recorded, never scored. Every number is in
[EVIDENCE.md](EVIDENCE.md) and [evidence.json](evidence.json), from the
records in `runs/`.

## Results, all three models

| | A, no hexa | B, hexa |
|---|---:|---:|
| Build passes the gate | 15 / 15 | 15 / 15 |
| **Hidden defects per build, mean** | **1.87** | **0.53** |
| **Builds with 0 hidden defects** | **0 / 15** | **8 / 15** |
| SQLite change passes the gate | 14 / 15 | 15 / 15 |
| Hidden defects after the change, mean | 2.27 | 0.80 |
| Builds that wrote any tests | 0 / 15 | 15 / 15 |
| Build cost, median | $0.12 | $1.68 |
| Change cost, median | $0.11 | $0.33 |
| Total spend, 15 builds and 15 changes | $4.25 | $42.71 |
| Build time, median | 0.8 min | 14.9 min |
| Source files / lines, median | 2 / 127 | 21 / 582 |
| Test lines, median | 0 | 689 |
| Cyclomatic complexity mean / max, median | 3.0 / 6 | 2.2 / 11 |
| Source files the change touched, median | 1 | 3 |

**The defect gap is not chance.**

- A build without hexa has 1.33 more hidden defects than one through hexa
  (bootstrap 95% CI +0.93 to +1.73).
- After the change, it has 1.47 more (+1.00 to +1.87).
- Zero-defect builds, 0 of 15 against 8 of 15, differ at p = 0.002 (Fisher's
  exact test).

### By model

| Model | Defects per build, A → B | 95% CI of the gap | 0-defect builds, A → B | Total $, A → B |
|---|---|---|---|---|
| Opus | 2.2 → **0.2** | +1.6 to +2.4 | 0 → **4** of 5 | $2.95 → $27.82 |
| Sonnet | 2.0 → **0.4** | +1.2 to +2.0 | 0 → **3** of 5 | $1.17 → $10.07 |
| Haiku | 1.4 → 1.0 | −0.2 to +1.2 | 0 → 1 of 5 | $0.13 → $4.82 |

For Opus and Sonnet the gap is clear. For Haiku it is smaller, and its
interval reaches below zero.

## Which defects

| Probe | What it catches | A, build / after change | B, build / after change |
|---|---|---|---|
| P1 | A URL with a code point above U+00FF is accepted, then fails forever on redirect | **80% / 80%** | 0% / 0% |
| P2 | The request-body cap fails when there is no Content-Length | **100% / 100%** | 0% / 0% |
| P5 | A partially corrupted store loses good records on the next write | 7% / 0% | 13% / 0% |
| P7 | A long-lived process keeps writing into an orphaned store file | 0% / 47% | **40% / 80%** |
| P4, P6 | Malformed escapes; an empty STORE_DIR | 0% | 0% |

- **On its own, every model leaves input hardening undone.** P2 in every one
  of the 15 builds, and P1 in 12. Through hexa, neither appeared once. hexa's
  build diverges on several designs and red-teams them before building, and
  that is where those cases are caught.
- **hexa has a blind spot of its own: P7, the stale store handle.** It appears
  in 6 of hexa's 15 builds and 12 of 15 after the change, against 0 and 7 of 15
  without hexa. It is the one probe where hexa does worse. Its red-team does not
  look for it.

## Cost and shape

- **hexa costs about 10 times as much:** $42.71 against $4.25 for the same 15
  builds and 15 changes. It takes about 15 minutes a build instead of under one.
- **It builds about 4.6 times as much source,** in about 21 files instead of
  2, plus a median 689 lines of tests. Without hexa, not one build wrote a test.
- **Its functions are simpler on average** (cyclomatic complexity 2.2 against
  3.0), and its most complex function is more complex (11 against 6).
- **The change did not get cheaper through hexa's design.** It cost about 3
  times as much (median $0.33 against $0.11) and touched 3 source files
  against 1, because there was more code and more tests to move. It did pass
  in every hexa run. Without hexa, one change of 15 (Opus, run 3) left a
  server that no longer started. A two-file program absorbs a storage swap
  easily. Whether hexa's layering pays off under change needs a larger
  system, or a sequence of changes.

## Limits

- **One spec**, a URL shortener in TypeScript. It is small enough to fit in
  two files.
- **One change**, of one kind (a storage swap).
- **The probes are hexa's own** (`bench/probes`), written from earlier trials
  of this same spec. They are hidden from both arms and test behaviour from the
  outside, but they were chosen by hexa's authors.
- **Arm B used the shared sandbox** (edits in the workspace, `bun`, `npm`,
  `node`, `gate.sh`, `hexa analyze`). Arm A had the same tools without hexa.

## What to do with this

- **hexa's red-team is earning its keep.** It is the likely source of the P1
  and P2 catches. Worth keeping even in a cheaper mode.
- **Add the stale store handle to hexa's hunt.** P7 is the one defect hexa
  makes more often than a model alone.
- **Measure change over time.** A larger spec and a sequence of change
  requests is the test of hexa's architecture claim. This run cannot answer
  it.

## Reproduce

```bash
cargo build -p hexa-cli --release
pip install lizard==1.17.13
python3 -I bench/ab/greenfield/run_greenfield.py --arm A --model <model id> --run 1
python3 -I bench/ab/greenfield/run_greenfield.py --arm B --model <model id> --run 1
python3 -I bench/ab/greenfield/report_greenfield.py
```
