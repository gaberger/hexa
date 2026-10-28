#!/bin/sh
# Build and run habit-streak; arguments go to the tool.
#   ./run.sh add Read
#   ./run.sh check Read
#   ./run.sh status
set -e
cd "$(dirname "$0")"
cargo run --quiet -- "$@"
