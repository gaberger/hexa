# ADR-2611230900: A layer's import policy covers the files declared in that layer

**Status:** Completed
> 2026-11-23 Completed: landed on main at df9f5f9; gate `cargo test -p hexa-cli --test adr_2611230900_spec` passed (hexa loop stage done)
> 2026-11-23 Accepted: Accepted in craigjohnson's session: he judged it a bug fix (three projects found mapped domain files silently unchecked); he accepted the spec, and its sealed tests pass on main
**Date:** 2026-11-23
**Drivers:** Three projects, in Python, Go and Python again, found the same silent gap: a file `analyze.layers` puts in the domain is graded as domain, but the domain's `[[import_policy]]` never reads it. A Go project added `_ "os/exec"` to `domain/access_control.go` and to `perfpayload/encode.go` (declared domain in `analyze.layers`): the first was reported, the second was not, and nothing said why. A Python SDK whose domain is declared by map imported `os`, `ssl` and `subprocess` in it and graded A+. ADR-2611211324 makes `analyze.layers` the normal way a Python project declares its layers, so for Python the policy covered almost nothing. CLAUDE.md: a file's layer is "`.hexa/project.json → analyze.layers` first, then a crate named for its layer, then the folder names", and "a gate that degrades silently is worse than no gate".
**Supersedes:** none. Completes ADR-2609211430 §2 for `analyze.layers`, which came after it: path matching stays as decided, and a declared file is added to the layer it is declared in.

## Context

- ADR-2609211430 §2: "`layer` is a path substring, matched the same way as `hex_layer_rules.path_pattern`, so the policy scopes positively." ADR-2609221430 §7 anchors it to whole path segments and leaves out top-level `tests/`, `benches/` and `examples/`. When those were decided, a folder name was the only way a file entered a layer.
- `evaluate_import_policies` (`hexa-cli/src/commands/analyze.rs`) selects a policy's files with `layer_matches(rel, policy.layer)` alone.
- `LayerMap` (`hexa-analysis/src/layer_classifier.rs`) reads `analyze.layers`: each key is a project-relative path prefix on whole segments (a key naming a source file also names its module), mapped to a layer name; `LayerMap::classify` gives a declared file its declared layer, else the built-in folder classification. The layer names are `domain`, `ports`, `usecases`, `adapters/primary`, `adapters/secondary`, `infrastructure`, `composition-root`, `entry-point` (`HexLayer`, `hexa-analysis/src/domain.rs`).

## Decision

1. **A policy whose `layer` names a layer also covers the files declared in it.** When a policy's `layer`, with its leading and trailing `/` removed, is exactly a layer name (`domain` for the scaffolded `"/domain/"`, `ports`, `usecases`, `adapters/primary`, `adapters/secondary`, …), the policy applies to every file that an `analyze.layers` key assigns that layer, in addition to the files its path matches today.
2. **Only an explicit declaration adds a file.** A file is added by §1 only when an `analyze.layers` key covers it (by the rule `LayerMap::classify` applies to keys); the built-in folder classification adds nothing beyond what the path already matches, so a project with no `analyze.layers` is checked exactly as before.
3. **The exclusions and the judgement are unchanged.** A file under a top-level `tests/`, `benches/` or `examples/` is out of scope as before (ADR-2609221430 §7); a file both matched by path and declared is judged once; allow, deny and severity are read as before.
4. **Nothing else changes.** A policy whose `layer` is a path that names no layer (`"netbox_plugin/utilities"`) stays path-only; the boundary rules, the grade formula and every other rule kind are as before.

## Consequences

- A project that declares its domain by `analyze.layers` gets the domain policy it was written for: the I/O imports it was silently allowed are now reported, and its grade can fall by the errors they were always meant to be.
- A project that declares nothing, or names policies by a path that is not a layer, is checked exactly as before.

## Implementation

**Gate:** `cargo test -p hexa-cli --test adr_2611230900_spec`

Every phase ends with `cargo test --workspace` and `hexa analyze .`.

## References

- ADR-2609211430: the domain imports only what it is allowed (§2)
- ADR-2609221430: every reference is read and judged (§7)
- ADR-2611211324: hexa reads Python (`analyze.layers` as the way in)
- `hexa-analysis/src/domain.rs` (`HexLayer`), `hexa-analysis/src/ports.rs`
