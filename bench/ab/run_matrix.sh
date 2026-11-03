#!/usr/bin/env bash
# Run every task x arm x model x run, in a random order, then write the report.
#
#   MODELS="<model id> <model id> ..." ARMS="A B C" RUNS=5 JOBS=6 bench/ab/run_matrix.sh
#
# Every run is recorded, including failures, timeouts and voided runs.
set -u
cd "$(dirname "$0")"
./selftest.sh || { echo "refusing to run: the rubric failed its own self-test"; exit 2; }
MODELS="${MODELS:?set MODELS to the model ids to compare, space-separated}"
ARMS="${ARMS:-A B C}"
RUNS="${RUNS:-5}"
TIMEOUT="${TIMEOUT:-900}"
FIX="${HEXA_AB_FIXTURES:-../../docs/benchmarks/fixtures}"
TASKS="${TASKS:-$(ls "$FIX" | sed 's/\.json$//')}"
mkdir -p runs
{
  echo "hexa_commit=$(git -C ../.. rev-parse HEAD 2>/dev/null)"
  echo "claude_cli=$(${HEXA_AB_REAL_CLAUDE:-/opt/node22/bin/claude} --version 2>/dev/null)"
  echo "lizard=$(python3 -m lizard --version 2>/dev/null)"
  echo "bun=$(bun --version 2>/dev/null)"
  echo "models=$MODELS arms=$ARMS runs=$RUNS timeout=$TIMEOUT jobs=${JOBS:-1}"
  echo "started=$(date -u +%FT%TZ)"
} > runs/MANIFEST.txt
plan=()
for m in $MODELS; do for a in $ARMS; do for t in $TASKS; do for r in $(seq 1 "$RUNS"); do
  plan+=("$m $a $t $r")
done; done; done; done
JOBS="${JOBS:-1}"
# Each run has its own workspace and git repository, so runs are independent.
printf '%s\n' "${plan[@]}" | shuf | xargs -P "$JOBS" -L 1 sh -c \
  'python3 -I run_one.py --model "$0" --arm "$1" --task "$2" --run "$3" --timeout "'"$TIMEOUT"'"' 
python3 -I report.py runs --json runs/evidence.json --md runs/EVIDENCE.md >/dev/null
echo "report: runs/EVIDENCE.md, runs/evidence.json"
