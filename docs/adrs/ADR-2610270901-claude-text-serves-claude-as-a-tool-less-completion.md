---
id: ADR-2610270901
status: accepted
date: 2026-10-27
---
# ADR-2610270901: `claude-text` serves Claude as a tool-less completion

**Status:** Accepted
**Date:** 2026-10-27
**Drivers:** On 2026-10-27 `hexa spec draft` and `hexa spec tests` could not run. Every tier names an OpenRouter model whose key returns `401 User not found`; local `qwen3.5:27b` answers five tokens in 15 s and times out on a spec; and `claude-code` is refused because ADR-2610221540 §2 forbids a spec author or test writer that can read the repository. The operator: *"just use claude -p."* The drafts for ADR-2610261356, ADR-2610261420 and ADR-2610270853 were then made by hand-copying the `spec draft` prompt into `claude -p --tools ""`. That is the verb bypassed, and the gate's own isolation enforced by a script.

## Context

`claude -p` can be a completion. Measured on 2026-10-27, from an empty directory:

| Flags | Context besides the prompt |
|---|---|
| `--tools ""` | a SessionStart hook's output, MCP server instructions, the user's email, the environment block |
| `--tools "" --setting-sources "" --strict-mcp-config --disable-slash-commands` | the user's email and the environment block |

`--bare` would drop the rest but accepts only `ANTHROPIC_API_KEY`, not the subscription login `claude` normally uses.

The existing adapter, `ClaudeCodeInferenceAdapter` (`hexa-infer/src/adapters/secondary/claude_code.rs`), runs `claude -p --dangerously-skip-permissions <prompt>` in the caller's directory: an agent with every tool, in the repository. Its `collapse_prompt` also keeps only the last user message, so a system prompt is dropped whenever there is one. `served_by_frontier_cli` (`hexa-infer/src/wiring.rs`) treats any id starting `claude` as that agent.

## Decision

1. **`claude-text` is a completion, served by `claude -p` with nothing but the prompt.** The model id `claude-text`, or an endpoint registered with provider `claude-text`, is served by `ClaudeCodeInferenceAdapter` in text mode. It runs `claude -p --tools "" --setting-sources "" --strict-mcp-config --disable-slash-commands --no-session-persistence --output-format text`, in a new empty directory that is removed afterwards, whatever the outcome. It never passes `--dangerously-skip-permissions`: with no tools there is nothing to permit.
2. **The system prompt is sent as the system prompt.** Text mode passes the request's system prompt with `--system-prompt` and the conversation as the prompt. A request with a system prompt and a user message delivers both.
3. **`claude-text` is not an agent.** `served_by_frontier_cli` is false for it, so `hexa spec draft` and `hexa spec tests` accept it, and the do-loop drives it as a per-step completion. A request that carries tool definitions is refused with an error naming `claude-text`: it cannot run them, and silently dropping them would be a gate that degrades.
4. **`claude-text:<model>` picks the model.** The part after the colon is passed as `--model`; bare `claude-text` passes none and gets the CLI's default. No model id is written outside `hexa-infer`.
5. **The bootstrap is named, once.** This ADR's spec is drafted the way the Drivers describe, because the route it adds is the route a draft needs. Its tests are written by hand, committed red against the unbuilt adapter, recorded and sealed with `hexa spec seal`. No later ADR takes this exception.

## Consequences

- `hexa spec draft <ADR> --model claude-text` and `hexa spec tests <ADR> --model claude-text --crate <dir>` work with no API key, on the operator's Claude login.
- Setting a tier to `claude-text` routes that tier's completions through the CLI. It costs whatever the login's plan costs. `hexa bench agentic` can measure it like any model.
- The user's email still reaches the model. It is not repository content, and §2 is about the implementation.

## Gate

`hexa spec verify ADR-2610270901`: the spec's sealed tests, each red before the code (ADR-2610221540 §5).

## References

- ADR-2610221540: the spec author and test writer see only the ADR and its ports (§2).
- ADR-2610270853: the first ADR whose tests need this route.
