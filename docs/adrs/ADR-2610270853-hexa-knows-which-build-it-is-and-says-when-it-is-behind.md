---
id: ADR-2610270853
status: proposed
date: 2026-10-27
---
# ADR-2610270853: hexa knows which build it is and says when it is behind

**Status:** Proposed
**Date:** 2026-10-27
**Drivers:** On 2026-10-27 the operator ran `hexa spec accept` and got `unrecognized subcommand 'accept'`. The `hexa` on PATH (`~/.local/bin/hexa`) was built that morning from a commit without the spec pipeline; the checkout was 47 commits ahead. Both binaries printed `hexa 26.10.0`, and `hexa self-update --check` said "Already up to date". Nothing could have told them apart. The operator: *"we need to autodetect if we are not running the latest."*

## Context

- `hexa-cli/build.rs` embeds no commit. `hexa --version` prints the crate version, which changes at release, not at commit.
- `hexa self-update --check` (ADR-2609151700) compares the version against GitHub releases. A source build is never behind a release with the same version, so it reports a stale source build as current.
- The session-start hook already prints a banner each session, and is where the operator would have seen a warning before running anything.

The failure has two shapes: a binary behind **its own source checkout** (the case above, local, no network), and a binary behind **a published release** (what `self-update --check` covers today, on request only).

## Decision

1. **Every build records its provenance.** `hexa-cli/build.rs` embeds the full commit of the source tree and whether it was dirty. A build outside a git tree embeds no commit. `hexa --version` prints the version, the short commit and `-dirty` when dirty: `hexa 26.10.0 (a4d8d3c)`. The types are `Build` in `hexa-core/src/domain/provenance.rs`.
2. **A binary behind its own checkout says so.** When hexa runs in a repository that contains its build commit, and that commit is an ancestor of `HEAD`, and at least one commit from it to `HEAD` changes a file the binary is built from (a crate's sources or manifest, `Cargo.lock`, `hexa-cli/assets/`), the result is `BehindCheckout { commits }`, counting only those commits. A docs-only gap is `Current`: rebuilding would produce the same binary.
3. **A build off the branch is named, not judged.** A build commit the repository has but that is not an ancestor of `HEAD` is `OffBranch`. A repository without the build commit is not this binary's source, and the checkout check makes no claim there.
4. **A binary behind a release says so.** A published release whose version is higher than the build's (ADR-2609151700's ordering) is `BehindRelease { latest }`. The session hook asks the feed at most once in 24 hours, caches the answer in `~/.hexa/`, and gives it two seconds; a failed fetch is not retried until the next window. Equal versions are not behind, whatever the commit.
5. **No comparison is not a pass.** A build with no commit, run where no release answer is cached, is `Unknown { why }`. `hexa doctor` prints it. `staleness` never returns `Current` for a check it could not make.
6. **It is said where it would have helped.** The session-start hook prints `advice` for each finding, one line each, with the command that fixes it (`cargo build -p hexa-cli --release` and its install path for a checkout gap, `hexa self-update` for a release). `hexa doctor` prints every finding, `Unknown` included. Nothing else prints it: hooks call hexa on every tool use, and a warning on each would be noise.
7. **Detection is pure, reading is not.** `staleness` and `advice` in `hexa-core/src/domain/provenance.rs` decide; the git reads and the release fetch are adapters in `hexa-cli`. `staleness` returns every finding, so a binary both behind its checkout and behind a release reports both.

## Consequences

- The case in Drivers prints, at session start: `hexa on PATH is 47 source commits behind this checkout (built from d4eb586): cargo build -p hexa-cli --release`.
- Each session start runs two local git commands when the build commit is known. The release feed is fetched at most once a day.
- Release binaries carry the tagged commit, so they too are checked against a checkout.
- Installing from a checkout is still by hand. `hexa self-update` from source is a separate decision.

## Gate

`hexa spec verify ADR-2610270853`: the spec's sealed tests, each red before the code (ADR-2610221540 §5).

## References

- ADR-2609151700: latest is the highest version, not the newest release. Kept as the release ordering (§4).
- ADR-2610221540: an ADR is compiled to a spec and the spec to sealed red tests before code.
