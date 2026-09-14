#!/usr/bin/env bash
# Start Conway's Game of Life. Pass --demo to run without a terminal.
set -euo pipefail
cd "$(dirname "$0")"
exec cargo run --quiet -- "$@"
