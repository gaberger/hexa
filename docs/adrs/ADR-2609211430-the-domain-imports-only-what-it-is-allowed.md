---
id: ADR-2609211430
status: accepted
date: 2026-09-21
---
# ADR-2609211430: The domain imports only what it is allowed, and a rule error costs grade

**Status:** Accepted
**Date:** 2026-09-21
**Drivers:** The dependency rule is checked edge by edge between layers, so an import that leaves the project entirely is invisible to it. A domain file that imports `sqlx` scores A+. The rules file can approximate a check, but an error-severity rule violation does not move the grade, so `hexa analyze --grade A` and `hexa scaffold --grade A` both pass with the error present. Reproduced on `main` at `1abe882` (below).

## Context

The README's Limits section already says it: the analyzer checks layer-to-layer edges, so a project can pull a third-party runtime into its domain and still score well. The headline rule ("domain imports only domain") is stricter than what is enforced.

`.hexa/ADR-rules.toml` can express a partial check today. `file_patterns` is a suffix match (`rel.ends_with`, `analyze.rs:1749`), so a rule cannot target `/domain/` directly, but `exclude_patterns` is a substring match (`rel.contains`, `analyze.rs:1753`), so a rule can reach the domain by excluding every other layer:

```toml
[[adr_rules]]
adr = "ADR-0000"
id = "domain-imports-no-runtime"
message = "The domain pulled in an infrastructure crate. Put it behind a port."
severity = "error"
file_patterns = [".rs"]
exclude_patterns = ["/adapters/", "/ports/", "/usecases/", "main.rs", "test"]
violation_patterns = ["use sqlx", "use reqwest", "use tokio", "use axum"]
```

On a tree with `use sqlx::PgPool;` in both `src/domain/order.rs` and `src/adapters/secondary/pg.rs`, this flags the domain file and not the adapter. It has three defects:

1. **It is a denylist.** A crate nobody thought to name passes. The property we want is the opposite: the domain may import the standard library and a short, deliberate list, and nothing else.
2. **It scopes by exclusion.** A new layer directory, or a project whose layers are named differently, silently falls into or out of scope.
3. **It does not cost grade.** Observed on the same tree:

| Command | Exit | Grade printed |
|---|---|---|
| `hexa analyze .` | 0 | A+ (99) |
| `hexa analyze . --grade A` | **0** | A+ (99) |
| `hexa analyze . --exit-code` | 1 | A+ (99) |
| `hexa analyze . --strict` | 1 | A+ (99) |

`--exit-code` and `--strict` fail correctly (`analyze.rs:575-580`, `:610`). `--grade` compares only the health score (`analyze.rs:586-603`), and `compute_health_score` (`hexa-analysis/src/domain.rs:181`) has no ADR term. `hexa scaffold --grade` computes its floor from `deep_analysis(...).health_score` (`scaffold.rs:193-231`) and never reads the rules file at all. So the letter the tool prints, and the floor the scaffold enforces, both disagree with `--exit-code` about the same tree.

Defect 3 is the serious one. It is not specific to this rule: every error-severity rule in every project's rules file is invisible to the grade.

Line-matching is also the wrong instrument for imports. `use sqlx::{self, PgPool}` split over lines, a TypeScript `import type`, and a Go `import ( ... )` block all defeat a substring pattern. The analyzer already parses imports with tree-sitter (`treesitter_adapter.rs:54`, `extract_imports`) to build layer edges.

## Decision

1. **An error-severity rule violation is a violation.** Each ADR violation with `severity = "error"` enters `compute_health_score` exactly as a boundary violation does, at 10 points each. Warnings stay out of the score, as they do today. After this, `--grade`, the printed letter, the scaffold floor and `--exit-code` agree on every tree.

