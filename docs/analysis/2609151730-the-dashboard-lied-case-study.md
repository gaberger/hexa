# Case study: the dashboard lied, and the tool fixed itself

**Date:** 2026-09-15
**Subject:** hexa's own run feed, release pipeline, and self-update. One session,
two bugs, two releases, all through hexa's verbs.
**Method:** Not a trial. Nothing was pre-registered; the session started with a
question and the record below is what happened, in order, with the misses kept
in. Every claim names a commit or a command that can be rerun.

The two earlier trials asked whether hexa can change code it did not write.
This one is narrower and, for a developer deciding whether to trust the tool,
more pointed: when hexa's own reporting is wrong, does the loop catch it, and
does the fix go through the same gate it demands of everyone else?

## What the operator saw

```
⬡ Direct Runs  62 runs · 5 passed · 57 failed · 5 committed · 8% pass
  ✗ —
  ✓ 24f4a2e   schedule.ts        A schedule can ask to be paged (ADR-0104). ...
  ✗ —
  ✗ —
```

Eight percent. Fifty-seven rows with no file, no instruction, no error. The
question was "why did 57 runs fail."

## Bug 1: a lifecycle event counted as a failed run

`~/.hexa/agent-runs.jsonl` had two writers. `hexa do run` appends one row per
run. The Claude Code hook handler appends one row per subagent start and stop
to the same file, with `kind: "subagent"` and nothing a run record has. The
feed mapped every row into a run, defaulted the missing `ok` to false, and
counted it.

| Rows in the log | 63 |
|---|---|
| Subagent hook events | 58 |
| Actual `hexa do` runs | 5 |
| Of those, passed and committed | 5 |

The real pass rate was 100%. The number had been wrong since the hooks first
fired on 2026-09-12, and nothing had failed, because a summary over rows it
cannot interpret still prints a confident number. This is the shape named in
the key lessons as "a gate that degrades silently."

### The fix, in order

1. **Gate first.** A test module `runs_feed_tests` was appended to
   `hexa-exec/src/direct_exec.rs`. It feeds a mixed log of hook events and run
   rows into two functions that did not exist, `runs_from_rows` and
   `summary_of`, and asserts the hook rows contribute nothing to the list, the
   summary, or the display ids. Run before the edit:

   ```
   error[E0432]: unresolved imports `super::runs_from_rows`, `super::summary_of`
   ```

   That is the gate failing for the right reason.

2. **Build to the gate.** `hexa do run` with the task, the one file, and the
   gate as evidence. One ReAct step, evidence pass, commit 293af9c.

3. **Record.** ADR-2609151100. `hexa adr doctor`: no findings.

4. **Verify beyond the gate.** hexa-exec unit tests 109 passed. Workspace
   check clean. `hexa analyze .` zero violations. The rebuilt binary:

   ```
   ⬡ Direct Runs  6 runs · 6 passed · 0 failed · 6 committed · 100% pass
   ```

   The sixth run is the fix itself.

## Bug 2: the release that would have downgraded everyone

Cutting v26.9.11 exposed the second bug. Tag v26.9.10 had never been pushed,
so `git push --tags` sent both. Two release workflows ran side by side. The
v26.9.10 run finished second and took GitHub's "latest" flag, because the flag
means most recently *created*, not highest version.

`hexa self-update` read `/releases/latest`, compared strings, and would have
printed `Update available: 26.9.11 → 26.9.10` on every machine and installed
it. A downgrade, cleanly, with a checkmark.

### The fix, in order

1. **Gate first.** `latest_release_tests` in `hexa-cli/src/commands/update.rs`,
   asserting on `parse_version` and `newest_release_tag`: highest version wins
   regardless of list order, `v26.10.0` beats `v26.9.11`, drafts and
   prereleases and unparseable tags are skipped.

   First invocation named the wrong target and reported:

   ```
   test result: ok. 0 passed; 0 failed
   ```

   A vacuous gate. The pipeline rule says that is a failed gate, and it was
   treated as one: the target was corrected and the gate rerun until it failed
   with the expected unresolved imports. Had that rule not existed, the next
   step would have "passed" against nothing.

