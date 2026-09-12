# hexa Architecture

The living map. It always describes HEAD. Decisions and their reasoning live in the
append-only [ADR ledger](docs/adrs/INDEX.md); if this file and an older document
disagree, this file wins, and the contradiction is worth an ADR.

## What hexa is

One binary that scaffolds hexagonal projects and keeps checking them. There is no
daemon, no database, and no background process. Every verb runs in-process and exits.

Two gates decide whether generated work counts:

| Gate | Asks | Fails when |
|---|---|---|
| the command | does it run? | your test command exits nonzero |
| the grade | is it the shape you asked for? | boundary analysis falls below the floor |

The first answers *does it run*. The second answers *is it the shape you asked
for*. A test suite cannot reach that second question. A program whose use case
imports a database driver passes its tests.

## The execution model

A single ReAct loop, in process. The differentiator is the quality of context
assembled for one loop, not the number of loops.

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset=".github/assets/diagrams/loop-dark.svg">
    <img src=".github/assets/diagrams/loop-light.svg" alt="The agent loop, ending in an edit the evidence command must accept." width="460">
  </picture>
</p>

- **Loop and tool protocol.** `hexa-exec/src/direct_react.rs` holds the ReAct loop.
  `simple_agent.rs` holds native function-calling with a text-mode JSON fallback.
  `direct_exec.rs` holds the single-shot path.
- **Curated, guarded tools.** They live in `hexa-exec/src/tools/`. Read and verify tools only
  (`repo_read`, `repo_grep`, `cargo_check`, `typescript_check`, `dep_audit`,
  `secret_scan`) plus the terminal `propose_edit`. No arbitrary shell. Tools reject
  path traversal, block critical paths, and cap output.
- **Code-graph context.** `gather_context` reads `graph-out/graph.json` and calls
  `hexa_graph::context::context_for(file)` and `rank_lessons`.
- **The gate is the sole authority on what commits.** A pass that exercised nothing
  is rejected as vacuous. A failed edit reverts atomically, so the next attempt
  matches against the original file rather than a half-applied one.
- **Worktrees separate agents.** Multi-agent work is harness subagents, one
  worktree each. The `pre-agent` hook blocks a code-writing subagent whose
  Agent call lacks `isolation: "worktree"`, and `subagent-stop` names every
  worktree branch holding commits the current branch lacks, with the
  `hexa dev worktree merge` command that lands them. hexa's own isolated runs
  (`hexa bench`) use `hexa/auto/<id>` worktrees beside the repo, hard-guarded
  against committing to the operator's tree. Interactive `hexa do` commits on
  the current branch.
- **Best-of-N across complementary models.** A run walks the ordered candidate list
  in `.hexa/project.json → inference.react_models` and commits the first candidate
  whose edit passes the gate. The gate picks the winner, not a classifier, so a
  mis-route costs latency and nothing else.
- **Frontier delegation.** A `claude-code` candidate hands the whole task to the
  operator's logged-in `claude` CLI, inside the same worktree, gate and commit. No
  API key, no VRAM ceiling.

## The build harness

The loop above handles *bounded* work: one file, one gate. Whole systems use
in-process fan-out of inference calls:

- **`hexa scaffold '<what>' --target <dir> --lang <l> --grade A`.** Write the
  deterministic floor, prove its gate on this machine, then build the description
  onto it. Gated on the build **and** the grade.
- **`hexa build '<challenge>' --target <dir> --gate '<cmd>'`.** Propose N divergent
  designs, red-team each, synthesize one spec, build until the gate passes. The spec
  is a disposable intermediate.
- **`hexa harden <path> --gate '<cmd>'`.** Hunt for bugs by failure-class lens,
  verify each finding skeptically (default-refute), fix the confirmed ones under the
  gate.
- `hexa build --harden` chains the last two.

What keeps it disciplined: the gate is the only authority, and the verifier defaults
to *refuting*, so plausible-but-wrong findings die before any edit is made.

## Workspace crates

Eight crates, one binary. The dependency direction is the architecture:

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset=".github/assets/diagrams/crates-dark.svg">
    <img src=".github/assets/diagrams/crates-light.svg" alt="hexa-cli depends on hexa-exec, which depends on hexa-infer, which depends on hexa-core." width="460">
  </picture>
</p>

| Crate | Role |
|---|---|
| **hexa-core** | The contract surface: the inference port and its mock, message/tool/validation types, value types. **Zero runtime dependencies**. Nothing below can bleed a runtime concern upward. |
| **hexa-infer** | Every inference adapter (Ollama, OpenAI-compatible, Anthropic, frontier CLI), the endpoint registry, tier resolution, and the local provider's identity. **No file outside this crate names a provider or a model.** |
| **hexa-exec** | The agent loop, per-run worktree isolation, the adversarial harness, transcript compression, the guarded tool library, the file-backed local store. |
| **hexa-analysis** | Tree-sitter boundary checking, the layer classifier, dead-export and cycle detection, rule conformance, the architecture fingerprint, six health detectors. Powers `hexa analyze`. |
| **hexa-graph** | The code knowledge graph: `context_for`, `rank_lessons`, community detection. Builds and reads `graph-out/graph.json`. |
| **hexa-git** · **hexa-parser** | Git plumbing over libgit2 · parsing utilities. |
| **hexa-cli** | The binary, and the only composition root. The one place adapters are wired together. |

