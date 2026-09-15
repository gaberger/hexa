# Investigation: are ADRs the immutable memory of design decisions?

**Date:** 2026-09-15
**Question:** hexa should manage ADRs as an immutable, curated memory of design
decisions, linked to `hexa memory`. How far is that from true today?
**Method:** read the verbs, the store, the git history, and every path by which
an ADR or a memory entry reaches an agent. Every number below has the command
that produced it.

## Short answer

ADRs are neither immutable nor linked to memory, and most of what the code
cites as a decision no longer exists. Memory has the right storage model and
almost no content. The two systems share no key, no reference, and no reader.

## 1. The ledger is mostly dangling pointers

| | |
|---|---|
| ADR files in `docs/adrs/` | 27 |
| Oldest ADR file | 2026-09-12 |
| Commits in this repository | 85, first on 2026-09-12 |
| Distinct ADR ids cited in code and docs | 140 |
| Cited ids with no file | **113** |

```bash
grep -rhoE "ADR-([0-9]{3}|[0-9]{4}-[0-9]{2}-[0-9]{2}-[0-9]{4}|[0-9]{10})" \
  --include='*.rs' --include='*.md' --include='*.toml' hexa-* docs README.md CLAUDE.md | sort -u
```

The most-cited decisions are all missing: ADR-001 (39 citations), ADR-2026-04-15-0100
(38), ADR-2608241500 (27), ADR-2026-04-27-0800 (25), ADR-047 (16). `CODEOWNERS`
justifies the founding-goals rule by ADR-2026-04-26-1500, which does not exist.
Three id schemes are in use across citations: three-digit, dated, and timestamp.
Only the timestamp scheme has files.

The history before 2026-09-12 was not carried into this repository. No ADR was
ever deleted or renamed here; they were never present. `hexa adr doctor` reports
"registry is consistent" because it checks the files that exist against each
other, never a citation against the ledger. A reader following a citation from
`direct_exec.rs` to "ADR-2026-06-04-1740 Path A" finds nothing, and no verb
tells them so.

**This is the actual state of the design memory: four in five references point
at a decision whose text is gone.**

## 2. ADRs are not immutable, and nothing says they should be

Three code paths rewrite an ADR file in place:

- `hexa adr accept | complete | supersede` rewrite the `Status:` line and add
  `Superseded-By:` (`hexa-cli/src/commands/adr/mod.rs:225`).
- `hexa docs migrate-adr --apply` prepends YAML frontmatter.
- `hexa adr doctor` can patch an ADR's body in a worktree as a proposed fix.

Six of the 27 files have been modified after their creating commit; four of
those were today. Nothing distinguishes a status change from an edit to
`## Decision`. There is no hook, test, hash, or CI check that an Accepted ADR's
body is what was accepted. The glossary's immutability rule covers exactly one
file, `founding-goals.md`, and it is enforced only by CODEOWNERS on a remote
that does not require reviews.

Contrast the memory store. `~/.hexa/memory.jsonl` is append-only: a rewrite is
a newer line, a delete is a tombstone, and the reader takes the newest
(`hexa-exec/src/local_store.rs:148`). That is the model the ADR ledger needs,
and it already exists in the other store.

## 3. The machine-readable surface of an ADR is thin

| Surface | Count | Verb that reads it |
|---|---|---|
| `## Gate` command | 21 of 27 | `hexa adr gates` |
| `Applies-To` path scope | **0 of 27** | `hexa adr governing` (prints "backfill needed") |
| `.hexa/ADR-rules.toml` rule citing an ADR | 1 | `hexa analyze` |
| YAML frontmatter | 2 of 27 | `hexa docs check` (25 warnings) |

The gate suite is the one surface with weight, and it is not reliable:

| Run | Result |
|---|---|
| system `cargo` 1.75 on PATH | 0 passed · **21 failed** |
| rustup `cargo` 1.98 on PATH | 20 passed · 1 failed (ADR-2609140020) |
| same, rerun | 20 passed · 1 failed (ADR-2609132158) |

