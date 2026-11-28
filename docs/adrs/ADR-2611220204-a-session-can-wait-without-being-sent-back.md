# ADR-2611220204: A session can wait without being sent back

**Status:** Rejected
> 2026-11-22 Rejected: reverted from local main on craigjohnson's rule for this hexa work — no changes to hexa core behavior (the Stop hook, `hexa finish`, `hexa loop`, landing hints). It was built, reviewed and Completed (7e78510), then taken out whole: code, sealed tests and spec. Kept here as the record of the decision and of the bound defect it found (ADR-2611161800 §5 resets its count after a let-through, so the bound is three blocks, one pass, repeating).
> 2026-11-22 Completed: landed on main at 0f2d8e9; gate `cargo test -p hexa-cli --test adr_2611220204_spec` passed (hexa loop stage done)
> 2026-11-22 Accepted: Accepted by craigjohnson in session (he approved the decision, accepted its spec, chose the fixture amendments and the untracked-file rule, and chose to land it): 30 sealed scenarios red at 5a267c1 pass at fb703e0; three code reviews' findings fixed; hexa A+ 100/100.
**Date:** 2026-11-22
**Drivers:** On 2026-11-21 one hexa session was sent back by the Stop hook more than forty times while it waited, correctly, on a 30-minute test run, two code reviews and the owner's approval. A second session (a Python SDK) reported the same loop on a branch whose landing is the owner's call, and that the hook told it to run `hexa dev worktree merge <branch> --force`. The same session found `.hexa/stop-blocks.json` left untracked by `hexa init`.
**Supersedes:** none. Amends ADR-2611161800 §5 (the Stop hook's bound) and the `unlanded` item of §1.

## Context

ADR-2611161800 §5 made the Stop hook send a session back while its own work is open, "at most three times in a row in one session, then lets the stop through and prints the list, so a session that cannot finish can still end and say why."

What `hexa-cli/src/commands/hook/mod.rs` (`stop_until_finished`) does:

- It keeps a count per session in `.hexa/stop-blocks.json`. While the count is under three it blocks; when it lets a stop through, it **removes the session's count**, so the next stop starts from zero. The bound is three blocks, one pass, three blocks, forever. The test for §5 (`hexa-cli/tests/a_plan_ends_on_main.rs`) checks only the fourth stop.
- Nothing can say "this is waiting, and why". A branch waiting on a person's approval, a test run still running and a review not yet reported read the same as work the agent walked away from.
- Open work is a list of `Item`s (`hexa-cli/src/commands/finish.rs`) with a kind, a text and a fix. The text is not an identity: all uncommitted files are one item whose text carries a count, a task's text carries its position and a `[ ]`/`[>]` marker that change when another task is added or done, and an unlanded branch's text carries its commit count. "The same open work" cannot be told from the text.
- Five hints tell an agent to `--force` a merge: the `unlanded` item (`finish.rs`, `branch_items`), `land.rs`'s rebase-conflict message, `worktree.rs`'s merge-order note, the SubagentStop line in `hook/mod.rs`, and `adr/doctor.rs`. `hexa dev worktree merge` is a file-level cherry-pick (`worktree.rs`): it puts the branch's files on the base in one new commit and leaves the branch's own commits off the base, matching branches by substring, so `merge feat/x --force` also takes `feat/x-2`. On a gate-first branch the commit that seals the tests (the spec's `red` when no `--red-at` was given) is one of the commits left behind: the spec then names a commit main cannot reach.

`hexa init` keeps the task ledger out of version control (`task_ledger::ensure_ignored` writes `.hexa/.gitignore`), but not `stop-blocks.json`.

## Decision

1. **Every open item has a stable key.** `Item` gains a `key`, the same while the work is the same:
   - `uncommitted:<path>`, one item per file;
   - `task:<title>`, and `closeout:land` / `closeout:close:<ADR>` for the two close-out steps;
   - `unlanded:<branch>@<tip sha>`, so a new commit on the branch is new work;
   - `adr:<id>`;
   - `unpushed:<upstream>`.

   The text is what is printed; the key is what is compared.
2. **A session records that its work is waiting, and why.** `hexa loop hold '<reason>'` writes, on this session's loop entry in `.hexa/loop.json`, the reason, the time and the keys of the session's blocking open work (what the Stop hook would block on now). It is refused, and nothing is written, when the reason is empty or blank, or when nothing is open. A second hold replaces the first; the first is kept in the entry's hold history. `hexa loop hold --clear` removes it. `hexa loop resume` does not carry a hold into a new session. A hold changes nothing else: `task done` on a close-out step, `loop stage done` and the landing check refuse exactly as before.
3. **The Stop hook honours a hold for the keys it holds, and nothing else.** A file this session wrote counts as its open work whether git tracks it yet or not: a new file is work as much as an edited one. An open item whose key is in the hold does not block. An item whose key is not, a file touched afterwards, a task added afterwards, a new commit on a held branch, blocks as before, and the block's reason lists only those. While the hold covers everything open, the hook lets the stop through with one line naming the reason and how many items wait. A hold whose keys have all closed is inert: the hook ignores it and does not write `loop.json`; `hexa loop` shows it as stale. `hexa loop` and the SessionStart summary print an active hold and its reason.
4. **Three in a row is a bound, not a cycle.** `stop-blocks.json` keeps, per session, the count and the keys it last blocked on. After three blocks in a row on the same keys, every later stop is let through with the list printed, until the keys change; then the count starts again. When nothing is open the session's entry is removed. A count written by an older hexa (a bare number) is read as that count with no keys.
5. **No hint tells an agent to force a merge.** Each of the five hints names a landing that keeps the branch's commits: from the base's checkout, `git merge --ff-only <branch>` when the base is an ancestor of the branch; otherwise, merge the base into the branch, re-run the gate, then fast-forward. None names `hexa dev worktree merge … --force`.
6. **The stop count stays out of version control.** `ensure_ignored` writes `stop-blocks.json` into `.hexa/.gitignore` with the ledger's entries, and the Stop hook calls it the first time it writes a count, so a project that never re-ran `hexa init` is covered.

## Consequences

- A session waiting on a long run, a reviewer or the owner says so once and ends its turns, and the reason is on the record for the next session and the person.
- A hold does not let new work go unfinished: anything opened after it blocks as before. It can cover work the agent never started; what that buys it is bounded by decision 4, which already lets any stop through after three blocks, so a hold trades three blocks for a written reason the person reads at the next session start.
- The bound in ADR-2611161800 §5 means what it says.
- A scratch file a session writes and never adds to git now blocks its stop like any other uncommitted file: delete it, commit it, or hold it with a reason.
- Hints stop pointing at the step that can leave a sealing commit off main.
- In hexa's own repository `.hexa/loop.json` is committed, so a hold's reason travels with the branch. In a project made by `hexa init` it is ignored, and a hold is a local record only.
- Not decided here: `hexa do gate`'s automatic landing rebases a branch onto a moved base (`land.rs`, ADR-2611161800 §4), which rewrites a sealing commit the same way. That is a separate decision, with its own ADR.

## Implementation

**Gate:** `cargo test -p hexa-cli --test adr_2611220204_spec`

| Phase | Work | Decisions |
|---|---|---|
| 1 | `Item.key` for every kind | 1 |
| 2 | `hexa loop hold`, shown by `hexa loop` and SessionStart | 2 |
| 3 | The Stop hook: the hold, the bound, the count's keys | 3, 4 |
| 4 | The five hints | 5 |
| 5 | `stop-blocks.json` ignored | 6 |

Each test runs against a temporary git repository, with `hexa hook stop` fed a Stop payload on stdin. Every phase ends with `cargo test --workspace` and `hexa analyze .`.

## References

- ADR-2611161800: a plan ends on main (the Stop hook, `hexa finish`, landing)
- ADR-2610221540: the spec, the seal and the red commit; ADR-2611081151 §1: red commits kept off the branch with `--red-at`
- `hexa-cli/src/commands/hook/mod.rs`, `hexa-cli/src/commands/finish.rs`, `hexa-cli/src/commands/land.rs`, `hexa-cli/src/commands/worktree.rs`, `hexa-cli/src/commands/adr/doctor.rs`, `hexa-cli/src/commands/loop_cmd.rs`, `hexa-cli/src/commands/task_ledger.rs`
