#!/usr/bin/env bash
# Start 2048. Pass --demo to play a fixed game without a terminal.
set -euo pipefail
cd "$(dirname "$0")"
if command -v bun >/dev/null 2>&1; then
  exec bun src/main.ts "$@"
fi
tsc
exec node dist/main.js "$@"
