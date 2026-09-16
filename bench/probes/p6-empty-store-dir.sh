#!/usr/bin/env bash
# P6 — an empty STORE_DIR must not silently write into the working directory.
source "$(dirname "$0")/lib.sh"
T=$(mktemp -d); cp -r . "$T/app" >/dev/null 2>&1; cd "$T/app" || skip "copy failed"
BEFORE=$(find . -maxdepth 1 -type f | wc -l)
STORE_DIR="" timeout 25 bun run src/main.ts shorten "https://example.com/x" >/dev/null 2>&1
AFTER=$(find . -maxdepth 1 -type f | wc -l)
cd /; rm -rf "$T"
[ "$AFTER" -le "$BEFORE" ] && pass "nothing written to the working directory"
fail "wrote $((AFTER-BEFORE)) file(s) into the working directory"
