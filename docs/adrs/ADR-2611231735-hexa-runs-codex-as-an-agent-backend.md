# ADR-2611231735: hexa runs Codex as an agent backend, as it runs Claude Code

**Status:** Accepted
> 2026-11-24 Accepted: Codex runs as an agent backend like Claude Code: same spawner, bypass flag fixed, refused as test writer; built gate-first and verified against the real CLI
**Date:** 2026-11-23
**Drivers:** Craig has `codex` (codex-cli 0.171.0, logged in with a ChatGPT plan, no API key) and asked hexa to use it. hexa reaches a subscription agent CLI only through `claude -p` (`ClaudeCodeInferenceAdapter`). A second agent CLI is configuration for the operator and a new adapter for hexa, so it is a new provider and a decision, not a bug fix. It departs from the Python-and-bug-fixes scope Craig set on 2026-11-22; he lifted it for this on 2026-11-23.
**Supersedes:** none. Extends ADR-2610221540 §2 (an agent CLI is never the spec author) to a second CLI.

## Context

- `hexa-infer/src/adapters/secondary/claude_code.rs` spawns `claude -p --dangerously-skip-permissions` behind an injectable `ProcessSpawner`, and `hexa-infer/src/wiring.rs` (`adapter_for_endpoint`) picks it by the registered provider name `claude-code`. A model it serves is an agent: it reads the repository itself, so `hexa spec tests` refuses it as the test writer.
- `codex exec [PROMPT]` is Codex's non-interactive form: the prompt on the argument or on stdin, `-m <model>`, `-s <sandbox>`, `--dangerously-bypass-approvals-and-sandbox`, `--skip-git-repo-check`, `--ephemeral`, `--json`, and `-o <file>` for the last message. It needs no API key when logged in.
- `hexa inference add` offers templates for HTTP providers only (`hexa-cli/src/commands/inference.rs`).

## Decision

1. **A provider named `codex` is served by a `CodexInferenceAdapter`.** `hexa-infer/src/adapters/secondary/codex.rs` implements `IInferencePort` (`hexa-core/src/ports/inference.rs`) by spawning `codex exec` behind the same `ProcessSpawner` that `ClaudeCodeInferenceAdapter` uses, so a test supplies a fake spawner and no test needs a `codex` binary. Its constructor is `CodexInferenceAdapter::with_spawner(model: Option<String>, spawner: Arc<dyn ProcessSpawner>)`, where `ProcessSpawner` and `SpawnedProcess` are those of `hexa-infer/src/adapters/secondary/claude_code.rs`, public at `hexa_infer::adapters::secondary::claude_code`. `ProcessSpawner` is an `async_trait` with `async fn spawn(&self, program: &str, args: &[String]) -> Result<SpawnedProcess, InferenceError>`; `SpawnedProcess` has `stdout: Vec<u8>`, `stderr: Vec<u8>` and `exit_code: i32`; `InferenceError` is `hexa_core::ports::inference::InferenceError`. The tests live in the `hexa-cli` crate, which depends on `hexa-infer`; a test of §5 or §6 runs the `hexa` binary (`env!("CARGO_BIN_EXE_hexa")`) in a temporary project, with a fake `codex` executable first on `PATH`, as the other `hexa-cli` spec tests do, and never calls a `hexa_cli` library path. The skeleton `CodexInferenceAdapter` exists at the red commit and returns `ProviderUnavailable` from every call, so a test builds and fails; a test observes an adapter only through `IInferencePort` calls, never by downcasting it. A fake spawner stands in for `codex` by writing the reply to the path that follows `-o` in the arguments it receives. A registered endpoint is `hexa_infer::Endpoint` (`hexa-infer/src/endpoint.rs`: `id`, `url`, `provider`, `model`, `models`, `status`, `requires_auth`, `secret_key`, `health_checked_at`, `quality_score`, `quantization_level`), and `adapter_for_endpoint(&Endpoint) -> Box<dyn IInferencePort>` is `hexa_infer::wiring::adapter_for_endpoint`. `adapter_for_endpoint` in `hexa-infer/src/wiring.rs` builds it for provider `codex`.
2. **The permission-bypass flag is hardcoded, as Claude's is.** Every `complete()` passes `--dangerously-bypass-approvals-and-sandbox`, with no constructor knob and no environment override, because a `codex exec` that stops to ask for approval exits without an answer. A test asserts the flag is in the arguments the spawner received. Craig chose this to match the existing adapter (2026-11-23).
3. **The reply is the agent's last message.** The adapter runs `codex exec --skip-git-repo-check --ephemeral -o <temp file>` and returns the file's contents as the text. A non-zero exit, or an empty file, is an `InferenceError`, never an empty success.
4. **The model id is passed through.** A registered model other than the bare id `codex` is passed as `-m <model>`; the bare id `codex` passes no `-m`, so Codex's own default model is used, as `claude-code` passes no `--model`.
5. **A Codex-served model is an agent, never a completion.** `served_by_frontier_cli_in` in `hexa-infer/src/wiring.rs` and `completion_path_open` in `hexa-cli/src/commands/spec_ledger.rs` treat provider `codex` as they treat `claude-code`: `hexa spec tests` and `hexa spec draft` refuse it, with the same message naming the rule (ADR-2610221540 §2).
6. **`hexa inference add codex` registers it.** It runs `codex login status`, refuses with the command to run when not logged in, and writes an endpoint with provider `codex` and no URL and no key reference. `hexa inference test` reports it reachable when `codex --version` exits 0.

## Consequences

- Not decided here: delegating the whole do-loop to Codex. `hexa-exec/src/frontier.rs` still delegates a task to `claude` alone; that is a later ADR.
- A project with Codex can use it for completions and the agent path that goes through `IInferencePort`, with no key in any file.
- Codex runs unsandboxed, as Claude does. The ADR says so rather than hide it.
- `spec tests` keeps its rule: neither agent CLI writes the tests. A second CLI adds no way around the gate.

## Implementation

`hexa-infer/src/adapters/secondary/codex.rs` (new), `hexa-infer/src/adapters/secondary/mod.rs`, `hexa-infer/src/wiring.rs`, `hexa-cli/src/commands/spec_ledger.rs`, `hexa-cli/src/commands/inference.rs`.

## Gate

`hexa spec verify ADR-2611231735`: the spec's sealed tests, each red before the code.

## References

- ADR-2610221540 §2: the test writer sees only the ADR and its ports.
- ADR-2610270901: `claude-text`, the tool-less Claude completion.
- `claude_code.rs`: the adapter this one follows.
