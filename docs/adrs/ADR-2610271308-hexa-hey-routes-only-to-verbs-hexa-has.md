---
id: ADR-2610271308
status: accepted
date: 2026-10-27
---
# ADR-2610271308: `hexa hey` routes only to verbs hexa has

**Status:** Accepted
**Date:** 2026-10-27
**Drivers:** On 2026-10-27 every command `hexa hey` can route to was run against `hexa --help`. 15 of the 24 name a verb that does not exist: `brain validate`, `brief`, `git status|log|diff`, `inbox list`, `report audit`, `secrets status`, and seven `sched …` commands. All of them belong to the daemon, scheduler and briefing systems removed on 2026-09-12 (`d82b1d8`). Meanwhile no rule routes to the verbs hexa is built around: `spec`, `loop`, `harden`, `memory`, `adr gates`, `doctor`. `CLAUDE.md` tells agents to prefer `hexa hey`, and no test checks a single route.

## Context

`classify_intent` (`hexa-cli/src/commands/hey.rs`) is an ordered list of substring rules, each returning a `TaskIntent`. Those that read the filesystem, remote-shell (trusted hosts) and workplan paths, need the adapter. The rest are a pure mapping from text to a command, written into a primary adapter where nothing can check them.

## Decision

1. **The routing table is pure.** `route` and `hexa_commands` in `hexa-core/src/domain/intent.rs` decide which command a request names. `hey.rs` asks `route` first, then keeps its remote-shell and workplan handling, which read files. The types are `Route` and `Routed` in that file.
2. **Every hexa route names a real verb.** Every string `hexa_commands` returns is a hexa command (`hexa <it> --help` succeeds, subcommands included). A test checks the whole table against the binary, so a verb that is removed breaks the route that names it.
3. **The dead routes are gone.** Nothing routes to `brain`, `sched`, `inbox`, `brief`, `report` or `secrets`. Requests for git state route to `Shell` commands (`git status`, `git log --oneline -20`, `git diff --stat`), which are not hexa's.
4. **The core verbs have routes.** Asking for the architecture grade routes to `analyze .`; where the work stands to `loop`; what to do next to `go`; checking every decision's gate to `adr gates`; checking the install to `doctor`; updating hexa to `self-update --check`; hardening or bug-hunting a path to `harden <path>`; searching what was learned to `memory search <words>`. A route that needs an argument the request does not contain is not routed.
5. **No match is said, not guessed.** When no rule matches, `route` returns `Routed::None`, and `hexa hey` prints that it found no verb and points at `hexa --help`. It never runs a nearest guess.
6. **What changes or deletes is marked.** A route whose command changes the tree, git or the install (`harden`, `self-update` without `--check`, `dev worktree cleanup`) is `destructive: true`, and `hey` asks before running it even with `--yes`.

## Consequences

- `hexa hey` covers what the shipped skills cover, and a skill may recommend it again.
- A verb that is renamed or removed fails the routing test, not a user.
- Remote shell and workplans behave as before.

## Gate

`hexa spec verify ADR-2610271308`: the spec's sealed tests, each red before the code (ADR-2610221540 §5).

## References

- ADR-2026-04-14-0000: `hexa hey`, as first decided.
- `d82b1d8`: the commit that removed the systems the dead routes name.
