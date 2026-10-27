---
name: hexa-workflow
description: Do a change the gate-first way — decide in an ADR, compile it to a spec a person accepts, seal tests that fail first, build to them, and record where the work stands. Use when the user asks to "add a feature", "implement this decision", "write the spec", "what's next", "where does the work stand", "remember this", "is this claim true", or "what has inference cost".
---

# hexa workflow — decide, spec, gate, build

hexa does not trust a change because its tests pass. It trusts a change whose
gate was written before the code, by something that never saw the code, and
which failed until the code existed. This skill is that loop, verb by verb.

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

## 4. Build to the sealed tests

```bash
hexa loop gate "hexa spec verify ADR-<id>"
hexa do run "<task>" --file <path> --evidence "<command>"   # one file, committed only on exit 0
hexa build "<challenge>" --target <dir> --gate "<command>"  # a whole system
hexa spec verify ADR-<id>          # each scenario's test, run alone
hexa spec verify --prove ADR-<id>  # each test existed and failed at its red commit
```

## 5. Harden, then ship

```bash
hexa harden <path> --gate "<command>"   # hunt, verify each finding, fix under the gate
hexa analyze .                          # the architecture grade
hexa spec verify --ci                   # what CI runs
hexa loop stage done                    # writes the evidence into the ADR
```

## Along the way

| Need | Verb |
|---|---|
| Where the work stands, this session's and every other's | `hexa loop` |
| The steps this work is made of | `hexa loop task` |
| The next right thing | `hexa go` |
| A plain-English request, routed to the verb it names | `hexa hey "<request>"` — says so when no verb matches; asks before anything that changes the tree, even with `--yes` |
| Project and git state at a glance | `hexa status` |
| A lesson you will otherwise forget | `hexa memory store lesson:<topic> "<text>"` |
| What was learned before | `hexa memory search <query>` |
| Test a claim before acting on it | `hexa verify "<claim>"` — CONFIRMED / REFUTED / INCONCLUSIVE |
| Which module the gate does not actually cover | `hexa gate coverage --gate "<command>"` |
| A workplan for a larger change | `hexa plan create <requirements> --adr ADR-<id>`, then `hexa plan status` |
| What inference has cost | `hexa spend` |

A decision whose gate is not built yet is declared in `.hexa/project.json →
adr.unbuilt` with a reason; `hexa adr gates` then reports it rather than failing
on it, and fails if its gate starts passing.
