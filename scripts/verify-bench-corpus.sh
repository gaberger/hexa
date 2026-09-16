#!/usr/bin/env bash
# Every benchmark fixture must FAIL its own oracle on the code it ships.
# A fixture whose oracle already passes measures nothing: a model that edits
# nothing would score a pass. Same rule as bench/selftest.sh, applied to the
# agentic corpus (ADR-2609160100).
set -uo pipefail
DIR="${1:-docs/benchmarks/fixtures}"
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
BAD=0; N=0
for f in "$DIR"/*.json; do
  N=$((N+1)); ID=$(python3 -c "import json,sys;print(json.load(open('$f'))['id'])")
  python3 - "$f" "$T" <<'PY'
import json,sys,os
fx=json.load(open(sys.argv[1])); root=sys.argv[2]
for rel,content in fx["oracle"]["setup_files"].items():
    p=os.path.join(root,rel); os.makedirs(os.path.dirname(p),exist_ok=True)
    open(p,"w").write(content)
PY
  CMD=$(python3 -c "import json;print(json.load(open('$f'))['oracle']['command'])")
  if (cd "$T" && eval "$CMD") >/dev/null 2>&1; then
    echo "  VACUOUS  $ID — the oracle already passes on the code it ships"; BAD=$((BAD+1))
  else
    echo "  ok       $ID — oracle red on the shipped code"
  fi
done
echo
[ "$BAD" = 0 ] && { echo "CORPUS: PASS — $N fixtures, each red before the agent touches it"; exit 0; }
echo "CORPUS: FAIL — $BAD of $N fixtures cannot measure anything"; exit 1
