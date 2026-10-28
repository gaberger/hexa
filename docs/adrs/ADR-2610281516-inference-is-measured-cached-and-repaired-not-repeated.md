---
id: ADR-2610281516
status: proposed
date: 2026-10-28
---
# ADR-2610281516: inference is measured, cached, and repaired rather than repeated

**Status:** Proposed
**Date:** 2026-10-28
**Drivers:** On 2026-10-28 one design review of a 676-line draft took 14 completions and 9–11 minutes; stage 8 sent byte-for-byte the prompt stage 5 had sent, and three synthesizer calls deduplicated a single reviewer's list. Generating the example app's tests took 16 full regenerations, about two minutes each, and most were rejected for one compile error in the test file — a borrow-check slip a single follow-up would have fixed. None of this shows in `hexa spend`: `claude-text` records 0 tokens and no cost, so the cost of a hexa run is invisible. Its prompt also travels as one argv argument, which Linux caps at 128 KiB.

## Context

Every goldfish call is a fresh, tool-less completion by design (ADR-2610271736 §3); isolation forbids sharing a conversation, not sharing a cache. A completion at temperature 0 on identical input is safe to reuse. The spec writer must never see code (ADR-2610221540 §2), but its own reply and the compiler's verdict on that reply are not code.

## Decision

1. **`claude-text` reports what it cost.** It runs `claude -p --output-format json`, takes the reply from `result`, and records `usage.input_tokens`, `usage.output_tokens`, `usage.cache_read_input_tokens` and `total_cost_usd` in the spend log. The prompt is written to the process's stdin, never passed as an argument.
2. **An identical tool-less completion is answered once.** `CompletionCache` in `hexa-infer/src/ports.rs` (`get(key) -> Option<String>`, `put(key, reply)`) is backed by `~/.hexa/cache/completions/`, one file per key. The key is the sha256 of the model id, the system prompt and the user prompt. Only requests with no tools are cached. A hit is recorded in the spend log with source `cache` and zero tokens. `HEXA_NO_CACHE=1` and a `--fresh` flag on `hexa design` and `hexa drift` bypass it.
3. **A goldfish prompt puts what reviewers share first.** `goldfish_prompt` in `hexa-core/src/domain/design.rs` orders its input: hexa's framing, the document, the referenced sources, then the standard and the persona. Every reviewer of one review then shares the longest possible prefix, which a provider's prompt cache can reuse. The inputs are unchanged; only their order is.
4. **A stage with one reviewer has no synthesizer call.** `needs_synthesis(stage)` in `hexa-core/src/domain/design.rs` is true only when `stage_reviewers(stage)` has more than one persona. For a single reviewer, its list items are the stage's findings, numbered as before.
5. **A rejected test generation is repaired, not regenerated.** When `hexa spec tests` rejects a generation because the test file does not compile, names no test for some scenario, or holds an unpassable site, it sends the writer one follow-up: its own reply, and the error text (the compiler's messages about the test file, the missing scenario ids, or the sites). It does this at most twice, and each repaired file passes every check again from the start. The follow-up never contains source from outside the test file.

## Consequences

- `hexa spend` shows what a design review, a drift run and a test generation cost.
- Re-running a review or a drift on unchanged documents costs nothing; stage 8 within one review is free.
- A full review drops from 14 completions to 11, before caching.
- A test file with one slip costs one short follow-up instead of a fresh generation.
- A cached reply can be stale only if the model behind an id changes; `--fresh` and `HEXA_NO_CACHE` exist for that.

## Implementation

`hexa-infer/src/adapters/secondary/claude_code.rs` (JSON output, stdin), `hexa-infer/src/ports.rs` and `hexa-infer/src/complete.rs` (`CompletionCache`), `hexa-core/src/domain/design.rs` (`goldfish_prompt` order, `needs_synthesis`), `hexa-exec/src/egm.rs`, `hexa-exec/src/spec_author.rs` (the repair turn).

## Gate

`hexa spec verify ADR-2610281516`: the spec's sealed tests, each red before the code.

## References

- ADR-2610271736: goldfish isolation, which a cache keyed on the whole input preserves.
- ADR-2610221540: the writer never sees code.
- ADR-2610281442: the checks a repaired generation must pass again.
- ADR-2610270901: `claude-text`.
