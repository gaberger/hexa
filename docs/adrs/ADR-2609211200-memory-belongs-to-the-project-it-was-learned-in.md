---
id: ADR-2609211200
status: accepted
date: 2026-09-21
---
# ADR-2609211200: Memory belongs to the project it was learned in

**Status:** Accepted
**Date:** 2026-09-21
**Drivers:** `hexa memory` wrote to one file per *user*. Two repositories worked on by the same operator shared one memory, and two sessions with the same `agent_id` in different repositories shared one restart checkpoint. Reproduced: a `adr:0007:why` stored in `/tmp/probe` was listed by `hexa memory list` in an unrelated directory that had never been scaffolded.

## Context

`hexa_exec::local_store` resolved every feed to `~/.hexa` (or `$HEXA_HOME`), and
`MEMORY` was a bare filename joined to it. Nothing in the path carried a
project, and nothing in the *keys* did either:

- `adr:0007:why` names a different decision in every repository that has an
  ADR 7. The second project to store it silently overwrote the first, because
  memory is newest-wins by design.
- `lesson:*` learned in one codebase surfaced as grounding context in an
  unrelated one. `memory_search` is exposed to the model during GROUND, so the
  bleed reached the agent's reasoning, not only the CLI's output.
- `restart:checkpoint:{agent_id}` keyed on the agent id alone. A checkpoint
  carries the open workplan, the stage and the edit count, so a session in
  project B could resume into project A's state.

No error was raised in any of these cases. The wrong answer looked exactly like
a normal answer — the same failure shape as a gate that degrades silently.

The other feeds under `~/.hexa` do not have this problem. Proposals, the
agent-run feed and the inference log describe the machine and what it ran;
none of their records means something different in another repository.

## Decision

1. **Memory is project-scoped by default.** The store is
   `<project>/.hexa/memory.jsonl`, where the project is the nearest ancestor of
   the working directory holding a `.hexa/` directory — the same marker
   `.hexa/project.json` and `.hexa/ADR-rules.toml` already use. The ascent stops
   at `$HOME`: above it there is no project, only other people's directories.
2. **Outside a project the store is `~/.hexa/memory.jsonl`.** A directory that
   was never scaffolded behaves as it did before, so `hexa memory` still works
   from anywhere.
3. **The shared user store is reachable only by naming it.**
   `hexa memory --global` (`MemoryScope::Shared`) reads and writes
   `~/.hexa/memory.jsonl` for entries that genuinely are cross-project, and for
   what a pre-scoping install already wrote. **Nothing falls back between the
   scopes**: an implicit fallback is the bleed this decision exists to stop. The
   model-facing `memory_search` tool and the hook checkpoints have no access to
   the shared scope at all.
4. **`$HEXA_HOME` still overrides both scopes.** It is an explicit statement of
   where hexa keeps state, and explicit wins.
5. **A restart checkpoint is keyed on project *and* agent id** —
   `restart:checkpoint:{project}:{agent_id}`, each component slugged so a project
   named like a path cannot split the key into another's namespace. Project
   scoping already separates checkpoints; this keeps them apart in the shared
   store too, where one `$HEXA_HOME` may serve every project.
6. **Every non-JSON `hexa memory` path prints the file it used.** The scope was
   invisible, which is how one key came to mean two decisions at once.

## Consequences

- A lesson stored in one repository no longer reaches another's grounding. That
  is the point, and it is also the migration cost: entries written before this
  change live in `~/.hexa/memory.jsonl` and are read with
  `hexa memory --global list`, then re-stored in the project that owns them.
- `.hexa/memory.jsonl` now sits beside `.hexa/project.json` in the repository, so
  a team can commit the lessons that belong to the codebase — which is what the
  tool's own model already claimed about rules and decisions.
- A project that was never scaffolded shares the user store with every other
  such directory. This is the pre-existing behaviour, kept deliberately: the
  alternative is `hexa memory` failing outside a project.
- The observability feeds stay per-user. If one of them later grows a meaning
  that differs per repository, it needs its own decision, not this one's.

## Implementation

- `hexa-exec/src/local_store.rs`: `MemoryScope`, `memory_dir`, `memory_path`,
  `resolve_memory_dir` (environment and working directory injected, so no test
  writes the process environment — ADR-2609131749), and `*_scoped` / `*_in`
  variants of the five memory operations.
- `hexa-cli/src/main.rs`, `hexa-cli/src/commands/memory/mod.rs`: the `--global`
  flag and the printed store line.
- `hexa-cli/src/commands/hook/mod.rs`: `checkpoint_key`.

**Gate:** `cargo test -p hexa-cli --test memory_is_project_scoped` — store in
project A, assert project B sees neither the entry nor the key; assert a
subdirectory reads the project above it; assert the fallback outside a project
is the user store and that a project does not inherit it; assert `--global`
reaches the user store and only it. Written before the change and verified to
fail against the previous code (3 of its 4 tests failed; the fourth cannot
fail under a single global store).

## References

- ADR-2608241500 — the local file-backed store that replaced SpacetimeDB.
- ADR-2609131749 — a test never writes the process environment.
- ADR-2609151930 — accepted decisions are written to memory as `adr:<id>`,
  which is the key shape that collided across projects.
