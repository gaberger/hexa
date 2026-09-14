#!/usr/bin/env bash
# Start Minesweeper. Pass --demo to play a scripted game without a terminal.
set -euo pipefail
cd "$(dirname "$0")"
cargo build --release --quiet
exec ./target/release/minesweeper "$@"