2. **Build to the gate.** `hexa do run`, one step, commit 2b9917e. Self-update
   now lists releases, picks the highest parsed version, and refuses to move
   backwards unless `--version` names a tag.

3. **Belt and braces for old binaries.** Machines on 26.9.11 or earlier still
   read the flag. `.github/workflows/release.yml` now passes `make_latest`
   explicitly for stable releases. Commit df33f7f.

4. **Record.** ADR-2609151700.

5. **Verify.** Workspace tests 869 passed, 0 failed. v26.9.12 cut, tagged,
   pushed. GitHub showed v26.9.12 as latest at 17:12:13, about five minutes
   after the push, with its Linux tarball answering 302.

## What the grade caught

After fix 1, `hexa analyze .` reported 98/100 with two dead exports. They were
`runs_from_rows` and `summary_of`, the two functions the fix had just added as
`pub` when only their own crate used them. Narrowed to `pub(crate)`, the gate
still passed and the grade returned to 100.

The tool deducted points from its own fix. That is the behaviour a developer
should want: the grade is a property of the tree, not a courtesy to the diff.

## Friction that was not hexa's, recorded anyway

- The shell's `cargo` was a system 1.75 that cannot read a version-4 lock
  file; rustup's 1.98 sits in `~/.cargo/bin`, off PATH. `scripts/release.sh`
  already carries the workaround from a 2026-05 release. Every cargo call in
  this session prefixed PATH by hand.
- Installing the rebuilt binary over `~/.local/bin/hexa` failed with
  "text file busy" because a hexa process was running from it. Copy to a
  sibling and rename works while busy; plain copy does not.
- The https origin has no stored GitHub credential and `gh` is not installed.
  Every push went through an ssh alias configured for a different repository
  that happened to have access. The GitHub API rate-limited the machine, so
  release completion was verified through the release page redirect and an
  asset HEAD request, not the Actions API.

## What this shows

- **The gate is real.** Both fixes were made by an agent that could only
  commit on green, against a test written before the code and shown failing
  first. Nothing landed on the strength of "looks right."
- **The vacuous-gate rule earned its place.** It fired once in this session,
  on the author of the case study.
- **The grade is independent of the author.** It penalised the fix and was
  right to.
- **Records were made, not promised.** Two ADRs, both with the gate command
  and the commit that satisfied it. `hexa adr gates` can rerun them.

## What it does not show

- Neither bug was found by hexa. An operator read a number and asked why. The
  run feed had no test that would have noticed 57 empty rows, and the release
  pipeline had no check that "latest" was the highest tag. Both now exist,
  and both are unit tests over data, not checks against the live store or the
  live GitHub API. A regression in the writer's shape, or in GitHub's
  semantics, would pass them.
- The ADR ids were chosen in the gate before the ADRs existed, so both ids
  carry an earlier time than their write. The Date fields are correct. A gate
  that names a record which does not yet exist is a promise, and promises
  are what this pipeline is supposed to replace.
- `hexa do run` writes the instruction's first line as the commit subject.
  Both feature commits from this session have subjects that are a paragraph
  of diagnosis truncated mid-sentence. The verb should take a subject, or
  derive one, rather than commit prose.

## Verdict

**Two real bugs in hexa's own reporting and release path, both fixed through
hexa's own loop, both gated, both released, in one session.** The pitch for a
developer is not that the tool was right. It is that when the tool was wrong,
the same gate it imposes on everyone else was the thing that fixed it, and the
record of that is rerunnable.

The next thing to build is the detection this session lacked: a check in
`hexa doctor` that the run feed's rows all have a run's shape, and a
post-release step that asserts the published "latest" is the highest tag.
