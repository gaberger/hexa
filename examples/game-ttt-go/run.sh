#!/usr/bin/env bash
# Start tic-tac-toe. Pass -demo to run without a terminal.
set -euo pipefail
cd "$(dirname "$0")"
exec go run ./cmd/ttt "$@"