## The loop, and the hooks that keep it

Decide (an ADR) → Gate (the command that must exit 0, written before the code)
→ Build → Harden. `hexa loop` records where a project's work stands, as one
entry in hexa memory: the ADR, the gate and the stage. `hexa do`, `hexa build`
and `hexa harden` record their gate as they run.

`hexa init` installs Claude Code hooks that call the binary. Each hook is one
short process that reads the harness's JSON payload; the payload's
`session_id` keys the session state in `~/.hexa/sessions/`.

| Hook | What it does |
|---|---|
| `session-start` | prints the architecture fingerprint and where the work stands |
| `route` | sizes the prompt (T1 trivial, T2 a change with a shape, T3 feature-sized); on T2 and T3 prints the loop, on T3 drafts a workplan |
| `pre-edit` | boundary check; in a T2 or T3 session with no gate recorded, stops the edit in mandatory mode and warns in advisory mode |
| `pre-bash` | stops destructive commands |
| `pre-agent` | a code-writing subagent must have `isolation: "worktree"` |
| `subagent-start`, `subagent-stop` | record the subagent; on stop, name worktree branches with unmerged commits |
| `post-edit` | runs `hexa analyze --file` on the edited file |

`lifecycle_enforcement` in `.hexa/project.json` is `mandatory` (stop) or
`advisory` (warn).

## State

All of it is files on disk.

| What | Where |
|---|---|
| Lessons, gaps, decisions | `~/.hexa/memory.jsonl` (`hexa memory`) |
| Where each project's work stands | the same file, key `loop:<project>` (`hexa loop`) |
| Session state, keyed by the harness session id | `~/.hexa/sessions/agent-<session_id>.json` |
| Agent and subagent run feed | `~/.hexa/agent-runs.jsonl` (`hexa do runs`) |
| Token spend | `~/.hexa/inference-log.jsonl` |
| Registered inference backends | `~/.hexa/inference-servers.json` (`hexa config inference`) |
| Code knowledge graph | `graph-out/graph.json` (`hexa graph build`) |
| ADRs, workplans | `docs/` |
| Project config and rules | `.hexa/project.json`, `.hexa/ADR-rules.toml` |

Files rather than a database because every reader is a short-lived process on one
machine. A cache in front of a file that only a short-lived process reads is not a
cache. It is a second source of truth that can disagree with the first.

## Tiered inference routing

A task's tier selects a model from `.hexa/project.json → inference.tier_models`.

| Tier | Use case |
|------|----------|
| T1 | scaffold / transform / script / classification |
| T2 | standard codegen |
| T2.5 | complex reasoning |
| T3 | frontier work |

The do-loop selects separately via `inference.react_models`. Choose both empirically
with **`hexa bench agentic`**. It runs fixtures through the *real* loop in an
isolated worktree and scores per-model pass rates (corpus in
`docs/benchmarks/`). External coding-leaderboard rank does not predict agentic-loop
performance: measured here, the top-leaderboard local model scored last on the grid.

## Hexagonal rules, enforced

`hexa analyze` walks the AST and checks these. hexa obeys them itself: **A+ / 100 /
0 boundary violations**.

| # | Rule |
|---|---|
| 1 | `domain/` imports only `domain/` |
| 2 | `ports/` imports `domain/` only |
| 3 | `usecases/` imports `domain/` + `ports/` only |
| 4 | adapters import `ports/` **only**, never the domain directly |
| 5 | adapters never import other adapters |
| 6 | the composition root is the only file that imports an adapter |
| 7 | relative imports in scaffolded TypeScript use `.js` extensions (NodeNext) |

Rule 4 is the one implementations break. An adapter needing a domain type gets it
because the **port re-exports it**. Each adapter then has exactly one edge into
the core.

**Known gap:** the analyzer checks layer-to-layer edges and does not check
third-party imports, so a project can pull a runtime into `domain/` and still score
A+. Rule 1 is stricter than what is enforced. See
[`docs/analysis/2609120100-real-io-proof.md`](docs/analysis/2609120100-real-io-proof.md).

## Governance

- **ADRs are append-only.** A changed decision gets a new ADR that supersedes the
  old one. Nothing is edited or deleted. Lifecycle `Proposed → Accepted → Completed`,
  or `Rejected | Abandoned | Superseded | Deprecated`, changed only through
  `hexa adr accept|complete|supersede`.
- **Epochs** group ADRs by design era, so an old decision can be read in the context
  it was made. `hexa adr reindex` regenerates the [INDEX](docs/adrs/INDEX.md).
- **`founding-goals.md` is the one artifact agents may not author or amend.** Editing
  it requires a human commit under CODEOWNERS; retiring a goal additionally requires
  a Retirement-ADR explaining why it no longer serves the project.

## Build & test

```bash
cargo build -p hexa-cli --release
cargo test --workspace
hexa analyze .
hexa ci --standalone-gate
```
