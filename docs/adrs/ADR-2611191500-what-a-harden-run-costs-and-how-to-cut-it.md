# ADR-2611191500: What a `hexa harden` run costs, and how to cut it

**Status:** Proposed (§1 implemented; §2–§5 proposed)
**Date:** 2026-11-19
**Drivers:** Strata's telemetry (`strata-tools/telemetry/run4.json`, the week to 2026-11-19) puts `hexa.inference_by_source.harden` at 258 calls and $89.51, out of $95.72 for all of hexa's inference. That is 94% of the spend on one verb.

## Context: where the money goes

Measured from `~/.hexa/inference-log.jsonl`, whose rows match the telemetry total exactly, with each call attributed to its run in `~/.hexa/runs/harden-*.json`:

- **The runs:** 18 harden runs. On average each cost **$4.97** and made **14.3 calls**, at **$0.35 per call** (median $0.31, p90 $0.56, max $1.47).
- **The most expensive targets:**
  - `apps/lintel`: $12.39 over 37 calls.
  - `apps/lintel/renderer`: hardened three times on 2026-11-16, for $19.17 in all.
  - Four dashboard files in harness: $6.75 to $7.56 each.

How the code produces that:

- **Every call is a full agent session on the frontier CLI's default model.** That is `claude -p --output-format json`, through `frontier_agent()` (`hexa-exec/src/lib.rs`) and `run_claude` (`hexa-exec/src/frontier.rs`). With no `--model` given (`hexa-cli/src/commands/build.rs`, `run_harden`), the CLI's default is Opus, and the log records the answers as Opus.
  - `inference.tier_models` is not used on this path. It configures only the local fallback reviewer (`review_model`, `hexa-exec/src/adversarial.rs`).
- **Calls per run are `4 + C + K`, with no cap:**
  - 4 hunt lenses (`LENSES`), in parallel, each with a 600 s deadline.
  - One skeptical verifier per candidate `C`, in parallel, 600 s each.
  - One fix session per confirmed finding `K`, in sequence, 900 s each.
  - Candidates are merged before verification only on an exact location-and-title match (`merge_duplicates`).
- **What each call sends:**
  - **The prompt is short.** It holds the instructions, the claim and up to eight lessons.
  - **The code is not inlined.** Each session reads the target itself with tools. So four lenses and every verifier read the same files again, and each turn sends the session's growing context back.
  - **Most of the money is context, not output.** Output tokens cost about $15 of the $89.51 (17%). Uncached input is negligible. The other ~83% is cache writes and reads inside these multi-turn sessions, which the spend log does not record (`take_answer` keeps only `input_tokens` and `output_tokens`).
- **Cost by phase** (two runs with logs, `apps/lintel/renderer`):

  | Phase | Cost per call | Share of run |
  |---|---|---|
  | Hunt | $0.75–0.81 per lens | 34–54% |
  | Verify | ~$0.24 per claim | 21–24% |
  | Fix | ~$0.39 per fix | 25–41% |
- **A timed-out agent kept spending.**
  - The `claude` child was spawned with `.output()` and no `kill_on_drop`. When the 600 s or 900 s deadline fired, the future was dropped and the process kept running. Whatever it spent was never recorded, because nobody read its answer.
  - Meanwhile `claude_run_retry` started a replacement, up to `DEFAULT_RETRIES = 3`. So one slow call could have up to four sessions billing at once.
  - How often this happened is not in the data: timeouts are not logged.
- **The model's price is the largest single factor.** One run that was pinned to Haiku (`counter.rs`, 2026-11-16) cost $0.005–0.014 per call, about 40× less. Whether it finds as much has not been measured.
- **Only a daily cap exists.** `inference.budget_usd_per_day` is checked before each call. There is no limit per run, and harness sets none.

## Decision

1. **Implemented: a call past its deadline is killed.**
   - `run_claude` sets `kill_on_drop(true)`, so a dropped timeout stops the agent, and the retry no longer runs beside an orphan.
   - It gets no process group of its own, so Ctrl-C at the terminal still reaches it.
   - Findings are unchanged: a timed-out call's answer was already discarded.
   - Gate: `cargo test -p hexa-exec --lib frontier::tests::a_claude_past_its_timeout_is_killed_not_abandoned`. It was red before the change (the fake `claude` was still running after its timeout) and is green after.
2. **Proposed: record what the context cost.** In `take_answer`, keep `cache_read_input_tokens`, `cache_creation_input_tokens` and `num_turns` from the envelope. This changes only what is observed, and it is needed before §3 or §4 can be judged on more than the 83% estimate.
3. **Proposed: an opt-in cap per run, `hexa harden --max-cost <usd>`.**
   - It sums the recorded cost of the run's calls, and issues no new hunt, verification or fix once the sum passes the cap.
   - The report says the run stopped at the cap and lists the findings that were not verified or fixed.
   - Off by default, so a default run is unchanged.
   - Gate: a fake `Frontier` and spend log. With the cap set, no call is issued past it. Without it, the run makes `4 + C + K` calls.
4. **Proposed, measured first: a cheaper tier for candidate generation.**
   - The hunt is 34–54% of a run and only proposes candidates. Every candidate is then verified skeptically, one per session. So a cheaper hunter (Sonnet, through `on_model`) risks missing candidates, not admitting false ones.
   - Verification and fixes stay on the frontier default.
   - Ship it only after `hexa bench`-style A/B runs on targets with known findings show the cheaper hunter's recall holds. Its saving is about 30% of a run at Sonnet prices.
5. **Proposed: do not harden an unchanged tree again.**
   - Key the last clean pass by the target's tree hash and the gate. A run on the same key reports the earlier result unless `--again` is given.
   - `apps/lintel/renderer` cost $19.17 over three passes in one day.
   - Model answers vary, so this is a policy choice (repeat passes do find new things) more than a pure saving. It belongs to the operator.

**Rejected for now:**
- **One hunt session for all four lenses.** Cheaper, but the lenses exist because a single focus misses classes of bug.
- **Verifying claims in batches.** Each claim is judged alone so one verdict does not colour another.
- **Merging candidates by location alone.** This merges distinct bugs.

All three trade findings for money, which this ADR does not do without measurement.

## Gate

`cargo test -p hexa-exec --lib frontier::tests::a_claude_past_its_timeout_is_killed_not_abandoned`

## Consequences

- The orphaned-agent leak is closed. The saving is real but unmeasured, because timeouts were never logged.
- The large savings (§3, §4) need a decision and a measurement, not just a patch: a per-run cap stops a run early, and a cheaper hunter can miss bugs.
- ADR-2610221834 (fix test-first in parallel worktrees) doubles fix sessions. If it is accepted, its cost should be weighed against this ADR's numbers.
