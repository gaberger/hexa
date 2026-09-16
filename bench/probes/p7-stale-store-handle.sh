#!/usr/bin/env bash
# P7 — a long-lived process must not keep writing into an orphaned store file.
#
# If the store's directory is cleaned (unlink + recreate) while a server is
# running, a cached connection held by inode still points at the deleted file.
# Writes are then acknowledged with 201 and are invisible to every other
# process and after a restart: silent data loss behind a green gate.
#
# Either detect the replacement and reopen, or fail the write. Never accept it
# and lose it.
source "$(dirname "$0")/lib.sh"
start_server || skip "server did not start"
# Warm the connection.
C0=$(post_url '{"url":"https://example.com/warm"}' | head -1 | code_of)
[ -z "$C0" ] && skip "could not create a first record"
# Replace the store underneath the running process, as a cleanup would.
rm -rf "$STORE"/* 2>/dev/null
R=$(post_url '{"url":"https://example.com/after-cleanup"}')
ST=$(echo "$R" | tail -1); C=$(echo "$R" | head -1 | code_of)
if [ "$ST" != "201" ] || [ -z "$C" ]; then
  pass "write refused after the store was replaced (status $ST)"
fi
# Accepted. A separate process must be able to see it.
OUT=$(bun run src/main.ts resolve "$C" 2>/dev/null | tail -1 | tr -d '[:space:]')
[ "$OUT" = "https://example.com/after-cleanup" ] && pass "accepted and visible to another process"
fail "accepted with 201 but invisible to another process — acknowledged write lost"
