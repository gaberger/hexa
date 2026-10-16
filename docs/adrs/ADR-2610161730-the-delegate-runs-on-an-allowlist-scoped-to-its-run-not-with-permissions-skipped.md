---
id: ADR-2610161730
status: proposed
date: 2026-10-16
---
# ADR-2610161730: The delegate runs on an allowlist scoped to its run, not with permissions skipped

**Status:** Proposed
**Date:** 2026-10-16
**Drivers:** weave and hexa were scored against Anthropic's *The AI-native SDLC playbook* (https://claude.com/resources/articles/the-ai-native-sdlc-playbook). One of the five largest gaps it found was that hexa's frontier delegate runs `claude -p --dangerously-skip-permissions` in the operator's repository on every call. Its "edit only this file" is prompt text, and the commit sweeps in files that were already dirty in the target's package. weave's decision 0253 (Proposed) wants a managed-settings floor with bypass mode disabled on every fleet host, and that floor would stop every hexa run until hexa stops depending on bypass. The operator said "yes" to filing these gaps.

## Context

The playbook's practices for this:

- "Execution is sandboxed. Agent jobs run in containers under a network policy with short-lived scoped tokens, and
  hold no production credentials by default." (Stage 5, CI/CD.)
- Managed settings set `"disableBypassPermissionsMode": "disable"`, so "no engineer, project file or command-line
  flag can widen the rules". (Stage 5, managed settings.)
- "Build-phase hooks should be fast and scoped to the file that changed." (Stage 3.)

What hexa does today, on `main` at 3da3bc9:

- **Every frontier call skips permissions.** `claude_args` pushes `--dangerously-skip-permissions` unconditionally
  (`hexa-exec/src/frontier.rs:41`). The inference adapter does the same, by design: "The flag is hardcoded here, NOT
  behind a constructor knob, specifically so it cannot be accidentally omitted" (`hexa-infer/src/adapters/secondary/claude_code.rs:208-219`).
  The reason given is that `claude -p` exits 1 non-interactively without it (`claude_code.rs:22-25`). That held for
  the CLI of the time. Claude Code 2.1.302 offers `--permission-mode dontAsk`, which denies whatever is not
  pre-allowed instead of prompting, plus `--tools` and `--allowedTools`.
- **Reviewers can write too.** `hexa harden`'s review prompts go through the same `run_claude`, so a call meant to
  read gets the same unlimited tools as a call meant to edit.
- **"Edit ONLY the file" is a request.** `direct_react.rs:858` puts it in the prompt: "Edit ONLY the file `{}` in
  this repository so that the shell command `{}` exits 0." Nothing checks it afterwards.
- **The commit takes more than the target.** `commit()` (`hexa-exec/src/direct_exec.rs:1036-1056`) stages the
  target plus every path that was dirty when the run started, if it is in the target's package directory. A
  delegate's edit to another file during the run is not committed. It is not reverted either: the evidence command
  ran with that edit in the tree, so a green gate can depend on an uncommitted change, for example a loosened test.
- **The delegate is marked but not scoped.** `run_claude` sets `HEXA_INTERNAL=1` (`frontier.rs:63`) so the
  project's hooks know it is hexa. It sets no run id and no target, so a hook cannot hold the delegate to its file.
  weave's decision 0251 (Proposed) needs both.
- **What weave says about it.** Every hexa grant in weave carries the warning "runs hexa's frontier agent (claude -p
  --dangerously-skip-permissions) in the repository, on the operator's Claude login and budget", and weave parks the
  call for approval because it "cannot bound what that agent does".

## Decision

### 1. Every call names its tools; none skips permissions

`claude_args` takes a `Scope` and builds:

- `--permission-mode dontAsk`. Anything not on the allowlist is denied, never prompted.
- `--tools <set>`. Only the listed built-in tools exist in the session.
- `--allowedTools <rules>`. Only these run without asking.
- **No `--dangerously-skip-permissions`**, anywhere. The test that asserts the flag is present becomes a test that
  asserts it is absent, for every scope.

The scopes:

| Scope | Used by | `--tools` | `--allowedTools` |
|---|---|---|---|
| `Complete` | the inference adapter (T3 completions), plan and route prompts | `""` (none) | none |
| `Review` | harden's reviewers | `Read,Grep,Glob` | `Read`, `Grep`, `Glob` |
| `Edit { target, evidence }` | `do run`, harden's fixes, build's per-file steps | `Read,Grep,Glob,Edit,Write,Bash` | `Read`, `Grep`, `Glob`, `Edit(<target>)`, `Write(<target>)`, `Bash(<evidence>)` |

A new test file is allowed only when harden's fix asks for one. It is added as a second `Edit(<test path>)` rule, and
its name is decided before the call.

### 2. The run is visible to the repository's hooks

Before an `Edit` call, hexa does two things:

- it writes a ticket at `<repo>/.git/hexa/runs/<run id>.json` holding `{ target, extra: [<allowed test path>],
  pid: <hexa's pid>, startedAt }`;
- it adds `HEXA_RUN_ID` and `HEXA_TARGET_FILE` to the child's environment, next to `HEXA_INTERNAL=1`.

