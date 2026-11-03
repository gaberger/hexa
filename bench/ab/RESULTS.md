# hexa A/B: results, part 1 (an existing hexagonal codebase)

> **What this part measures, and what it does not.** In every task here, the
> code each arm starts from is already laid out in hexa's layers (`domain/`,
> `ports/`, `usecases/`, `adapters/`) and is graded by hexa's rules. Arm A is
> "the same architecture, without hexa's tooling", not a model designing
> freely. The blank-slate comparison, where arm A gets a behaviour-only spec
> and an empty folder and chooses its own design, is in `greenfield/`.

**Question (from the protocol):** with the model, prompt and budget held the
same, does hexa change how often AI-written code is both correct and the right
shape?

**Answer, from 360 runs on three models:** on small, well-specified tasks, no.
Every arm is already at or near 100%, and hexa's effect is on cost and diff
size, not correctness. On a refactor of genuinely entangled dependencies, yes,
for the smaller models. Working alone, Haiku left a boundary violation behind
in 2 of 5 runs and Sonnet in 1 of 5, with every test green. Through hexa, all
15 runs reached A+ on all three models. Opus did not need hexa, and paid
about 1.4× the tokens for it.

The refactor result rests on **one task with 5 runs per arm**. Its point
estimates meet the protocol's bar, but its confidence intervals still reach
zero. It is a signal to scale up, not a settled result.

| | Runs | Spend |
|---|---:|---:|
| Change existing code (5 fixtures) | 225 | $19.44 |
| Shape traps (2 tasks) | 90 | $24.52 |
| Refactor entangled dependencies (1 task) | 45 | $19.39 |
| **Total** | **360** | **$63.35** |

Every run is in `runs/<model>/<arm>/<task>--r<n>.json`, with 0 void after the
leak recheck. `runs/EVIDENCE.md` and `runs/evidence.json` hold every metric.
The manifests (`runs/MANIFEST-*.txt`) record what ran.

## The arms

| Arm | What runs |
|---|---|
| **A**, without hexa | `claude -p` alone in a fresh copy of the task |
| **B**, with hexa | `hexa do run`, with hexa's two gates as the evidence: the task's command and `hexa analyze . --grade A` |
| **C**, rules only | arm A, plus hexa's `ADR-rules.toml` pasted into the prompt, with no gates |

The models were Opus 5.5, Sonnet 5.5 and Haiku 5.5, each pinned for every call
in every arm. Every arm ran under one sandbox: edits inside the workspace, the
task's build and test commands, and in arm B, `hexa analyze`. Each run is
judged by a hidden oracle (the original tests, restored, plus any oracle tests)
and by `hexa analyze`. **Accepted-correct** means the oracle passes and the
grade is ≥ A with 0 violations.

## 1. Refactor entangled dependencies: hexa's gate made the difference

`examples/stress/linkstore-svc-tangled` is the bookmark service (HTTP,
SQLite, 27 tests including end-to-end tests on a live port), tangled the way
code decays:

- the use case opens SQLite and the wall clock itself;
- the JSON wire shapes live in the domain, and the HTTP adapter reads them there;
- a port's default method reads the use case's limits;
- the domain reads time through the `Clock` port;
- the SQLite adapter names a domain type;
- url and tag import each other.

It grades F 25. There is no README, no marker and no comment that states the
rules. The prompt asks for clean dependencies and no cycles, without changing
behaviour.

| Model | Accepted-correct A / B / C | B−A (95% CI) | Tokens B/A | $ per accepted-correct A → B |
|---|---|---|---:|---|
| Haiku | 60% / **100%** / 60% | **+40** [0, +80] | 0.63× | $0.081 → **$0.049** |
| Sonnet | 80% / **100%** / 100% | **+20** [0, +60] | 1.04× | $0.417 → **$0.335** |
| Opus | 100% / 100% / 100% | +0 | 1.42× | $0.802 → $0.971 |

- **The oracle passed in all 45 runs.** No arm broke behaviour, so every miss
  is a shape false-green: the tests pass and the architecture is wrong.
- **Every miss was the same rule**, an adapter importing a domain type
  directly (`wire.rs` → `Bookmark`, `sqlite_store.rs` → `BookmarkId`). That is
  hexa's rule 4: adapters import `ports/` only. Many hexagonal styles let
  adapters use domain types, so an agent working on its own judgement does not
  treat it as a tangle. The agents fixed every structural tangle in every run:
  the use case opening SQLite, the port reading the use case, and the cycle.
- **Telling the rules was not enough for Haiku; enforcing them was.** With the
  rules pasted in (arm C), Haiku still missed in 2 of 5 runs. Through hexa,
  which names the failing edge and blocks the result until the grade holds, it
  missed in none. For Sonnet, telling was enough (C 100%).
- **hexa was cheaper per good result for the models it helped.** Haiku spent
  0.63× the tokens per run through hexa. Opus, which needed no help, spent 1.42×.
