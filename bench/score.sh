#!/usr/bin/env bash
# score.sh <dir> [dir...] — the full scorecard for one or more implementations.
#
# Runs, in order, everything the trial measured by hand:
#   1. selftest      — is the probe battery fit to judge anything at all?
#   2. gate          — does the implementation do what was asked?
#   3. probes        — what defects survive a green gate?
#   4. architecture  — hexa analyze, if the binary is on PATH
#   5. blast radius  — diff against a base commit, if the dir is a git repo
#                      and BASE=<rev> is set
#
# Exits non-zero if the self-test fails, so no scorecard is ever produced by an
# instrument that has not been shown to discriminate (ADR-2609160100).
set -uo pipefail
cd "$(dirname "$0")"
BENCH="$PWD"
[ $# -ge 1 ] || { echo "usage: score.sh <dir> [dir...]    (optional: BASE=<rev>)"; exit 2; }

echo "== 1. the instrument"
./selftest.sh >/tmp/score-st.$$ 2>&1
if [ $? -ne 0 ]; then
  echo "   REFUSING TO SCORE — the probe battery failed its own self-test:"
  sed 's/^/   /' /tmp/score-st.$$; rm -f /tmp/score-st.$$; exit 2
fi
tail -1 /tmp/score-st.$$ | sed 's/^/   /'; rm -f /tmp/score-st.$$

for T in "$@"; do
  A="$(cd "$T" 2>/dev/null && pwd)" || { echo; echo "== $T: no such directory"; continue; }
  NAME="$(basename "$A")"
  echo
  echo "== $NAME  ($A)"

  printf '   %-18s ' "gate"
  if "$BENCH/gate.sh" "$A" >/tmp/score-g.$$ 2>&1; then echo "PASS"; else
    echo "FAIL"; grep -E '^  FAIL' /tmp/score-g.$$ | sed 's/^/     /' | head -4
  fi
  rm -f /tmp/score-g.$$

  DEF=0; SKIP=0
  for p in probes/p*.sh; do
    "$p" "$A" >/tmp/score-p.$$ 2>&1; rc=$?
    case $rc in
      1) DEF=$((DEF+1)); printf '   %-18s %s\n' "defect" "$(sed 's/^[^ ]* *DEFECT *//' /tmp/score-p.$$ | head -1)";;
      2) SKIP=$((SKIP+1));;
    esac
    rm -f /tmp/score-p.$$
  done
  printf '   %-18s %d defect(s) over %d probe(s)%s\n' "probes" "$DEF" "$(ls probes/p*.sh | wc -l)" \
    "$([ $SKIP -gt 0 ] && echo ", $SKIP skipped")"

  if command -v hexa >/dev/null 2>&1; then
    G=$( (cd "$A" && hexa analyze . 2>/dev/null) | grep -oE 'grade: [A-F+]+ — score [0-9]+' | head -1)
    printf '   %-18s %s\n' "architecture" "${G:-not gradeable}"
  fi

  if [ -n "${BASE:-}" ] && git -C "$A" rev-parse --verify "$BASE" >/dev/null 2>&1; then
    STAT=$(git -C "$A" diff --shortstat "$BASE"..HEAD -- src 2>/dev/null | sed 's/^ *//')
    DOM=$(git -C "$A" diff --name-only "$BASE"..HEAD -- src/domain 2>/dev/null | wc -l)
    OUT=$(git -C "$A" diff --name-only "$BASE"..HEAD -- src 2>/dev/null \
          | grep -v '^src/adapters/secondary/' | grep -v '^src/main.ts' | wc -l)
    printf '   %-18s %s\n' "blast radius" "${STAT:-no change}"
    printf '   %-18s %s\n' "domain files" "$DOM $([ "$DOM" = 0 ] && echo '(untouched)')"
    printf '   %-18s %s\n' "outside adapter" "$OUT file(s)"
  fi
done
