---
name: hexa-workflow
description: Do a change the gate-first way — decide in an ADR, compile it to a spec a person accepts, seal tests that fail first, build to them, and record where the work stands. Use when the user asks to "add a feature", "implement this decision", "write the spec", "what's next", "where does the work stand", "remember this", "is this claim true", or "what has inference cost".
---

# hexa workflow — decide, spec, gate, build

hexa does not trust a change because its tests pass. It trusts a change whose
gate was written before the code, by something that never saw the code, and
which failed until the code existed. This skill is that loop, verb by verb.

## Which path

- **A small change** (a fix, one behaviour, no new port, adapter or
  dependency): record a gate, make the edit, commit it through the gate. That
  is `hexa loop gate 'cargo test'`, the edit, then
  `hexa do gate --files src/domain/paging.rs -e 'cargo test' -m 'paging: an even split adds no empty page'`
  (or `hexa do run`, section 4, which makes the edit and commits it).
  Sections 0–3 do not apply.
- **An ADR-backed feature** (a new port, adapter or dependency, or anything
  an ADR decides): sections 0–5 in order. Section 2 ends at a person: **an
  agent stops and asks; it never runs `hexa spec accept` itself.**
- **Outward-facing verbs** (`hexa pr open`, `hexa issue file`,
  `hexa issue sync`, `hexa gap pr`): by default they print what they would
  send. Pass `--send` only when the person has said to send it, in this
  conversation. `hexa forge status` reads the forge and sends nothing.

**Committing source is `hexa do gate`; `git commit` is for docs.** `do gate`
runs the gate on the change already in your tree and commits only the files
you name, and only if the gate passes. The pre-bash hook refuses a
`git commit` that would commit project source and prints the `do gate`
command that does it, filled in. A `git commit` of docs, ADRs, specs or
markdown only, or one that finishes a merge, is never refused.

If the project declares its full gate (`"gate": {"full": "cargo test"}` in
`.hexa/project.json`), recording a narrower command such as
`hexa loop gate 'cargo test --test paging'` keeps it as the fast gate, for
quick checks while you work. The full gate still decides
`hexa loop stage done` and is the evidence `do gate` is given.

**A new session picks up where the last one stopped.** When this session has
recorded nothing, `hexa status`, `hexa loop` and the session-start banner show
the previous session's ADR, gate, stage and checklist, labelled as the
previous session's. `hexa loop resume` carries that work into this session, so
the hooks accept edits under its gate.

Several verbs commit, and some reset. Before running `do run`, `harden`,
`build`, `spec tests`, `bench` or `gate coverage` in a tree with other work in
it, read `/hexa-git-effects`: what each commits, on which branch, what it
resets, and how to get your work back (`git reflog`).

## 0. Design (optional, before the decision)

For a feature-sized change, stress-test the idea before deciding it, with the
Elephant-Goldfish Model (EGM; see `docs/ATTRIBUTIONS.md`). The elephant drafts
with the project's context; each goldfish reviewer is one tool-less completion
that sees only the document and the source it names.

```bash
hexa design brainstorm "<idea>"          # concepts_brief.md, then the critic goldfish's critique
hexa design draft                        # design_draft.md from the brief
hexa loop design <draft>                 # record the draft; the loop's stage is design
hexa design review <draft>               # stages 5 to 9; the report records the draft's sha256
hexa design resolve <report> --accept S6-1,S7-2   # apply only the findings you accept
hexa design review <draft>               # again: a revised draft needs a review of its new bytes
git commit <draft>                       # a person settles the design by committing it
hexa design adr <draft>                  # a proposed ADR that cites it; the loop moves to decide
```

`hexa design adr` refuses a draft that is untracked, uncommitted, or changed
since a review covering stages 5 to 8. `hexa status` (or `hexa go`, which
prints only the next step) names the next of these steps. Skip this section for a small change: an ADR may be written directly.

## 1. Decide

Write an ADR in `docs/adrs/` with numbered decisions (`1.`, `2.` …) under
`## Decision`. Name the port and domain files it decides in backquotes, and
write new signatures into them now — they are what the spec author is shown.

```bash
hexa adr doctor              # every cited ADR id resolves to a file
hexa loop adr ADR-<id>       # record the decision this work is under
```

## 2. Spec — the gate, written from the decision alone

```bash
hexa spec draft ADR-<id>     # scenarios from the ADR and its ports, never the code
hexa spec check ADR-<id>     # every decision has an example and a negative
hexa spec show ADR-<id>      # what "correct" will mean
```

`spec draft` refuses an agent model, because an agent can read the code. Use
a completion model; `--model claude-text` is the logged-in `claude` CLI with no
tools.

**A person accepts the spec. An agent never does.** Stop and ask:

```bash
hexa spec accept ADR-<id>
```

## 3. Seal the tests, red

```bash
hexa spec tests ADR-<id> --crate <dir>   # one test per scenario; each must fail now
```

The commit holding them is recorded as `red` and the files are sealed. No
change to a sealed test is accepted afterwards; a requirement changes through
the ADR, then the spec, then a new acceptance.

