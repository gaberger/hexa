# ADR-2611041000: The claude-code adapter runs the model it reports

**Status:** Proposed
**Date:** 2026-11-04
**Drivers:** The bench/ab A/B run (`bench/ab/RESULTS.md`) pinned one model per arm and checked it on every call. hexa's agent-mode `claude -p` calls arrived with no `--model`, so they ran on whatever the CLI defaults to. The bench had to add the flag itself to keep the comparison honest. Run outside the bench, the same calls are recorded as having used the requested model, and as costing nothing.

## Context

`ClaudeCodeInferenceAdapter` in `hexa-infer/src/adapters/secondary/claude_code.rs` implements `IInferencePort` (`hexa-core/src/ports/inference.rs`). It has two modes (ADR-2610270901):

- **Text** (`claude-text`), which passes `--model` when one is named, and
- **Agent** (`claude -p` with tools), which does not.

In agent mode, `complete` and `complete_stream` build their arguments with `args_for_prompt(prompt)`, which takes no model. Then they return `InferenceResponse { model_used: request.model, input_tokens: 0, output_tokens: 0, … }`. Three things are wrong with that:

1. **The model asked for is not the model run.** `InferenceRequest.model` names it, the CLI never hears of it, and the call runs on the CLI's default.
2. **The response says otherwise.** `model_used` echoes the request instead of reporting what ran, so every log and spend record downstream names a model that may not have run.
3. **The cost is recorded as zero.** Agent mode reads `--output-format text`, which carries no usage, so `hexa spend` misses these calls entirely.

`hexa-exec/src/frontier.rs` already does all three correctly for its own calls. It passes `--model` from `hexa_infer::wiring::frontier_model_arg`, asks for `--output-format json`, and records the usage the CLI reports.

## Decision

1. **Agent mode passes the request's model to the CLI.** When `frontier_model_arg(request.model)` is `Some(m)`, the arguments carry `--model m`. When it is `None` (`claude`, `claude-code`, or no model), no `--model` is passed, as today.
2. **Agent mode asks the CLI for JSON** (`--output-format json`) and answers with that JSON's `result` text as the content, so callers see the same text as today.
3. **`model_used` is the model the CLI reports**: the model in the JSON's `modelUsage` that has the most output tokens. Only when the CLI reports none does `model_used` fall back to `request.model`.
4. **The response carries the usage the CLI reports.** `input_tokens`, `output_tokens`, `cache_read_tokens` and `cache_write_tokens` come from the JSON's `usage`, not from a constant 0.
5. **Output that is not JSON is still an answer.** If the CLI's stdout does not parse as JSON, the whole stdout is the content, the tokens are 0 and `model_used` is `request.model`, as today. A CLI that cannot report usage costs this record its usage, not the caller its answer.
6. **`--dangerously-skip-permissions` stays where it is.** Nothing in this ADR changes it (`always_passes_dangerously_skip_permissions` holds).

## Consequences

- A caller that names a model gets that model, in agent mode as in text mode.
- Logs and `hexa spend` name the model that ran and count what it cost.
- G1 (`founding-goals.md`) is served better: the model is chosen by the tier in `.hexa/project.json`, and now that choice reaches the call.
- Agent-mode output is parsed once more; a malformed reply degrades to today's behaviour (decision 5).

## Gate

`cargo test -p hexa-infer claude_code`, with a fake `ProcessSpawner`:

- a request naming a frontier model spawns `claude` with `--model <m>` and `--output-format json`;
- a request for `claude-code` spawns without `--model`;
- given JSON with `result`, `usage` and `modelUsage`, the response's content, tokens and `model_used` come from it;
- given stdout that is not JSON, the content is that stdout, the tokens are 0 and `model_used` is the request's model;
- `--dangerously-skip-permissions` is still in every agent-mode argument list.
