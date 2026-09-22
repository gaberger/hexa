---
id: ADR-2609221830
status: accepted
date: 2026-09-22
---
# ADR-2609221830: A citation is an id in text people read

**Status:** Accepted
**Date:** 2026-09-22
**Drivers:** `hexa adr doctor` cannot see a four-digit dangling citation. The id matches on its first three digits, a guard drops the match because a digit follows, and the checker reports "registry is consistent" about a file it never read. A silent skip, in the tool that certifies the rest (ADR-2609122048). Found on `main` at `5c1a9d3` while testing two unrelated verbs by hand.

## Context

The citing-side reader matched three shapes, the last two at fixed widths: ten digits, or three. A regex alternation has no notion of "and nothing more after it", so the reader also matched the leading digits of anything longer. A guard after the match dropped any hit followed by a digit, which stopped a truncated id being *reported* — and in doing so dropped the real one with it.

The result is not a wrong answer. It is no answer:

```bash
mkdir -p /tmp/adrgap/docs/adrs /tmp/adrgap/src && cd /tmp/adrgap
printf -- '---\nid: ADR-001\nstatus: accepted\ndate: 2026-01-01\n---\n# ADR-001: x\n' \
  > docs/adrs/ADR-001-x.md
printf '// cites a four-digit id that has no file\npub fn f() {}\n' > src/lib.rs
hexa adr doctor        # "No findings — registry is consistent"
```

Widening the pattern is one line. It was tried first, and it made things worse: matching the digit run and stopping at a letter turned seven placeholder comments — an id whose final digits were never filled in, written with letters in their place — into fabricated dangling citations to a decision nobody had ever cited. That is a checker inventing work, which is worse than a checker missing some.

With the placeholder case handled, widening surfaced **60 findings**. Nearly all were synthetic ids inside test fixtures: a unit test writing a short id to prove the parser refuses it. Excluding test code brought it to **9**, and all nine were prose — comments and documentation that *discuss* an id rather than cite a decision. Two of the nine were inside the checker's own source, in comments explaining this very behaviour.

That is the real question, and it is not about regular expressions: **what makes an id a citation?** The session that found this hit the answer five times by accident, once in each direction — a page naming the ids it said were unresolved, a test fixture, a correction to that page, a comment in the checker, and a warning about the trap that tripped the trap.

## Decision

1. **An id written in text people read is a citation.** Documentation, comments, configuration and `CODEOWNERS` all count. If an id appears where a reader would follow it, it must resolve.

2. **Test code is not text people read for decisions.** A fixture is data for an assertion, not a claim that a decision exists. `tests/` directories and `#[cfg(test)]` modules are excluded from citation scanning. This is the line founding goal G1 already draws for provider names, applied to the same purpose.

3. **An example must not be written as an id.** A test needing an id builds it at run time; prose describing an id says what shape it has rather than spelling it. This is a constraint on writing, not on the checker, because the alternative — an escape syntax, or a list of ids that do not count — is a second registry to keep in step with the first.

4. **A match followed by any letter or digit is not an id.** This covers both the truncation and the placeholder: a longer numeric run is never cut into a shorter id, and a placeholder whose digits were never filled in is not a citation to the digits that were.

5. **A fenced code block is an example.** In Markdown, an id inside a fence is sample input, sample output, or a configuration illustration — the same category as a test fixture. Three of the sites this change surfaced were exactly that: a configuration example inside an accepted ADR, a quoted terminal transcript in a case study, and a reproduction script on the evidence page. Two of those are records that must not be edited to suit a checker; an accepted ADR is append-only (ADR-2609151930), and rewriting a transcript to make a tool pass is falsifying evidence. The rule is what changes, not the record.

6. **The pattern is one reader.** The duplicate-detection side already used a non-truncating pattern; the citing side had its own. Two patterns disagreeing about what an id is produced a checker that contradicted itself depending on which half was asked.

## Consequences

- **A four-digit dangling citation now fails the build.** It always should have. Nothing in the repository depends on it being invisible.
- **Prose can no longer use a real-looking id as an example.** Nine sites changed, including two in the checker's own source. The constraint is mildly annoying to write against and is the price of the check meaning anything.
- **Test fixtures must build their ids.** `format!("ADR-{digits}")` rather than a literal. Mechanical, and already done in this change's own gate.
- **Excluding test code is a judgement that could be wrong.** A citation genuinely belonging in a test — a comment explaining why a test exists, citing the decision that required it — is no longer checked. That is the cost, taken deliberately: the alternative is 51 findings about test data, which would make the checker useless and get it switched off.
- **The checker reports more than it did.** It was silent; it is now not. That is the direction ADR-2609122048 asks for.

## Implementation

- `hexa-cli/src/commands/adr/doctor.rs`: one pattern for both readers; the boundary guard widened from digits to alphanumerics; `tests` added to the skipped directories; `#[cfg(test)]` modules and fenced code blocks blanked out before scanning, preserving line numbers so findings still point at the right line.
- Four comments in the checker's own source and the ADR module, which used a truncated id as an example of truncation.

**Gate:** `cargo test -p hexa-cli --test an_id_is_not_a_prefix_of_another`, extended and shown failing first. The cases that must fail before: a four-digit dangling citation is reported; a placeholder id is not; a fixture in test code is not; the id named in a finding is the one that was written.

Already in place from the same session: CI runs `hexa adr doctor`, so a dangling citation fails the build rather than waiting for someone to run it.

## References

- ADR-2609122048: a tool that reports "nothing found" must prove it looked. This is that rule applied to the tool that checks the others.
- ADR-2609151930: every cited id resolves to a file — the rule this checker enforces, which it could not fully see.
