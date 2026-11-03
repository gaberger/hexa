# Evaluation rubric: hexa A/B, by model

This rubric goes with the A/B protocol (`ab-protocol.pdf`). That protocol names accuracy and cost
but has no rubric for size or complexity, and it records no per-model evidence.
Every run here produces one JSON record (`runs/<model>/<arm>/<task>--r<n>.json`)
with all the fields below. `report.py` summarizes the records by model × arm.

## Arms

| Arm | What runs | Prompt |
|---|---|---|
| A | `claude -p` alone, in a fresh workspace | task + target file + gate command |
| B | `hexa do run … --file … --evidence <gate> --model <M> --also <test file>` | the same text |
| C | arm A, plus the target project's `ADR-rules.toml` pasted in | the same text + rules |
| 0 | self-test only: no agent | — |

The prompt names the same gate command in every arm, so B is not the only arm
told what "done" means.

## 1. Accuracy

| Field | Definition |
|---|---|
| `oracle_pass` | The hidden oracle exits 0 with ≥ 1 passing test and 0 failing. The oracle runs on a **copy** of the workspace after the agent finishes, and is never written into the agent's workspace. |
| `oracle_tests_passed` / `_failed` | Counts from the oracle run, for partial credit. |
| `own_gate_pass` | The gate in the prompt (`bun test <test file>`) exits 0 on what the agent left. |
| `false_green_correctness` | `own_gate_pass` and not `oracle_pass`. |
| `false_green_shape` | `own_gate_pass` and the architecture gate fails. n/a where shape does not apply. |
| `accepted_correct` (**primary**) | `oracle_pass` and (grade ≥ A and 0 violations, where shape applies). |
| `timed_out` | Hit the wall-clock cap. The run still counts, as a failure. |

**Shape for the "change" family.** The five fixtures are lone files with no
layers, and `hexa analyze` grades every one **F 0**, which the self-test shows.
If the grade counted, accepted-correct would be 0 in every arm. For this family
shape is recorded and marked `gradeable: false`, and accepted-correct is the
oracle alone. The greenfield and shape-trap families are where the grade applies.

## 2. Tokens and cost

These are measured the same way in every arm. `bin/claude` is placed first on
PATH and wraps every `claude` call, whether the agent makes it (A, C) or hexa
does (B). It always asks the real CLI for JSON and logs `usage` and
`total_cost_usd` per call.

| Field | Definition |
|---|---|
| `calls` | Model calls in the run. |
| `input_tokens`, `output_tokens`, `cache_read_tokens`, `cache_write_tokens` | Summed over calls. |
| `usd` | Sum of `total_cost_usd` the CLI reported. |
| `wall_s` | Wall time of the agent step. |
| per model × arm | Median per run, **$ per accepted-correct**, **tokens per accepted-correct**. |

## 3. Lines of code

| Field | Definition |
|---|---|
| `src_nloc` | Non-blank, non-comment lines in the target source file(s). |
| `test_nloc` | The same, for test files. |
| `test_to_src` | `test_nloc / src_nloc`. |
| `lines_added`, `lines_removed`, `files_changed` | `git diff --numstat` against the starting commit, with everything the agent left staged. |

## 4. Complexity

From `lizard` 1.17.13, which reads Rust, Go and TypeScript the same way.

| Field | Definition |
|---|---|
| `functions` | Function count in the source. |
| `ccn_mean`, `ccn_max` | Cyclomatic complexity per function, mean and max. |
| `fn_len_max` | Longest function, in NLOC. |

These say whether a passing result is also a larger or more tangled one. Shape
gates add files and indirection, so they matter.

## 5. Model identity: evidence is per model, or it is void

A run is **void** (listed in the report, left out of the rates, never deleted) if:

- any call asked for a model other than the pinned one. `bin/claude` adds
  `--model` when a caller leaves it out: hexa's text-mode adapter
  (`hexa-infer/src/adapters/secondary/claude_code.rs:351`) never passes one;
- the CLI reports another model serving calls (`modelUsage`). A helper model
  the CLI uses for titles and summaries is allowed only if it is named in
  `HEXA_AB_HELPER_MODELS`, and the record says so;
- a transcript from the run mentions oracle material (`benchmarks/fixtures`,
  `oracle.test`, `setup_files`). The fixtures are in the hexa repo on disk, so
  this is checked rather than assumed;
- a call could not be metered, or no call was recorded at all.

## 6. Reporting

- Per model × arm: every metric above, over every launched run.
- Per model: **B−A** and **C−A** on accepted-correct, paired by task, with a 95%
  CI from a bootstrap that resamples tasks and then runs within each task.
- Proportions, not medians, for pass/fail metrics. Medians for size, cost and
  complexity.
- `runs/MANIFEST.txt` records the hexa commit, the `claude` CLI version, lizard,
  bun, models, arms, runs and the timeout.

## 7. Self-test (`selftest.sh`, no model calls)

- **Negative control:** every oracle must fail the untouched starting code.
  All 5 do.
- **Positive control:** a known-correct answer must pass its oracle.
  `selftest/header-safe-url.good.ts` does.

`run_matrix.sh` refuses to start until both hold.

## Found while building the self-test

- **`header-safe-url`'s oracle contradicted its own test name (fixed).** The
  test "a code point above U+00FF cannot ride in a header" expected `café` to be
  rejected, but `é` is U+00E9, below U+00FF, so an agent that followed the
  stated rule failed. The case now uses U+0101, which matches the test name and
  the fixture's provenance (non-Latin-1 URLs breaking the header). The first
  A/B run (`runs/`) used the old oracle; every arm passed it anyway.
- **Single-file fixtures always grade F 0** (see §1).

## Not covered yet

- Only the **change** family (5 fixtures) has oracles today. The 8 greenfield
  tasks, the 8 shape traps and the 3 `linkstore-svc` features each need an
  oracle and a pass condition written and frozen before any run. `run_one.py`
  takes new tasks through `load_task`.
- Arm B for greenfield should use `hexa scaffold … --grade A` (`hexa build` has
  no `--grade` and no `--model`).
- The blinded reviewer-model defect count is not implemented.
