---
name: hexa-maintain
description: Keep a hexa install and a hexa project current — the binary, the managed CLAUDE.md sections, the embedded skills and assets, the docs, the API contract, and the project's own checks. Use when the user asks "is hexa up to date", "update hexa", "refresh CLAUDE.md", "sync the skills", "check the docs", "check the API contract", or "run the checks".
---

# hexa maintain — the install and the project, current

## The binary

```bash
hexa --version          # the version and the commit it was built from
hexa self-update --check
hexa self-update        # the newest published release
hexa doctor             # every check, including whether this binary is behind
```

`hexa self-update` installs releases. A binary built from a checkout is
current only with that checkout: a session opens with a warning when it is
behind, and the warning carries the rebuild command.

## What hexa manages inside a project

```bash
hexa refresh            # the hexa-managed sections of CLAUDE.md, in place
hexa skill list         # skills embedded in the binary, and the project's own
hexa skill sync         # write the embedded skills into .claude/skills/ (never overwrites)
hexa skill show <name>
hexa assets list        # everything embedded: scaffold templates, provider presets, skills, schemas
hexa assets sync        # write them into .claude/
```

`sync` never overwrites, so a skill that changed in the binary does not reach a
project that already has it. Compare with `hexa skill show <name>` and replace
the file by hand when they differ.

## The docs

```bash
hexa docs check         # freshness and terminology across docs/ and module READMEs
hexa docs glossary      # the terms `check` enforces
hexa adr doctor         # every ADR id cited anywhere resolves to a file
```

## The API contract

For a project whose driving ports carry `@hexa:api` tags:

```bash
hexa api list           # one line per operation
hexa api spec           # write the OpenAPI document the tags declare
hexa api check          # exit 1 when the committed document has drifted
hexa api adapter        # generate the primary HTTP adapter for a tagged port
hexa api test --base-url http://127.0.0.1:8080   # every operation against a running test instance
```

`api test` creates and deletes data. Point it at a test instance only.

## The project's own checks

```bash
hexa dev ci             # every hexa enforcement gate
hexa dev validate       # build → test → analyze → validate
hexa dev worktree list  # every active worktree
hexa dev worktree cleanup   # remove worktrees whose branches are merged into main
hexa status             # project, config and git state
```

## Loose ends in transcripts

```bash
hexa insight punch-list   # numbered gaps and follow-ups in recent transcripts that nothing routed anywhere
```
