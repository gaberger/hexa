#!/usr/bin/env bash
# selftest.sh — test the rubric, not the subject.
#
# Every probe must PASS against fixtures/sound and report DEFECT against
# fixtures/unsound. A probe that passes both is vacuous: it cannot fail, so its
# green means nothing. A probe that fails both is broken. Either way the bench
# is not fit to judge anything and this script exits non-zero.
#
# This is the same rule the project applies to gates (`evidence_is_vacuous`),
# applied to the measuring instrument. A bench without it is speculation with a
# table. ADR-2609160100.
set -uo pipefail
cd "$(dirname "$0")"
SOUND=fixtures/sound; UNSOUND=fixtures/unsound
BAD=0; N=0
printf '%-28s %-22s %-22s %s\n' PROBE "ON SOUND" "ON UNSOUND" VERDICT
printf '%s\n' "------------------------------------------------------------------------------------"
for p in probes/p*.sh; do
  N=$((N+1)); name=$(basename "$p" .sh)
  "$p" "$SOUND"   >/tmp/st-s.$$ 2>&1; S=$?
  "$p" "$UNSOUND" >/tmp/st-u.$$ 2>&1; U=$?
  word(){ case $1 in 0) echo pass;; 1) echo DEFECT;; *) echo "skip($1)";; esac; }
  if [ "$S" = "0" ] && [ "$U" = "1" ]; then V="ok — discriminates"
  elif [ "$S" = "0" ] && [ "$U" = "0" ]; then V="VACUOUS — cannot fail"; BAD=$((BAD+1))
  elif [ "$S" != "0" ]; then V="BROKEN — fails known-good"; BAD=$((BAD+1))
  else V="UNUSABLE"; BAD=$((BAD+1)); fi
  printf '%-28s %-22s %-22s %s\n' "$name" "$(word $S)" "$(word $U)" "$V"
  [ "$BAD" != "0" ] && { [ "$S" != "0" ] && sed 's/^/    sound: /' /tmp/st-s.$$ | head -2; }
done
rm -f /tmp/st-s.$$ /tmp/st-u.$$
echo
# The gate must be green on BOTH fixtures. That is the point of the bench: the
# defects the probes catch all hide behind a passing behavioural gate.
if [ -x ../bench/gate.sh ]; then
  for f in "$SOUND" "$UNSOUND"; do
    ../bench/gate.sh "$f" >/tmp/st-g.$$ 2>&1 && G=PASS || G=FAIL
    echo "behavioural gate on $(basename "$f"): $G"
    [ "$G" = "FAIL" ] && { sed 's/^/    /' /tmp/st-g.$$ | tail -3; BAD=$((BAD+1)); }
  done; rm -f /tmp/st-g.$$
fi
echo
if [ "$BAD" = "0" ]; then echo "SELFTEST: PASS — $N probes, each proven to discriminate"; exit 0; fi
echo "SELFTEST: FAIL — $BAD of $N probes are not fit to judge anything"; exit 1
