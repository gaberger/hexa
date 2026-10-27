# Drift report — 2026-10-27

Elephant-Goldfish Model `egm-drift-report`, run against hexa at `1e19308`.
Scope: the eight living documents. The 55 ADRs are history and were not
checked; `hexa adr doctor` covers their citations. Each section was checked
by a separate read-only reviewer; the serious findings were re-verified by
hand.

**Code wrong, not the docs** (the doc states a rule the code breaks):

- The grader misses inline adapter → adapter calls: eight `hexa-exec/src/tools/*` adapters call `crate::local_store::…` / `crate::telegram_notifier::…`, and `hexa analyze` reports 0 violations.
- Founding goal G1: model and provider names in `hexa-cli/src/commands/inference.rs` (`qwen3:4b` default, `"llama3"` fallback, `DEFAULT_MODELS`, `Command::new("ollama")`).
- `HEXA_OLLAMA_HOST` is documented and never read (`host_env` is `OLLAMA_HOST`, so discovery checks `OLLAMA_HOST` twice).
- `config inference add <template> --model M` silently drops `--model`.
- `bootstrap --profile ci` is validated and then ignored.
- Spend from `hexa build`/`scaffold` is logged as `harden`.

**Docs wrong:** the rest — stale counts, hidden verbs used as primary
(`hexa analyze`, `hexa init` are hidden aliases of `hexa dev analyze`,
`hexa dev init`), and descriptions that predate code.

## README.md

- Doc says "with all 165 of its source files in a layer" (README.md:466) — `hexa analyze .` reports `coverage 169/169 files in a layer`.
- Doc says "`hexa --help` lists every verb" (README.md:184) and uses `hexa init` (README.md:103) and `hexa analyze` (README.md:131-133, 206-209, 242, 286, 369, 380) as primary verbs — both are `#[command(hide = true)]` (hexa-cli/src/main.rs:303-305, 354-356), help text "(hidden) … use `hexa dev analyze` / `hexa dev init` instead".
- Sample scaffold output "✓ 2 designs → 2 critiques → build GREEN" (README.md:113) — code prints `"{} designs → {} critiques → spec {}ch → build {}"` (hexa-cli/src/commands/scaffold.rs:152).
- Sample output "✓ architecture grade: A+ — score 100/100 (floor A)" (README.md:115) — code appends `, {} boundary violation(s)` (hexa-cli/src/commands/scaffold.rs:206).

## ARCHITECTURE.md