A scenario's test is anchored in history, as its seal is: `spec verify` fails
when it points somewhere else with no record. `spec tests --only` and
`--replace` record their own moves. To rename or relocate a test yourself,
commit the new file, then run `hexa spec move ADR-<id> --scenario <S>=<file>::<name>
--reason "<why>"`, which shows the diff, then again with `--confirm <sha256>`.

When the scenarios are unchanged and only a sealed test's setup must change
(a fixture, never an assertion), record it with `hexa spec amend ADR-<id> --file
<test> --reason "<why>"`, which shows the diff, then again with `--confirm
<sha256>`. Commit it in two steps, in this order: the spec record alone first
(`git commit docs/specs/ADR-<id>.json`), then the test through `hexa do gate
--files <test> -e '<cmd>'`. `do gate` refuses while a spec record differs from
HEAD, and accepts the sealed file once the recorded amendment matches it.

`hexa spec tests` writes Rust. A scenario tested in vitest or by a bash gate
script is bound to a test you write first, after hexa watches it fail at its
commit (ADR-2611191120). The ref names its runner by extension:
`<file>.rs::<fn>` (cargo), `<file>.ts::<test name>` (vitest, from the nearest
package.json; exactly that one test must run and pass), `<file>.sh[::<arg>]`
(bash, exit 0). Other languages get a template under `test.runners` in
`.hexa/project.json`.

```bash
hexa spec red ADR-<id> <commit> --test "S3=web/src/a.test.ts::adds two" --test S4=ci/check-x.sh
hexa spec seal ADR-<id>
```

## 4. Build to the sealed tests

```bash
hexa loop gate "hexa spec verify ADR-<id>"
hexa do run "<task>" --file <path> --evidence "<command>"   # an agent edits; commits on this branch only if the evidence passes
hexa build "<challenge>" --target <dir> --gate "<command>"  # a whole system
hexa spec verify ADR-<id>          # each scenario's test, run alone
hexa spec verify --prove ADR-<id>  # each test existed and failed at its red commit
```

`hexa do run` commits on the current branch: `--file`, plus each file named
with `--also` (repeatable) for a change that needs more than one file. Start
from a clean tree. It refuses while other tracked files in the same package
are already modified, unless you name them with `--also` or pass
`--include-dirty` to sweep them in on purpose. Details and recovery:
`/hexa-git-effects`.

