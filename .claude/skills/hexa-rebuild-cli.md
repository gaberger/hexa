---
name: hexa-rebuild-cli
description: Rebuild the hexa release binary and atomically replace the busy on-PATH copy
triggers:
  - rebuild hexa-cli
  - rebuild hexa cli
  - update hexa binary
  - install new hexa
  - hexa cli changes
  - replace hexa binary
---

# hexa-rebuild-cli — Rebuild and Atomically Install hexa-cli

**Use this skill when**: you've edited `hexa-cli/src/**` and need the new binary on PATH. Distinct from `/hexa-dev-rebuild`, which targets `hexa-nexus` (the daemon). This is for the user-facing `hexa` CLI.

The catch: the sched daemon, hooks, and any open shell are likely already executing the on-PATH `hexa`, so a naive `cp` fails with `Text file busy`. The fix is to write to a sibling path and `mv` (rename) — the rename atomically swaps the inode, the running process keeps its old inode mapping, and the next `hexa` invocation picks up the new binary.

## Step 1 — Build

```bash
cargo build -p hexa-cli --release
```

If `cargo` isn't on PATH (some shells don't source `~/.cargo/env`), use the absolute path:

```bash
~/.cargo/bin/cargo build -p hexa-cli --release
```

If the build fails, STOP and report. Do not proceed to install a stale binary.

## Step 2 — Locate the on-PATH hexa

```bash
which hexa
```

Common locations: `/home/<user>/.local/bin/hexa`, `/usr/local/bin/hexa`, `~/.cargo/bin/hexa`. Capture the path; you'll need it in step 3.

## Step 3 — Atomic replace

```bash
cp target/release/hexa <PATH-FROM-STEP-2>.new
mv <PATH-FROM-STEP-2>.new <PATH-FROM-STEP-2>
```

**Why two steps**: `cp` directly to the busy target fails with `Text file busy` on Linux. Writing to `.new` first and then `mv` works because `mv` (rename) on the same filesystem unlinks the old inode and creates a new directory entry pointing at the new inode in one syscall. The running process keeps executing the unlinked inode until it exits; new `hexa` invocations get the new binary.

**Same filesystem requirement**: `mv` is only atomic when source and destination are on the same filesystem. If `target/release/` and `~/.local/bin/` are on different mounts, copy to a temp file *next to* the destination first.

## Step 4 — Verify

```bash
hexa --version
```

The version string should reflect the new build. If the daemon is using sched-related changes, also restart it so the daemon picks up the new binary:

```bash
hexa sched daemon-restart
```

## Step 5 — Sanity check

If your changes touched a specific subcommand, exercise it:

```bash
hexa <changed-subcommand> --help
```

## Common pitfalls

- **`cp` directly** — fails with `Text file busy` whenever a daemon, hook, or shell is running `hexa`. Always use `cp .new` + `mv`.
- **Forgetting daemon-restart** — the long-running sched daemon keeps the OLD binary loaded in memory until restart. CLI invocations get the new binary, but daemon-side behavior (hook routing, tick logic) stays old.
- **Cross-filesystem mv** — silent fallback to copy-then-delete loses atomicity and can fail mid-way leaving no `hexa` on PATH. Stage the temp file on the destination filesystem.
- **Building debug instead of release** — `cargo build -p hexa-cli` produces `target/debug/hexa`, not `target/release/hexa`. The on-PATH binary is the release build.

## Why not `hexa doctor self-update`

`hexa doctor` doesn't currently have a self-update flow. If/when it does, prefer it — but the atomic-rename pattern stays correct as the underlying mechanism.

## ARGUMENTS

No arguments required. Run with: `/hexa-rebuild-cli`
