# Inference

hexa names no model in its source. Every model is read from configuration, and
every provider lives in one crate, `hexa-infer`. This page covers the
configuration.

## Providers

`hexa config inference` manages the registry at `~/.hexa/inference-servers.json`.

```bash
hexa config inference list                       # what is registered
hexa config inference add ollama                 # local server, default URL
hexa config inference add ollama http://gpu-box:11434 --model qwen2.5-coder:14b
hexa config inference add openrouter --model meta-llama/llama-4-maverick
hexa config inference test <id>                  # send one prompt, confirm a reply
hexa config inference discover                   # scan for local and remote servers
hexa config inference remove <id>
```

Provider types: `ollama`, `vllm`, `openai-compat`, `openrouter`, `groq`,
`cerebras`, `sambanova`, `together`, `gemini`. Every cloud provider speaks the
OpenAI chat protocol through one adapter.

### API keys

Keys are environment variables. hexa reads them at dispatch and writes them to no
file.

| Provider | Variable |
|---|---|
| OpenRouter | `OPENROUTER_API_KEY` |
| Groq | `GROQ_API_KEY` |
| Gemini | `GEMINI_API_KEY` |
| Anthropic | `ANTHROPIC_API_KEY` |

The local server's address is `OLLAMA_HOST`. It accepts `host:port` or a full
URL. hexa normalises both.

### The frontier path

`hexa scaffold`, `hexa build` and `hexa harden` delegate to a logged-in `claude`
CLI. That path needs no API key and has no memory ceiling. `hexa do` reaches it as
a fallback when the local candidates fail.

## Tiers

A task's tier selects a model. The mapping lives in `.hexa/project.json`.

```json
{
  "inference": {
    "tier_models": {
      "t1":   "qwen3:4b",
      "t2":   "gemma4-12b",
      "t2.5": "devstral-small-2:24b"
    },
    "react_models": ["devstral-small-2:24b", "claude-code"]
  }
}
```

| Tier | Work |
|---|---|
| `t1` | Scaffolding, transforms, classification. The cheapest model that works. |
| `t2` | Ordinary code generation. |
| `t2.5` | Cross-file reasoning and design. |
| `t3` | Frontier work. Handled by the `claude` path, not by this table. |

`react_models` is the ordered candidate list for `hexa do`. The loop runs each in
turn and commits the first whose edit passes the gate. The gate picks the
winner, not a classifier, so a wrong order costs latency and nothing else.

A project that configures no model for a tier gets an error naming the missing
key. There is no default model anywhere in the source.

## Benchmarking

Leaderboard rank does not predict performance in the agent loop. Measure.

```bash
hexa bench agentic                          # the corpus, through the real loop
hexa bench agentic --model qwen2.5-coder:14b --arms fast
hexa config inference bench <id>            # code-gen, reasoning, identity prompts
```

`hexa bench agentic` runs each fixture in an isolated worktree and reports edit
rate and evidence pass rate per model. The corpus is in `docs/benchmarks/`.

## The local ceiling

The same task, three languages, per-model pass rate on a 16 GB GPU:

| Model | Rust | TS | Go |
|---|---|---|---|
| devstral-small-2:24b | 5/5 | 3/3 | 2/3 |
| gemma3:12b | 4/5 | 2/3 | 1/3 |
| qwen2.5-coder:14b | 0/5 | 2/3 | 0/3 |
| gpt-oss:20b | 0/5 | 1/3 | 0/3 |

TypeScript is forgiving. Rust and Go are strict, and weaker models fail both.
The top-of-the-leaderboard local model scored last. The ceiling depends on your
language, and `hexa bench agentic` tells you where it is on your hardware.

## Hardware

Memory is the constraint before compute. A 24B model at Q4 needs about 15 GB
resident. A 14B model needs about 9 GB. The resource governor in `hexa-exec`
checks available memory before each candidate and routes to the frontier path
when the local model will not fit.

`hexa config inference gpu-check` confirms the local server is using the GPU.


## Discovery

`hexa doctor` and `hexa bootstrap` find every path to a model the machine
holds, without being told:

| Path | Discovered from |
|---|---|
| local server | `HEXA_OLLAMA_HOST`, then `OLLAMA_HOST`, then the default address; probed |
| anthropic | `ANTHROPIC_API_KEY` present (`ANTHROPIC_BASE_URL` optional); the key is never printed |
| openai-compatible | `HEXA_INFERENCE_URL` (`HEXA_INFERENCE_MODEL`, `HEXA_INFERENCE_KEY`); probed |
| vllm | `HEXA_VLLM_HOST` (`HEXA_VLLM_MODEL`, `HEXA_VLLM_KEY`); probed |
| registered endpoints | `~/.hexa/inference-servers.json`, from `hexa config inference add`; probed |
| claude | a `claude` CLI on PATH |

Any one open path is enough to run. Bootstrap fails only when there is none.

## Spend and budget

Every inference hexa makes appends one line to `~/.hexa/inference-log.jsonl`:
model, input and output tokens, the source (`complete` for a local or API
provider, `react` for the frontier candidate in the loop, `harden` for the
adversarial pass), and a cost in dollars when the provider reported one. A
local model reports none and is not priced. A `claude -p` call reports its
own `total_cost_usd`, and that figure is recorded as given.

```bash
hexa spend            # today, last 7 days, all; by source and by model
hexa spend --json     # the same, with an explain block
```

A daily budget is optional:

```json
{ "inference": { "budget_usd_per_day": 5.0 } }
```

With it set, the frontier path refuses to run once today's priced spend has
reached it, and says so. Local calls are never refused; they cost nothing.
