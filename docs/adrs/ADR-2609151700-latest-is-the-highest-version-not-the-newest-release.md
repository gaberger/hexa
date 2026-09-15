---
id: ADR-2609151700
status: accepted
date: 2026-09-15
supersedes: []
superseded_by: null
depends_on: []
components: []
modules: []
---

# ADR-2609151700: Latest is the highest version, not the newest release

**Status:** Accepted
**Date:** 2026-09-15
**Drivers:** Tags v26.9.10 and v26.9.11 were pushed in one command. Both release workflows ran; the v26.9.10 one finished second and became GitHub's "latest". `hexa self-update` on any machine would have installed v26.9.10 over v26.9.11.

## Context

`hexa self-update` resolved its target by asking GitHub for
`/releases/latest`. That endpoint returns the most recently *created*
non-draft, non-prerelease release. It says nothing about version order.

The release workflow is triggered by any `v*.*.*` tag push. When two tags
land together, two runs execute concurrently and finish in whatever order the
runners allow. The one that creates its release last takes the "latest" flag.
On 2026-09-15 that was the older tag, and the update path would have carried
every machine backwards while printing "Update available: 26.9.11 → 26.9.10"
as if that were progress.

Nothing failed. The endpoint answered, the tag parsed, the comparison was a
string inequality, and the downgrade would have installed cleanly.

## Decision

1. self-update lists releases and picks the highest parsed `vX.Y.Z` among
   those that are neither draft nor prerelease. GitHub's flag is not consulted.
   `parse_version` and `newest_release_tag` in
   `hexa-cli/src/commands/update.rs` are the pure functions that decide.
2. An unnamed update never moves backwards. If the installed version is higher
   than the newest published release, self-update says so and stops.
   `--version <tag>` still installs exactly what was named, because a
   deliberate rollback is a decision, not an accident.
3. The release workflow passes `make_latest: true` for stable releases, so the
   flag is set explicitly rather than won by finishing order. This protects
   binaries older than this ADR, which still read `/releases/latest`.

## Consequences

- Version order is decided by three integers, not by lexical order and not by
  creation time. `v26.10.0` beats `v26.9.11`.
- Machines on 26.9.10 or earlier still trust the flag; the workflow change is
  what keeps them safe until they update once.
- A future prerelease is skipped by the picker and does not take the flag.

## Implementation

- `hexa-cli/src/commands/update.rs`: list endpoint, `parse_version`,
  `newest_release_tag`, downgrade guard. Landed at 2b9917e via
  `hexa do run`, gated on `cargo test -p hexa-cli --lib latest_release_tests`,
  which failed before the edit because the functions did not exist.
- `.github/workflows/release.yml`: `make_latest` on the release step.

## References

- ADR-2026-04-08-0929 — self-update.
- ADR-2609151100 — a lifecycle event is not a run; the fix this release carries.
- ADR-2609132122 — a recorded gate is run or it is prose.
