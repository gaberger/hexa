#!/usr/bin/env bash
# Ground-truth gate for the URL-shortener challenge. Black box: it only runs
# the program from outside. Written before any arm started. Exit 0 = done.
set -uo pipefail
D="${1:?usage: gate.sh <dir>}"
cd "$D" || exit 2
PORT=$((20000 + RANDOM % 20000))
STORE=$(mktemp -d); export STORE_DIR="$STORE"
FAIL=0
ok(){ echo "  ok   $1"; }
no(){ echo "  FAIL $1"; FAIL=1; }
cleanup(){ [ -n "${SRV:-}" ] && kill "$SRV" 2>/dev/null; wait "${SRV:-}" 2>/dev/null; rm -rf "$STORE"; }
trap cleanup EXIT

start(){ bun run src/main.ts serve --port "$PORT" >/tmp/gate-srv.$$ 2>&1 & SRV=$!
  for _ in $(seq 1 60); do curl -s -o /dev/null "http://127.0.0.1:$PORT/_" && return 0; sleep 0.5; done
  echo "server never came up:"; tail -5 /tmp/gate-srv.$$; return 1; }

start || { no "server start"; exit 1; }

URL="https://example.com/a/very/long/path?q=1"
CODE=$(curl -sf -X POST "http://127.0.0.1:$PORT/shorten" -H 'content-type: application/json' \
  -d "{\"url\":\"$URL\"}" | grep -oE '"code"[[:space:]]*:[[:space:]]*"[^"]+"' | sed 's/.*"\([^"]*\)"$/\1/')
[ -n "$CODE" ] && ok "POST /shorten -> code $CODE" || no "POST /shorten returned no code"

ST=$(curl -s -o /dev/null -w '%{http_code}' -X POST "http://127.0.0.1:$PORT/shorten" \
  -H 'content-type: application/json' -d "{\"url\":\"$URL\"}")
[ "$ST" = "201" ] && ok "POST /shorten status 201" || no "POST /shorten status $ST, want 201"

LOC=$(curl -s -o /dev/null -w '%{redirect_url}' "http://127.0.0.1:$PORT/$CODE")
RST=$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$PORT/$CODE")
[ "$RST" = "302" ] && ok "GET /<code> status 302" || no "GET /<code> status $RST, want 302"
[ "$LOC" = "$URL" ] && ok "redirect Location matches" || no "Location '$LOC' != '$URL'"

NF=$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$PORT/zzzznope")
[ "$NF" = "404" ] && ok "GET /<unknown> 404" || no "unknown code gave $NF, want 404"

BAD=$(curl -s -o /dev/null -w '%{http_code}' -X POST "http://127.0.0.1:$PORT/shorten" \
  -H 'content-type: application/json' -d '{"url":"javascript:alert(1)"}')
[ "$BAD" = "400" ] && ok "non-http scheme 400" || no "bad scheme gave $BAD, want 400"

CLI=$(bun run src/main.ts shorten "https://cli.example/z" 2>/dev/null | tail -1 | tr -d '[:space:]')
[ -n "$CLI" ] && ok "CLI shorten -> $CLI" || no "CLI shorten printed nothing"
RES=$(bun run src/main.ts resolve "$CLI" 2>/dev/null | tail -1 | tr -d '[:space:]')
[ "$RES" = "https://cli.example/z" ] && ok "CLI resolve round-trips" || no "CLI resolve gave '$RES'"

bun run src/main.ts resolve zzzznope >/tmp/gate-un.$$ 2>/dev/null
UEC=$?; USOUT=$(cat /tmp/gate-un.$$); rm -f /tmp/gate-un.$$
{ [ "$UEC" != "0" ] && [ -z "$USOUT" ]; } && ok "CLI resolve unknown exits nonzero, silent" \
  || no "CLI resolve unknown exit=$UEC stdout='$USOUT'"

# Restart: same STORE_DIR, new process. The code must still resolve.
kill "$SRV" 2>/dev/null; wait "$SRV" 2>/dev/null; SRV=""
start || { no "server restart"; exit 1; }
R2=$(curl -s -o /dev/null -w '%{redirect_url}' "http://127.0.0.1:$PORT/$CODE")
[ "$R2" = "$URL" ] && ok "code survives restart" || no "after restart Location '$R2'"
CLI2=$(bun run src/main.ts resolve "$CLI" 2>/dev/null | tail -1 | tr -d '[:space:]')
[ "$CLI2" = "https://cli.example/z" ] && ok "CLI code survives restart" || no "after restart CLI gave '$CLI2'"

[ "$FAIL" = "0" ] && echo "GATE: PASS" || echo "GATE: FAIL"
exit $FAIL
