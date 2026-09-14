#!/usr/bin/env bash
# The gate. It must exit 0. Written before the code, and not derived from it.
set -euo pipefail
set -o pipefail
cd "$(dirname "$0")"

say() { printf '  %s\n' "$1"; }

# 1. Everything passes with the test-only accessors on.
cargo test -p ringbuf --features internals --lib --tests --quiet
say "1 tests pass with --features internals"

# 2. At least 15 tests really exist and really run, counted with the same
#    features as the run. `cargo test` also exits 0 when it runs zero tests.
COUNT=$(cargo test -p ringbuf --features internals --lib --tests -- --list | grep -c ': test$')
[ "$COUNT" -ge 15 ] || { echo "FAIL: only $COUNT tests exist, want 15+"; exit 1; }
say "2 test count is $COUNT"

# 3. The crate also works for a normal caller, with no extra features.
cargo test -p ringbuf --quiet
say "3 tests pass with no extra features"

# 4. The demo starts and finishes.
./run.sh >/dev/null
say "4 run.sh starts and exits 0"

echo "GATE PASSED"
