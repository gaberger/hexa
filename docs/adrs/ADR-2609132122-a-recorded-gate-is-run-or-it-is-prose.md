# ADR-2609132122: A recorded gate is run, or it is prose

**Status:** Accepted
**Date:** 2026-09-13
**Drivers:** Eighteen of this repository's twenty-two ADRs record a gate. Running all eighteen for the first time, seventeen passed and one could not run at all: `cargo test -p hexa-infer serves -p hexa-cli tier_coverage` is not valid cargo — a test filter cannot follow two `-p` flags. That gate was written into an accepted ADR and has never once been executed.

## Context

The project's rule is that the executable gate replaces the written spec, and that a spec which
cannot be run is an ADR rather than a gate. ADR-2609132059 made `hexa loop stage done` run the gate
of the work in hand. Nothing has ever run the gate of a *finished* decision.

So a gate is checked once, on the day it is written, by whoever is at the keyboard — and after that
it is a sentence in a document. The one that could not run was never checked even then: it was
typed, and the loop recorded a corrected version, and the ADR kept the broken one.

Eighteen recorded gates are a regression suite that nobody was running. Seventeen of them still
pass, which is worth knowing, and is exactly the claim nobody could have made before now.

## Decision

1. **`hexa adr gates` runs the gate of every ADR that records one**, extracting the first
   backquoted command under `## Gate`, and reports each as passed, failed, or unrunnable.

2. **Three outcomes, again.** Passed and failed are the command's exit status. Unrunnable is a gate
   that is not a command this can execute — prose, a placeholder, or invalid syntax — and it is
   reported as such rather than counted either way, because "we could not run it" and "it failed"
   send a reader to different places.

3. **A failing or unrunnable gate fails the command.** A decision whose gate no longer passes is
   either a decision that was reverted, in which case the ADR is wrong, or a regression, in which
   case the code is. Both need someone; neither is a warning.

4. **The ADR whose gate never ran is corrected, not excused.** `cargo test -p hexa-infer serves &&
   cargo test -p hexa-cli tier_coverage`, which is what the loop actually recorded and what has
   been passing all along.

## Consequences

- The decision record becomes a regression suite. Eighteen gates is a slow command, which is why it
  is a verb someone runs rather than a hook.
- A gate that names a moving target — a test since renamed — now fails loudly instead of quietly
  describing something that no longer exists.
- Only the first backquoted command under the heading is taken. An ADR whose gate is two commands
  in prose is reported unrunnable, which is a fair description of a gate nobody can execute.
- This does not check that a gate covers its ADR's decisions. Twice today a numbered decision went
  unimplemented while its gate passed, and that gap stays open; this is the smaller, mechanical
  half.

## Gate

`cargo test -p hexa-cli adr_gates`: a gate section yields its first backquoted command; an ADR with
no gate section yields none; prose with no command is unrunnable; and the outcomes are counted so
that any failure or unrunnable gate fails the command.

## Evidence

`hexa adr gates 2>&1 | tail -2` at 528629f with uncommitted changes on 2026-09-13 21:25 UTC:

```text

  19 passed · 0 failed · 0 unrunnable · 4 without a gate
```
