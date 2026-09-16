# Shared helpers for probes. Each probe is a standalone script taking a target
# directory, printing one line, and exiting 0 (clean) or 1 (defect present).
# Exit 2 means the probe could not run, which is neither a pass nor a fail.
set -uo pipefail
TARGET="${1:?usage: <probe> <target-dir>}"
PROBE_NAME="$(basename "${BASH_SOURCE[1]:-probe}" .sh)"
pass(){ echo "$PROBE_NAME PASS  $*"; exit 0; }
fail(){ echo "$PROBE_NAME DEFECT $*"; exit 1; }
skip(){ echo "$PROBE_NAME SKIP  $*"; exit 2; }
cd "$TARGET" 2>/dev/null || skip "no such directory"
[ -f src/main.ts ] || skip "no src/main.ts"
STORE="$(mktemp -d)"; export STORE_DIR="$STORE"
SRV=""
cleanup(){ [ -n "$SRV" ] && kill "$SRV" 2>/dev/null; rm -rf "$STORE"; }
trap cleanup EXIT
start_server(){
  PORT=$((20000 + RANDOM % 20000))
  bun run src/main.ts serve --port "$PORT" >/tmp/probe-$$.log 2>&1 & SRV=$!
  for _ in $(seq 1 60); do curl -s -o /dev/null "http://127.0.0.1:$PORT/_probe" && return 0; sleep 0.5; done
  return 1
}
post_url(){ curl -s -X POST "http://127.0.0.1:$PORT/shorten" -H 'content-type: application/json' --data-binary "$1" -w '\n%{http_code}'; }
code_of(){ grep -oE '"code"[[:space:]]*:[[:space:]]*"[^"]+"' | sed 's/.*"\([^"]*\)"$/\1/'; }
