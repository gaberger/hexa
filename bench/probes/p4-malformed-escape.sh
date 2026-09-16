#!/usr/bin/env bash
# P4 — a malformed percent escape in the path is a 404, never a 500.
source "$(dirname "$0")/lib.sh"
start_server || skip "server did not start"
ST=$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$PORT/%")
[ "$ST" = "404" ] && pass "GET /% returns 404"
fail "GET /% returns $ST"
