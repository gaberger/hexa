# ADR-2609131822: A local backend is asked one question at a time

**Status:** Accepted
**Date:** 2026-09-13
**Amends:** ADR-2609131702, whose fallback stands; this fixes how it calls.
**Drivers:** The first real fallback run: `1 of 4 lenses answered`, the other three failing with `error sending request` at 5m02s. The client timeout is 300 seconds, so all three had simply not finished. A trivial request to the same gateway, measured alone, takes 24.8 seconds.

## Context

The hunt fans its lenses out in parallel, and so does verify. That is right for the frontier: four
agents, four separate processes, work that genuinely overlaps.

It is wrong for a local backend. One model on one machine serves one request at a time; four
arriving together do not go faster, they queue inside the server, and the three at the back of the
queue spend their whole 300-second budget waiting rather than working. The result is not a slow
review, it is a review that loses three quarters of its lenses and reports the loss honestly —
which is the previous ADR working, on top of a mistake this one removes.

The fan-out also gets worse exactly when it is least affordable: a bigger target means a bigger
prompt for every lens at once.

## Decision

1. **Calls to the local reviewer are serialised.** One at a time, process-wide, however many lenses
   or claims are in flight. The frontier keeps its parallelism; only the fallback queues, and it
   queues in the client where the wait is visible rather than in the server where it is not.

2. **The phase says it is queuing.** A lens waiting for the local reviewer is not a lens that has
   stalled, and the heartbeat already distinguishes them; the note names which.

## Consequences

- A four-lens hunt against a local model takes about four times one call instead of failing three.
  On this machine that is minutes, and minutes of working is better than five of waiting.
- A local review is now bounded by the model's speed rather than by a timeout, which is the honest
  constraint and the one worth optimising against.
- The frontier path is unchanged in every respect.

## Gate

`cargo test -p hexa-exec serialised`: two local calls issued together do not overlap, and the
frontier path is not serialised by the same permit.
