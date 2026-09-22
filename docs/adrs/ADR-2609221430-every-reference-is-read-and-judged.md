---
id: ADR-2609221430
status: accepted
date: 2026-09-22
---
# ADR-2609221430: Every reference in the file is read, and every one is judged

**Status:** Accepted
**Date:** 2026-09-22
**Drivers:** A code review of `main` at `08e7cda` found three ways ordinary code walks past the domain import policy that ADR-2609211600 was meant to close, plus smaller gaps. The README said the check misses exactly three things (macro-generated code, reflection, FFI). That was not true. Every case below was reproduced on `08e7cda` with the shipped `ADR-rules.toml` before any fix was written.

## Context

What held up in review: the `names_external` design — external means `std`/`core`/`alloc` or a `Cargo.toml` dependency key — is sound. Ten of eleven adversarial cases were already caught: generic arguments, trait bounds, `impl … for`, `dyn` types, struct fields, function pointers, and a `use sqlx as db` alias. Doc comments, comments, string literals and local types stayed clean. That behaviour must not regress, and the negative controls in the gate exist to hold it.

What did not hold up, each file under `src/domain/`:

| # | Case | Finding on `08e7cda` | Cause |
|---|---|---|---|
| 1 | `println!("{:?}", std::fs::read("x"));` | **none** | Macro arguments are a `token_tree`; `collect_rust_references` reads `scoped_identifier` nodes, and a token tree contains none |
| 1 | `let _ = vec![std::fs::read("x")];` | **none** | same |
| 1 | `assert!(std::env::var("X").is_ok());` | **none** | same |
| 1 | `let _ = format!("{:?}", sqlx::query("x"));` | **none** | same |
| 2 | `std::collections::HashMap::<u8,u8>::new()` then `std::fs::read("x")`, one line | **none** | `evaluate_import_policies` de-duplicated sites on `(line, first_segment)` **before** judging. The permitted `std::collections` claimed `(1, "std")`, and the denied path was dropped unjudged |
| 2 | same line, order reversed | error | same key, first one wins |
| 3 | workspace `members = ["crates/core"]`; inline `sqlx::query("x")` in that member | **none** (the `use sqlx::PgPool;` form is caught) | `project_names` read `Cargo.toml` at the root and one directory down only |
| 4 | `[target.'cfg(unix)'.dependencies] nix` → `nix::unistd::getpid()` | **none** | `project_names` read `dependencies`, `dev-dependencies`, `build-dependencies` and `workspace.dependencies`, not `target.*.dependencies` |
| 5 | TS `import pg = require("pg");` | **none** | `collect_ts_references` skips every `import_statement`, and `extract_imports` reads a `source` field the import-equals form does not have |
| 6 | TS `` require(`pg`) `` | warning | A `template_string` with no substitutions is a literal, but only `string` nodes were treated as one |
| 7 | `<sqlx::PgPool as Default>::default` | **none** | The outer `scoped_identifier` text starts with `<`, so the first segment read as `<sqlx` |

Row 3 is the sharpest: in one file the `use sqlx::PgPool;` declaration was an error while the inline `sqlx::query("x")` two lines down was not. The same crate, in the same file, got two answers.

Two more findings from reading the code:

- **Silent skips.** In `evaluate_import_policies`, `let Ok(source) = read_to_string(..) else { continue }` and the same on `extract_imports` dropped a file from the check with no finding, and `extract_module_references` returned an empty list on a parse error. A check that finds nothing and a check that never looked printed the same thing — which ADR-2609122048 exists to forbid.
- **Layer match by substring.** `rel_slashed.contains(p.layer)` means `/domain/` also matches `tests/domain/…` and `src/adapters/domain_helpers/…`.

Unrelated to the policy, found in the same review:

- **`hexa assets sync --force` wrote a dead MCP entry.** `sync_mcp_json` wrote `{"command": "hexa", "args": ["mcp"]}`, but the binary has no `mcp` subcommand and answers `error: unrecognized subcommand`. Any client loading `.mcp.json` started a server that exited immediately.
- **Stale permission from the old name.** `.claude/settings.json` and the settings template still permitted `mcp__hex__hex_*`.

## Decision

1. **Macro arguments are read.** Inside a Rust `token_tree`, a run of `identifier (:: identifier)+`, optionally led by `::`, is a path reference, judged like any other. Nested token trees are walked. This covers hand-written code passed to a macro; code a macro *generates* is not covered and stays a documented limit.

2. **Judge first, then de-duplicate findings.** Every declaration and every reference is classified and judged individually. De-duplication applies to *findings*, keyed on `(file, line, policy, the name reported)`, where the name is the `deny` prefix that matched or, for an `allow` miss, the external package. A permitted or out-of-scope reference can never suppress anything.

