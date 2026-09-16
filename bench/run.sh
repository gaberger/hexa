#!/usr/bin/env bash
# run.sh <dir> [dir...] — run every probe against every target, print a matrix.
# Refuses to run until selftest.sh passes, so no result is ever produced by an
# instrument that has not been shown to discriminate.
set -uo pipefail
cd "$(dirname "$0")"
if [ "${SKIP_SELFTEST:-0}" != "1" ]; then
  ./selftest.sh >/tmp/run-st.$$ 2>&1 || { echo "refusing to run: the bench failed its own self-test"; cat /tmp/run-st.$$; rm -f /tmp/run-st.$$; exit 2; }
  rm -f /tmp/run-st.$$
fi
[ $# -ge 1 ] || { echo "usage: run.sh <dir> [dir...]"; exit 2; }
PROBES=(probes/p*.sh)
printf '%-28s' "PROBE"; for t in "$@"; do printf '%-16s' "$(basename "$t")"; done; echo
printf '%s\n' "$(printf '%.0s-' $(seq 1 $((28 + 16 * $#))))"
declare -A TOT
for p in "${PROBES[@]}"; do
  printf '%-28s' "$(basename "$p" .sh)"
  for t in "$@"; do
    "$p" "$t" >/dev/null 2>&1; rc=$?
    case $rc in 0) printf '%-16s' "ok";; 1) printf '%-16s' "DEFECT"; TOT[$t]=$(( ${TOT[$t]:-0} + 1 ));; *) printf '%-16s' "skip";; esac
  done; echo
done
printf '%s\n' "$(printf '%.0s-' $(seq 1 $((28 + 16 * $#))))"
printf '%-28s' "defects"; for t in "$@"; do printf '%-16s' "${TOT[$t]:-0}"; done; echo
