---
id: ADR-2609211245
status: accepted
date: 2026-09-21
---
# ADR-2609211245: The TypeScript gate runs where the network is

**Status:** Accepted
**Date:** 2026-09-21
**Drivers:** `hexa init --scaffold --lang ts` shipped a project whose own gate failed. `npm test` ran `tsc && node --test dist/`, and on Node 22 that exits 1 with `Cannot find module …/dist` — the runner tries to load the directory as a module instead of searching it. The scaffold's gate test existed and asserted the right thing; it was skipped unless `HEXA_TEST_NPM=1`, and nothing set it.

## Context

Two failures, and only the second one explains how the first survived.

**The command.** `node --test <dir>` meant "search this directory for test
files". On Node 22 it means "run this path", and a directory is not a module,
so the freshly scaffolded project fails its first gate with a stack trace
before a single test runs. The Rust and Go scaffolds are fine; only TypeScript
names its test files through the runner's discovery.

**The gate that never ran.** `the_typescript_scaffold_passes_npm_test_after_install`
already asserted `# pass 4` on a fresh scaffold. It returns early unless
`HEXA_TEST_NPM=1` is set, because a test that silently reaches the network is a
coin flip rather than a test — a defensible guard. But no workflow set the
variable, so the assertion had never executed on any branch. This is
ADR-2609132158 again, one layer down: a gate that never runs on the branch is
not a gate, and the thing it was guarding broke in the field.

The repair is not obvious, which is why it is written down. `node --test
dist/*.test.js` — the shell glob the 2048 example uses — passes the scaffold
today and silently stops covering anything the moment a test is added in a
subdirectory. It fails loudly when it matches nothing, which the recursive form
does not, but a gate that quietly narrows is worse than one that is quietly
empty: the empty one at least shows `# pass 0`, and hexa's own
`evidence_is_vacuous` already rejects that shape from node's TAP output.

## Decision

1. **The scaffold's test script is `tsc && node --test "dist/**/*.test.js"`.**
   Quoted, so node does the globbing rather than the shell, and recursive, so a
   test added anywhere under `src/` is part of the gate once compiled.
2. **The scaffold declares the runtime it needs** — `engines.node >= 22`, with
   `@types/node` on `^22` to match. The old `^20` types described a runtime the
   test command no longer works on.
3. **CI sets `HEXA_TEST_NPM=1` and pins Node 22** via `actions/setup-node`. The
   opt-in guard stays for local runs; the network exists in CI, so that is
   where the gate runs. Pinning the version means the job tests the runtime the
   scaffold claims, not whatever the runner image happens to ship.
4. **The gate covers the property that broke it.** After the first `npm test`,
   the test writes `src/core/added-later.test.ts` and asserts the run grows to
   `# pass 5`. A non-recursive repair fails this; verified by making it.

## Consequences

- A scaffolded TypeScript project passes `npm install && npm test` on Node 22
  with no edits, which is the floor `hexa init --scaffold` promises.
- The TypeScript gate costs CI one `npm install` (~6s here) and pulls two dev
  dependencies from the network. That is the price of testing what a user gets.
- Node 20 and earlier are no longer supported by the scaffold. Node 20 left
  maintenance in April 2026 and the directory form was already broken on
  everything newer, so the choice was which of the two to break; `engines`
  makes it say so at install time rather than at first gate.
- A zero-match glob still exits 0. Within hexa that is caught — `tests_observed`
  parses `# pass N` from node's TAP and `evidence_is_vacuous` rejects a gate
  that ran nothing. A user running `npm test` by hand on a project with no
  tests left sees a pass. Closing that needs a flag node does not have.

## Implementation

- `hexa-cli/assets/scaffold/ts/package.json.tmpl` — the test script, `engines`,
  and the `@types/node` bump.
- `.github/workflows/ci.yml` — `actions/setup-node@v4` pinned to 22, and
  `HEXA_TEST_NPM: 1` on the test step.
- `hexa-cli/tests/scaffold_is_executable.rs` — the nested-test assertion.

**Gate:** `HEXA_TEST_NPM=1 cargo test -p hexa-cli --test scaffold_is_executable`
— scaffolds TypeScript, runs `npm install && npm test`, and asserts both that
four tests pass on the untouched scaffold and that a fifth added under
`src/core/` joins them. Verified to fail against the previous template
(`Cannot find module …/dist`) and against the non-recursive repair
(`expected 5 passing, got # pass 4`).

## References

- ADR-2609132158 — a gate that never runs on the branch is not a gate.
- ADR-2609121400 — the executable gate replaces the written spec; the scaffold
  exists to hand you something a gate can run against.
