#!/usr/bin/env bash
# P1 — a URL containing a code point above U+00FF must not be accepted and then
# fail forever on redirect. Either reject it up front, or serve it correctly.
source "$(dirname "$0")/lib.sh"
start_server || skip "server did not start"
R=$(post_url '{"url":"https://example.com/café/日本"}')
ST=$(echo "$R" | tail -1); C=$(echo "$R" | head -1 | code_of)
[ "$ST" = "400" ] && pass "rejected up front (400)"
[ -z "$C" ] && fail "status $ST with no code"
G=$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$PORT/$C")
[ "$G" = "302" ] && pass "accepted and resolves (302)"
fail "accepted $ST then resolve gives $G — code permanently dead"
