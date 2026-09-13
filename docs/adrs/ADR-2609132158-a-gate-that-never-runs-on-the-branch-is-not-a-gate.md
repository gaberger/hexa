# ADR-2609132158: A gate that never runs on the branch is not a gate

**Status:** Accepted
**Date:** 2026-09-13
**Amends:** ADR-2609122048, which made this repository run its own lint gate; this says where.
**Drivers:** The commit that reordered CI (d4b6ee9) was made on `adr/local-provider-auth` and its
message said "the ordering itself is verified by the next CI run." No such run existed or could
exist: the workflow triggers on `push` to `main` and on pull requests targeting `main`, and that
branch is neither. The claim was false at the moment it was written.

## Context

hexa's own pipeline puts work on an `adr/*` branch — that is what `hexa loop` records against and
what the Decide step produces. Until that branch reaches `main`, nothing runs the gates on it. The
build, the suite, the architecture grade and the asset check all sit idle while the work that most
needs them is in flight.

Two failures follow, and both have now happened here:

1. **A commit can claim verification that was never scheduled.** Writing "CI verifies this" into a
   message on an unwatched branch is the same defect as a tool printing a result it did not
   compute. It reads as evidence and is not.
2. **A regression is found at merge, not at push.** v26.9.7 and v26.9.8 shipped with the A+ floor
   unverified because a lint nit stopped the job before the grade step. Reordering the steps fixed
   the ordering; it did not change the fact that the first time those steps ran against that work
   was after it had already landed.

The objection to watching more branches is cost. It does not apply: this repository is public, and
GitHub-hosted runners are free for public repositories. The concurrency group is already keyed on
`github.ref` with `cancel-in-progress`, so a branch under active pushing runs one job, not a queue.

## Decision

1. **CI runs on every `adr/**` branch, not only on `main`.** These are the branches hexa's own
   workflow creates. Work that is gate-first must meet its gates while it is being written.

2. **The `pull_request` trigger stays scoped to `main`.** A branch that is both pushed and has an
   open PR would otherwise run the same job twice for one change.

3. **No new steps, no new jobs.** The same four gates, in the order d4b6ee9 fixed. The
   change is which refs reach them.

4. **A commit message may claim a CI result only for a ref the workflow watches.** This is the rule
   the incident produced; it is prose, not a pattern, and no check enforces it.

## Consequences

- An ADR branch is red or green while the work is in progress, which is when the answer is useful.
- Pushes to a branch that is not `main` and not `adr/**` are still unwatched. A PR to `main` covers
  them, which is the path such a branch should take anyway.
- Run count rises roughly with the number of ADR-branch pushes. Free on a public repository, and
  cancelled on supersede.

## Gate

A push to an `adr/**` branch produces a CI run for that head SHA:

```sh
git push origin HEAD:adr/ci-reaches-this-branch
curl -s "https://api.github.com/repos/gaberger/hexa/actions/runs?per_page=20" \
  | python3 -c "import json,sys; print(any(r['head_sha'].startswith('<sha>') and r['name']=='CI' for r in json.load(sys.stdin)['workflow_runs']))"
```

The gate is written against the trigger, not against the file: reading `on.push.branches` back out
of the YAML would test the change against itself. Only a real push proves GitHub agrees.

## Evidence

To be filled by the run this ADR's own branch produces.
