# ADR-2609121400: hexa is a scaffolding system with two gates

**Status:** Accepted
**Date:** 2026-09-12
**Epoch:** hexa
**Drivers:** This is the first decision in the ledger. It records what hexa is, so that every later decision has something to be measured against.

## Context

An AI agent writes working code in minutes. It also writes code whose tests
pass and whose shape is wrong: a use case imports a database driver, a domain
type depends on an HTTP client. The suite is green and nothing fails. The next
change is a little harder, and the one after that is harder still.

A written specification does not close that gap. A specification is prose.
Prose cannot fail, so code drifts from it in silence, and a document that
cannot fail is indistinguishable from a document that is wrong.

## Decision

1. **Two gates decide whether generated code counts.** A command that must
   exit 0, and an architecture grade that must hold. Both are executable, so
   both fail the moment they stop being true. A gate that runs no tests is a
   failed gate, not a passed one.

2. **The floor is deterministic.** `hexa init --scaffold` writes the same
   files on every machine, from templates compiled into the binary. The floor
   has its own gate, and that gate runs before any model call. A skeleton that
   does not build on your machine makes everything measured after it
   meaningless.

3. **The architecture is hexagonal because its rules can be checked by a
   machine.** `hexa analyze` walks the import graph and grades the boundaries:

   | # | Rule |
   |---|---|
   | 1 | `domain/` imports only `domain/` |
   | 2 | `ports/` imports `domain/` only |
   | 3 | `usecases/` imports `domain/` and `ports/` only |
   | 4 | adapters import `ports/` only; a port re-exports the domain types an adapter needs |
   | 5 | adapters never import other adapters |
   | 6 | the composition root is the only file that imports an adapter |

   A dependency that breaks a rule is a graph property, not a matter of
   taste. That is what lets the grade be a gate instead of a suggestion.

4. **Three languages: Rust, Go and TypeScript.** Each has a compiler, and a
   compiler is a gate in the agent loop. Every detector that feeds the grade
   reads all three languages, or declares in its output that it does not
   apply to the language it was run on. A detector never reports zero for a
   tree it did not read. Each detector ships a fixture per language, wired
   and broken.

5. **Lessons ship as rules, not prose.** Every scaffold carries
   `.hexa/ADR-rules.toml`, and `hexa analyze` runs its `[[adr_rules]]` from
   then on. A rule that flags correct code is worse than no rule, because it
   teaches people to skim past the output. A lesson no pattern can check goes
   in `CLAUDE.md`, marked as such.

6. **Model and provider names live inside the inference crate only.** A
   caller that names a model cannot be re-pointed by editing configuration,
   which is the property model-independence means.

7. **hexa obeys its own rules.** `hexa analyze .` on this repository reports
   the grade the README claims, over every crate. The analyzer carries no
   project directory names; what a project excludes from its own grade it
   declares in `.hexa/project.json` under `analyze.exclude`.

## Verification

- `cargo test --workspace` passes.
- `hexa analyze . --exit-code` exits 0 on this tree, and the grade matches
  the README.
- `hexa init --scaffold` in each of the three languages yields a tree on
  which every detector reports zero, or `n/a` with a reason.
- Every `hexa` command named in the shipped documents, skills and templates
  resolves against the binary, and every link resolves
  (`hexa-cli/tests/shipped_docs_name_real_verbs.rs`).

## Consequences

**Positive.** Done has one meaning: the gates pass. The grade means the same
thing in all three languages. The README's claims are checkable, and
`docs/EVIDENCE.md` names the command for each.

**Negative.** A strict gate makes the task harder. Weaker models fail Rust
and Go where they pass TypeScript. That trade is the point.

**Neutral.** The code generation is a frontier model. hexa contributes the
floor, the gates, the grade and the adversarial pass. It does not do the
writing.
