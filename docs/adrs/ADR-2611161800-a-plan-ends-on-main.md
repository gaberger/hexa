# ADR-2611161800: A plan ends on main

**Status:** Accepted
> 2026-11-20 Accepted: Accepted in bulk on 2026-11-20 at Gary Berger's instruction ("accept all adrs"); not individually reviewed by the agent
**Date:** 2026-11-16
**Drivers:** "When looking at the work done by hexa, it often just leaves work in an unfinished state, hasn't completely finished all of the open plan records and merged them into the main branch. hexa has to go and make sure that all work is completed." Then: "hexa needs to create a plan, and the end of the plan is to finish all the tasks and then to close out the ADR and commit everything into the main branch. The goal should be always to have a continuous integration such that the branch can create all the updates from all the agents."
**Supersedes:** none. Builds on the loop (ADR-2611160730 §4), `hexa do gate` (ADR-2611160730 §3), the task ledger and `hexa dev worktree merge`/`cleanup`, and `hexa adr complete`.

## Context

What one working day in harness left behind, on 2026-11-16:

- The loop checklist read 17 of 24. At least four of the open seven had shipped in earlier commits and were never checked off. The checklist said "unfinished" about finished work, and nobody could tell which.
- 17 workplans, all `pending-planner` with no steps, six of them named `approved-`. The hook router drafts one from every large prompt, subagent briefs included (`plan/mod.rs`, ADR-2026-04-11-0227 auto-invoke). Nothing ever plans one, closes one or deletes one.
- 7 worktrees whose branches were already on main. Nothing removed them, so they read as work in flight.
- Every session's `ended:` line in `hexa loop` shows stage gate or decide: the work stopped and the record stayed open.

The hooks that exist do not stop this:

- The blocking Stop hook runs `hexa analyze` only. A session can stop with uncommitted files, an open checklist item, commits not pushed and a branch not merged.
- SubagentStop prints the worktrees with unmerged commits and lets the agent go.
- `hexa loop stage done` needs a passing gate and takes the evidence. It does not look at the checklist, the branches or the ADR's status, so "done" can be recorded with half the work on a side branch.

So hexa records where work starts and does not hold it to an end. The end the person wants is fixed and the same every time: the tasks done, the ADR closed, everything on main.

## Decision

A plan has one end, and hexa holds every session to it.

1. **`hexa finish` lists the open work and how to close each item.** It is read-only. It reports one list, each item with the command that closes it:
   - uncommitted changes to tracked files;
   - commits not on the upstream (not pushed);
   - checklist tasks not done, including the one being done;
   - worktree branches with commits not on main, with the command that lands them;
   - worktree branches already on main and clean, with the command that removes them;
   - draft workplans that were never planned, with the command that discards them;
   - the ADR the loop is under, when it is not Completed.

   `--json` prints the list as data. `--exit-code` exits 1 when anything is open. Nothing open prints `nothing open: the work is on main`.
2. **Every plan ends with the same two steps, added by hexa.** Recording an ADR for the loop (`hexa loop adr`) appends `Land on main` and `Close <ADR>` to the end of the checklist, once. They stay last: a task added later goes before them. They are checked by hexa, not by hand:
   - `hexa loop task done` on `Land on main` passes only when no worktree branch holds commits not on main and the tree is clean;
   - `Close <ADR>` passes only when every other task is done and the ADR is Completed.
3. **`hexa loop stage done` is the close-out.** It is refused, with the open items named, until `hexa finish` has nothing open for this work except the ADR. Then it runs the gate on main, appends the evidence, sets the ADR to Completed with the reason `landed on main at <sha>; gate <command> passed`, and checks off `Close <ADR>`. Done means on main and closed, not "the gate passed somewhere".
4. **Integration is continuous: a gated commit lands.** `hexa do gate` on a branch other than main lands the commit after making it:
   - It rebases the branch onto main, and re-runs the gate if the rebase moved anything.
   - It fast-forwards main: the main checkout when that checkout is clean, or the `main` ref when no checkout has it.
   - When the rebase conflicts, the gate fails on main, or the main checkout has uncommitted changes, the commit stays on the branch and the reason is reported. Nothing is forced, and `hexa finish` lists that branch until it lands.
   - `--no-land` keeps a commit on its branch on purpose.

   Every agent's work therefore reaches main as each piece passes, not in one merge at the end. **Landing is local and never pushes.** A push can need someone's approval (a public repository, a protected branch), so it stays a step of its own, and `hexa finish` lists the commits not pushed.
5. **A session does not stop with its own work open.** The blocking Stop hook (`hexa hook stop`) runs the session-scoped `hexa finish`:
   - the files this session touched and did not commit;
   - its checklist;
   - its own branch, when that is not landed;
   - once all of those are done, its ADR's close-out.

   When something is open, the hook returns `block` with the list as the reason, so the agent carries on and finishes. It blocks at most three times in a row in one session, then lets the stop through and prints the list, so a session that cannot finish can still end and say why. Commits not pushed are said in the reason but never block, for the reason in §4. SubagentStop doing the same for a subagent's worktree is a later slice.
