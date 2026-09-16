# Retired probes

A probe lives here when `selftest.sh` proved it cannot discriminate and the
reason is not fixable. Keeping it in `probes/` would add a column that is
always green, which is the vacuous-gate failure the bench exists to prevent.

## p3-whitespace-location

**Retired 2026-09-16. Reason: the defect is unreachable on this runtime.**

The probe asserts that a URL submitted with surrounding whitespace does not put
whitespace into the `Location` response header. The unsound fixture carries the
defect in its own code: it validates with `new URL()` but stores and returns the
untrimmed string.

It still cannot be observed. Bun normalises header values when the `Response` is
constructed, so the wire bytes are already trimmed:

```
Location: https://example.com/spaced^M$
```

Two versions were tried. The first read `curl`'s `%{redirect_url}`, which
normalises. The second read the raw header with `curl -D -`, which showed the
runtime had already stripped it. There is no third reading: the platform
prevents the defect.

This does not mean the invariant is unimportant, only that on Bun it is
enforced below the application. A port of this bench to a runtime that passes
header values through unchanged should restore the probe and re-run
`selftest.sh` to confirm it discriminates there.

No published result changes: the probe reported clean for all three trial arms,
which is consistent with the defect being unreachable rather than absent.
