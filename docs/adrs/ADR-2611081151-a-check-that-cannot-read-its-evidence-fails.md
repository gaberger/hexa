# ADR-2611081151: A check that cannot read its evidence fails, and a seal is anchored in history

**Status:** Proposed
**Date:** 2026-11-08
**Drivers:** the workflow value assessment of 2026-11-08 (six agents, 39 workflows, planted defects). Its two highest-ranked fixes share one cause. Four groups found independently that a gate check passes output it cannot read. Two found that a sealed test can be loosened without a trace. The owner asked for both to be fixed together (2026-11-08).

## Context

**Gates.** Two parsers decide whether a gate's run proved anything. `hexa-exec/src/direct_exec.rs` `tests_observed` serves `hexa do` and `hexa adr gates`. `hexa-cli/src/commands/loop_cmd.rs` `gate_outcome` serves `hexa loop stage done`. Each returns "unknown" for output it does not recognise, and each caller treats unknown as a pass. The assessment showed what passes as a result:

- `true`, an empty `node --test`, and `cargo test --no-run` pass in `do`, `adr gates` and `loop stage done`.
- A script that exits 0 after printing a FAILED test passes, because failure counts are never read.
- `echo "test result: ok. 5 passed"` passes `adr gates`.
- `loop stage done` closes a stage with no gate recorded at all.
- `gate coverage --gate false` reports full coverage, because it never runs the gate on the intact tree.

Node 23 and later print `ℹ tests 4` instead of TAP's `# pass 4`, so a TypeScript project on current Node is "unreadable" today: it passes without being counted.

**Seals.** A sealed test's authority is the hash in `docs/specs/<ADR>.json` `sealed` (ADR-2610221540 §6). `accepted_content()` leaves that map out of the acceptance hash, as it must: sealing happens after acceptance. Nothing else anchors it, so two things launder a seal:

- Loosen the test, write its new hash into the JSON, commit. `spec verify --prove --ci` goes green on an implementation with four planted defects, because `--prove` checks only that a test of that name was red at the red commit, never that its body is the one that was red.
- Through `hexa do`, in two gated commits: one removes the `sealed` map, which leaves nothing to guard, and the next edits the test. `broken_seals` reads seals from the working tree's JSON, so the run's own edit to that JSON decides what it is checked against.

## Decision

1. **A sealed file's expected content comes from git history, not from the JSON.** For each sealed file, hexa computes the sha256 of the file's blob at the first commit in the branch's history that added it, then applies that file's amendments in order. Each amendment's `from` must equal the hash so far, and its `to` becomes the new hash. The result is the expected hash. A file whose current hash differs is **edited**. A recorded `sealed` hash that differs from the expected one is **rewritten**: the seal moved without an amendment. An amendment whose `from` breaks the chain is a **broken chain**. `hexa spec verify` fails on all three. A hash written into the JSON by hand no longer moves a seal. Every move is an amendment: `hexa spec amend`, which shows the diff and records who confirmed it, or `hexa spec tests --replace`, which records its `--because`.

   *Amended in the build (2026-11-08, approved by the owner):* this first said "the blob at the red commit". hexa keeps red commits off the branch, pinned by local `refs/hexa/red/*`, so a clone does not have them and two machines would anchor differently. For a generated test, the commit that adds it to the branch holds the same bytes, and every clone computes the same anchor from it.

2. **A seal that history cannot anchor is not trusted, and never hides an edit.** When no commit in the branch's history holds the file, the seal is **unanchored**, and `verify --ci` fails it by name. The file is still held to its recorded hash, so an unanchored seal that was also edited is reported as edited, with or without `--ci`. Seals already on main when this shipped, which hexa's own verbs had moved before every move was recorded, carry one **baseline** amendment per unexplained move, so the chain from history to the record is complete.

   *Amended in the build:* this first said "not an ancestor of HEAD" and asked for re-anchoring with `--red-at`. Under the anchor of §1, every committed seal is anchored, and the baseline replaces the re-anchoring.