3. **Every manifest in the workspace is read.** `rust_manifests` reads the root `Cargo.toml`; when it has a `[workspace]` table, that table is authoritative — `members` with `*` globs expanded, minus `exclude`. Without one there is nothing to expand, so the tree is walked instead, skipping `target/`, `node_modules/` and dot directories. From each manifest: the package name for `rust_crates`, and dependency keys from `dependencies`, `dev-dependencies`, `build-dependencies`, `workspace.dependencies`, and every `target.<cfg>.{dependencies,dev-dependencies,build-dependencies}`.

4. **TypeScript import-equals is an import.** `import x = require("m")` is reported by `extract_imports` as an import of `m`. A `` require(`m`) `` or `` import(`m`) `` whose template has no substitutions is judged as the literal `m`. With substitutions it stays a computed-load warning.

5. **Qualified paths are unwrapped.** For a path whose text starts with `<`, the type inside `<… as …>` is walked as a reference in its own right.

6. **Nothing is skipped silently.** A file inside a policy's `layer` that cannot be read or parsed yields a **warning** naming the file and the reason. It does not move the grade — a file that could not be read is not a violation — and `--strict` fails on it.

7. **Layer matching is anchored to path segments.** A `layer` of `/domain/` matches a path whose segments contain `domain` whole. Files under a top-level `tests/`, `benches/` or `examples/` directory are out of scope for `import_policy`.

   **Deviation, recorded.** The Context above named two substring false positives. Only one was real. `/domain/` was never a substring of `/domain_helpers/`, so that case already behaved; and `src/adapters/domain/…` still matches after anchoring, because `domain` there *is* a whole segment. Deciding that a `domain` directory stops being one because of its parent would need a rule about which parents disqualify a layer, which this ADR does not make. What anchoring delivers is the `tests/`, `benches/` and `examples/` exclusion, and the guarantee that a future `domain_x` sibling stays out. Both cases are pinned as controls in the gate.

8. **The MCP entry is removed.** `sync_mcp_json` stops writing a `hexa` server and removes the one it previously wrote, matched on command `hexa` with args `["mcp"]` so a server the user re-pointed is left alone. Every other entry is untouched. `mcp__hex__hex_*` is removed from `.claude/settings.json` and `hexa-cli/assets/templates/hexa-claude-settings.json`. If hexa ever serves MCP, that gets its own ADR and its own gate.

## Consequences

- **More findings on existing projects,** mainly `format!`/`assert!` bodies in domain code and inline references in nested workspace members. Each is a real reach outside the domain that the policy already forbade. `docs/UPGRADING.md` gains an entry naming the new forms.
- **Macro scanning is token-level, not semantic.** `my_macro!(sqlx :: query)` with odd spacing is still a path, and a macro whose arguments merely look like a path to a dependency is flagged. That is rare and self-inflicted, and the finding names the path so it can be allowed deliberately.
- **`extract_module_references` returns a `Result`.** It had one caller, so this is a signature change rather than a second function — the alternative would have been two readers of the same thing, which ADR-2609211200's sibling problem in `hexa-infer` had just been cleaned up.
- **`[workspace]` is authoritative when present.** An excluded member's manifest is not read, so a dependency only that member declares is not a known external name. A walk that also picked up excluded manifests would make `exclude` mean nothing.
- **The README limits section** now lists what remains unseen: code a macro generates, reflection, FFI, and a module loaded by a computed name — the last reported as a warning, not skipped.

## Implementation

- `hexa-analysis/src/treesitter_adapter.rs`: `collect_rust_token_tree` (Decision 1); the qualified-path branch in `collect_rust_references` (5); `import_require_specifier` and `has_substitution` (4); `extract_module_references` returns `Result` (6).
- `hexa-cli/src/commands/analyze.rs`: `evaluate_import_policies` judges before de-duplicating and reports unreadable files (2, 6); `denied_name` keys findings on the reported name (2); `layer_matches` anchors to segments (7); `rust_manifests`, `expand_member`, `walk_manifests` and the `target.*` tables in `project_names` (3).
- `hexa-cli/src/commands/assets_cmd.rs`: `sync_mcp_json` (8).
- `.claude/settings.json`, `hexa-cli/assets/templates/hexa-claude-settings.json` (8).
- `README.md` limits section, `docs/EVIDENCE.md`, `docs/UPGRADING.md`.

**Gate:** `cargo test -p hexa-cli --test domain_import_references_complete`, written first and shown failing against `08e7cda` — **23 of its 38 tests failed**, and the 15 that passed were the negative controls and the cases that already worked. One fixture per row of the Context table, against the shipped rules file.

## References

- ADR-2609211600: a reference is an import; this completes it.
- ADR-2609211430: the import policy, and the rule that warnings do not move the grade.
- ADR-2609122048: a tool that reports "nothing found" must prove it looked.
- ADR-2609132122: a recorded gate is run, or it is prose.
