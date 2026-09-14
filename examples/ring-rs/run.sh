#!/usr/bin/env bash
# Show the ring buffer at work. This crate is a library, so this runs the demo.
set -euo pipefail
cd "$(dirname "$0")"
exec cargo run --quiet --example demo "$@"
