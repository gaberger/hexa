# Change request 01: swap the storage adapter

The file-backed store is being replaced. Links must now be persisted in a
SQLite database instead of a file, using Bun's built-in `bun:sqlite` module.
No new package dependency is permitted or required.

## What must be true afterwards

- Links persist in a SQLite database file inside the directory named by
  `STORE_DIR`. The old file-based store is gone, not left alongside.
- The in-memory cache still sits in front of the new store, unchanged, behind
  the same contract.
- Every externally visible behaviour is identical. `./gate.sh .` must still
  exit 0, unchanged. Do not edit `gate.sh`.
- Your own test suite must still pass.

## What this change is measuring

Blast radius. This is a storage-technology swap. A system whose layers are
genuinely separated should absorb it inside the storage adapter and its
wiring, and nowhere else.

The following will be measured after you finish, by the evaluator:

- Lines added and removed.
- Files touched.
- Files touched outside `src/adapters/secondary/` and the composition root.
- **Whether anything under `src/domain/` changed at all.**
- Wall clock and inference cost.

Do not optimise for these numbers by leaving dead code behind or by skipping
work the change requires. Make the change your method says to make.
