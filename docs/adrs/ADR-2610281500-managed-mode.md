---
id: ADR-2610281500
status: proposed
date: 2026-10-28
---
# ADR-2610281500: managed mode, hexa driven by a harness that owns every model call

**Status:** Proposed
**Date:** 2026-10-28
**Drivers:** gary/hexa#13. parley drives hexa as its coding agent (parley D21, D22) inside a sandbox whose only network is parley's metered model proxy (parley design 0003 §4.4, §4.5). Today hexa decides for itself where a model call goes: the registry, `.hexa/project.json`, `ANTHROPIC_*`, `HEXA_OLLAMA_*` and a tool-use path that goes straight to Ollama. It writes its state into the repository's `.hexa/`. Its evidence commands inherit its whole environment. `hexa do run` prints coloured lines rather than a result a caller can parse. Under a harness, each of these is a leak: of budget, of the token, or of the repository's cleanliness.

## Context

parley's contract is in its design 0003 §4.5, quoted in gary/hexa#13. Every completion passes through one function, `RegistryBackends::complete` in `hexa-infer/src/wiring.rs`. Tiers are resolved in several places (`tiers.rs`, `direct_exec.rs`, `direct_react.rs`, `egm.rs` and others), each reading `.hexa/project.json` itself. There is no central resolver for `.hexa`: about a hundred call sites join the path themselves. The tree lock lives in the git common dir, which is not the working tree. parley never enables the `claude -p` delegate: parley D23 runs `claude -p` outside the sandbox, behind the proxy.

## Decision

1. **Entering managed mode.** `HEXA_MANAGED=1` enters it, and no other value does. `hexa-infer/src/managed.rs` holds `Managed::from_env(read)`, which takes an injected environment reader and returns the settings or a `ManagedError` naming the variable and what is wrong. Settings:
   - `HEXA_MANAGED_URL`: an `http://` or `https://` URL with no path beyond `/` and no query.
   - The token: read once from the inherited file descriptor named by `HEXA_MANAGED_TOKEN_FD` (a decimal fd of at least 3), at most 4 KiB, trimmed of one trailing newline, and non-empty. The fd is closed after the read. The token is never taken from the environment.
   - `HEXA_MANAGED_TIERS`: a JSON object with exactly the keys `t1`, `t2`, `t2.5`, `t3` and `react`, each `{"model": "<non-empty>", "api": "anthropic" | "openai"}`.
   - `HEXA_STATE_DIR`: an absolute path.
   - `HEXA_MANAGED_DEADLINE` (optional): Unix seconds.
   - `HEXA_MANAGED_DELEGATE`: refused when set. Managed mode never runs `claude`. The delegate of design 0003 §4.5 is deferred until a harness needs it; parley does not (D23).
2. **Fail loudly, before any model call.** In managed mode, a missing or malformed setting exits with status 64. It prints a one-line message on stderr and, with `--json`, the result object with `code: "config"`. No completion is attempted and nothing is written.
3. **Every completion goes to the managed URL.** When managed mode is active, `RegistryBackends::complete` ignores the registry, `HEXA_COMPLETION_COMMAND`, `ANTHROPIC_*`, `HEXA_OLLAMA_*`, `HEXA_VLLM_*`, `HEXA_INFERENCE_*`, `.env` files and the tool-use Ollama path. It routes each request by its model's `api` in the tier map. Anthropic requests go to `<url>/v1/messages` with `x-api-key: <token>`; OpenAI requests go to `<url>/v1/chat/completions` with `Authorization: Bearer <token>`. A model that is not in the tier map is refused, and no call is made.
4. **Tiers come from the map.** In managed mode, every tier lookup (`tier_model`, `react_models` and the direct and react resolvers) answers from `HEXA_MANAGED_TIERS`. `--model` and the `HEXA_*_MODEL` variables are ignored, and so is `.hexa/project.json`'s `inference`.
5. **Retries and refusals.** A connection error or a 5xx from the managed URL is retried up to 5 times, waiting 2, 4, 8, 16 and 32 s. A 401 or 402 is not retried. hexa stops at once with code `auth` (401) or `budget` (402).
6. **State goes to `HEXA_STATE_DIR`.** In managed mode, everything hexa writes for itself goes under `HEXA_STATE_DIR`: `loop.json`, memory, the run logs, the inference log and the do-run worktrees. `HEXA_HOME` is treated as `HEXA_STATE_DIR`. A managed `hexa do run` leaves the working tree with only the task's changes and its commit. It creates no `.hexa/` and no `.hexa-autoruns/`.
7. **Evidence never sees the token.** In managed mode, evidence commands and every other child process run with `HEXA_MANAGED_URL`, `HEXA_MANAGED_TOKEN_FD`, `HEXA_MANAGED_TIERS` and every `ANTHROPIC_*` variable removed. The token fd is already closed (§1). hexa sets `PR_SET_DUMPABLE=0` on itself once it has read the token.
8. **Deadline.** With `HEXA_MANAGED_DEADLINE`, hexa makes no model call and runs no evidence after the deadline. It stops with code `timeout` early enough to print its result.
9. **The result.** `hexa do run --json` prints one JSON object on stdout when it exits: `{"ok": bool, "commit": "<sha>" | null, "attempts": n, "code": <code> | null, "message": "…"}`. `code` is one of `gate_failed`, `timeout`, `budget`, `auth`, `model_error`, `config` and `internal`. Nothing else is printed on stdout. The exit status is 0 when `ok`, 1 when not, and 64 for `config`. `--json` also works outside managed mode.

## Consequences

- A harness can run hexa with a budget it controls, and hexa cannot spend around it, leak the token to code the agent wrote, or dirty the repository with its own state.
- Outside managed mode nothing changes: every decision above applies only when `HEXA_MANAGED=1`, except `--json` (§9).
- The delegate is not available under a harness until it is decided again; such a harness gets the react loop.
- About a hundred `.hexa` path sites stay as they are. §6 is enforced by a test that runs a managed `do run` in a fresh repository and checks the working tree afterwards, not by rewriting every site.

## Implementation

`hexa-infer/src/managed.rs` holds the settings, parsing, routing and retries. `hexa-infer/src/wiring.rs` and `tiers.rs` defer to it when it is active. `hexa-exec` resolves its state paths through `HEXA_STATE_DIR` in managed mode, and strips the environment of its children (`run_evidence`). `hexa-cli/src/commands/direct.rs` gains `--json`, the exit codes and the deadline. The gate is `hexa-infer/tests/adr_2610281500_spec.rs` (the settings and routing, against a stub server) and `hexa-cli/tests/adr_2610281500_spec.rs` (a managed `hexa do run --json` in a fresh repository against a stub server).

## Gate

Written first and shown failing before the code:

- `cargo test -q -p hexa-infer --test adr_2610281500_spec` — the settings, the routing, the retries and refusals (§1–§5).
- `cargo test -q -p hexa-infer --test adr_2610281500_wiring_spec` — process-wide activation with real variables and a real token fd (§3, §4, §7).
- `cargo test -q -p hexa-cli --test adr_2610281500_spec` — a managed `hexa do run --json` end to end against a stub (§2, §5–§9).

Outside this repository, parley's `tests/shape_hexa_through_proxy.rs` runs the same contract against parley's real model proxy and real models.