- hexa-core holds "the inference port and its mock" (ARCHITECTURE.md:129) — no mock in hexa-core; the only mock is `MockProcessSpawner` in hexa-infer/src/adapters/secondary/claude_code.rs:651.
- hexa-core has "Zero runtime dependencies" (ARCHITECTURE.md:129) — hexa-core/Cargo.toml depends on tokio (sync), chrono, uuid, futures, async-trait, serde, serde_json, thiserror.
- Lists hexa-core ports as `edit`, `spec`, `provenance` (ARCHITECTURE.md:129) — omits `ports::gate_ledger`, `ports::intent`, and the top-level `quantization` and `resource_governor` modules (hexa-core/src/lib.rs:26-27).
- "No file outside this crate names a provider or a model" (ARCHITECTURE.md:130) — hexa-cli/src/commands/inference.rs:23 (`PROVIDER_TEMPLATES`), :149 (`default_value = "qwen3:4b"`), :292/:314 (`Command::new("ollama")`), :351 (`"llama3"`); hexa-exec/src/frontier.rs:29/:58 spawn `claude`.
- hexa-exec use cases touch only `Tool`, `Worktrees`, `Frontier`, `RunLog`, `MemoryStore`, `Provenance` (ARCHITECTURE.md:131) — also `GitHistory` (hexa-exec/src/ports.rs:68, wired by `default_git_history()`), and `telegram_notifier` is a concrete adapter behind no port (hexa-exec/src/telegram_notifier.rs:10).
- `default_tools()` registers read/verify tools only plus `propose_edit` (ARCHITECTURE.md:42-44) — it registers 16, including `web_search`, `adr_draft`, `spec_draft`, `code_patch`, `workplan_emit`, `adr_status_set`, `escalate_to_operator`, `cost_meter`, `memory_search` (hexa-exec/src/lib.rs:59-74); the read/verify restriction is `ALLOWED_TOOLS` in hexa-exec/src/direct_react.rs:40-41; `propose_edit` is handled inside direct_react.rs, not registered.
- "Adapters never import other adapters", inline paths checked, "0 boundary violations" (ARCHITECTURE.md:214, 225, 236) — secondary adapters in `hexa-exec/src/tools` call other secondary adapters inline: escalate_to_operator.rs:110 (`crate::telegram_notifier`), memory_search.rs:94, cost_meter.rs:67, adr_draft.rs:116, spec_draft.rs:105, code_patch.rs:246, workplan_emit.rs:202, adr_status_set.rs:149 (`crate::local_store`). `hexa analyze` reports none.
- "165/165 files in a layer" (ARCHITECTURE.md:213-214) — now 169/169.
- `post-edit` hook "runs `hexa analyze --file`" (ARCHITECTURE.md:167) — `post_edit` (hexa-cli/src/commands/hook/mod.rs:674-688) only touches the loop and counts edits; `analyze --file` is a separate PostToolUse entry (hexa-cli/assets/templates/hexa-claude-settings.json:89).
- Hook table (ARCHITECTURE.md:159-167) omits `session-end`, `observe-pre`, `observe-post`, `observe-stop` (`hexa hook --help`).
- `.hexa/loop.json` records "the ADR, the gate and the stage" (ARCHITECTURE.md:148-149, 179) — it is keyed by session (`{"sessions": {id: entry}}`, hexa-cli/src/commands/loop_cmd.rs:181-196, 241), entries also hold tasks and evidence, and writers take `.hexa/loop.lock` (loop_cmd.rs:210-222).
- hexa-cli: "Every verb is a primary adapter" (ARCHITECTURE.md:135) — it also holds the secondary adapter `ReqwestProbe` (hexa-cli/src/http_probe.rs:23).

## docs/EVIDENCE.md

- `cargo test -p hexa-exec --lib vacuous` "Expected: `5 passed`" (EVIDENCE.md:50-53) — 7 tests in `mod vacuous_tests` (hexa-exec/src/direct_exec.rs:384).
- `cargo test -p hexa-infer --lib local_provider` "Expected: `5 passed`" (EVIDENCE.md:195-198) — 6 (hexa-infer/src/local_provider.rs:96, incl. `no_test_in_this_crate_writes_the_process_environment`).
- `cargo test -p hexa-exec --lib runs_feed_tests` "Expected: `3 passed`" (EVIDENCE.md:243-246) — 4 (hexa-exec/src/direct_exec.rs:1970, incl. `persisted_run_keeps_its_steps`).
- `cargo test -p hexa-cli --test domain_import_references_complete` "Expected: `38 passed`", and the text reasons from 38 (EVIDENCE.md:472-479) — 39 tests (incl. `hexa_has_no_mcp_subcommand`, line 456).
- TypeScript scaffold test is `tsc && node --test "dist/**/*.test.js"` (EVIDENCE.md:348-349) — the shipped script has a zero-test-file guard between them (hexa-cli/assets/scaffold/ts/package.json.tmpl); EVIDENCE.md:360-361 describes the guard but the quoted command omits it.

## docs/GETTING-STARTED.md

