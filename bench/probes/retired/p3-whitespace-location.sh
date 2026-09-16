#!/usr/bin/env bash
# P3 — a URL with surrounding whitespace must not put whitespace in Location.
#
# Reads the raw response header rather than curl's %{redirect_url}, which
# normalises the value and hid the defect. selftest.sh caught that as VACUOUS.
source "$(dirname "$0")/lib.sh"
start_server || skip "server did not start"
R=$(post_url '{"url":"  https://example.com/spaced  "}')
ST=$(echo "$R" | tail -1); C=$(echo "$R" | head -1 | code_of)
[ "$ST" = "400" ] && pass "rejected (400)"
[ -z "$C" ] && skip "status $ST with no code"
RAW=$(curl -s -o /dev/null -D - "http://127.0.0.1:$PORT/$C" | grep -i '^location:' | sed 's/^[Ll]ocation:[[:space:]]*//' | tr -d '\r')
case "$RAW" in
  "https://example.com/spaced") pass "Location header is exact";;
  *" "*) fail "Location header carries whitespace: '$RAW'";;
  "") skip "no Location header";;
  *) fail "Location header is '$RAW'";;
esac
