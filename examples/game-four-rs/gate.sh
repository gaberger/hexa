#!/usr/bin/env bash
# The gate. It must exit 0. Written before the code, and not derived from it.
set -euo pipefail
cd "$(dirname "$0")"
BIN=./target/release/connect-four

say() { printf '  %s\n' "$1"; }

# 1. build
cargo build --release --quiet
say "1 build ok"

# 2. tests, and at least 20 of them
SUMMARY=$(cargo test --release 2>&1 | grep -E '^test result: ok\.' || true)
[ -n "$SUMMARY" ] || { echo "FAIL: no passing test summary"; exit 1; }
PASSED=$(printf '%s\n' "$SUMMARY" | sed -E 's/^test result: ok\. ([0-9]+) passed.*/\1/' | paste -sd+ - | bc)
[ "$PASSED" -ge 20 ] || { echo "FAIL: only $PASSED tests passed, want 20+"; exit 1; }
say "2 tests ok ($PASSED passed)"

# 3. the output contract, over 100 seeds
for s in $(seq 1 100); do
  OUT=$("$BIN" --demo --seed "$s")
  N=$(printf '%s\n' "$OUT" | wc -l)
  LAST=$(printf '%s\n' "$OUT" | tail -n 1)
  case "$LAST" in
    "RED WINS"|"YELLOW WINS"|"DRAW") ;;
    *) echo "FAIL: seed $s ended with '$LAST'"; exit 1 ;;
  esac
  BODY=$(printf '%s\n' "$OUT" | head -n -1)
  SHAPED=$(printf '%s\n' "$BODY" | grep -cE '^[.RY]{7}$' || true)
  [ "$SHAPED" -eq "$((N - 1))" ] || { echo "FAIL: seed $s has a malformed line"; exit 1; }
  [ "$(((N - 1) % 6))" -eq 0 ] || { echo "FAIL: seed $s line count $N is not 6m+1"; exit 1; }
  M=$(((N - 1) / 6))
  [ "$M" -ge 7 ] || { echo "FAIL: seed $s finished in $M moves"; exit 1; }
done
say "3 output contract ok over 100 seeds"

# 4. the same seed repeats
A=$("$BIN" --demo --seed 1)
B=$("$BIN" --demo --seed 1)
[ "$A" = "$B" ] || { echo "FAIL: seed 1 is not repeatable"; exit 1; }
say "4 seed 1 repeats"

# 5. 1000 seeds, 1000 distinct games
DUPES=$(for s in $(seq 1 1000); do "$BIN" --demo --seed "$s" | cksum; done | sort | uniq -d)
[ -z "$DUPES" ] || { echo "FAIL: seeds collided"; exit 1; }
say "5 1000 seeds are distinct"

# 6. end of input never hangs
timeout 2 ./run.sh </dev/null >/dev/null 2>&1 || { echo "FAIL: run.sh did not exit 0 in 2s"; exit 1; }
say "6 run.sh exits on end of input"

# 7. run.sh is executable
[ -x run.sh ] || { echo "FAIL: run.sh is not executable"; exit 1; }
say "7 run.sh is executable"

echo "GATE PASSED"