**Counted or uncounted.** A gate or evidence command passes on what a test
runner reports, not on exit 0 alone: zero tests is a failure, and output no
known runner wrote (cargo test, go test, node --test, bun test, pytest,
unittest and Django's test runner, `hexa spec verify`) is a failure. A command that runs no tests — a build, a
lint, `hexa analyze . --exit-code` — must be declared uncounted, or it fails
every time:

```bash
hexa do run "<task>" --file <path> --evidence "cargo clippy -- -D warnings" --uncounted
hexa loop gate "hexa analyze . --exit-code" --uncounted
```

The commit and every report then say `uncounted`, so a reader knows no test
was counted.

vitest's summary and JSON report are counted too, and so is cargo-nextest's
`Summary [ … ] N tests run: P passed, F failed, S skipped`. Colour codes are
stripped first. A gate that is exactly one bash script (`bash ci/check-x.sh`)
counts as one test.

**Python: make the type checker part of the test run.** Python has no
compiler, so without one the first gate an agent meets is the tests it wrote
itself. With the `pytest-mypy` plugin installed and `[tool.mypy] strict = true`
in `pyproject.toml`, mypy runs as part of pytest: a type error is a failing
test, and hexa counts the result like any pytest run:

```bash
hexa loop gate "pytest --mypy -q"
hexa do gate --files <file> -e "pytest --mypy -q" -m "<what changed>"
```

A type error fails the gate with nothing committed. Run pytest the way the
project does (for example through `uv run`), with the package importable
(`[tool.pytest.ini_options] pythonpath`), so the plugin and the code are found.

**Where tests run.** A project that builds elsewhere sets `test.remote` in
`.hexa/project.json` to a template with `{commit}` and `{cmd}`. `spec verify`,
`spec red` and `loop stage done` then send every test for the commit concerned
through it, in one run, and refuse an unpushed commit or a tree that differs
from HEAD. They never fall back to running here; `--local` runs here, on purpose.
`loop stage done` reads the remote itself (`git ls-remote`) to check that HEAD
is pushed, refusing with `head-not-pushed`, and sends the recorded evidence
command with the gate. `hexa harden` gates each fix on the runner as a scratch
candidate commit of the tree (`refs/heads/hexa-gate/<id>`, deleted after) and
keeps a fix only when it passed there. A hexa verb that runs on the runner
itself (its batch exports `HEXA_ON_RUNNER=<commit>`) runs its tests in place.
`hexa do gate` sends its gate there too: it pushes a candidate commit of the
named files to `refs/heads/hexa-gate/<id>`, runs the gate on the remote runner
for that commit, and moves the branch only on a verdict it accepts, then pushes
the branch and reads its head back (the branch must track a remote branch).
Tests first: commit them while they fail with `--expect-red`, which refuses a
pass, zero tests and a compile error, never lands, and prints the
`hexa spec red <ADR> <sha>` to run next. `--dry-run` shows what it would push
and run. The last stdout line is one JSON object: `ok`, `commit`,
`remote_head`, `tests`, and on failure `code`, `hint`, `field`, `retryable`.

## 5. Harden, then ship

```bash
hexa harden <path> --gate "<command>"   # hunt, verify each finding, fix under the gate
hexa analyze .                          # the architecture grade
hexa spec verify --ci                   # what CI runs
hexa drift --since <base>               # do the docs this change touched still describe the code?
hexa loop stage done                    # writes the evidence into the ADR
```

## Along the way

| Need | Verb |
|---|---|
| Where am I: the loop (ADR, gate, stage, checklist), what is in flight in this repo, and the next step (JSON when piped; `--all` for every project) | `hexa status` |
| The next step alone: the design draft's step, else record a gate, fix the red gate, or the pending task. Read-only | `hexa go` |
| Where the work stands, this session's and every other's | `hexa loop` |
| Carry the previous session's ADR, gate, stage and checklist into this one | `hexa loop resume` |
| Commit an edit you made yourself, through the gate | `hexa do gate --files src/domain/paging.rs -e 'cargo test' -m 'paging: an even split adds no empty page'` |
| The steps this work is made of | `hexa loop task` |
| A plain-English request, routed to the verb it names | `hexa hey "who uses src/parser.rs"` — exits 1 when no verb matches; asks before anything that changes the tree, every time |
| Which decisions are done, how far each is built (from evidence, not status), who is on it | `hexa adr board` (`--text` for a table, `--html <file>` for a page, `--strict` to fail when a status is behind its evidence) |
| Every feature the decisions name, and where each stands (proposed, accepted, completed, retired) and in which release | `hexa adr features` (`--md` for a fact sheet, `--md --check <file>` to fail when the committed sheet is stale, `--state`, `--adr`, `--all`, `--strict`); `hexa adr features extract <ADR>` drafts an ADR's `## Features` table |
| Several agents in one repo: claim a task and the paths it may edit, close it, merge in order | `hexa agent register --task <name> --lease <path>`, `hexa agent report --id <id> --outcome passed --commit <sha>`, `hexa dev worktree merge-order` |
| Whether a native C/C++ dependency (DuckDB, say) is rebuilt by each cargo command | `hexa dev build-keys` (`--selection "<cargo args>"` to compare commands) |
| A lesson you will otherwise forget | `hexa memory store lesson:<topic> "<text>"` |
| What was learned before | `hexa memory search <query>` |
| Test a claim before acting on it | `hexa verify "<claim>"` — CONFIRMED / REFUTED / INCONCLUSIVE |
| Which module the gate does not actually cover | `hexa gate coverage --gate "<command>"` |
| A workplan for a larger change | `hexa plan create <requirements> --adr ADR-<id>`, then `hexa plan status` |
| What inference has cost | `hexa spend` |

**Before starting work in a repository other agents use:** run `hexa adr board`
and `hexa agent list`; do not start an ADR someone holds or edit a leased path;
`hexa agent register` your task and paths first and `hexa agent report` it with
the commit. A decision counts as built when its phase sections carry a
`**Built:**` line naming the commit, so add one when the evidence is in.

A decision whose gate is not built yet is declared in `.hexa/project.json →
adr.unbuilt` with a reason; `hexa adr gates` then reports it rather than failing
on it, and fails if its gate starts passing.

## Closing out

These are the rules a model skips, because nothing says *when*. There are four
moments.

1. **Before you start.** `hexa agent register --task <name> --lease <path>…`,
   before the first edit, not after the work: a claim made afterwards held
   nothing while the work was done. Write the gate first (`hexa loop gate`).
2. **When you open the pull request.** The same pull request carries the ADR's
   `**Built:**` line: which phases, the date, the commit, the tests that show
   it. Credit is part of the change, and a change with no `**Built:**` line is
   unfinished.
3. **When work merges** — yours, or one you merge for someone else. Run
   `hexa adr board`. An ADR whose work merged and still reads 0 built is your
   finding, fixed now: add the `**Built:**` line from the code and a test, as a
   small change of its own. `hexa adr board --strict` exits 1 when a status is
   behind its evidence.
4. **Before you report, stop, or ask what is next.** Every item still open is
   written down first, and it is not in your message. A decision not yet made
   is an ADR (`docs/adrs/`). Work not yet done is a workplan:
   `hexa plan draft "<what is left>"`, then `hexa plan status`. Your message
   then points at them. A list that exists only in the conversation is lost
   with it. Then close your claim:
   `hexa agent report --id <id> --outcome <passed|failed|killed> --commit <sha>`.

Do not end a turn with a menu, or a question about work you were already asked
to do: take the next item. If the next item needs a decision that is not yours,
write it as an ADR and say that.