The first row is every cargo gate failing on "lock file version 4 requires
-Znext-lockfile-bump", reported as "gate failed" with no reason. The gate that
failed in row two passed by hand seconds later. The gate that failed in row
three is a `curl` to the GitHub API, which was rate-limiting this address.
`run_evidence` returns only exit status and text; the runner cannot tell "the
decision is broken" from "this machine cannot run the check." The suite that
is supposed to be the decision record as a regression suite gives a different
answer on each run and never says why.

## 4. What actually reaches an agent

**ADRs.** At session start the fingerprint lists the three newest ADR files by
filename, with a one-line summary
(`hexa-analysis/src/fingerprint_extractor.rs:453`). Newest by name, not by
relevance, not by status. A Proposed ADR outranks an Accepted one if its
timestamp is later. Beyond those three lines, no ADR text reaches any agent:
the do-loop's tool allowlist (`direct_react.rs:40`) has no ADR reader, and the
`adr_draft` tool is explicitly excluded.

**Memory.** Both executor paths call `gather_context`, which loads every memory
entry (cap 200) and ranks the top six by code-graph relevance to the task
(`direct_exec.rs:736`). This is the one live link between memory and an agent.
Two weaknesses: it does not filter by key prefix, so `loop:` and
`workplan:active:` JSON blobs compete with lessons for the six slots; and it is
read-only at task start. The `memory_search` tool exists and is registered, but
the do-loop does not allow it, and the SOP GROUND phase it was written for was
removed in ADR-2608241500. An agent mid-task cannot ask what is already known.

## 5. What memory holds

```
lines 8 · lesson: 2 · loop: 6 · first 2026-09-12 · last 2026-09-13
```

Two lessons, both from a different project's session, and six loop-state
writes from a version of `hexa loop` that has since moved its state to
`.hexa/loop.json`. Keys are ad-hoc namespaces: `lesson:`, `gap:`, `loop:`,
`workplan:active:`, `restart:checkpoint:`. Two of those are read by hooks;
none is documented; none is validated on write.

No memory entry names an ADR. No ADR names a memory entry. No code path in
`commands/adr/` touches the memory store, and none in `commands/memory/`
touches an ADR. The two ADRs written today record their gate and their commit,
and the two lessons in memory record the bug they came from, and there is no
way to get from one to the other.

## 6. Verdict

- **Immutable:** no. In-place status rewrites, no integrity check, four
  post-creation edits today alone.
- **Curated:** partially. The index, the doctor, and the gate suite exist. The
  doctor does not check citations; the gate suite is environment-sensitive and
  silent about why it fails.
- **Memory of design decisions:** 27 of 140 cited decisions have text. The
  rest are ids in comments.
- **Linked to `hexa memory`:** not at all. Zero cross-references in either
  direction.

The one thing working as intended is the lesson ranking in `gather_context`,
and it has two lessons to rank.

## 7. What to build, in order

ADR-2609151930 proposes the design. The order is by what closes the largest
gap for the least change:

1. **A citation check in `hexa adr doctor`.** Every `ADR-…` id in code and docs
   must resolve to a file, or the doctor fails. The 113 orphans then get one of
   two treatments: a stub ADR that records "history not carried; decision
   summarised here" with the summary recovered from the citing comment, or the
   citation is rewritten to the ADR that now governs. Either way the reader
   lands somewhere.
2. **Content hashes for Accepted ADRs.** `hexa adr accept` records the body
   hash below the frontmatter in a ledger line; the doctor and CI fail if the
   body no longer matches. Status changes become appended ledger events, not
   file rewrites. The file stays readable; the ledger is the truth.
3. **The link.** `hexa adr accept` writes `adr:<id>` into memory with title,
   one-line decision, gate, and commit. `hexa memory store` accepts `--adr <id>`
   and refuses a `lesson:` without provenance. `gather_context` filters on
   `lesson:` and `adr:` prefixes and ranks both.
4. **Applies-To backfill and a session-start selection by scope**, so the three
   ADRs a session sees are the ones governing the files it will touch, not the
   three newest.
5. **A gate runner that names its failure.** Prefix PATH the way `release.sh`
   does, classify "cargo unusable" and "network unreachable" as unrunnable
   here, and print the last lines of output for a real failure.

Item 1 is a day's work and changes what every citation in the codebase means.
Item 5 is an hour and makes the gate suite trustworthy. Items 2 and 3 are the
ADR's substance.
