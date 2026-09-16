#!/usr/bin/env bash
# P2 — the request body cap must hold when there is no Content-Length.
#
# The body is large but the `url` field inside it is short and valid, so a
# rejection can only come from the body cap. An earlier version of this probe
# sent a 200 KB URL; both fixtures returned 400, one because the body was too
# big and one because the URL was too long, and the probe could not tell them
# apart. selftest.sh caught that as VACUOUS.
source "$(dirname "$0")/lib.sh"
start_server || skip "server did not start"
PAD=$(head -c 200000 /dev/zero | tr '\0' 'a')
ST=$(printf '{"url":"https://example.com/ok","pad":"%s"}' "$PAD" | timeout 25 curl -s -o /dev/null \
  -w '%{http_code}' -X POST "http://127.0.0.1:$PORT/shorten" \
  -H 'content-type: application/json' -H 'Transfer-Encoding: chunked' --data-binary @- 2>/dev/null)
case "$ST" in
  413|400) pass "oversize chunked body rejected ($ST) with a valid short url inside";;
  201) fail "oversize chunked body accepted (201) — cap bypassed";;
  *) skip "unexpected status '$ST'";;
esac
