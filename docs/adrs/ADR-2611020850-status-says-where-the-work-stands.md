# ADR-2611020850: `hexa status` says where the work stands

**Status:** Proposed
**Date:** 2026-11-02
**Drivers:** In strata on 2026-11-02, `hexa status` printed the cwd, `.hexa/ found`, the HEAD hash and the branch. At that moment main was 81 commits ahead of an origin that refused connections, the tree had two modified files, and the loop's gate had been replaced by an unrelated run (ADR-2611020815). None of it showed. Each fact was found by hand, one command at a time.

## Context

`hexa-cli/src/commands/status.rs` is 56 lines. Its doc comment promises "service health"; no check
exists. Everything a person needs in order to answer "where does the work stand" is already on
disk or one git command away, and other verbs already compute it: `hexa loop` reads the session's
loop entry, `git rev-list --left-right --count @{u}...HEAD` gives ahead and behind, and
`git status --porcelain` gives the dirty tree. `status` just does not read any of it.

The trap to avoid is the one the lessons name: a report that degrades silently. If the upstream
cannot be reached, `status` must not print a cached "up to date" or omit the line. It must say it
could not tell.

## Decision

1. **Git: ahead, behind, and dirty.** Ahead and behind are counted against the upstream as last
   fetched, and the line says so (`ahead 81, behind 0 of origin/main, as of the last fetch`). No
   upstream is said, not omitted. Modified, staged and untracked counts are shown, with the paths
   when there are ten or fewer.
2. **Remote reachability is opt-in and honest.** `--remote` runs `git ls-remote --exit-code` with a
   short timeout. Unreachable is printed as unreachable with the error's first line. Without
   `--remote`, the line says reachability was not checked. `status` never fetches.
3. **Loop.** This session's ADR, stage, gate, task progress (`n of m done`, current task), and
   `last_run` when ADR-2611020815 kept a gate. Other live sessions get one line each, as the hooks
   already print them.
4. **The doc comment says what the command does.** "Service health" is removed; hexa has no
   services.
5. **`--json`** emits the same fields, for hooks and scripts.

## Consequences

- `status` runs a few git commands; it stays local and fast unless `--remote` is given.
- The strata situation would have shown on one screen: 81 ahead, two files modified, gate kept as
  last_run, and `--remote` would have printed the refused connection.

## Gate

Not recorded yet: this decision is proposed and not built, and hexa's unbuilt list may only shrink (ADR-2610271110 §6), so it records no runnable gate until the work starts. The gate will be an integration test, status_says_where_the_work_stands in hexa-cli, that proves: in a temp repo with a bare
upstream, a commit ahead shows ahead 1; a modified and an untracked file show in the dirty
counts with their paths; with no upstream the line says so; --remote against an unreachable URL
prints unreachable and exits 0; without --remote the line says reachability was not checked; a
loop entry with a gate, tasks and last_run shows all three; and --json carries the same fields.
