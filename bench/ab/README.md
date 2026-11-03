# bench/ab: per-model evidence for the hexa A/B protocol

A harness for the A/B protocol (`ab-protocol.pdf`): with the model, prompt and
budget held the same, does hexa change how often AI-written code is both correct
and the right shape? It records accuracy, tokens, cost, lines of code and
complexity for every run, **against the model that produced it**.
[RUBRIC.md](RUBRIC.md) defines every field and rule.

From the repository root:

```bash
cargo build -p hexa-cli --release      # arm B runs target/release/hexa
pip install lizard==1.17.13            # plus bun, jq and the claude CLI
bench/ab/selftest.sh                   # no model calls: oracles discriminate, controls hold
MODELS="<model id> <model id>" ARMS="A B C" RUNS=5 bench/ab/run_matrix.sh
python3 -I bench/ab/report.py bench/ab/runs
```

`run_matrix.sh` writes `bench/ab/runs/EVIDENCE.md` and `bench/ab/runs/evidence.json`.

One run by hand:

```bash
python3 -I bench/ab/run_one.py --task header-safe-url --arm B --model <model id> --run 1
```

| File | Role |
|---|---|
| `bin/claude` | Shim first on PATH: pins the model, meters tokens and $ for every call in every arm |
| `run_one.py` | One task × arm × model × run → one JSON record in `runs/<model>/<arm>/` |
| `run_matrix.sh` | The whole matrix in random order; refuses to start unless the self-test passes |
| `report.py` | Per model × arm table; B−A and C−A with a task-clustered bootstrap CI |
| `selftest.sh` | Negative and positive controls, no model calls |
| `index.html` | The report page; renders `runs/evidence.json`, or says no runs have been made |

The harness names no model: `MODELS` (and `--model`) is the only source of
model ids, as `CLAUDE.md` asks. Paths default to this checkout; override them
with `HEXA_AB_FIXTURES`, `HEXA_AB_RULES`, `HEXA_AB_HEXA` and
`HEXA_AB_REAL_CLAUDE`. `HEXA_AB_HELPER_MODELS` lists any model, besides the
pinned one, that may serve calls without voiding the run.

**Status:** 360 runs on three models across three task families. Results and
what they mean: [RESULTS.md](RESULTS.md).

**Before a real run:** run it where the agent can't reach this checkout. The
oracles live in `docs/benchmarks/fixtures`, and the examples in `examples/` are
finished builds. The harness flags a transcript that touches oracle material,
but isolation is stronger than detection.

## The report page

`index.html` is published on the site at `https://gaberger.github.io/hexa/ab/`.
It renders only from `runs/evidence.json`. Copy that file next to the published
page, at `site/static/hexa/ab/runs/evidence.json` in the site repository, to
publish results. To preview another evidence file locally, use
`?data=<relative path>.json`.
