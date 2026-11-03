#!/usr/bin/env bash
# Is the rubric fit to judge anything? No agent, no model calls, no cost.
#  - every oracle must FAIL the untouched starting code (negative control)
#  - a known-correct answer must PASS its oracle (positive control)
# run_matrix.sh refuses to start until this passes.
set -u
cd "$(dirname "$0")"
OUT=$(mktemp -d)
FIX="${HEXA_AB_FIXTURES:-../../docs/benchmarks/fixtures}"
fail=0
HEXA_BIN="${HEXA_AB_HEXA:-../../target/release/hexa}"
for need in python3 jq bun git; do
  command -v "$need" >/dev/null || { echo "FAIL  missing tool: $need"; exit 1; }
done
[ -x "$HEXA_BIN" ] || { echo "FAIL  no hexa binary at $HEXA_BIN (cargo build -p hexa-cli --release)"; exit 1; }
python3 -m lizard --version >/dev/null 2>&1 || { echo "FAIL  lizard is not installed (pip install lizard==1.17.13)"; exit 1; }
for f in "$FIX"/*.json; do
  t=$(basename "$f" .json)
  python3 -I run_one.py --task "$t" --arm 0 --model selftest-neg --out "$OUT" >/dev/null
  if [ ! -f "$OUT/selftest-neg/0/$t--r1.json" ]; then
    echo "FAIL  $t: the run wrote no record"; fail=1
  elif jq -e '.accuracy.oracle_pass == false' "$OUT/selftest-neg/0/$t--r1.json" >/dev/null; then
    echo "ok    $t: oracle fails the untouched start"
  else
    echo "FAIL  $t: oracle PASSES the untouched start, so it proves nothing"; fail=1
  fi
done
for spec in tasks/*.json; do
  t=$(basename "$spec" .json)
  python3 -I run_one.py --task "$t" --arm 0 --model selftest-neg --out "$OUT" >/dev/null
  r="$OUT/selftest-neg/0/$t--r1.json"
  if [ ! -f "$r" ]; then
    echo "FAIL  $t: the run wrote no record"; fail=1
  elif jq -e '.accuracy.accepted_correct == false' "$r" >/dev/null; then
    echo "ok    $t: the untouched start is not accepted ($(jq -r '"oracle " + (if .accuracy.oracle_pass then "pass" else "fail" end) + ", grade " + (.shape.grade // "none")' "$r"))"
  else
    echo "FAIL  $t: the untouched start is accepted, so the task proves nothing"; fail=1
  fi
done
for g in selftest/*.good.*; do
  t=$(basename "$g"); t="${t%%.good.*}"
  python3 -I run_one.py --task "$t" --arm 0 --model selftest-pos --apply "$g" --out "$OUT" >/dev/null
  if [ ! -f "$OUT/selftest-pos/0/$t--r1.json" ]; then
    echo "FAIL  $t: the run wrote no record"; fail=1
  elif jq -e '.accuracy.accepted_correct == true' "$OUT/selftest-pos/0/$t--r1.json" >/dev/null; then
    echo "ok    $t: a correct answer is accepted"
  else
    echo "FAIL  $t: a correct answer FAILS the oracle"; fail=1
  fi
done
rm -rf "$OUT" /tmp/hexa-ab-*-0-*  # the shared cargo cache stays
[ $fail -eq 0 ] && echo "selftest: pass" || { echo "selftest: FAIL"; exit 1; }
