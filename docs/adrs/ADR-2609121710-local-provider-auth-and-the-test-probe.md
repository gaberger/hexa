# ADR-2609121710: Registered local providers must be able to authenticate, and `test` must prove it

**Status:** Proposed
**Date:** 2026-09-12
**Drivers:** registering a Tenstorrent vLLM server (gpt-oss-120b on :7000, bearer-protected) as a hexa provider on tt-quietbox. Three separate things stood between `hexa config inference add` and a working local model, and none of them said so.

## Context

The server is a plain OpenAI-compatible endpoint that requires `Authorization: Bearer <key>`.
`curl` with the key returns 200 on `/v1/models`. What hexa did:

1. **`add vllm … --key <key>` never sends the key.** `Endpoint::requires_auth` is false for the
   `vllm` / `ollama` / `llama-cpp` class (hexa-infer/src/endpoint.rs, the provider-tier match), so
   the key is stored and ignored. A vLLM behind `--api-key` — the normal way to expose one — is
   therefore unreachable through the type named after it.
2. **A literal key is silently treated as a variable name.** `resolve_secret()` returns the key
   as-is only when it starts with `sk-`; anything else is looked up in the environment and, when
   absent, the candidate is skipped. A JWT (`eyJ…`), a TT-Studio key, a hex token: all fail with
   no message. The intent (a reference, never a secret on disk) is right; the heuristic is not
   the intent.
3. **`hexa config inference test` proves nothing about auth.** It GETs `/api/tags` then
   `/v1/models` with no headers (hexa-cli/src/commands/inference.rs, the `/v1/models` fallback),
   so an authenticated server always reports 401 from `test` while dispatch would have worked —
   or the reverse, when the reference does not resolve. The verb that exists to answer "can hexa
   talk to this?" cannot answer it.

Observed together, the operator sees: `add` says "registered", `test` says 401, `do` quietly
falls through to `claude-code`, and the box's own silicon sits idle while a frontier model does
the work. That is the failure hexa's founding goal G1 is meant to prevent.

## Decision

1. `requires_auth` is decided by the presence of a key, not by the provider class. A `vllm` or
   `ollama` endpoint registered with `--key` sends it; one registered without does not.
2. `--key` distinguishes a reference from a literal explicitly: `--key-env NAME` stores a
   reference; `--key VALUE` stores a literal and warns once that a secret is now on disk in
   `~/.hexa/inference-servers.json`. `resolve_secret()` resolves by which field is set, and a
   reference that does not resolve at dispatch is a named error ("TT_STUDIO_GATEWAY_KEY is not
   set in this shell"), never a silent skip.
3. `hexa config inference test` sends the resolved credential on both probes and, when
   `/v1/models` succeeds, sends one minimal chat completion to the configured model and reports
   the round-trip time. Its exit code is the answer to "will `hexa do` reach this model".
4. Not changed: `scaffold`, `build` and `harden` keep the frontier path (`claude -p`). The owner
   confirmed that on 2026-09-12: "No use frontier". A local path for those verbs is a separate
   decision.

## Consequences

- A bearer-protected local server works through the provider type named after it.
- Secrets stay out of the registry by default, and an operator who puts one there is told.
- `test` becomes a gate in hexa's own sense: it can fail, with an exit code, for the reason that
  matters.
- Existing registries: an entry with `apiKeyRef` that resolves in the environment keeps working;
  one holding a literal that is not `sk-…` starts working instead of being skipped.

## Implementation

- `hexa-infer/src/endpoint.rs`: `requires_auth` from key presence; split `secret_key` into
  `key_env: Option<String>` / `key_literal: Option<String>` with a migration reader for the
  current `apiKeyRef`; `resolve_secret()` returns `Result<(), ResolveError>` naming the variable.
- `hexa-cli/src/commands/inference.rs`: `--key-env`; the warning on `--key`; `test` sends auth
  and one chat completion; non-zero exit on failure.
- Tests: a bearer-protected stub server that 401s without the header and 200s with it, driven
  through `add vllm --key`, `add openai-compat --key-env`, and `test`; a registry fixture holding
  a literal JWT that must dispatch.

## References

- hexa-infer/src/endpoint.rs (`resolve_secret`, provider tiers)
- hexa-cli/src/commands/inference.rs (`test`, the `/v1/models` fallback)
- ADR-2609121400 (two gates; a document that cannot fail cannot be trusted)
- Observed on tt-quietbox, 2026-09-12, registering `openai/gpt-oss-120b` on `http://127.0.0.1:7000`.
