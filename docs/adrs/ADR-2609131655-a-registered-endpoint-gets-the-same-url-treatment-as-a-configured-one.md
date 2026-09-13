# ADR-2609131655: A registered endpoint gets the same URL treatment as a configured one

**Status:** Accepted
**Date:** 2026-09-13
**Epoch:** hexa
**Drivers:** *"Why can't we drop into localai?"* — because `hexa hey` against the registered tt-gptoss endpoint answers `Unknown provider: http://127.0.0.1:7000 has no model 'openai/gpt-oss-120b'`. That endpoint serves exactly that model. The gateway never saw the request: hexa posted to a path that does not exist, got a 404, and blamed the model.

## Context

`OpenAiCompatAdapter` posts to `{base_url}/chat/completions`. Every convenience constructor builds
that base with the API version on it — `ollama()`, `vllm()` and `openrouter()` all append `/v1`. The
registry path does not: `complete::adapter_for` passes `endpoint.url` through untouched.

So a URL registered as `http://127.0.0.1:7000` becomes a POST to
`http://127.0.0.1:7000/chat/completions`, which returns 404, while
`http://127.0.0.1:7000/v1/chat/completions` returns 401 for a missing key — the gateway is up and
listening, and would have answered a properly addressed request.

The tool leads you into it. `hexa inference list` prints
`Register new: hexa config inference add ollama http://host:11434 --model <name>` — a URL with no
`/v1`, which is right for the Ollama adapter and wrong for every other provider family, all of
which fall through to the OpenAI-compatible one.

And the diagnosis is worse than the fault: 404 is reported as `has no model`, which sends you to
check the model name, the registry entry and the gateway's catalogue — none of which are wrong.
This is the third instance today of a confident wrong answer standing in for an unexamined one
(ADR-2609131617, ADR-2609131646).

## Decision

1. **A base URL with no path gets `/v1`.** `http://host:7000` and `http://host:7000/` both become
   `http://host:7000/v1`. A URL that already carries a path is respected exactly —
   `https://openrouter.ai/api/v1` and a deliberate `http://host:8000/inference` are left alone. One
   place, the adapter's constructor, so every caller gets it: the registry path, the convenience
   constructors, and anything added later.

2. **A 404 names the URL it posted to.** `no model 'x' at http://host:7000/v1/chat/completions`
   rather than `has no model 'x'`. A reader can then see the path, which is the thing that is
   usually wrong.

## Consequences

- The endpoint already in this machine's registry starts working without being re-registered, and
  so does every endpoint anyone registered by following the tool's own instructions.
- A deliberate non-standard path still works, because only a pathless URL is touched.
- This removes the transport blocker under "drop into localai". It does not make `hexa harden` use
  a local model: that harness shells out to `claude -p` and consults no endpoint at all, which is a
  separate decision about what an adversarial reviewer must be able to do.

## Gate

`cargo test -p hexa-infer base_url`: a pathless URL gains `/v1`, with and without a trailing slash;
a URL ending in `/v1` is unchanged; a URL with any other path is unchanged; and the 404 message
carries the full posted URL.

## Evidence

`cargo test -p hexa-infer base_url 2>&1 | grep -E 'base_url_tests::|test result: ok. 6'` at 4d014a0 with uncommitted changes on 2026-09-13 16:57 UTC:

```text
test adapters::openai_compat::base_url_tests::a_base_url_that_carries_a_path_is_left_alone ... ok
test adapters::openai_compat::base_url_tests::a_pathless_base_url_gains_the_api_version ... ok
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 82 filtered out; finished in 0.00s
```
