# hexa A/B, part 2: a blank slate (pilot)

**The question:** given only what the software must do, does a model build
better software through hexa than on its own?

- **Arm A, no hexa at all.** `claude -p` gets [SPEC.md](SPEC.md) and `gate.sh`
  in an empty folder and designs as it likes. Nothing it reads names an
  architecture: SPEC.md is `bench/CHALLENGE.md` with every word about
  ports, adapters, layers and contracts removed.
- **Arm B, with hexa.** The same SPEC.md, built through
  `hexa scaffold --lang ts --grade A`. hexa brings its architecture, its
  multi-design build and its two gates.
- **Round 2 is the same in both arms.** The agent alone gets
  [CHANGE.md](CHANGE.md) (swap file storage for SQLite, with no word about
  architecture) in the codebase its arm built. This tests the design, not
  the tool.

Both arms are judged only by instruments that ignore design:

- the black-box gate;
- the 6 hidden defect probes in `bench/probes`, which no arm sees;
- tokens and dollars;
- lines of code and cyclomatic complexity;
- the change's footprint.

hexa's grade is recorded, never scored.

**This is a pilot: one build and one change per model per arm, 6 of each.**
It shows the shape of the result and what it costs, not a settled answer.
Records: `runs/<model>/<arm>/url-shortener--r1.json`. 0 void.

## Round 1: the build

| Model | Arm | Gate | Hidden defects | Cost | Time | Source files / lines | Test lines | CCN mean / max |
|---|---|---|---|---:|---:|---|---:|---|
| Opus | A | pass | **2** (P1, P2) | $0.30 | 1 min | 2 / 130 | 0 | 3.3 / 9 |
| Opus | B | pass | **0** | $4.06 | 12 min | 22 / 508 | 559 | 2.2 / 11 |
| Sonnet | A | pass | **2** (P1, P2) | $0.13 | 1 min | 2 / 104 | 0 | 3.5 / 7 |
| Sonnet | B | pass | **1** (P7) | $1.56 | 9 min | 19 / 438 | 416 | 2.6 / 13 |
| Haiku | A | pass | **1** (P2) | $0.01 | 1 min | 4 / 151 | 0 | 2.6 / 6 |
| Haiku | B | pass | **1** (P7) | $0.47 | 13 min | 16 / 619 | 1019 | 2.0 / 8 |

The probes that fired:

- **P1:** a URL with a code point above U+00FF is accepted, then fails forever on redirect.
- **P2:** the request-body cap does not hold without a Content-Length.
- **P7:** a long-lived process keeps writing into an orphaned store file.

## Round 2: the change (agent alone in both arms)

| Model | Arm | Gate | Hidden defects | Cost | Source files touched | Lines +/− |
|---|---|---|---|---:|---:|---|
| Opus | A | pass | 3 (P1, P2, P7) | $0.32 | 1 | +32 / −34 |
| Opus | B | pass | 1 (P7) | $0.79 | 6 | +33 / −303 |
| Sonnet | A | pass | 3 (P1, P2, P7) | $0.11 | 1 | +25 / −31 |
| Sonnet | B | pass | 1 (P7) | $0.26 | 3 | +3 / −261 |
| Haiku | A | pass | 1 (P2) | $0.02 | 3 | +23 / −25 |
| Haiku | B | pass | 1 (P7) | $0.11 | 1 | +29 / −27 |

In arm B most of the removed lines are the old file store's own tests, which
the change request's "the old file-based storage is gone" took with it.

## What the pilot shows

- **Both arms build working software.** All 6 builds and all 6 changes pass
  the black-box gate.
- **hexa's builds carried fewer hidden defects:** 2 across three builds,
  against 5 without hexa. After the change, 3 against 7. The two arms fail
  differently. On its own, a model leaves input hardening undone (P1, P2).
  hexa's builds diverge on designs and red-team them before building, and
  they caught both. What they missed is P7. After the move to SQLite, every
  program except Haiku arm A's has it.
- **Without hexa, no model wrote a single test.** Through hexa, every build
  shipped 416 to 1019 lines of tests.
- **hexa costs 13 to 47 times as much per build,** and takes about ten
  minutes instead of one. It builds about 4 times as much source, spread over
  16 to 22 files instead of 2 to 4. Its functions are simpler on average, and
  its most complex function is more complex.
- **The change did not get cheaper through hexa's design.** Every change
  passed in both arms. In arm B it cost 2.4 to 5.5 times as much, because there
  is more code and more tests to move. A two-file program absorbs a storage
  swap easily too. Whether hexa's layering pays off under change likely needs
  a larger system and more than one change.

## Next

- **Scale it.** 5 builds per model per arm, about $30 to $40 a set at the
  pilot's prices, would put confidence intervals on the defect counts.
- **More than one change.** A sequence of change requests, where a small
  unlayered program would start to cost more to change.
- **A larger spec.** The URL shortener fits in two files, which leaves
  structure little to do.

## Reproduce

```bash
cargo build -p hexa-cli --release
python3 -I bench/ab/greenfield/run_greenfield.py --arm A --model <model id> --run 1
python3 -I bench/ab/greenfield/run_greenfield.py --arm B --model <model id> --run 1
```