The ticket is removed when the call returns, whether it passed, failed or timed out. A ticket left behind by a crash
names a dead pid, and readers ignore it. This is the contract weave's decision 0251 §3 reads. Any repository can use it.

### 3. Out-of-scope edits are refused and reverted

hexa snapshots the dirty set before the call, and diffs it against the dirty set after the call and before the
evidence command runs.

- A path that changed outside `{target} ∪ extra`, and is not in the start snapshot, is **restored** from the
  snapshot (`git checkout --` for a tracked file, removal for a new untracked one).
- The attempt is recorded as `scope_violation: [<paths>]`, and the attempt fails with "the delegate edited <paths>
  outside its scope; restored. hexa commits only <target>."
- The evidence command runs only on a tree where nothing but the scope changed. A green gate can no longer rest on an
  edit hexa will not commit.
- The pre-run dirty files stay as they are. They are the operator's.

### 4. The commit takes only the scope

`commit()` stages the target and `extra`, and nothing else. A pre-run dirty file is no longer swept in, even inside
the target's package.

When the evidence needs a supporting file that is uncommitted, hexa reports "evidence depends on uncommitted <paths>;
commit them first", and the operator commits it. This reverses the 2026-06-04 "commit-gap fix" in `direct_exec.rs`.
That fix made a commit reproduce a green state by taking files the run did not write. With §3, a green state is
reproducible from the target plus what is already committed, and anything else is the operator's to commit.

### 5. Containers, later

The allowlist is the floor. A `--isolate container` option, which runs the delegate in a container with the
repository mounted and only the target writable, is left to a later ADR. It needs an image per toolchain.

## Consequences

- hexa runs under a managed-settings floor that disables bypass mode (weave decision 0253). Without this ADR it cannot.
- A delegate cannot write outside its file, run commands other than the evidence, or reach the network through
  tools. Its only shell is the evidence command, word for word.
- A delegate that needs to run something other than the evidence, such as a formatter or a code generator, now
  fails. The operator adds that command to the evidence chain, for example `fmt && test`. That is deliberate.
- weave's warning text and its parking of hexa calls can be revisited once this ships. That is weave's decision, not
  this ADR's.
- **Given up:** the automatic inclusion of pre-run supporting files (§4). Operators who relied on it see a clear
  refusal instead of a commit.
- **Risk:** `dontAsk` behaving differently across CLI versions. hexa refuses to start a delegate on a CLI that does
  not list `dontAsk` in `claude --help`, and names the version it needs.

## Implementation

The changes, file by file:

- `hexa-exec/src/frontier.rs`:
  - `Scope`;
  - `claude_args(model, scope, prompt)`;
  - the ticket and the environment;
  - the CLI-version check.
- `hexa-infer/src/adapters/secondary/claude_code.rs`: `args_for_prompt` uses `Complete`, and the
  `always_passes_dangerously_skip_permissions` test becomes `never_passes_dangerously_skip_permissions`.
- `hexa-exec/src/direct_react.rs`: `claude_attempts` passes `Edit { target, evidence }`, and the scope check runs
  before the evidence.
- `hexa-exec/src/direct_exec.rs`: `commit()` stages only the scope; `scope_violation` goes on `DirectResult`.
- `hexa-exec/src/adversarial.rs`: reviewers use `Review`, and fixes use `Edit` with their test path in `extra`.

**Gate**, written before the code: `cargo test -p hexa-exec --test delegate_scope`, a scripted frontier in a scratch
repository:

- the argv for each scope: no `--dangerously-skip-permissions`; `--permission-mode dontAsk`; the exact `--tools` and
  `--allowedTools`;
- a scripted delegate that edits the target and one other tracked file: the other file is restored, the attempt
  fails with `scope_violation`, and the evidence is not run on the out-of-scope tree;
- a delegate that creates an untracked file: it is removed;
- a pre-run dirty file in the target's package: it is untouched and not committed;
- the ticket exists during the call, names the target and hexa's pid, and is gone after a pass, a fail and a timeout;
- `HEXA_RUN_ID` and `HEXA_TARGET_FILE` reach the child's environment;
- a CLI whose `--help` lacks `dontAsk`: refused with the version message.

Plus `cargo test -p hexa-infer claude_code`.

## References

- ADR-2610141733: a claude model id is delegated, and every gated harden fix commits. Kept; this ADR changes what a
  fix may touch.
- ADR-2610160756: harden's batched gate. Unchanged; the scope applies per fix.
- ADR-2609121400: the executable gate replaces the written spec.
- weave decision 0251 (Proposed): reads §2's ticket. weave decision 0253 (Proposed): disables bypass, and needs this first.
- Anthropic, *The AI-native SDLC playbook*, Stage 5 (CI/CD sandboxing; managed settings).

## Open questions

1. **`dontAsk` or `--restricted`.** Claude Code's `--restricted` also confines the file tools to the working
   directory and refuses bypass. But it ignores the project's settings files, so weave's decision 0251 hooks would not run
   in the delegate. This ADR chooses `dontAsk`. Should hexa use `--restricted` with the project's hooks passed through
   `--settings` instead?
2. **Reverting §4's commit-gap fix.** Refuse with a message (recommended), or keep including pre-run dirty files
   behind an explicit `--include-dirty` flag?
