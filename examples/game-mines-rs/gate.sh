#!/usr/bin/env bash
# The gate. It was written before the code it checks, and it is not derived
# from it. It pins both endings, so neither branch can rot.
#
# `pipefail` is on, so a failure inside a pipe is never swallowed.
set -euo pipefail
cd "$(dirname "$0")"

# 1. The tests run, and there are many of them. "0 tests" is a failed gate.
#    Cargo prints one result line per test binary, so the counts are added up.
total=$(cargo test --quiet 2>&1 | tee /dev/stderr \
  | awk '/^test result: ok\./ { sum += $4 } END { print sum + 0 }')
echo "gate: $total tests passed"
[ "$total" -ge 30 ]

# 2. The thinking player wins. Deleting the win logic breaks this line.
cargo run --release --quiet -- --demo --seed 1 | tail -1 | grep -qx 'YOU WIN'

# 3. The reckless player loses. It opens cells in order, so it must hit a mine.
cargo run --release --quiet -- --demo --seed 1 --policy reckless | tail -1 | grep -qx 'GAME OVER'

# 4. The seed decides the board. Two seeds must print two different fingerprints.
a=$(cargo run --release --quiet -- --demo --seed 1 | head -1)
b=$(cargo run --release --quiet -- --demo --seed 2 | head -1)
[ "$a" != "$b" ]

# 5. The architecture holds. `hexa analyze` exits 0 only at A+.
#    The output is held in a variable first. `grep -q` closes its input as soon
#    as it matches, and `hexa analyze` aborts on a closed pipe rather than
#    ending quietly. Reading it whole keeps this step about the grade.
grade=$(hexa analyze .)
printf '%s\n' "$grade" | grep -q 'grade: A+'

echo "gate: green"
