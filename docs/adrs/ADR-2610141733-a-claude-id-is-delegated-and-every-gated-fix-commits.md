---
id: ADR-2610141733
status: accepted
date: 2026-10-14
---
# ADR-2610141733: A claude model id is delegated, a missing model says how to set one, and every gated harden fix commits

**Status:** Accepted
**Date:** 2026-10-14
**Drivers:** weave agents ran hexa v26.10.0 on 2026-10-14 and hit four faults. `hexa do run --model claude-opus-5-5` reported "loop ended with no edit" and committed nothing, while the edit was already on disk and the evidence passed by hand (runs `2026-10-14T16:49:39.646448517+00:00#1`, `…16:50:47.465767681…#1`, `…19:58:24.963946966…#1`, `…21:04:09.301249276…#1` in `~/.hexa/agent-runs.jsonl`, and five more on the same day). `hexa do run` without `--model` refused with "no model configured" in a project with no config. `hexa harden` confirmed 15 bugs, committed one, and left the rest in the tree. On `src/domain/log-chain.ts` it said "1 fixed" while its single commit changed code for four findings. And `hexa harden` takes no `--model` where `hexa do run` does.

## Context

**The no-edit runs.** `react_execute_best_of_n` sends a candidate to the `claude -p` delegate (`claude_execute`) only when `is_claude_model` says so, and that matched `claude-code`, `claude` and `claude-code*` only. `claude-opus-5-5` therefore went to the per-step ReAct loop. That loop sends each step to `hexa_infer::complete_raw` with hexa's tool schema. With no registered endpoint for the id, the wiring's prefix fallback picked `ClaudeCodeInferenceAdapter`, which flattens the request to a prompt and runs `claude -p --dangerously-skip-permissions`. The tools are dropped. That `claude` is a full agent with its own Edit tool, running in the process's directory, not in hexa's workspace. It made the edit itself and answered in prose. One answer read "There was no `propose_edit` tool in this session, so I made the edit directly". The loop saw no tool call and ended with "no edit" after zero steps, never running the gate or the commit. File size played no part: the same failure hit a 600-line file, and every `--model claude-code` run on the same files that day committed.

**The missing model.** `do_task` refused unless `resolve_model` found `--model`, `HEXA_DIRECT_MODEL` or `inference.tier_models.t2`, and the message told the operator to set `t2`. The default ReAct path then ignored `t2`. `resolve_react_models` reads only `--model`, `HEXA_REACT_MODEL` and `inference.react_models`/`react_model`, so a project that followed the advice got an empty candidate list and panicked in `select_best_of_n` ("outcomes must be non-empty"). Reproduced with the installed binary. The refusal named no command, because no command sets a model: `hexa bootstrap` writes an empty `tier_models` and its comment says "a project configures its tiers with `hexa config`", which has only `inference` (providers).

The refusal is right. Founding goal G1, and `candidate_models`' own oracle, say hexa does not pick a model nobody chose: a hardcoded last resort cannot be re-pointed by configuration and looks exactly like a run on the right model. A silent fallback to the frontier would also spend money nobody approved. What was wrong is that the advice led to a panic and that no verb did the configuring.

**The harden counts.** The fix loop decided "did the agent edit?" by comparing the *set* of dirty paths before and after each fix. After the first fix dirtied the target, every later fix to the same file left the set unchanged. It was reported as "NO EDIT, the tree did not change" and its change stayed in the tree. The count said 1. The final commit, which took every path the pass dirtied, carried all four. A fix that failed the gate was also left in the tree. Every later fix was then gated on top of it, the final gate failed, and nothing was committed: the "15 confirmed, 1 committed" run. An agent that ran `git commit` itself left no dirty path and was likewise "NO EDIT".

## Decision

1. **A claude id that no registered endpoint serves is a frontier delegate.** `hexa_infer::wiring::served_by_frontier_cli(model)` is the one place that decides this. It is true when no registered endpoint advertises the id and the id names the CLI's family. `is_claude_model` asks it, so the id goes to `claude_execute`. That path runs the agent in hexa's workspace, gates the result with the evidence command, and commits. An id a registered endpoint serves (an Anthropic API key, say) keeps the per-step loop, where native tool use works.

2. **The delegate runs the model asked for.** The `Frontier` port gains `on_model(&self, model) -> Option<Arc<dyn Frontier>>`: the same agent, pinned to a model. The `claude` adapter passes it as `--model`. Its default is `None`, and a caller given `None` fails the run naming the model. It never runs on the CLI's default in silence. `hexa_infer::wiring::frontier_model_arg` maps the path names `claude-code` and `claude` to no pin. The run is recorded under the id that ran.

3. **A tool-carrying request is never sent to the `claude -p` completion adapter.** `RegistryBackends::complete` refuses it with an error that names the delegate. That adapter cannot honour tools, and running it starts an unsandboxed agent in whatever directory hexa was started from. A loud error replaces a silent edit outside the workspace.

