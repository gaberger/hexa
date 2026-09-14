# ADR-2609131408: Sessions in one checkout see each other

**Status:** Accepted
**Date:** 2026-09-13
**Epoch:** hexa
**Drivers:** Two Claude sessions worked in the blacksheep checkout at once on 2026-09-13. Each recorded its ADR and gate into `.hexa/loop.json`, which holds one entry, so each erased the other's. One ran its tests on top of the other's uncommitted parser experiment without knowing it was there. The project owner: *"we should provide awareness and coordination, not conflict."*

## Context

The loop is per checkout: `.hexa/loop.json` under the project directory. That is right — a
decision, a gate and a stage belong to a body of work in a tree — and it is not enough when two
sessions share the tree, because the file has one slot and nothing in it says whose. Neither
session was wrong to record; the file was wrong to forget.

Every command Claude Code runs carries `CLAUDE_CODE_SESSION_ID` and `CLAUDE_PID` in its
environment. A hook sees every edit. Between them, hexa knows which session recorded what and
which files each has touched, and today it throws that away.

A lock is the wrong tool. The second session is usually doing legitimate, file-disjoint work, and
the project's own lesson is to parallelize by file boundary. What it needs is to know the boundary.

## Decision

1. **The loop file holds one entry per session.** `{"sessions": {"<id>": {adr, gate, stage,
   evidence, tasks, files, pid, updated}}}`. Every existing reader and writer sees only its own
   entry, so `hexa loop`, the hooks and the stage rules are unchanged in meaning. A flat file from
   before this ADR reads as the current session's entry and is rewritten in the new shape on its
   next write. `hexa loop clear` clears one entry; the file goes when the last entry does.

2. **The session's identity is hexa's own contract, not a host's.** A host sets `HEXA_SESSION_ID`
   and `HEXA_SESSION_PID` for the commands it runs; that is the whole integration for Codex or any
   other agent host, one line in its hook configuration. Claude Code's `CLAUDE_CODE_SESSION_ID` and
   `CLAUDE_PID` are recognised natively so that host needs no line. A plain terminal is its POSIX
   session, `local:<sid>`, which is stable for the life of the terminal and whose leader is a pid.
   A session is live while its pid exists.

3. **Every edit is recorded to the session's entry.** The post-edit hook appends the file's path,
   deduplicated, capped at the forty most recent. This is the boundary the other session needs.

4. **Awareness, in three places, never a block.** `hexa loop` and the session-start and prompt
   hooks print one line per other live session: its stage, ADR, gate and touched files. The pre-edit
   hook, when the file about to be edited is in another live session's list, prints who touched it,
   under what ADR, how long ago. It exits 0. Coordination is the reader's decision; the tool's job
   is to make the fact visible before the edit, not after the diff.

5. **Ended sessions are shown as ended, not hidden.** A dead pid's entry stays until cleared, marked
   `(ended)`, so a reviewer can see what a session that crashed or was closed had recorded.

## Consequences

- Two sessions can now hold two ADRs and two gates in one checkout, and each prompt tells each
  session what the other is under and where it has been.
- The loop file changes shape. It is machine state and should not be committed, which ADR-…
  (machine state stays out of the tree) will settle; until then a committed loop file carries the
  session ids of whoever wrote it, which is harmless.
- A file touched by both sessions is announced, not prevented. If a project wants prevention, that
  is `lifecycle_enforcement: mandatory` extended to this notice, and a later decision.

## Gate

`cargo test -p hexa-cli sessions_see`: the identity resolves hexa's variable, then the host's, then
the terminal's; two sessions record different ADRs and each reads its own;
the other appears with its liveness; a flat pre-ADR file reads as the current session's and is
migrated on write; touched files deduplicate and are found from the other session only while it is
live; clearing removes one entry and the file only when empty.

## Evidence

`cargo test -p hexa-cli sessions_see 2>&1 | grep -E 'sessions_see_each_other|test result: ok. 5'` at 6303d5b with uncommitted changes on 2026-09-13 14:12 UTC:

```text
test commands::loop_cmd::sessions_see_each_other::the_session_is_hexas_own_variable_then_the_hosts_then_the_terminal ... ok
test commands::loop_cmd::sessions_see_each_other::a_flat_file_from_before_reads_as_the_asking_session_and_migrates_on_write ... ok
test commands::loop_cmd::sessions_see_each_other::clearing_removes_one_entry_and_the_file_only_when_empty ... ok
test commands::loop_cmd::sessions_see_each_other::two_sessions_record_two_adrs_and_each_reads_its_own ... ok
test commands::loop_cmd::sessions_see_each_other::touched_files_deduplicate_and_are_seen_from_the_other_session_only_while_it_is_live ... ok
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 243 filtered out; finished in 0.00s
```
