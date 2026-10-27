---
id: ADR-2610261356
status: accepted
date: 2026-10-26
---
# ADR-2610261356: A primary adapter may drive a use case; the docs say so

**Status:** Accepted
**Date:** 2026-10-26
**Drivers:** Issue #19. A project's primary adapter imported `crate::usecases` and graded A+ with 0 boundary violations, while the `CLAUDE.md` that `hexa dev init` wrote into that project said "**adapters/primary/** imports from **ports/** only." A user who read the rule and trusted the grade got two answers.

## Context

The analyzer has one table of allowed imports,
`hexa-analysis/src/layer_classifier.rs::allowed_targets`. Since the first
commit (7fa9d1f, 2026-09-12) it has said:

```rust
// Primary adapters may drive use cases.
HexLayer::AdaptersPrimary => &[HexLayer::Ports, HexLayer::Usecases],
HexLayer::AdaptersSecondary => &[HexLayer::Ports],
```

and `primary_adapters_may_drive_usecases_but_secondary_may_not` pins it.

Five documents stated the stricter rule instead:

| Document | Text |
|---|---|
| `hexa-cli/assets/templates/claude-md-hexa-section.md` (shipped into every scaffold) | "adapters/primary/ imports from ports/ only" |
| `CLAUDE.md` | "`adapters/primary/` and `adapters/secondary/` import `ports/` only" |
| `README.md`, `ARCHITECTURE.md` | "adapters import `ports/` **only**" |
| ADR-2609121400 | "adapters import `ports/` only" |

No ADR recorded the allowance. It existed only in code.

### Which side should move

Measured on this tree with hexa 26.10.0, grading with the table as it is and
with `AdaptersPrimary => &[HexLayer::Ports]`:

| Project | Today | Ports-only |
|---|---|---|
| hexa itself (`.`) | A+ 100 | **F 0** |
| `examples/game-2048-ts`, `game-life-rs`, `linkstore-svc`, `ratelimiter-proof`, `url-shortener-rs` | A+ 100 | A+ 100 |
| `examples/stress` | F 31 | F 21 |
| `examples/game-ttt-go` | F 0 | F 0 |

hexa's own CLI commands are primary adapters that call the analysis and
execution use cases. The allowance supports hexa's own design. Tightening
the table would grade hexa F and break `hexa_grades_itself_by_ratchet`.

## Decision

1. **The table stands.** A primary (driving) adapter may import `ports/` and
   `usecases/`. A secondary (driven) adapter imports `ports/` only. Neither
   imports `domain/` directly.
2. **The docs follow the table.** The template, `CLAUDE.md`, `README.md` and
   `ARCHITECTURE.md` now state rule 4 that way. ADR-2609121400 is history and
   is not edited; this ADR refines its rule 4.
3. **The docs cannot drift again.**
   `hexa-cli/tests/docs_state_the_enforced_layer_rule.rs` asks
   `get_violation_rule` whether primary → usecases is allowed and fails if any
   of the four documents states rule 4 differently, in either direction.
4. **A project that wants the stricter rule** keeps it locally (for example, a
   test that reads `src/adapters/primary/`). Making it configurable per project
   is issue #21.

## Consequences

- Projects scaffolded after this change get a `CLAUDE.md` that matches their
  grade. Existing projects get it with `hexa refresh`.
- A change to the table now fails a test until the docs change with it.
- Generated HTTP adapters still import `ports/` only (ADR-2610100005). That is
  a stricter choice for generated code, not a contradiction: allowed is not
  required.

## Implementation

- `hexa-cli/assets/templates/claude-md-hexa-section.md`, `CLAUDE.md`,
  `README.md`, `ARCHITECTURE.md`: rule 4 restated.
- `hexa-cli/tests/docs_state_the_enforced_layer_rule.rs`: the gate. It failed
  on the old template with "states rule 4 as \"4. **adapters/primary/** imports
  from **ports/** only.\", but the analyzer allows a primary adapter importing
  usecases".
- Gate: `cargo test --workspace --release --locked` and the CI steps.

## Gate

`cargo test -p hexa-cli --test docs_state_the_enforced_layer_rule`: every shipped statement of rule 4 matches `allowed_targets`, so a change to the table fails until the docs change with it.

## References

- Issue #19; related #20 (`--grade A` passes a violation), #21 (layer-scoped project rules)
- ADR-2609121400 — the two gates, rule 4 as first written
- ADR-2610100005 — the HTTP adapter imports the port only
