# ADR-2609132049: The parser crate goes with the host it bound

**Status:** Accepted
**Date:** 2026-09-13
**Drivers:** `hexa-parser` is 1120 lines with zero tests. It has zero tests because nothing has ever run it: it is a NAPI binding, `crate-type = ["cdylib"]`, built to be loaded by a Node.js process that left with HexFlo on 2026-09-12.

## Context

Asked which part of the workspace to test next, the answer by density was this crate. Reading it
first produced a better answer. The evidence that nothing consumes it is unanimous:

- no workspace member depends on it; only the `members` list names it;
- there is no `package.json`, no `index.js`, no `.node` artefact — nothing on the Node side to load
  the library it builds;
- no conditional import, no re-export at any package root, nothing calling `dlopen` or `libloading`;
- `hexa graph consumers hexa-parser/src/lib.rs`, over a graph of 3002 nodes and 3868 edges, reports
  `SAFE TO REMOVE — no inbound importers or entity consumers`.

The project's own rule is to trace consumers before deleting and to grep for the two things a graph
cannot see — a re-export at a package root and a conditionally-compiled import. Both were checked
by hand as well as by the graph.

Writing tests for it would have raised a coverage number over code no caller reaches. That is the
kind of green that hides rather than informs, and this repository spent a day removing exactly that
shape.

## Decision

1. **The crate is deleted**, with its workspace member entry.

2. **Every stale reference goes in the same change**, because documentation about a deleted feature
   never fails and sits there being confidently wrong. That is `CLAUDE.md`, `ARCHITECTURE.md`, and
   the boundary rule table in `hexa-exec/src/tools/workspace_boundary_check.rs`.

3. **The boundary table is repaired, not merely trimmed.** It names seven crates, of which two
   exist: `hexa-nexus`, `hexa-analyzer`, `hexa-agent` and `hexa-desktop` are all gone, and the
   table has been checking boundaries between crates that are not there. It is rewritten from the
   workspace as it is.

4. **`ASSET_GENERIC_MARKERS` in doctor keeps its `hexa-parser` entry.** That list names strings a
   shipped asset must *not* contain, and an asset referring to a crate that no longer exists is
   more wrong than one referring to a crate that does. The rationale survives the deletion.

5. **The change ends with a workspace build**, per the project's rule that a change which deletes
   or restructures is not done until the build is green.

## Consequences

- 1120 lines stop compiling on every build, and the zero in the coverage table stops being a
  standing accusation about untested code when it was really a fact about dead code.
- The tree-sitter dependencies that only this crate used leave the lockfile.
- If a Node host returns, the crate is one `git revert` away, and its grammars will need the newer
  tree-sitter in any case.
- The boundary rule table becomes true for the first time since those crates were removed, which
  means it can start failing for real reasons.

## Gate

`cargo build --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`,
and no occurrence of `hexa-parser` outside this ADR and the asset marker list.

## Evidence

`cargo test --workspace 2>&1 | tail -1; hexa analyze . 2>&1 | grep 'Architecture grade'; echo -n 'clippy exit: '; cargo clippy --workspace --all-targets -- -D warnings >/dev/null 2>&1; echo $?` at 89e20d4 with uncommitted changes on 2026-09-13 20:53 UTC:

```text

  ⬡ Architecture grade: A+ — score 100/100
clippy exit: 0
```