2. **A new rule kind, `[[import_policy]]`, governs what a layer may import from outside the project.**

   ```toml
   [[import_policy]]
   adr = "ADR-2609211430"
   id = "domain-imports-only-what-it-is-allowed"
   layer = "/domain/"
   allow = ["serde", "thiserror"]
   deny  = ["std::fs", "std::net", "std::process", "std::env", "std::io"]
   message = "The domain imported something from outside the project that it is not allowed to know about. Put the capability behind a port, or add it to `allow` deliberately and say why in the ADR."
   severity = "error"
   ```

   - `layer` is a path substring, matched the same way as `hex_layer_rules.path_pattern`, so the policy scopes positively.
   - Imports come from the tree-sitter extraction that already builds layer edges, not from line matching.
   - An import that resolves inside the project is out of scope for this rule; layer edges already govern it.
   - An external import is allowed if it matches the language's standard library or an `allow` prefix, **unless** it matches a `deny` prefix. `deny` wins. This keeps standard-library I/O out of the domain without listing every safe module.
   - Prefixes match on module-path boundaries (`serde` matches `serde::Deserialize`, not `serde_json`).

   Classification per language:

   | | Inside the project | Standard library | External |
   |---|---|---|---|
   | Rust | `crate::`, `self::`, `super::`, the package's own crate name | `std`, `core`, `alloc` | any other first segment, including `extern crate` |
   | TypeScript | relative (`./`, `../`), `tsconfig` path aliases | `node:` specifiers and bare Node built-ins | any other bare specifier; `import type` counts |
   | Go | paths under the module path in `go.mod` | first path element contains no dot | everything else |

3. **Every scaffold ships a domain `import_policy`.** Default `allow` is empty beyond the standard library. Default `deny` is the standard library's I/O surface:
   - Rust: `std::fs`, `std::net`, `std::process`, `std::env`, `std::io`
   - TypeScript: `node:fs`, `node:net`, `node:http`, `node:child_process`, `node:process` (and their bare forms)
   - Go: `os`, `net`, `net/http`, `database/sql`, `os/exec`, `io/ioutil`

   The scaffold's own domain code must pass it.

4. **`[[adr_rules]]` gains `path_patterns`**, a substring match on the relative path, applied alongside `file_patterns`. This retires scoping-by-exclusion for the existing rules. Existing rules files keep working because the field defaults to empty, meaning no restriction.

5. **`hexa analyze` prints the formula with the new term**, and `--json`'s `explain.formula` changes with it:
   `100 - 10*(violations + rule_errors) - 15*circular_deps - min(dead_exports, 20) - min(unused_ports, 10)`

## Consequences

- **Grades drop on trees that already carry rule errors.** A CI job gating on `--grade A` may go red after upgrading. That is the intended correction, since those trees were never at the grade they printed, but it needs a release note naming the change and the one-line fix (resolve the error, or downgrade the rule to `warning` deliberately).
- **Real domains use real crates.** `serde`, `uuid`, `chrono`, `rust_decimal` and similar will fail the default policy until they are allowlisted. Each addition is one line and a reason. The friction is the point: the allowlist is the written record of what the domain depends on.
- **Standard-library time and randomness are allowed by default.** `std::time::SystemTime::now()` and `rand` inside the domain make it nondeterministic without touching I/O. This ADR does not forbid them. A project that wants a pure domain adds them to `deny`. A later ADR may make that the default.
- **Reflection, dynamic `import()`, and Go `plugin`** are not static imports and are not seen. That limit goes into the README's Limits section in the same change.
- **The README's Limits section changes.** The third-party-runtime gap moves from open to covered, with the classification table above as its documented boundary.

## Implementation

- `hexa-analysis/src/domain.rs`: `compute_health_score` takes a rule-error count. Update its callers (`analyzer.rs:444`) and the four `health_score_*` tests.
- `hexa-cli/src/commands/analyze.rs`: pass error-severity ADR violations into the score before the grade is computed. `check_adr_compliance` (`:1667`) currently runs after scoring, so it moves earlier or its count is threaded through. Add `path_patterns` to the rule struct (`~:1536`) and the matcher (`~:1748`). Update `explain.formula` (`:1803`).
- `hexa-cli/src/commands/scaffold.rs`: the floor at `:193-231` uses the same score, so it inherits the fix. Assert it in a test rather than assuming it.
- New: `import_policy` parsing, and an evaluator that consumes the layer and import data the analyzer already extracts (`treesitter_adapter.rs`, `layer_classifier.rs`). External-vs-internal classification per the table.
- `hexa-cli/assets/templates/ADR-rules.toml`: the domain `import_policy` per language, with the message above.
- `README.md`: the Limits section, as above.

**Gate:** `cargo test -p hexa-cli --test domain_import_policy`, written first and shown failing against `1abe882`:

1. Domain imports `sqlx` → one error; the grade drops by 10; `analyze --grade A+` exits 1 on an otherwise perfect tree.
2. The same import in `adapters/secondary/` → no finding.
3. Domain imports an allowlisted crate → no finding.
4. Domain imports `std::fs` → error via `deny`, even though `std` is allowed.
5. `use sqlx::{self,\n PgPool};` split across lines → caught.
6. TypeScript: `import type { Pool } from "pg"` and `import fs from "node:fs"` in the domain → both caught; `import { X } from "./x"` → not.
7. Go: `"os"` and `"github.com/jackc/pgx/v5"` in the domain → both caught; an import under the module path → not.
8. `hexa scaffold --grade A` on a fixture whose domain imports a denied module → exits non-zero.
9. A warning-severity violation → grade unchanged; `--strict` still exits 1.

Tests 1 and 8 must fail on `1abe882`. Tests 2 and 9 pass on both sides and exist to pin behaviour that must not regress.

**Evidence to append when the stage is marked done:** `hexa analyze . --json | jq '.explain.formula, .health_score'` on hexa itself. Hexa's own tree carries two pre-existing warnings (`narrowing-cast-needs-a-check`, `gate.rs:184-185`) and no rule errors, so its score must not change.

## Reproduction (before)

```sh
T=$(mktemp -d); mkdir -p $T/.hexa $T/src/domain $T/src/adapters/secondary; cd $T
printf '[package]\nname="rt"\nversion="0.1.0"\nedition="2021"\n' > Cargo.toml
cat > .hexa/ADR-rules.toml <<'EOF'
[[adr_rules]]
adr = "ADR-0000"
id = "domain-imports-no-runtime"
message = "The domain pulled in an infrastructure crate."
severity = "error"
file_patterns = [".rs"]
exclude_patterns = ["/adapters/", "/ports/", "/usecases/", "main.rs", "test"]
violation_patterns = ["use sqlx"]
EOF
printf 'use sqlx::PgPool;\npub struct Order;\n' > src/domain/order.rs
printf 'use sqlx::PgPool;\npub struct PgStore;\n' > src/adapters/secondary/pg.rs
hexa analyze . --grade A; echo "exit $?"      # prints A+, exits 0
hexa analyze . --exit-code; echo "exit $?"    # exits 1
```

## References

- ADR-2609121400: the shipped rules file and its message-first convention.
- ADR-2609211200: the most recent change to what `.hexa/` holds, for the migration-note pattern.
- README, Limits: the documented layer-to-layer gap this closes.

## Implementation status

Implemented. §1, §4 and §5 are gated by
`cargo test -p hexa-cli --test rule_errors_cost_grade` (seven tests; three
failed against `1abe882`). §2 and §3 are gated by
`cargo test -p hexa-cli --test domain_import_policy` — the nine cases above
plus one for §3 — and by eight unit tests over the pure classification in
`hexa-analysis/src/import_policy.rs`.

Three things landed differently from the plan above, and the differences are
the record rather than the plan:

1. **The rule-error term is applied in `analyze::deep_analysis`, not in
   `analyzer.rs:444`.** `hexa-analysis` does not read the rules file, and
   giving it that job to make one call site correct would have moved the
   rules engine into a crate that analyses structure. `deep_analysis` is the
   one door `hexa analyze`, `--json` and the scaffold floor all read the score
   from, so the four surfaces agree without three copies of the arithmetic.
   `compute_health_score` takes the count as its fifth argument, so the
   formula is still in one place. The cost is that the rules are evaluated
   twice per `hexa analyze` — once for the score, once for the compliance
   section — and the second evaluation no longer announces itself.

2. **One `[[import_policy]]` ships, not three.** One rules file serves all
   three languages, and the prefixes are namespaced by language in practice
   (`std::fs` matches nothing in a Go project). Three blocks sharing a prefix
   like `net` would report one import twice and charge the grade for both.

3. **`extern crate` is not covered**, though the table above lists it. The
   Rust extractor reads `use_declaration` only, and teaching it a second node
   kind changes the import graph every other analysis is built on. It is
   recorded in the README's Limits section alongside reflection, dynamic
   `import()` and Go `plugin` — the other things a static import declaration
   does not show.

The reporter also changed: an import policy's message names the import it
found, so sites under one rule no longer say the same thing, and printing only
the first one reported `pg` and then listed a line that was about `node:fs`.
Sites with differing messages now print one line each.

**Evidence.** hexa's own tree: A+ 100/100, `violations 0 · cycles 0 · dead
exports 0 · unused ports 0`, unchanged, which is what this ADR named as the
check that the repository was not silently re-scored. Its own rules file gains
no policy: hexa has no `/domain/` directory — `hexa-analysis/src/domain.rs` is
a file — so the shipped policy would match nothing here and a no-op rule is
noise.