3. **An agent run is checked against HEAD's seals and may not touch the record.** `hexa do` and `hexa harden` read every spec's `sealed` map from HEAD, not from the working tree. A run whose change touches any `docs/specs/*.json` is refused the way an edit to a sealed test is. The file is restored, nothing is committed, and the refusal names the file. A sealed file listed at HEAD that is gone after the run is still an edit; a file absent both at HEAD and after the run is still not (issue #50).

4. **One reader decides what a gate's run proved.** `hexa-core/src/domain/evidence.rs` holds the only parser of runner output. It returns:
   - **Passed:** the run exited 0, a recognised runner reported at least one test, and no output line reported a failed test.
   - **Failed:** the run exited non-zero, or any recognised line reported a failed test, even with exit 0.
   - **Vacuous:** the run exited 0 and every recognised runner reported zero tests.
   - **Unrecognised:** the run exited 0 and no line was from a runner the reader knows.

   It recognises `cargo test`, `go test`, `node --test` in TAP and in the spec reporter Node 23+ prints, `bun test`, and the summary of `hexa spec verify` (`N passed · M failed`). `tests_observed`, `evidence_is_vacuous` and `gate_outcome` are replaced by it.

5. **Only Passed passes.** `hexa do`, `hexa adr gates`, `hexa loop stage done` and `hexa gate coverage` accept Passed and nothing else. An Unrecognised result fails with the runners the reader knows. A Vacuous result fails as "measured nothing". `hexa loop stage done` with no recorded gate fails and says so.

6. **A gate that counts no tests says so.** Some gates are not test runners: a build, a lint, a `curl` check. A person declares one uncounted with `hexa loop gate --uncounted '<cmd>'`, `hexa do run … --uncounted`, or a `## Gate (uncounted)` heading in an ADR. For an uncounted gate, the exit status alone decides. Every line that reports it, and every commit message `hexa do` writes for it, carries the word `uncounted`. Unrecognised output is never treated as uncounted without the declaration.

7. **Coverage is measured from a passing gate.** `hexa gate coverage` runs the gate once on the unmodified tree first. If that run is not Passed (or, for a declared uncounted gate, does not exit 0), it reports no coverage, gives the reason, and exits non-zero. It never reports a hole count from a gate that could not pass.

The API this decides, in `hexa-core` (zero runtime dependencies, so every rule above is testable without a repository):

```rust
// hexa-core/src/domain/evidence.rs
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Evidence {
    Passed { tests: u64 },
    Failed { reason: String },
    Vacuous,
    Unrecognised,
    /// Declared uncounted (§6) and exited 0.
    Uncounted,
}
impl Evidence {
    /// Passed or Uncounted (§5, §6).
    pub fn passes(&self) -> bool;
}
/// `uncounted` is the declaration of §6; without it the exit status alone never passes.
pub fn read_evidence(exit_ok: bool, output: &str, uncounted: bool) -> Evidence;

// hexa-core/src/domain/spec.rs
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Amendment {
    pub file: String,
    pub from: String,
    pub to: String,
    pub by: String,
    pub reason: String,
    pub at: String,
    pub base_commit: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SealVerdict {
    Holds,
    /// The file's bytes differ from the expected hash.
    Edited,
    /// Listed, tracked at HEAD, and gone now.
    Gone,
    /// The `sealed` record differs from the expected hash: moved without an amendment.
    Rewritten,
    /// An amendment's `from` does not continue the chain; the index of the first such.
    BrokenChain(usize),
    /// No blob at the red commit to anchor to.
    Unanchored,
}
/// Judge one sealed file. `at_red` is the sha256 of its blob at the red commit
/// (None when history has none); `recorded` is the `sealed` map's hash;
/// `current` is the file's sha256 now (None when absent); `amendments` are all
/// of the spec's, oldest first: only those for `file` apply.
pub fn judge_seal(
    file: &str,
    at_red: Option<&str>,
    amendments: &[Amendment],
    recorded: Option<&str>,
    current: Option<&str>,
) -> SealVerdict;
```

## Consequences

- **Gates that passed by being unreadable now fail.** A `make check` or a shell script gate stops passing until it either prints a recognised runner's output or is declared uncounted. Declaring it is one flag, and the word `uncounted` then shows wherever its result is reported. That trade is the point: a gate that degrades silently is worse than no gate (CLAUDE.md, key lessons).
- **Fake runner output still fools any text reader.** A command that echoes `test result: ok. 5 passed` reads as Passed. This ADR stops a gate passing by accident, not by forgery. What stops forgery is §3: an agent cannot rewrite the record it is checked against, and gate commands are written by people.
- **A forged amendment is no longer silent.** Someone with commit access can still append a fake entry to `amendments`. `verify` lists every amendment with its author and reason, so a forged one shows up for review.
- **The existing seals are baselined, not re-proved.** Nine seals on main had been moved by `--replace`, re-sealing and re-proofs before every move was recorded: three in ADR-2610221540 and one each in ADR-2610270853, ADR-2610270901, ADR-2610271308, ADR-2610271736, ADR-2610301129 and ADR-2610301330. Each gap carries one baseline amendment, written once and reviewed in one commit. Every sealed file matched its record when this shipped, so the baseline records history; it does not excuse an edit.
- **Older sealed fixtures were amended.** Five gate-parsing fixtures in ADR-2610221540's suite used `true`, which §5 no longer passes, and its `verify` fixtures were not git repositories, which §2 cannot anchor. Each was changed through `hexa spec amend`, with the scenarios unchanged. S35 of this spec ran `hexa do` with no model reachable and now drives the single-shot path through a completion command.
- **`gate coverage` gets slower by one gate run**, the price of not reporting coverage from a gate that fails.
- **Out of scope:** the red check that accepts a test that only panics (issue #41, the assessment's third fix) is a separate ADR.

## Gate

`hexa spec verify ADR-2611081151`

## Applies-To: hexa-core/src/domain/evidence.rs, hexa-core/src/domain/spec.rs, hexa-exec/src/direct_exec.rs, hexa-cli/src/commands/loop_cmd.rs, hexa-cli/src/commands/adr/mod.rs, hexa-cli/src/commands/spec_ledger.rs, hexa-cli/src/commands/gate.rs

## References

- ADR-2610221540: the gate-first pipeline, the seal (§6) and vacuous gates.
- ADR-2611062141: `hexa spec amend`, whose amendments §1 chains.
- ADR-2611020815: a run does not replace the loop's gate.
- Issue #50: a sealed file absent at HEAD and after a run is not an edit.
- Issue #41: the red check (out of scope here).
