# Upgrading

Behaviour changes that need a hand when you move between versions. Each entry
names the commit that carried it, what changes for a project that already
exists, and the one command that resolves it.

A change that needs nothing from you is not listed here; it is in the commit
log.

## Memory is stored per project (`97dc8f0`)

`hexa memory` used to write to one file per *user*, so every project on the
machine shared one store. It now writes to `.hexa/memory.jsonl` inside the
project, found by walking up from the working directory to the nearest `.hexa/`
(ADR-2609211200).

**What changes for you.** Entries written before this commit are still in
`~/.hexa/memory.jsonl`, and a project will not see them. Nothing is lost, and
nothing moved on its own — copying a lesson into a project that may not own it
is a guess the tool does not make.

```bash
hexa memory --global list      # everything written before the change
```

Re-store the entries that belong to a project from inside that project. Leave
the ones that are genuinely about the machine — model calibration, a lesson
about your own setup — where they are; `--global` reaches them from anywhere.

Outside a project, `hexa memory` still uses `~/.hexa/memory.jsonl`, so running
it in a directory that was never scaffolded behaves as it always did.

## An error-severity rule violation now costs grade (`6be13ef`)

A finding from the project's own `.hexa/ADR-rules.toml` at `severity = "error"`
now subtracts 10 points from the architecture score, the same as a boundary
violation (ADR-2609211430 §1). Before this, `hexa analyze --exit-code` failed on
such a tree while `--grade A` passed and the tool printed A+.

**What changes for you.** A project that already carries error-severity
findings will score lower than it did, and a CI job gating on `--grade A` may
go red on a tree that passed yesterday. The tree did not get worse; the grade
stopped disagreeing with the exit code about it.

```bash
hexa analyze . --json | jq .score_components   # rule_errors is now a term
```

Two ways forward, both one line. Resolve the finding, which is what the rule
asked for. Or decide the rule is advisory in this project and set that rule to
`severity = "warning"` — warnings stay out of the score, which is what
`--strict` is for.

## Scaffolded TypeScript projects require Node 22 (`1abe882`)

The generated `package.json` declares `engines.node >= 22` and its test script
uses a glob rather than a directory, because `node --test dist/` stopped
discovering tests and started trying to load the directory as a module
(ADR-2609211245).

**What changes for you.** Only new scaffolds. A project generated before this
commit keeps its old script; if it fails with `Cannot find module …/dist`, the
fix is that one line:

```json
"test": "tsc && node --test \"dist/**/*.test.js\""
```
