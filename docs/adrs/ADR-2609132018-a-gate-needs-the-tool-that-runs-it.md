# ADR-2609132018: A gate needs the tool that runs it, and doctor checks for it

**Status:** Accepted
**Date:** 2026-09-13
**Amends:** ADR-2609131617, which made doctor prove it looked; this is the same rule one step earlier.
**Drivers:** CI runs `cargo clippy --workspace --all-targets -- -D warnings`. Clippy is not a default component of a rustup toolchain, was absent here, and `hexa doctor` reported `All checks passed` on a machine that could not run its own project's gate. A branch sat red for twenty minutes with nothing local able to say so.

## Context

ADR-2609131617 made doctor resolve a configured tier against the backends that can actually serve
it, rather than printing the configuration back. The same gap sits one step earlier: a project
declares gates in CI, and doctor never asks whether this machine can run them.

Clippy is the instance that bit. `cargo build` and `cargo test` come with every toolchain; `clippy`
and `rustfmt` are components that may or may not be installed, and their absence is silent until a
push. A developer cannot fail a gate locally that they have no way to run.

This is not a plea for more checks. It is the same rule as the tier one: a tool that reports on an
installation must report on the parts of it the project actually depends on.

## Decision

1. **Doctor checks the toolchain components the project's own gates need — and only those.** For
   this Rust project that is `clippy` alone, because CI denies warnings with it. The first cut of
   this decision also asked for `rustfmt`, on the assumption that a formatting gate exists; no
   workflow, script or rule in this repository runs one, and doctor duly failed on a tool nothing
   needs. A check that invents a requirement is the same false report as a check that invents a
   result.

2. **A missing component is a failure with the command that fixes it** — `rustup component add
   clippy` — not a warning to be scrolled past. A gate nobody here can run is not a gate.

3. **Only for the project type in hand.** A Go or TypeScript project is not told about clippy. The
   check follows the same `Cargo.toml` detection the section above it already does.

4. **Presence, not version.** Doctor asks whether the component answers, which is what determines
   if the gate can run. Matching CI's exact version is a different problem and not one doctor can
   solve from here.

## Consequences

- A fresh clone on a fresh toolchain is told what it is missing before the push rather than after.
- One process spawn per component on a command that is run by hand.
- `hexa bootstrap`, which installs prerequisites, is the natural place to add them rather than
  report them. That is a larger change and is not made here; doctor naming the exact command is
  most of the value.

## Gate

`cargo test -p hexa-cli toolchain`: a component that answers is reported present; one that does not
is a failure naming `rustup component add <name>`; and no component is checked for a project that
is not Rust.

## Evidence

`cargo test -p hexa-cli toolchain 2>&1 | grep -E 'toolchain_tests::|test result: ok. 3'` at 6648d45 with uncommitted changes on 2026-09-13 20:21 UTC:

```text
test commands::doctor::toolchain_tests::presence_is_decided_by_whether_the_component_answers ... ok
test commands::doctor::toolchain_tests::components_are_required_per_project_type ... ok
test commands::doctor::toolchain_tests::an_unknown_component_is_absent_rather_than_a_panic ... ok
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 272 filtered out; finished in 0.01s
```
