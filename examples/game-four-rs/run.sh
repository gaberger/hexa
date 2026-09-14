#!/usr/bin/env bash
# Start Connect Four. Pass --demo --seed <n> to play a scripted game.
set -euo pipefail
cd "$(dirname "$0")"
cargo build --release --quiet
exec ./target/release/connect-four "$@"
