# Drift report 2026-11-21

Model: `claude-text`
Reviewer: persona `codebase_details`, prompt `skills/egm-drift-report/SKILL.md`, one tool-less completion per document, given the document and the source it names.
Written by hexa drift, from the Elephant-Goldfish Model (EGM, https://github.com/google/elephant-goldfish-model, Google LLC, Apache-2.0) at commit a8380ab5f0aba392c97e31c79f9a6666b3f09f14. ADRs are history and are not checked.

## README.md

- 🔴 **Blocker**, README "Architecture" ("It excludes two paths, its embedded scaffold templates and its benchmark fixtures") vs `.hexa/project.json` `analyze.exclude`. The config excludes five paths: `hexa-cli/assets/scaffold`, `hexa-cli/assets/claude-mods`, `bench/fixtures`, `scripts` and `evals`. Three of them go unmentioned, so the "all 165 of its source files" coverage claim rests on a smaller tree than the README describes. `docs/EVIDENCE.md` ("hexa excludes two paths") carries the same stale count.
- 🔴 **Blocker**, README "Architecture" ("the append-only ADR ledger") vs `docs/analysis/2609151900-adrs-as-memory-investigation.md` §2 and §6. The investigation says ADRs are not immutable:
    - `hexa adr accept | complete | supersede` rewrite the `Status:` line in place.
    - `docs migrate-adr --apply` prepends frontmatter.
    - `adr doctor` can patch bodies.
    - No hash or CI check guards an Accepted ADR's body.
    Content hashes and ledger events appear only as proposal item 2 (ADR-2609151930). No supplied source shows they were built. "Append-only" therefore asserts a property the code is documented not to have.
- 🟡 **Major**, README "Architecture" ("Seven crates") vs `docs/EVIDENCE.md` ("hexa graded itself over six of eight crates"). `project.json` layer keys name seven crates (`hexa-analysis`, `-cli`, `-core`, `-exec`, `-git`, `-graph`, `-infer`). The README and EVIDENCE disagree on the crate count, and neither says whether a crate was removed.
- 🟡 **Major**, README "Why hexagonal", layer table and Rule 6 ("a file declared one"), vs `.hexa/project.json` `analyze.layers`.
    - The config uses layer values the README never lists: `"composition-root"` (`hexa-infer/src/wiring.rs`) and `"infrastructure"` (`hexa-cli/build.rs`).
    - The README names only `domain`, `ports`, `usecases` and `adapters/primary|secondary`, and says a misspelt layer stops the run.
    - The README never says how a file is declared a composition root, or what rules apply to `infrastructure`.
    - The `analyze.adapters` map (`cli`, `treesitter`, `inference-http`, `git`) is also undocumented: what it does and how it affects rule 5 are both unstated.
- 🟡 **Major**, README "Evidence", refactoring trial ("The detector defect is recorded in …refactoring-trial.md") vs `docs/analysis/2609120500-refactoring-trial.md` "Recorded, not fixed". The trial doc says the `unused_ports` defect is **fixed since**: the re-analysis gives 0 unused ports, score 82, grade B. The README presents the defect and the C grade as the current state.
- 🟡 **Major**, README "Evidence", refactoring trial ("changed no logic, edited no test") vs the trial doc's Measure 1. The pre-registered constraint "do not modify `domain/`" was broken: 3 `domain/` files changed. The README omits this.
- 🟡 **Major**, README "Evidence", refactoring trial ("A second project, not identified here"). The linked trial doc names it (`brain`). The README's stated reason, that the trial is not re-runnable because the project is unidentified, contradicts the source it cites.
- 🟡 **Major**, README "The problem" ("113 of them had no file") vs `docs/EVIDENCE.md` ("539 errors across 114 orphan ids"). The orphan count differs, 113 against 114.
    - The README implies the drift was closed by the gate.
    - EVIDENCE shows the orphans were resolved by `--stub-orphans` Historical stubs that record the citing lines, not recovered decisions.
    - `--stub-orphans` and `docs/adrs/historical/` are not mentioned.
- 🟡 **Major**, README "Limits" ("The code generation is a frontier model") vs `.hexa/project.json` `inference`. hexa's own config points every tier, `react_model` and the first `react_models` entry at the local `devstral-small-2:24b`, with `claude-code` as fallback. LoRA augmentation is enabled with `qwen2.5-coder:32b`. Neither the LoRA path nor the local-first default is documented.
- 🟡 **Major**, README "What hexa does" ("Every scaffold ships a `.hexa/ADR-rules.toml` that `hexa analyze` runs") vs the `.hexa/ADR-rules.toml` header. Per the header, `[rules]` and `[[hex_layer_rules]]` are read by `hexa enforce check-file`; only `[[adr_rules]]` and `[[import_policy]]` are read by `analyze`. The `hexa enforce` verb and this split are undocumented.
    - The header also says `path_pattern` is an *unanchored substring* match.
    - That contradicts the README/UPGRADING account of layers matching whole path segments.
- 🟡 **Major**, README "Evidence", six projects (each "from a single sentence") vs `docs/EVIDENCE.md`: "What produced them is not recorded, and cannot be recovered". No model, date or hexa version is on file. The README does not disclose this provenance gap, or the `PROVENANCE.md` record that exists only from ADR-2609221900.
- 🟡 **Major**, README "Why hexagonal" / "Evidence". `docs/EVIDENCE.md` and the refactoring trial say `cohesion`, `duplication`, `god types` and `dead layers` are Rust-only and print `n/a` on Go/TypeScript. The README presents one grade across Rust, Go and TypeScript and never states that several detectors do not run for two of the three languages.
- 🟡 **Major**, README "What hexa does", loop paragraph ("keeps the ADR, the gate, the evidence and the stage") vs `.hexa/loop.json`.
    - Entries also carry `files`, `pid`, `pid_start`, `running`, `tasks` (with `closeout`), `last_run`, `gate_at`, `gate_uncounted` and `fast_gate`, none of them documented.
    - Session `00fa741f…` has a gate that runs no test runner (`hexa analyze … | grep`) and no `gate_uncounted` field. Under the README's `--uncounted` rule it should not pass.
    - Session `54c69c15…` records `adr` ADR-2610281628 but gate `hexa spec verify ADR-2610281442`. The "which decision the work is under" claim is not enforced as a match.
- 🟡 **Major**, README "Built with hexa", badge snippet. The text says to copy `.github/assets/badges/built-with-hexa.svg`, but the HTML snippet references `.github/assets/built-with-hexa.svg` (no `badges/`). That path is not the one the document's own header uses.
- ⚪ **Nit**, README "The problem" ("CI fails the build if one does") vs `docs/EVIDENCE.md`. The checking command is `hexa adr doctor --strict`. The README does not name `--strict` or say whether the citation check fails without it.
- ⚪ **Nit**, README "Limits" / quick start vs `.hexa/project.json` `adr.unbuilt`. Two ADRs are declared unbuilt: ADR-2610161730 (delegate allowlist) and ADR-2610221834 (parallel harden). `hexa adr gates` reports them as `unbuilt (declared)`. Neither the mechanism nor these gaps appear in Limits.
- ⚪ **Nit**, README "Limits", language support ("It scaffolds Rust, Go and TypeScript"). The `.hexa/ADR-rules.toml` header uses `.py` examples, and `.hexa/loop.json` shows in-flight ADR-2611211324 "hexa reads python…" at stage `gate`. Python support is undocumented, whether it is partial or pending.
- ⚪ **Nit**, `.hexa/project.json` `autonomous` (`enabled: true`, `max_concurrent_agents: 3`, `rollback_on_failure`) and `auto_repair` (`project_path: examples/ebay-clone/backend`, which is not among the README's examples). Both are undocumented configuration surfaces.

Source supplied (each cut at 12000 bytes): `docs/analysis/2609151900-adrs-as-memory-investigation.md`, `docs/EVIDENCE.md`, `.hexa/ADR-rules.toml`, `.hexa/loop.json`, `docs/analysis/2609120500-refactoring-trial.md`, `docs/analysis/2609151730-the-dashboard-lied-case-study.md`, `docs/UPGRADING.md`, `.hexa/project.json`
Unresolved paths (outside the project or not found; not read): `domain/`, `ports/`, `usecases/`, `lib.rs`, `adapters/primary|secondary/`, `built-with-hexa.svg`, `go.mod`, `tests/`, `benches/`, `examples/`

## ARCHITECTURE.md

- **🔴 Blocker. "Interactive `hexa do` commits on the current branch" (Execution model, *Worktrees separate agents*) vs `hexa-exec/src/direct_react.rs` `react_execute`.**
    - The ReAct loop calls `direct_exec::want_isolation(&task)`, then `RunWorkspace::acquire(&slug, isolate, …)`, then `workspace.assert_off_operator_tree()`.
    - The ADR-2606071323 comment says to "confine the run to its own worktree unless the operator opted out (`isolate:false`). Never silently fall back to the operator tree."
    - `.hexa/ADR-rules.toml` quotes the default as `task.isolate.unwrap_or(true)`.
    - So the default do-loop run is isolated. The document instead presents worktree isolation as exclusive to `hexa bench`.
    - The document also never mentions the `isolate` opt-out, the per-repo tree lock (`direct_exec::lock_tree`, "serialize against every other run on this repository, in any process"), or `assert_off_operator_tree`.
- **🔴 Blocker. "The loop is given what it uses … The loop never names an adapter" (Execution model) vs `hexa-exec/src/direct_react.rs`.**
    - The loop calls `hexa_infer::complete_raw(&req)` directly. Inference is not carried in `ExecDeps` or behind any port in `hexa-exec/src/ports.rs`.
    - `react_model_from_config()` reads `.hexa/project.json` with `std::fs::read_to_string`.
    - `resolve_react_model` reads the `HEXA_REACT_MODEL` env var.
    - The loop also uses `direct_exec::dirty_paths`, `refuse_credentials` and `refuse_dirty_riders`, which are module calls and not injected ports.
    - The document's list of what `ExecDeps` carries is therefore not the full set of what the loop uses.
- **🔴 Blocker. "No file outside this crate names a provider or a model" (Workspace crates, hexa-infer row) vs `hexa-exec/src/ports.rs`.**
    - `FrontierError::Display` renders `"spawn claude: {e}"` and `"claude -p timed out…"`.
    - The `Frontier` trait docs and `SpendLog::record_frontier` name `claude -p --output-format json`.
    - `hexa-exec/src/frontier.rs`, a secondary adapter per `.hexa/project.json`, spawns the `claude` CLI.
    - The document itself places the `claude-code` frontier candidate in hexa-exec, so the absolute claim is contradicted.
    - The `no-model-name-outside-inference` rule in `.hexa/ADR-rules.toml` does not include `claude` or `claude-code` in `violation_patterns`, so nothing would catch this.
- **🟡 Major. Model selection for the do-loop (Execution model, *Best-of-N*; Tiered inference routing: "The do-loop selects separately via `inference.react_models`") vs `direct_react.rs::resolve_react_model`.**
    - The code's precedence is: `--model`, then `HEXA_REACT_MODEL`, then `inference.react_model` (singular), then `direct_exec::resolve_model` (the single-shot default).
    - None of these are documented.
    - `.hexa/project.json` carries both `react_model` and `react_models`. The document never says how the two interact, or whether an explicit `--model` or the env var bypasses the Best-of-N walk.
- **🟡 Major. Tier table row "T3 | frontier work" (Tiered inference routing) vs `.hexa/project.json` `inference.tier_models`.**
    - The project config defines only `t1`, `t2` and `t2.5`, all set to `devstral-small-2:24b`. There is no `t3` key.
    - The document says a task's tier "selects a model from `tier_models`" but does not state what T3 resolves to when absent.
- **🟡 Major. Undocumented loop limits and failure modes in `direct_react.rs`.**
    - `DEFAULT_MAX_STEPS = 12`, clamped to `1..=40`.
    - `MAX_TOKENS = 4096`.
    - `NO_PROGRESS_LIMIT = 3`.
    - Duplicate-call detection.
    - `MAX_CTX_TOKENS = compress::LOOP_BUDGET_TOKENS`, with a single cheap-model summarisation on overflow, and the `COMPRESSION_KEY` envelope telling hexa-infer not to re-compress.
    - "Loop ended with no edit" when the model emits no tool call.
    - The terminal `CommitFailed` outcome: evidence passed, commit failed, no retry.
    - The document's "Curated, guarded tools" and "gate is the sole authority" bullets mention none of these.
    - The tool-guard sentence also omits the subprocess timeouts and path allowlist that the module doc lists.
- **🟡 Major. `.hexa/loop.json` description ("records where a project's work stands: the ADR, the gate and the stage") vs the actual file.**
    - The file is a `sessions` map keyed by session id. One committed file holds many concurrent records.
    - Each record carries fields the document does not describe: `pid`, `pid_start`, `files`, `tasks` (with `closeout`), `last_run`, `gate_at`, `gate_uncounted`, `fast_gate`, `evidence`, `running`.
    - It has a `done` stage that the Decide→Gate→Build→Harden loop does not name.
    - Some records have no `adr`. One names `ADR-2610281628` with a gate for `ADR-2610281442`.
    - Because the document says this file "is committed … travels with the pull request", note that it currently carries machine-specific absolute paths (`/home/gary/…`, `/tmp/claude-1001/…`) and a remote rsync/ssh gate command.
    - The document also does not reconcile session-keyed state in `loop.json` with "Session state … `~/.hexa/sessions/agent-<session_id>.json`" in the State table.
- **🟡 Major. Port inventory (hexa-exec row; *Curated, guarded tools*) vs `hexa-exec/src/ports.rs`.**
    - `ports.rs` also defines `GapLog` (ADR-2610301500, backing `hexa gap add`).
    - It defines `Forge`, `ForgeStatus` and `ForgeBoth` (ADR-2611031914, with `ForgeGithub` via `gh` and `ForgeForgejo` via REST), plus `ForgeHosts` and `TargetLock`, and an identity port (cut).
    - None appear in the document.
    - The corresponding adapters (`forge_github.rs`, `forge_forgejo.rs`, `telegram_notifier.rs`) and the `forge_filing.rs` use case are classified in `.hexa/project.json` but are absent from the crate description.
- **🟡 Major. State table vs ports and config.**
    - Missing: `$HEXA_HOME/forges.json` (`ForgeHosts::path`, which also implies a `HEXA_HOME` override that is undocumented) and the `GapLog` store location.
    - "budget in `.hexa/project.json`": hexa's own `project.json` has no budget key, and the default is not stated.
    - `lifecycle_enforcement` is documented as a `project.json` key but is absent from hexa's own config. Its default (mandatory or advisory) is unstated.
- **🟡 Major. Composition roots: "Each library crate's root (`lib.rs`) is its composition root … `hexa_infer::wiring`" (Workspace crates) vs `.hexa/project.json`.**
    - `hexa-infer/src/wiring.rs` is separately declared as the layer `composition-root`. hexa-infer's root is therefore a non-`lib.rs` file.
    - The layer map also uses an `infrastructure` layer (`hexa-cli/build.rs`).
    - Neither `composition-root` nor `infrastructure` as a layer value appears in "Where a file's layer comes from" or in the rule table.
- **🟡 Major. API contract (`@hexa:api` "reads the same in Rust, Go and TypeScript") vs `hexa-analysis/src/treesitter_api.rs`.**
    - Undocumented behaviour:
      - `Language::Python` returns empty `ApiFacts` silently, so a tagged Python port is neither read nor reported as a stray tag.
      - Files whose syntax tree nests deeper than `MAX_TREE_DEPTH = 512` are refused with an error.
      - A port is recorded when either the trait or interface *or any one method* is tagged.
      - A tag counts only if a comment line *begins* with it.
      - Trailing comments (`fn a(); // note`) are excluded.
      - A second tag, `@hexa:status`, is parsed into `statuses`.
    - The document mentions none of these.
- **🟡 Major. "Lessons, gaps, decisions → `.hexa/memory.jsonl`" (State) vs `.hexa/memory.jsonl` and `ports.rs::MemoryStore`.**
    - The file is an append log with `{"deleted":true}` tombstones and keys prefixed `lesson:` and `gap:`.
    - `MemoryScope::{Project, Shared}` explicitly forbids fallback between scopes.
    - The document describes `--global` and "outside a project" but not the no-fallback rule or tombstone semantics.
    - It is also unclear whether gaps go to `memory.jsonl` or to the separate `GapLog` port.
- **🟢 Nit. Crate diagram (`crates-light.svg` alt text: "hexa-cli → hexa-exec → hexa-infer → hexa-core").**
    - It shows 4 of the 7 crates, while the text says "The dependency direction is the architecture".
    - `hexa-exec/src/ports.rs` depends directly on `hexa_core::ports::forge`. The edges to hexa-analysis, hexa-graph and hexa-git are not shown.
- **🟢 Nit. `.hexa/ADR-rules.toml` contents.**
    - The document mentions only `[[import_policy]]`.
    - It omits `[rules] forbidden_paths` and `[[hex_layer_rules]]` (read by `hexa enforce check-file`, a verb the document never names), and the `[[adr_rules]]` lesson rules that `hexa analyze .` runs.
    - The substring `path_pattern` classification in `hex_layer_rules` looks like a second classification path. That sits uneasily with "One classifier serves the grade, `--file`, the inventory and every other report."
- **🟢 Nit. Undocumented `project.json` keys.**
    - `autonomous.*` (`max_concurrent_agents`, `rollback_on_failure`), `auto_repair.*` (pointing at `examples/ebay-clone/backend`), `inference.sop_via_router`, `inference.timeout_secs`, `inference.lora.*`, `analyze.exclude` and `adr.unbuilt` are all present but undescribed.
    - `adr.unbuilt` records ADR-2610221834 (parallel harden) and ADR-2610161730 (delegate allowlist) as not built. The document does not flag either.
- **🟢 Nit. `Frontier::on_model` (`ports.rs`).**
    - An adapter that cannot pin a model returns `None`, and the caller must fail naming the model (ADR-2610141733).
    - The *Frontier delegation* bullet does not state this failure mode, nor the `OverBudget`, `Spawn` and `Timeout` error cases.

Source supplied (each cut at 12000 bytes): `hexa-exec/src/direct_react.rs`, `hexa-exec/src/ports.rs`, `hexa-analysis/src/treesitter_api.rs`, `.hexa/loop.json`, `.hexa/project.json`, `.hexa/memory.jsonl`, `.hexa/ADR-rules.toml`, `docs/analysis/2609120100-real-io-proof.md`
Source not supplied (over the 8-file budget): `founding-goals.md`
Unresolved paths (outside the project or not found; not read): `simple_agent.rs`, `direct_exec.rs`, `hexa-exec/src/tools/`, `direct_react.rs`, `.hexa/graph/graph.json`, `hexa/auto/<id>`, `ports/`, `net/http`, `adapters/secondary/`, `complete.rs`, `wiring.rs`, `do_task.rs`, `lib.rs`, `docs/adrs/`, `~/.hexa/sessions/`, `~/.hexa/memory.jsonl`, `~/.hexa/sessions/agent-<session_id>.json`, `~/.hexa/agent-runs.jsonl`, `~/.hexa/inference-log.jsonl`, `~/.hexa/inference-servers.json`, `docs/`, `docs/specs/<ADR>.json`, `~/.hexa/release-check.json`, `docs/benchmarks/`, `domain/`, `usecases/`, `adapters/`, `adapters/primary/`

## docs/EVIDENCE.md

- 🔴 **Blocker**: The "hexa passes its own analyzer" section in the evidence document contradicts `.hexa/project.json`.
    - The document says "hexa excludes two paths — `hexa-cli/assets/scaffold` … and `bench/fixtures`" under `analyze.exclude`.
    - `analyze.exclude` actually lists five: `hexa-cli/assets/scaffold`, `hexa-cli/assets/claude-mods`, `bench/fixtures`, `scripts` and `evals`.
    - `scripts` and `evals` are whole directories kept out of the grade. That bears directly on "the scan covers every crate" and on `coverage 165/165`.
- 🔴 **Blocker**: The "Prose that could not fail" section says the doctor step "sits between the architecture grade and lint in `.github/workflows/ci.yml`." It does not.
    - In `.github/workflows/ci.yml`, `Lint` is a separate `lint` job that runs in parallel with `gates`.
    - Within `gates`, `Decision registry is consistent` comes after `Architecture grade` and `Installed assets are in sync`.
    - The workflow's own comment says lint "never runs before one in the same job".
- 🔴 **Blocker**: The "Memory belongs to the project it was learned in" claim says "the shared per-user store is reached only by naming it". The page's own reproduction contradicts this.
    - In the reproduction, `hexa memory list` in an unscaffolded directory reads `…/.hexa/memory.jsonl (no project here — shared)` without `--global`.
    - `ARCHITECTURE.md` (State table) agrees with the reproduction: `~/.hexa/memory.jsonl` is used "outside one, or with `--global`".
    - `CLAUDE.md` also says `--global` "names the shared user store", which does not make naming the only route.
    - The claim should say the shared store is used implicitly outside a project.
- 🟡 **Major**: The "Every cited ADR id resolves to a file" claim says `hexa adr doctor` covers ids cited in `CODEOWNERS`.
    - `CODEOWNERS` cites `ADR-2026-04-26-1500`. That is a dated id format the page never mentions.
    - Under the matching behaviour the page itself documents (a three-digit match followed by a digit is discarded), this citation is the kind the page says is invisible to doctor.
    - The unqualified "fails on an `ADR-…` id … that has no file" therefore overstates coverage. The later "four-digit citation that dangles is invisible" caveat is not carried back to this claim.
- 🟡 **Major**: The page says `[[import_policy]]` lives "in `.hexa/ADR-rules.toml`", but this repository's `.hexa/ADR-rules.toml` has no `[[import_policy]]` table.
    - The file's header lists only `[rules]`, `[[hex_layer_rules]]` and `[[adr_rules]]`.
    - The page should name the shipped scaffold rules file under `hexa-cli/assets/scaffold` explicitly, or the repo's own file has drifted from what scaffolds ship.
- 🟡 **Major**: The page says `.hexa/ADR-rules.toml` "is enforced by `hexa analyze`". That holds only for part of the file.
    - The file's own header says `[rules]` and `[[hex_layer_rules]]` are read by `hexa enforce check-file`, and only `[[adr_rules]]` by `hexa analyze`.
- 🟡 **Major**: The anchoring described in "Every reference is read" conflicts with the rules file.
    - The page says domain path matching is now anchored: `tests/domain/x.rs` is no longer flagged.
    - `.hexa/ADR-rules.toml` still documents `path_pattern` as "an unanchored substring match (see hexa_analysis::layer_classifier)", where `/domain/` would match `tests/domain/`.
    - The page does not say whether anchoring applies only to import policy or also to layer classification, so one of the two documents is stale.
- 🟡 **Major**: The `no-model-name-outside-inference` rule in `.hexa/ADR-rules.toml` would not catch the models this repository actually uses.
    - The page presents the rule as a guard on "A model or provider name outside the inference boundary".
    - Its `violation_patterns` cover `gpt-4`, `gpt-5`, `claude-3`, `claude-4`, `qwen`, `llama-3`, `mistral` and `ollama`.
    - The configured or resolved models are `devstral-small-2:24b` (`.hexa/project.json`) and `openai/gpt-oss-120b` (`PROVENANCE.md`). Neither `devstral`, `gpt-oss` nor `openai` is a pattern.
    - This unstated limit undercuts the "provider is named in one place" claim.
- 🟡 **Major**: The "hexa passes its own analyzer" section in the evidence document says the earlier analyzer graded "six of eight crates".
    - `ARCHITECTURE.md` says "Seven crates, one binary" and lists seven.
    - The page should either state the crate count at that time or correct the figure.
- 🟡 **Major**: "The examples pass their gates" contradicts itself.
    - It states as fact that "Each was built by `hexa scaffold` or `hexa build` from one sentence".
    - It then says that what produced them "is not recorded, and cannot be recovered."
    - The first sentence is an unevidenced provenance claim on a page that requires a command per claim.
- 🟡 **Major**: The `PROVENANCE.md` description is incomplete, and its resolution source is undocumented.
    - The page lists the fields `hexa build` writes as date, version, challenge, gate, result and models.
    - The actual `PROVENANCE.md` also records "Designs proposed", "Critiques", and the do-loop candidate list.
    - Its resolved tiers (`openai/gpt-oss-120b`) differ from `.hexa/project.json → inference.tier_models` (`devstral-small-2:24b`). "Resolved from configuration" therefore draws on a source the page does not name, presumably `~/.hexa/inference-servers.json`.
    - A committed `PROVENANCE.md` at this repository's root, with result **did not pass**, goes unmentioned.
- 🟡 **Major**: "Prose that could not fail" carries stale and duplicated text.
    - The paragraph "The 2026-09-15 figures were measured with … the same truncation … 114 stub files" appears twice.
    - "Two honest qualifications" says "the count above is a shell pipeline on this page, not a test". The section states that pipeline count was removed and replaced by the checker's result.
    - "this session hit that twice" is session-relative and unverifiable.
- 🟡 **Major**: The 113/114 counts are inconsistent across documents.
    - The "Every cited ADR id resolves" section reports "539 errors across 114 orphan ids" from the first doctor run.
    - The audit section reports 113 ids with no file.
    - `CLAUDE.md` repeats "113 of 140" as exact, while this page says those figures are approximate.
- 🟢 **Nit**: `.hexa/project.json` has an `analyze.adapters` map (cli, treesitter, inference-http, git), which the page does not mention.
    - It also uses layer values beyond the five in `hex_layer_rules`: `composition-root` (`hexa-infer/src/wiring.rs`) and `infrastructure` (`hexa-cli/build.rs`).
    - The page says only that layers are "declared there under `analyze.layers`" and that "a misspelt layer stops the run". The accepted layer set is not stated.
- 🟢 **Nit**: The scaffold section says the TypeScript gate is "skipped unless `HEXA_TEST_NPM=1`".
    - It omits that CI also sets `HEXA_NO_SKIP=1` (`.github/workflows/ci.yml`), under which a would-be skip fails.
    - It also omits that the generated axum adapter's gate needs `HEXA_TEST_CARGO_NET=1`.
- 🟢 **Nit**: The final "Not yet validated" bullet says the three places "now cite the ADR-citation audit below". That audit section appears above it on the page.
- 🟢 **Nit**: `.hexa/project.json → auto_repair.project_path` points at `examples/ebay-clone/backend`, which is not among the examples the page lists. If it no longer exists, this is a dangling reference in the config the page relies on.

Source supplied (each cut at 12000 bytes): `.hexa/ADR-rules.toml`, `.hexa/project.json`, `ARCHITECTURE.md`, `CLAUDE.md`, `PROVENANCE.md`, `CODEOWNERS`, `.github/workflows/ci.yml`
Unresolved paths (outside the project or not found; not read): `src/`, `hexa-cli/assets/scaffold`, `bench/fixtures`, `hexa-cli/tests/scaffold_is_executable.rs::a_violation_planted_in_any_crate_is_seen`, `hexa-core/`, `hexa-cli/`, `examples/stress/game-sokoban-rs-tangled`, `examples/url-shortener-rs`, `5/5`, `hexa-analysis/tests/*_per_language.rs`, `docs/`, `analysis/2609151730-the-dashboard-lied-case-study.md`, `vX.Y.Z`, `.hexa/*.toml`, `docs/adrs/`, `docs/adrs/historical/`, `~/.cargo/bin`, `src/domain/`, `crates/core`, `crates/*`, `tests/domain/x.rs`, `src/adapters/domain_helpers/x.rs`, `src/adapters/domain/x.rs`, `src/domain/x.rs`, `/domain/`, `/domain_helpers/`, `src/adapters/domain/`, `tests/`, `.mcp.json`, `analysis/2609151900-adrs-as-memory-investigation.md`, `analysis/2609120500-refactoring-trial-preregistered.md`, `analysis/2609120500-refactoring-trial.md`, `analysis/2609120300-brownfield-trial.md`, `domain/`, `docs/specs/`

## docs/GETTING-STARTED.md

- **🟡 Major. "a `.hexa/ADR-rules.toml` that `hexa analyze` will run from now on"** (Scaffold a project) vs `.hexa/ADR-rules.toml` header.
    - The file's own header splits its sections between two readers. `[rules]` (`forbidden_paths`) and `[[hex_layer_rules]]` are read by `hexa enforce check-file`. Only `[[adr_rules]]` is read by `hexa analyze`.
    - The document implies `hexa analyze` runs the whole file, and it never mentions `hexa enforce check-file` at all.
    - A reader who edits `hex_layer_rules` will expect `hexa analyze` to change. It won't.
- **🟡 Major. "grade, boundary violations" and "nonzero on any boundary violation"** (Check the shape) vs `.hexa/project.json` `analyze.layers` / `analyze.adapters`.
    - The config that defines layers and adapters for the analyzer is `analyze.layers` (path → layer map) and `analyze.adapters` (adapter name → paths). The document documents only `analyze.exclude`.
    - A reader is not told how boundary violations are determined. They are also not told that unmapped paths fall back to the substring `path_pattern` rules, which the TOML header says are unanchored substring matches.
- **🟡 Major. "The models it pulls come from `.hexa/project.json`"** (Set up inference) vs `.hexa/project.json` `inference`.
    - The file names models under several keys:
      - `tier_models.{t1,t2,t2.5}`
      - `react_model`
      - `react_models`
      - `lora.augment_model` (`qwen2.5-coder:32b`)
    - The document does not say which of these `hexa bootstrap` pulls.
    - `react_models` contains `"claude-code"`, which is not a pullable local model. The document gives no rule for how bootstrap treats such entries.
    - "A project that configures no models" is also undefined, since it is unclear which key counts.
- **🟡 Major. "`hexa bootstrap` … writes `.hexa/project.json`"** together with "The models it pulls come from `.hexa/project.json`."
    - Bootstrap both reads and writes the same file, and the document does not say:
      - whether an existing file is preserved, merged or overwritten;
      - what a freshly written file contains, which decides whether it configures any models;
      - whether models are read before or after the write.
- **🟡 Major. "`lifecycle_enforcement: \"mandatory\"`"** (Where the work stands).
    - The document does not say which file or key path holds this setting, or what other values are allowed.
    - The supplied `.hexa/project.json` has no such key.
    - A reader cannot enable the behavior from this description.
- **🔵 Nit. "The analyzer has no built-in list of directory names to skip"** vs `.hexa/ADR-rules.toml` `[rules] forbidden_paths = ["node_modules", ".git", "dist", ".env", "target"]`.
    - The scaffolded rules file does ship a built-in directory-name list. It is consumed by `enforce check-file`, not `analyze`, but the document does not draw that distinction, so a reader may assume `target`/`node_modules` are skipped by the scan.
    - The document also omits that each `[[adr_rules]]` entry carries its own `exclude_patterns`, which are substring matches (for example `"test"`, `"example"`, `"config"`, `"infer"`). These are a second, per-rule exclusion mechanism alongside `analyze.exclude`.
- **🔵 Nit. "`hexa analyze . --exit-code` # nonzero on any … rule error"** vs `.hexa/ADR-rules.toml` `severity`.
    - Rules have `severity = "error"` or `"warning"`. Four of the six shipped rules are warnings:
      - `no-hardcoded-host-port`
      - `narrowing-cast-needs-a-check`
      - `test-must-not-redefine-its-subject`
    - The document should state explicitly that warning-severity hits do not fail the gate. Otherwise "rule error" reads as "any rule hit".
- **🔵 Nit. "Rust, Go and TypeScript are supported"** vs `.hexa/ADR-rules.toml`.
    - `no-innerhtml-with-data` also targets `.js`/`.jsx`.
    - `test-must-not-redefine-its-subject` targets only `_test.go`, `.test.ts`, `.spec.ts`, with no Rust equivalent.
    - So rule coverage per language is uneven, which the document does not mention.
- **🔵 Nit. "`--profile ci` … Needs an API key in the environment"** (bootstrap flags table).
    - The environment variable name is not stated, which is an unstated assumption.
    - Neither supplied source shows a `ci` profile or its configuration.
- **🔵 Nit. Undocumented `.hexa/project.json` sections.**
    - The file the document points readers to also contains:
      - `autonomous` (`enabled`, `max_concurrent_agents: 3`, `rollback_on_failure`)
      - `auto_repair` (`project_path` hardcoded to `examples/ebay-clone/backend`, `max_iterations: 20`, `top_k_files: 3`)
      - `inference.timeout_secs: 600`, `sop_via_router`, `lora`
      - `adr.unbuilt`
    - None of these are mentioned or deferred to another document.
    - `hexa do run` behavior (revert on failure) may interact with `autonomous.rollback_on_failure`, and the document does not say whether it does.

Source supplied (each cut at 12000 bytes): `.hexa/project.json`, `.hexa/ADR-rules.toml`

## docs/INFERENCE.md

- 🔴 **Blocker — Contradicts the code: "every provider lives in one crate, `hexa-infer`" (intro).** `.hexa/project.json` → `analyze.layers` classifies `hexa-exec/src/frontier.rs` as `adapters/secondary`, so the frontier `claude` provider adapter lives in `hexa-exec`, not `hexa-infer`. `hexa-cli/src/http_probe.rs` is also an `adapters/secondary` file that probes endpoints outside `hexa-infer`. The claim should name the frontier adapter's real location or be narrowed.
- 🟡 **Major — The Tiers JSON example disagrees with the actual `.hexa/project.json`.** The doc says "The mapping lives in `.hexa/project.json`" and shows `t1: qwen3:4b`, `t2: gemma4-12b`, `t2.5: devstral-small-2:24b`. The real file maps all three tiers to `devstral-small-2:24b`. The doc does not say the block is illustrative, so a reader will take it as this repository's configuration.
- 🟡 **Major — Undocumented keys in `inference`.** `.hexa/project.json` contains keys the page never mentions:
    - `react_model` (singular, `"devstral-small-2:24b"`), which sits alongside the documented `react_models` with no explanation of how the two interact or which one wins.
    - `timeout_secs` (600).
    - `sop_via_router` (true).
    - `lora.augment_model` (`qwen2.5-coder:32b`), `lora.corpus_qa_count` (5) and `lora.enabled` (true). This is a further configured model the page does not account for.
- 🟡 **Major — "The resource governor in `hexa-exec`" (Hardware) is incomplete.** `analyze.layers` lists two governor files: `hexa-core/src/resource_governor.rs` (domain) and `hexa-exec/src/resource_governor.rs` (usecases). The page omits the domain half, and it gives no thresholds or how the governor measures available memory.
- 🟡 **Major — Missing source links throughout.** The page names behaviour but no files. The relevant files exist per `analyze.layers`, and the page should link them:
    - `hexa-infer/src/tiers.rs` (tier resolution, and the "error naming the missing key")
    - `hexa-infer/src/registry.rs` (`inference-servers.json`)
    - `hexa-infer/src/discover.rs` and `reach.rs` (Discovery table)
    - `hexa-infer/src/spend.rs` and `spend_report.rs` (Spend and budget)
    - `hexa-infer/src/wiring.rs` (composition root)
    - `hexa-exec/src/do_task.rs` and `direct_react.rs` (the `react_models` loop)
    - `hexa-exec/src/spec_author.rs` (the `claude-text` refusal of `claude-code`)
    - `hexa-exec/src/adversarial.rs` (the `harden` source)
    Without these links, a reviewer cannot verify the claims and future drift checks are not cheap.
- 🟡 **Major — "`hexa do` reaches it as a fallback when the local candidates fail" (The frontier path) misdescribes the mechanism.** Per the config, the frontier is reached only because `claude-code` is the last entry in `react_models`. It is configured ordering, not a built-in fallback. A project that omits `claude-code` from `react_models` gets no fallback. The page should say so.
- 🟢 **Nit — The provider-type list is inconsistent with the rest of the page.** "Provider types" omits `anthropic`, `claude-code` and `claude-text`, yet the API-key table lists `ANTHROPIC_API_KEY` and Discovery lists an `anthropic` path.
- 🟢 **Nit — Model name mismatch.** The Tiers example uses `gemma4-12b`, while the benchmark table uses `gemma3:12b`. Neither appears in `.hexa/project.json`. A reader cannot tell whether `gemma4-12b` is a real tag.
- 🟢 **Nit — Discovery env vars are less precise than the API-key section.** The API-key section documents only `OLLAMA_HOST`. Discovery introduces `HEXA_OLLAMA_HOST` with higher precedence, plus `HEXA_INFERENCE_*`, `HEXA_VLLM_*` and `ANTHROPIC_BASE_URL`. The page does not say whether dispatch, as opposed to discovery, honours `HEXA_OLLAMA_HOST`.
- 🟢 **Nit — `budget_usd_per_day` is documented but absent from `.hexa/project.json`.** Because it is absent, the page should state the behaviour when it is unset ("no budget enforced") explicitly, rather than only implying it with "optional".

Source supplied (each cut at 12000 bytes): `.hexa/project.json`
Unresolved paths (outside the project or not found; not read): `~/.hexa/inference-servers.json`, `t2.5`, `docs/benchmarks/`, `~/.hexa/inference-log.jsonl`

## docs/UPGRADING.md

- 🔴 **Blocker**: "What the grade leaves out is decided by folder, not substring" (`9672371`), together with `.hexa/ADR-rules.toml`. The shipped rules file still says `path_pattern is an unanchored substring match (see hexa_analysis::layer_classifier)`. It also lists `forbidden_paths = ["node_modules", ".git", "dist", ".env", "target"]` as bare words with no segment delimiters. The document says matching moved to whole path segments and that `layer_classifier` no longer substring-matches. Either the rules-file comment is stale or the document overstates the change. A reader cannot tell which matcher still uses substrings.
- 🟡 **Major**: "An error-severity rule violation now costs grade" (`6be13ef`), together with the `exclude_patterns` in `.hexa/ADR-rules.toml`. These patterns are bare substrings: `"test"`, `"example"`, `"infer"`, `"inference"`, `"config"`. This is the same bug class the first section says was fixed. For example, `"test"` exempts `contests/` and `"infer"` exempts any path containing `infer`. Error-severity `[[adr_rules]]` findings now subtract 10 points each. The document does not say that the scope of these rules is still decided by substring.
- 🟡 **Major**: `6be13ef` again. The rules file says `` `hexa analyze .` runs them `` and ships three `severity = "error"` rules out of the box:
    - `no-hardcoded-absolute-path`
    - `no-model-name-outside-inference`
    - `no-innerhtml-with-data`
    The document frames the grade drop as affecting "a project that already carries error-severity findings". It never says that the default rule set produces such findings, so most scaffolded projects are exposed.
- 🟡 **Major**: "Inline references now count as imports" and "Every reference is read, and every one is judged". Both sections depend on `[[import_policy]]` and its `layer` and `allow` fields. The header of `.hexa/ADR-rules.toml` lists only `[rules]`, `[[hex_layer_rules]]` and `[[adr_rules]]`, and the file contains no `[[import_policy]]` table. The document never says which file holds `[[import_policy]]` or what fields it has beyond `layer` and `allow`.
- 🟡 **Major**: "A grade cannot exceed the code it could classify" (`bf1bafc`). The document says "a misspelt layer stops the run" but never lists the valid layer names. `.hexa/project.json` uses `"infrastructure"` (`hexa-cli/build.rs`) and `"composition-root"` (`hexa-infer/src/wiring.rs`), which the document never mentions. A reader cannot tell a misspelling from an accepted value.
- 🟡 **Major**: `.hexa/project.json` has `analyze.exclude` and `analyze.adapters`. The section on what the grade leaves out documents only the built-in exclusions. It does not say that `analyze.exclude` exists or how it is matched (prefix, segment or substring). `analyze.adapters` (a map from adapter name to paths) is not explained anywhere, including how it relates to `analyze.layers`.
- 🟡 **Major**: The preamble promises that "each entry names the commit that carried it … and the one command that resolves it". Several entries break this:
    - The last three entries name an ADR, not a commit: `ADR-2609211600`, `ADR-2609221430`, `ADR-2609221700`.
    - The `9672371`, `0f0024f`/`342a903` and `316787e` entries give no command.
    - The `6be13ef`, `ADR-2609211600` and `ADR-2609221430` entries give a diagnostic (`jq .score_components`), not a resolution.
- 🟡 **Major**: "Memory is stored per project" (`97dc8f0`). The text says "Re-store the entries that belong to a project from inside that project" but gives no command for it. It also does not say how the per-entry tombstones and repeated keys are handled when copying. `.hexa/memory.jsonl` shows both: `"deleted":true` records, and `gap:a-demo-build-clobbers-the-loop-record` appearing twice with different timestamps. A user copying from `--global list` output by hand could revive deleted entries.
- 🟡 **Major**: The preamble says the document covers "versions". `Cargo.toml` sets `version = "26.11.15"`, but no entry names a release version. Entries are keyed only by commit or ADR, so a reader moving between versions cannot tell which entries apply to them.
- 🟢 **Nit**: Exclusion sets differ between sections without explanation.
    - `9672371` excludes `dist/`, `examples/`, `tests/`, `target/`, `node_modules/`, `test/` and `__tests__/`.
    - `ADR-2609221430` puts top-level `tests/`, `benches/` and `examples/` out of policy scope, so `benches/` is in one list only and the "top-level" qualifier appears only here.
    - hexa's own `.hexa/project.json` excludes `bench/fixtures`, which matches neither `benches/`.
- 🟢 **Nit**: `97dc8f0`. Walking up "to the nearest `.hexa/`" from any directory under `$HOME` that has no project would find `~/.hexa/` itself. So the "outside a project" fallback is also reached by the walk-up, and the document presents these as two separate paths. The case of a directory that has its own `.hexa/` but is not a scaffolded project is unstated.

Source supplied (each cut at 12000 bytes): `.hexa/project.json`, `.hexa/memory.jsonl`, `.hexa/ADR-rules.toml`, `Cargo.toml`
Unresolved paths (outside the project or not found; not read): `src/domain/distance.ts`, `tests/`, `contests/`, `dist/`, `examples/`, `target/`, `node_modules/`, `test/`, `__tests__/`, `domain/`, `ports/`, `usecases/`, `adapters/`, `.hexa/`, `~/.hexa/memory.jsonl`, `package.json`, `/domain/`, `src/adapters/domain_helpers/`, `benches/`, `.mcp.json`

