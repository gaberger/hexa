# Change 01: swapping the storage adapter

**Date:** 2026-09-16
**Task:** Replace the file-backed store with SQLite via Bun's built-in module.
No new package dependency. The cache stays in front of it, behind the same
port. The gate must still pass and must not be edited.
**Arms:** hexa, GitHub Spec Kit, BMAD-METHOD, each continuing from the system
it built in the first round.

This is the change the build round could not measure. A greenfield build shows
whether a framework can produce a layered system; only a change shows whether
the layering was real.

## Findings

1. **All three absorbed a database swap inside one adapter.** Zero files under
   `src/domain/` changed in any arm. Zero under `src/usecases/`, zero under
   `src/adapters/primary/`, and no port signature moved. All three touched
   exactly four source files, and all three landed on the same single line
   beyond the expected radius: a doc comment in the port that named the
   deleted file store.
2. **The architecture is therefore not a differentiator.** It was specified in
   prose to every arm, every arm built it correctly in round one, and every
   arm now absorbs a storage-technology change the way ports and adapters
   promises. hexa's analyzer was available to hexa alone and did not change
   the outcome.
3. **The defect gap held on entirely new code.** Six probes, each validated
   against known-good and known-bad references: hexa 0, BMAD 2, Spec Kit 4.
   The same ordering as the build round.

## Measured, by the operator

| | Spec Kit | BMAD | hexa |
|---|---|---|---|
| Gate | PASS | PASS | PASS |
| `gate.sh` edited | no | no | no |
| **Domain files changed** | **0** | **0** | **0** |
| Source diff | +95 / −64 | +112 / −92 | +298 / −244 |
| Source files touched | 4 | 4 | 4 |
| Beyond adapter + root | 1 comment | 1 comment | 1 comment |
| Wall clock | 16 min | 19 min | 25 min |
| Session tokens | 178,894 | 205,884 | 158,208 |
| Metered cost | $0.00 | $0.00 | **$9.66** |
| Tests after | 44 | 64 | 89 |
| Probed defects | **4** | **2** | **0** |

hexa wrote roughly three times the diff of either competitor to make the same
change, and spent $9.66 doing it, nearly all of it after the swap already
worked and the gate was already green.

## The bug that separated them

hexa's adversarial pass found a stale-handle fault in its own generated
adapter. The adapter validated its cached connection by checking that
`links.db` exists at its path. A path is not an identity. If the store is
replaced underneath a running process — a cleanup, a second instance — the
cached connection still points at the orphaned inode. Writes are acknowledged
with 201 and are invisible to every other process and after a restart.

The fix records the device and inode of the database and its write-ahead log
at open, and compares identity before the insert and again after the commit.
A read-back cannot reveal the fault, because a connection always sees its own
writes.

Probed across all three arms (`bench/probes/p7-stale-store-handle.sh`):

| | behaviour when the store is replaced under a live process |
|---|---|
| hexa | Detects the replacement and reopens. Write accepted and visible. |
| BMAD | Refuses the write with a 500. Safe, unavailable. |
| Spec Kit | Accepts with 201. The write is lost. |

The arm's own note is the honest summary of why its gate could not have caught
it: the gate is a black box over one process and one store directory that
nobody deletes. The adversarial pass is the only step that inspects the
mechanism.

## The bench blocked the measurement first

Per ADR-2609160100, the new probe could not score anything until it was shown
to discriminate. It passed both reference implementations on its first run and
was reported VACUOUS, which halted the whole bench. It became usable only once
the known-bad fixture carried a cached-append-handle defect of its own.

That is the rule working as intended on the first new probe written after it
was adopted.

## What each method surfaced

Worth recording because it is evidence about methods rather than about this
task.

**Spec Kit.** Its own constitution, written during the build round, said in
terms "There is no database" and "Durable state is a file". The change was
illegal under the project's own governance, so the method amended the
constitution first with a major version bump. Its analysis step then found a
durability requirement with zero task coverage, because the gate only ever
sends SIGTERM, and added a test where a child process saves and then SIGKILLs
itself. Its research phase caught, before any code existed, that Bun's SQLite
returns `null` where the port specifies `undefined` — unhandled, every unknown
code would have returned a redirect instead of a 404.

**BMAD.** Its first implementation enabled write-ahead logging; eight
concurrent shortener processes produced six "database is locked" failures,
because the journal-mode switch takes a lock the busy handler never sees. It
also found a constraint-error prefix match reporting a NOT NULL violation as a
taken code, and a connection open that both raced and leaked a handle when a
pragma threw. Thirty review findings: 8 patched, 4 deferred with evidence.

**hexa.** Six candidates, five confirmed, one refuted by the frontier
reviewer, and four of the five confirmed were the same stale-handle bug
restated. It also declined to use `hexa do run` on one test file, on the
grounds that the evidence command would have made the file its own oracle —
this project's own mirror-test failure, correctly identified by the arm rather
than by the tool.

## What this does not show

One change, one operator. The change was chosen by the evaluator, who also
maintains one of the three entrants. A storage swap is the case ports and
adapters is designed for; a change that cuts across layers — a new domain rule
every primary adapter must enforce — would be a harder test and has not been
run.

Three of the four Spec Kit defects and both BMAD defects are carried over from
the build round rather than introduced by this change. The stale-handle fault
is the only defect this round created, and only Spec Kit shipped it.

## References

- `docs/analysis/2609152230-build-trial-results.md` — round one.
- `docs/analysis/2609152100-gate-vs-spec-trial-preregistered.md` — the design;
  this is task type A, change 2.
- ADR-2609160100 — a rubric is tested against known poles.
- ADR-2609160300 — what hexa takes from BMAD.
