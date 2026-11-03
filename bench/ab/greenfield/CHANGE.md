# Change request: SQLite storage

Links must now be persisted in a SQLite database instead of a file, using
Bun's built-in `bun:sqlite` module. No new package dependency is permitted or
required.

## What must be true afterwards

- Links persist in a SQLite database file inside the directory named by
  `STORE_DIR`. The old file-based storage is gone, not left alongside.
- Repeated resolves are still served from the in-memory cache.
- Every externally visible behaviour is identical. `./gate.sh .` must still
  exit 0. Do not edit `gate.sh`.
- Any tests the project has must still pass.