- **Against the protocol's bar** (B beats A by ≥ 20 points at ≤ 1.5× the
  tokens), Haiku (+40 at 0.63×) and Sonnet (+20 at 1.04×) meet it on point
  estimates. Neither confidence interval excludes zero: with one task, the
  bootstrap can only resample 5 runs per arm. The report page marks that
  *inconclusive*.

## 2. Shape traps: at the ceiling

`sokoban-untangle` (game-sokoban-rs-tangled, F 34 to ≥ A without changing
behaviour) and `linkstore-cache` (add a read cache to the use case without
breaking a boundary, with a hidden oracle that counts store calls).

| Model | Accepted-correct A / B / C | Median $ per run A → B | Lines changed A → B |
|---|---|---|---|
| Opus | 100% / 100% / 100% | $0.61 → $0.43 | 176 → 91 |
| Sonnet | 100% / 90% / 100% | $0.22 → $0.16 | 78 → 57 |
| Haiku | 100% / 100% / 100% | $0.025 → $0.019 | 211 → 92 |

- **Both tasks were too easy.** `game-sokoban-rs-tangled` ships a README that
  lists its five violations, file by file, so every arm had the answer key.
- **Sonnet's one miss in arm B:** hexa ran its delegate once, the grade gate
  failed, and hexa reverted the edit. With a Claude delegate, `hexa do run`
  makes one attempt and does not feed the gate's verdict back for another.
- **These runs gave arm B a handicap**, fixed before the refactor run: the
  sandbox let the agent run only `cargo test`, so it could not run the
  `hexa analyze` it had been told to pass.

## 3. Change existing code: at the ceiling

hexa's five benchmark fixtures, each with a hidden oracle.

| Model | Accepted-correct A / B / C | $ per accepted-correct A → B |
|---|---|---|
| Opus | 100% / 100% / 100% | $0.179 → $0.167 |
| Sonnet | 100% / 100% / 100% | $0.079 → $0.066 |
| Haiku | 100% / 100% / 96% | $0.006 → $0.005 |

The fixtures are single files with no layers (hexa grades each F 0), so this
family is scored on the oracle alone. Every model passes every fixture, which
leaves no room for any arm to win.

## What this says about hexa

- **hexa's value shows up where the right shape is not obvious.** On a small,
  well-specified change, a current model gets it right alone. On an entangled
  refactor, it fixes what looks wrong and leaves what only hexa's rules call
  wrong.
- **The gate, not the rules text, is what guaranteed the shape.** Arm B reached
  A+ in all 15 refactor runs; arm C did not for Haiku.
- **hexa did not cost correctness anywhere.** The oracle pass rate in arm B
  was 100% in every family.
- **hexa usually costs the same or less per run, and produces smaller diffs.**
  The exception is a strong model on a task it can already do (Opus on the
  refactor, 1.42× tokens).
- **One-attempt delegation is a gap.** When the gate fails, hexa reverts
  rather than handing the verdict back. A retry with the gate's output would
  likely have turned Sonnet's one shape-family miss into a pass.

## Found and fixed along the way

- **`linkstore-svc` graded B 80**, though the README cites it as evidence,
  because it never declared its two split adapters (ADR-2610271649 §3).
  Declared, it is A+ 100.
- **`header-safe-url`'s oracle contradicted its test name**: "above U+00FF",
  but it rejected `é` (U+00E9). The case now uses U+0101. The change-family
  runs used the old oracle, and every arm passed it.
- **Harness bugs, all caught by the self-test or by reading a run, before they
  shaped a result:** the metering shim lost its executable bit; the leak check
  read other sessions' transcripts (7 runs wrongly voided, rechecked and
  lifted, with the reason kept in each record); a shared cargo target let one
  run test another's binary; the arm B sandbox hid `hexa analyze`; a reverted
  run counted as a false-green.

## Limits, and what to run next

- **One refactor task.** The result needs more entangled codebases before it
  is more than a signal: several sizes, several languages, and tangles that do
  not all reduce to one rule.
- **Not the protocol's full design.** No greenfield family and no blinded
  reviewer-model defect count. 8 tasks, not 24. 5 runs per arm.
- **Rule 4 is hexa's opinion.** The refactor misses are violations of a rule
  arm A was never told. That is the point of hexa, but it is also why arm C
  matters: for Sonnet, telling was enough.
- **The shape family ran with arm B's sandbox handicap.** Rerun it to compare
  it cleanly with the refactor family.
- **Next:** more entangled refactors (a larger Go or TypeScript codebase, and
  tangles a model's own sense of layering would also call wrong), a retry
  loop for hexa's Claude delegate, and the greenfield family.

## Reproduce

```bash
cargo build -p hexa-cli --release
pip install lizard==1.17.13
bench/ab/selftest.sh
MODELS="<model ids>" TASKS="linkstore-untangle" ARMS="A B C" RUNS=5 JOBS=6 bench/ab/run_matrix.sh
python3 -I bench/ab/report.py bench/ab/runs
```

[RUBRIC.md](RUBRIC.md) defines every field. [README.md](README.md) describes
the harness.