- `--profile ci` "Use a frontier API instead of a local server. Needs an API key" (GETTING-STARTED.md:32) — the profile is validated and stored (hexa-cli/src/commands/bootstrap/mod.rs:80-91) and nothing reads it; every step runs the same for every profile.
- "Nothing else has to be installed or started" / "A frontier model needs no server" (GETTING-STARTED.md:3-4, 37) — the prereq step requires Ollama, Rust, Bun and Cargo (bootstrap/prereq.rs:43-58, 75-77) and bootstrap fails without them unless `--skip-prereq`.
- "Exits nonzero unless both gates pass: the test command, and an architecture grade of A or better" (GETTING-STARTED.md:63-64) — a third condition: `boundary_violations == 0` (hexa-cli/src/commands/scaffold.rs:274-276); a vacuous test run also fails (scaffold.rs:172).
- "The analyzer has no built-in list of directory names to skip" (GETTING-STARTED.md:103) — `EXCLUDE_PATTERNS` (hexa-analysis/src/analyzer.rs:36-54) and skip lists in hexa-cli/src/commands/analyze.rs:939-944, 1104, 1610.
- Uses `hexa init …` (GETTING-STARTED.md:48) and `hexa analyze …` (GETTING-STARTED.md:80-84) as primary — both hidden aliases (hexa-cli/src/main.rs:303, 354).

## docs/INFERENCE.md

- "hexa names no model in its source" / "There is no default model anywhere in the source" (INFERENCE.md:3, 92) — `qwen3:4b` default (hexa-cli/src/commands/inference.rs:149), `"llama3"` fallback (:351), `DEFAULT_MODELS` registered by `config inference setup` (:1357-1362, 1410).
- "Gemini | `GEMINI_API_KEY`" (INFERENCE.md:34) — the template uses `api_key_env: GOOGLE_API_KEY` (hexa-cli/assets/inference-providers/gemini.yml:25); nothing reads `GEMINI_API_KEY`.
- `hexa config inference add openrouter --model meta-llama/llama-4-maverick` (INFERENCE.md:15) — template adds go to `add_from_template`, which takes no model; `--model` is dropped and the template's first model is registered (inference.rs:221-222, 493-497, 556).
- Local server discovery "`HEXA_OLLAMA_HOST`, then `OLLAMA_HOST`, then the default" (INFERENCE.md:139) — `host_env` is `"OLLAMA_HOST"` (hexa-infer/src/ports.rs:56), so discovery checks `OLLAMA_HOST` twice (hexa-infer/src/discover.rs:40; local_provider.rs:54-58). `HEXA_OLLAMA_HOST` is never read.
- "A project that configures no model for a tier gets an error naming the missing key" (INFERENCE.md:91) — silent fallback: `tier_model(..).or_else(|| tier_model("t2"))` (hexa-cli/src/commands/plan/mod.rs:1050-1051); `["t2.5","t2","t1"].find_map` (hexa-exec/src/adversarial.rs:500-502).
- The resource governor checks memory before each candidate and routes to frontier (INFERENCE.md:125-127) — only when a `claude-*` fallback is among the candidates (hexa-exec/src/direct_react.rs:753-765).
- Spend source "`harden` for the adversarial pass" (INFERENCE.md:152-153) — `claude_run` hardcodes `"harden"` for every call (adversarial.rs:407-410), so `build`/`scaffold` spend is logged as `harden`.
- "Keys are environment variables" (INFERENCE.md:27) — `resolve_key` also reads `$HEXA_ENV_FILE`, `~/.hexa/.env` and `<project>/.env` (hexa-infer/src/wiring.rs:161, 193-208).

## docs/UPGRADING.md

No drift detected

## docs/COMPARISON.md

- "The code generation is a frontier model" (COMPARISON.md:43) — `hexa do` generates with local `inference.react_models` via the local ReAct loop and uses `claude -p` only for a `claude-*` candidate (hexa-exec/src/direct_react.rs:785-789).

## bench/README.md

- "Both `score.sh` and `run.sh` refuse to produce anything until `selftest.sh` passes" (README.md:24-25, 58) — `run.sh` skips it with `SKIP_SELFTEST=1` (bench/run.sh:7).
- `score.sh` runs "the architecture grade" (README.md:9-10) — only `if command -v hexa` (bench/score.sh:51), silently skipped otherwise.
- `BASE=<rev>` reports "the blast radius of the change" (README.md:10-11) — the diff is limited to `-- src` with hardcoded `src/domain`, `src/adapters/secondary/`, `src/main.ts` (bench/score.sh:57-60).