6. **No plan nobody plans.** The router does not draft a workplan from a prompt. This already shipped in e5e91c5: auto-invoke is off unless a project asks for it, and a subagent brief is never drafted. A plan is made when someone asks for one (`hexa plan draft`, `hexa loop task add`), not inferred from prompt length. `hexa finish` lists the drafts left from before that, including the ones approved while still empty, with the command that removes them.

## Consequences

- "Done" means what the person means by it: tasks finished, ADR closed, everything on main. A loop can no longer read 17 of 24 when the work has shipped, because the close-out checks the tree, not the boxes.
- Agents' work meets on main in small steps. A conflict between two agents shows up at the second one's gated commit, while both changes are small, not at the end.
- Sessions take longer to stop, because the Stop hook sends unfinished work back. The three-block bound keeps a stuck session from looping forever.
- Landing touches main. It only ever fast-forwards and never forces. A dirty main checkout is never written to; the commit goes to `origin/main` and the checkout is told it is behind.
- Repositories that do not work on `main` read their base from the upstream's default branch, the same way `hexa dev worktree` does.

## Implementation

**Gate:** `cargo test -p hexa-cli --test a_plan_ends_on_main`

Tests come first, each against a temporary git repository:

1. `finish` lists an uncommitted change, an unpushed commit, an open task, a worktree branch with commits not on main, a merged clean worktree and an unplanned draft, each with its command. `--exit-code` exits 1. Exactly the same repository, once clean and landed, prints `nothing open` and exits 0.
2. `loop adr` appends `Land on main` and `Close <ADR>` once, and a task added afterwards goes before them.
3. `task done` on `Land on main` is refused while a worktree branch holds commits not on main, and the refusal names the branch.
4. `stage done` is refused while a task is open. When everything is landed it sets the ADR to Completed, with a reason naming the sha.
5. `do gate` on a branch fast-forwards a clean main to the new commit. A conflicting change stays on the branch and is reported.
6. The Stop hook, given a session with an uncommitted touched file, prints `"decision":"block"` with the file in the reason. On the fourth stop in a row it lets the stop through.
7. A long prompt through the router writes no draft.

Slices, in order:
1. `finish` and the Stop hook.
2. The close-out steps and `stage done`.
3. `do gate` lands.
4. The router stops drafting: already shipped in e5e91c5, and the test guards it.
5. SubagentStop holds a subagent to its worktree, as the Stop hook holds a session.

## Addendum, 2026-11-25: a pushed branch in review does not block a stop

**Driver.** datum, 2026-11-24 and 25. Its main is protected and takes work only through pull requests. In a worktree whose branch was pushed and waiting on its pull request, or still being built by other agents, the Stop hook blocked every stop of every agent whose directory it was, including a subagent told not to touch that branch. It counted the branch against a local `main` that had not moved since the clone ("19 commits" for a branch one commit ahead of `origin/main`), and it named `hexa dev worktree merge <branch> --force` as the fix. Each agent had to refuse it, dozens of times; the owner asked for it to stop ("can we get rid of this error", "fix it").

1. **A commit is on the base when the local base or origin's copy of it holds it.** `branch_items` counts a branch's commits that are on neither `refs/heads/<base>` nor `refs/remotes/origin/<base>`. A local base nobody moved after a fetch no longer makes merged work look unlanded.
2. **A branch pushed as it stands is in review, and the Stop hook says so without sending the session back.** When the session's branch has an upstream other than the base and that upstream holds exactly the branch's head, §5's "its own branch, when that is not landed" is reported (`in-review`, with the upstream's name) and is not a reason to block. A merge can need someone's approval, as a push can. A branch never pushed, or with a commit its upstream does not hold, blocks as before.
3. **`hexa finish` and `loop stage done` are unchanged by 2.** A pushed branch that is not on the base is still listed as `unlanded` and `Land on main` is still not done: the plan ends on main, and a pull request that is open has not ended.
4. **A branch with an upstream is never told to merge by force.** Its `unlanded` item says to push and land through its pull request. Only a branch that exists nowhere else is offered `hexa dev worktree merge`.

Gate: `cargo test -p hexa-cli --test stop_hook_pushed_branch --test a_plan_ends_on_main`.

## References

- ADR-2611160730: compliance by refusal. Covers the hooks and `do gate`.
- ADR-2611160750: leases follow the architecture. Its `merge-order` is the order the landing in §4 follows when several tasks finish together.
- ADR-2026-04-11-0227: prompt auto-invoke drafting, which §6 retires.