4. **The do-loop's last candidate is `inference.tier_models.t2`**, as `resolve_react_model` already documented. `react_models_from_config_value` falls back to it, and `react_execute_best_of_n` answers an empty list with the refusal instead of a panic. `do_task` checks the list the chosen loop will actually use.

5. **`hexa config model [<key> <model>]` sets and shows the do-loop's models.** `<key>` is a tier (`t1`, `t2`, `t2.5`, `t3`), which writes `inference.tier_models.<key>`, or `react`, which writes `inference.react_models` as a one-element list. The rest of `.hexa/project.json` is kept as is. The refusal reads: `no model configured — run \`hexa config model t2 <model>\` (or \`hexa config model react <model>\` for the ReAct loop's list), or pass --model <model>; it reads inference.react_models, then inference.tier_models.t2, in .hexa/project.json (env: HEXA_REACT_MODEL, HEXA_DIRECT_MODEL)`. There is still no default model.

6. **Harden commits each fix that passes the gate, on its own, and counts commits.** Before each fix the loop snapshots HEAD and the content of every changed or untracked file (`git status --porcelain -z -uall`). Afterwards:
   - If the agent committed, `git reset --soft` back to the snapshot folds its commit into the working tree, so its work is still gated and still committed under hexa's identity.
   - The fix's paths are those whose content differs from the snapshot. None means no edit.
   - Gate passes: those paths are committed as `fix(harden): <title>`, and the title goes in `fixed`. A path the operator had changed before the pass is not committed: its content would sweep the operator's work into hexa's commit. Such a fix goes in `uncommitted` and is named.
   - Gate fails: those paths are restored to the snapshot and new files are removed, so the next fix starts from a green tree.

   `fixed` means gated and committed, which is what the verdict line counts. The end-of-pass bulk commit is gone.

7. **`hexa harden --model <id>` pins the frontier agent**, the reviewer that reads the repository and the only one that edits. It fits the design because the frontier is the agent that matters in harden, and `do run` already takes `--model`. An id the frontier CLI does not serve is refused before the hunt. The local fallback reviewer remains `inference.tier_models`, because it is a different model class and a flag that means two things is a bug.

## Consequences

- `--model claude-<anything>` without a registry entry now costs a `claude -p` agent run with `--model`, gated and committed. Before, it cost the same run and threw the result away.
- `hexa do` on a fresh project still refuses. The refusal names a verb and every key, and following it works.
- A harden pass leaves a series of small commits, one per gated fix, instead of one. `git log` reads as the list of fixes, and a bad fix can be reverted alone.
- A harden fix that touches an operator-dirty file is left uncommitted and named. It used to be swept into hexa's commit.
- `Frontier` implementors that cannot choose a model need no change. They answer `None`, and a run that asked for a model fails saying so.

## Implementation

- `hexa-infer/src/wiring.rs`: `served_by_frontier_cli`, `frontier_model_arg`, and the tool-request refusal.
- `hexa-exec/src/ports.rs`: `Frontier::on_model`. `hexa-exec/src/frontier.rs`: the pinned `claude` agent and `--model`.
- `hexa-exec/src/direct_react.rs`: routing through `served_by_frontier_cli`, the pinned delegate, the t2 fallback, and no panic on an empty list. `hexa-exec/src/do_task.rs`: the loop-specific precheck. `hexa-exec/src/direct_exec.rs`: the refusal text.
- `hexa-exec/src/adversarial.rs`: the per-fix snapshot, gate, commit or revert, and `uncommitted`.
- `hexa-cli/src/commands/build.rs`: `hexa harden --model`, and the report's committed and uncommitted counts.
- `hexa-cli/src/commands/inference.rs` and `hexa-cli/src/main.rs`: `hexa config model`.

**Gate**, written before the code: `cargo test -p hexa-exec --test harden_commits_every_fix --test react_models_config_oracle && cargo test -p hexa-exec --lib direct_react::tests`:

- three fixes to one file give `fixed = [bug-1, bug-2, bug-3]`, three commits and a clean tree;
- a gate-breaking middle fix is reverted with its test file, the other two commit, and the final gate passes;
- a fix the agent committed itself is re-committed by hexa and counted;
- a t2-only config yields `[t2]`, and a react list still wins;
- `claude-opus-5-5` routes to the delegate, and a local tag does not;
- the refusal names `hexa config model t2 <model>`, `--model` and `inference.react_models`.

## References

- ADR-2606072044: evidence-gated best-of-N, where the candidate list comes from.
- ADR-2606080915: the frontier fallback as a delegate, not a per-step backend.
- ADR-2609122048: harden commits the tests it wrote. Kept: each fix's commit carries its test.
- ADR-2609131646: a gate that never ran is not a failed gate.
