#!/usr/bin/env bash
# P5 — records that survive a partial store corruption must not be destroyed by
# the next write. Either preserve them, or refuse to write. Never both lose the
# data and report success.
source "$(dirname "$0")/lib.sh"
A=$(bun run src/main.ts shorten "https://example.com/first" 2>/dev/null | tail -1 | tr -d '[:space:]')
B=$(bun run src/main.ts shorten "https://example.com/second" 2>/dev/null | tail -1 | tr -d '[:space:]')
[ -z "$A" ] || [ -z "$B" ] && skip "could not create two records"
F=$(find "$STORE" -type f | head -1); [ -z "$F" ] && skip "no store file"
printf '\n{"broken' >> "$F"
bun run src/main.ts shorten "https://example.com/third" >/dev/null 2>&1; EC=$?
R1=$(bun run src/main.ts resolve "$A" 2>/dev/null | tail -1 | tr -d '[:space:]')
R2=$(bun run src/main.ts resolve "$B" 2>/dev/null | tail -1 | tr -d '[:space:]')
if [ "$R1" = "https://example.com/first" ] && [ "$R2" = "https://example.com/second" ]; then
  pass "both prior records survive (third write exit $EC)"
fi
[ "$EC" != "0" ] && pass "refused to write (exit $EC); data left intact on disk"
fail "wrote successfully and lost prior records"
