# ADR-2611251652: A scenario's test binding is anchored in history, as its seal is

**Status:** Proposed
**Date:** 2026-11-25
**Drivers:** A sealed test that is renamed and weakened by hand passes `hexa spec verify --ci` and `--prove --ci`. Reproduced in a throwaway repository: `ci/t.sh` checked that the file `answer` holds `42`; it was moved to `ci/t3.sh`, weakened to check only that the file exists, and the spec's test refs and `sealed` entry were rewritten by hand. With `answer` holding `7`, both commands exit 0 and the spec carries no amendment. The same edit in place fails with "seal rewritten".
**Supersedes:** none. Closes a hole in ADR-2611081151 §1, which decided that every move of a seal is recorded.

## Context

A seal is anchored per path: the blob at the first commit that added the file, followed by that file's amendments (ADR-2611081151 §1). A new path therefore starts a new, empty chain. A scenario's `test` and the spec's `sealed` map are outside the acceptance hash, because both are written after acceptance. Nothing compares the test a scenario is bound to now with the test history says it was bound to, so moving the binding drops the old chain without a trace. `--prove` catches only a weakening that also passes at the red commit.

Moving a binding is sometimes right. `hexa spec tests --only` re-proves named scenarios into a new file (ADR-2610281628 §2) and `--replace` regenerates one (ADR-2610281442 §4); both record their reason only in a commit message. When this was written, this repository's history held seven moves of an unchanged scenario, and one other project's held one.

## Decision

1. **A scenario's binding has an anchor, taken from history.** The history read is the branch's first-parent line, over every place the spec may live: git's default simplification drops the line a merge did not take the spec from, so a side branch's changes are seen at its merge. A scenario's text is its `id`, `decision`, `kind`, `description`, `given`, `when` and `then`. The anchor is the oldest test ref recorded for it in the commits of its spec file in the branch's history since the scenario last took its current accepted text. A revision whose acceptance holds and whose text for that id differs ends the search. A revision whose acceptance does not hold is passed over, so an unaccepted edit of the text, reverted later, moves no anchor. A revision in which the scenario is pending does not end the search. A scenario with no bound revision in history has no anchor.
2. **A binding changes only by a recorded move.** The spec JSON holds a list `moves`, oldest first, of `{scenario, from, to, by, reason, at, base_commit}`, where `from` and `to` are whole test refs. `judge_binding` in `hexa-core/src/domain/spec.rs` applies a scenario's moves to its anchor in order: a `from` that is not the ref reached so far is a **broken chain**, and a current ref other than the one reached is **moved**. `hexa spec verify`, with or without `--ci` and with or without `--prove`, fails on both and names the scenario, the ref history expects and the ref the spec holds. A scenario with no anchor, and a pending one, hold. Every move already recorded in the revision the anchor is taken from, whether it led to that ref (a move made on a branch before it merged) or belongs to an older text's anchor, is not applied again; it stays in the record. `verify` prints how many moves a spec carries, and `--verbose` lists each with its reason, as it does amendments.
3. **The verbs that move a binding record it.** `hexa spec tests --only` and `--replace` write one move per scenario whose ref they change, with `--because` as the reason, in the commit that makes the change. `hexa spec red` refuses to bind a scenario that has an anchor to any other ref, and names `hexa spec move`. A scenario whose accepted text changed has no anchor, so re-proving it records no move.
4. **A move made by hand is reviewed, as an amendment is.** `hexa spec move <ADR> --scenario <id>=<ref> --reason "<why>"` prints a unified diff from the file the anchored chain reaches, at its last version in history, to the file `<ref>` names, then the command to confirm, ending in `--confirm <sha256 of the new file>`. It writes nothing without `--confirm`, and with it records the move only when the hash is the file's hash now. It records one scenario per call. The record also holds `sha256`, the hash of the file reviewed, and `verify` fails a move whose reviewed content is neither where the new file's history starts nor reached by one of its amendments, so one version cannot be shown and another committed. It refuses an empty reason, a spec whose acceptance does not hold, a `<ref>` whose test is not in the file, an old file with no version in history, and a scenario whose chain already reaches `<ref>`. The same verb records a move that history already holds: the spec's ref is already `<ref>`, and only the record is missing. It changes no test file, no seal and no acceptance.

## Port and domain signatures

In `hexa-core/src/domain/spec.rs`, re-exported by `hexa-core/src/ports/spec.rs`:

```rust
/// One recorded change of the test a scenario is bound to (§2).
pub struct Move {
    pub scenario: String,
    pub from: String,
    pub to: String,
    pub by: String,
    pub reason: String,
    pub at: String,
    pub base_commit: String,
}

pub enum BindingVerdict {
    Holds,
    /// The spec's ref is not the one the anchor and the moves reach.
    Moved { expected: String },
    /// A move's `from` does not continue the chain; the index of the first such.
    BrokenChain(usize),
}

/// The anchor (§1). `history` is the spec file's revisions, oldest first,
/// each with whether its acceptance holds. `current` is the scenario now.
pub fn binding_anchor(current: &Scenario, history: &[(bool, Spec)]) -> Option<String>;

/// Judge one binding (§2). `moves` are all of the spec's, oldest first:
/// only those for `scenario` apply.
pub fn judge_binding(scenario: &str, anchor: Option<&str>, moves: &[Move], current: Option<&str>) -> BindingVerdict;
```

## Consequences

- **A project whose history already moved a binding fails `verify` until each move is recorded** with `hexa spec move`. That is the point: those moves were never recorded. This repository records its own in the change that builds this: 27, of which 23 renamed a test function inside a file that did not change.
- Building this showed that `spec tests --only` re-sealed a file it cut old tests from without an amendment, so `verify` then failed with "seal rewritten". It now records that amendment, as ADR-2611081151 §1 requires.
- A move record, like an amendment, can still be typed into the JSON by hand. This decision makes a move visible in the record, the diff and `verify`'s count; it does not make forging a record impossible. The same holds for an acceptance, which is a hash anyone can compute: a forged acceptance of altered text, committed and then reverted, would restart the anchor (§1), and shows only in the spec's history. `hexa do` and `hexa harden` already refuse a run that touches a spec record (ADR-2611081151 §3).
- A shallow clone sees less history, and a binding with no visible anchor holds; `verify` says so. The seal anchor has the same limit.
- A scenario first bound on a side branch is anchored where that branch merged, so a move made inside the branch before the merge is judged only by `verify` run on the branch itself.
- In the `scenarios` shape with bare test names, a changed name is reported as moved, and `spec move` takes only a full `<file>::<name>` ref. No such history is known.

## Implementation

Spec after acceptance, tests first.

**Gate**, written first: `hexa spec verify ADR-2611251652` — every scenario of the accepted spec passes, and none is a mirror.

## References

- ADR-2611081151 (the seal anchored in history), ADR-2611062141 (`spec amend`), ADR-2610281628 (`--only`), ADR-2610281442 (`--replace`).
