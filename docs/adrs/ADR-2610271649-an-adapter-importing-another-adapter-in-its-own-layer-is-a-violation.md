---
id: ADR-2610271649
status: proposed
date: 2026-10-27
---
# ADR-2610271649: An adapter importing another adapter in its own layer is a violation

**Status:** Proposed
**Date:** 2026-10-27
**Drivers:** The 2026-10-27 drift report (`docs/analysis/2026-10-27_DRIFT_REPORT.md`) found eight tools in `hexa-exec/src/tools` calling `crate::local_store::…` and `crate::telegram_notifier::…`, two other secondary adapters, while `hexa analyze` reported 0 violations. `is_allowed_import` (`hexa-analysis/src/layer_classifier.rs`) returns true for any import whose two ends are in the same layer, so rule 5, "adapters never import other adapters", is checked only between primary and secondary. A probe of hexa with the rule applied within a layer found 48 import edges, about 20 adapter pairs. hexa has graded itself A+ through this.

## Context

A file's layer comes from `analyze.layers`, then a crate named for its layer, then folder names (ADR-2609241707). Nothing says which *adapter* a file belongs to, so two files in `adapters/secondary` are indistinguishable from two files of one adapter. Some of the 20 pairs are one adapter split across files: the CLI and its `fmt.rs`/`assets.rs` helpers, the two halves of the tree-sitter adapter, `vec_stream` and the HTTP inference adapters that share it, the git plumbing in `hexa-git` and the `hexa-exec` wrappers that implement `Worktrees` and `GitHistory` with it. The rest are real couplings: the tools reaching into `local_store` and `telegram_notifier`, `frontier.rs` recording spend through `hexa-infer`'s `spend`, and `hexa-infer`'s config readers (`discover`, `local_provider`, `spend`) calling `tiers` and `registry` directly.

## Decision

1. **A file belongs to exactly one adapter.** `AdapterMap::adapter_of` in `hexa-analysis/src/domain.rs` names it: the `analyze.adapters` group that lists it; else the longest `analyze.layers` key it is under; else the first path segment below `adapters/primary/` or `adapters/secondary/`; else the file itself. A file in no adapter layer has no adapter (`None`). A key naming a source file also names its module, as `analyze.layers` keys do: `src/store.rs` covers `src/store/Disk`.
2. **An import between two different adapters is a violation, in either layer.** An edge whose two ends are adapters (primary or secondary) and whose `adapter_of` differ is reported as "adapters must not import from other adapters". An import inside one adapter is allowed. A module declaring its own submodule (`adapters/mod.rs` → `adapters/secondary`) stays exempt.
3. **A project groups the files of one adapter.** `.hexa/project.json → analyze.adapters` maps a name to the path prefixes that make up one adapter: `{"cli": ["hexa-cli/src/commands", "hexa-cli/src/fmt.rs"]}`. `AdapterMap::new` rejects a path listed in two groups and a group whose paths are in different layers, with an error naming the path; `hexa analyze` reports it as a rule error rather than ignoring it.
4. **hexa groups the adapters that are one adapter.** `cli` (commands, `fmt.rs`, `assets.rs`), `treesitter` (`treesitter_adapter.rs`, `treesitter_api.rs`), `inference-http` (the `hexa-infer/src/adapters/secondary` modules and `vec_stream`), and `git` (`hexa-git`'s `history.rs`, `worktree.rs`, and `hexa-exec`'s `git_history.rs`, `git_worktrees.rs`).
5. **The real couplings go through ports.** The `hexa-exec` tools reach the store and the operator notifier through ports that `hexa_exec::default_deps()` wires, not `crate::local_store` or `crate::telegram_notifier`. `frontier.rs` records spend through a port. `hexa-infer`'s `discover`, `local_provider` and `spend` reach tiers and the registry through `hexa-infer`'s ports.
6. **hexa stays A+ under the corrected rule.** `hexa analyze .` reports 0 boundary violations with the within-layer check on, and `hexa_grades_itself_by_ratchet` holds it there. No coupling is listed as an exception.

## Consequences

- The grade means what rule 5 says. A project whose secondary adapters call each other loses 10 points per edge until it groups them honestly or puts a port between them.
- A project that relies on folder names gets per-adapter identity with no configuration; one that declares layers by path gets one adapter per declared key.
- ARCHITECTURE.md's rule 5 and its tool-registration paragraph are corrected with the change.

## Gate

`hexa spec verify ADR-2610271649`: the spec's sealed tests, each red before the code (ADR-2610221540 §5).

## References

- ADR-2609241707: a grade cannot exceed the code it could classify.
- ADR-2609122048: a module declaring its own submodule is nesting, not coupling.
- `docs/analysis/2026-10-27_DRIFT_REPORT.md`: the finding.
