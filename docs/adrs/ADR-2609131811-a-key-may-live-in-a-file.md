# ADR-2609131811: An endpoint's key may live in a file

**Status:** Accepted
**Date:** 2026-09-13
**Epoch:** hexa
**Drivers:** The registered tt-gptoss endpoint names `TT_STUDIO_GATEWAY_KEY`. That key exists on this machine, in `~/.config/tt-model/gateway.env`, and has all day. hexa reads `std::env::var` and nothing else, so every request went out unauthenticated and came back 401, and the fallback reviewer built this afternoon could not run.

## Context

`resolve_key` reads the process environment. That is correct and it is not sufficient: a key that
lives in a file is the normal case, not an exotic one. On this machine the gateway key is written
by another tool into its own config directory, and reaches a process only if someone remembers to
source it first.

The gap has a second edge. This project's own settings deny reading `.env`, correctly, to stop an
agent pasting a secret into a transcript. The effect all day was that the *tool* — which reads a
key into one HTTP header and never prints it (ADR-2609122043) — could not get at a credential while
the agent was repeatedly tempted to go and fetch it by hand. The rule pointed the wrong way: it
should forbid the agent and permit the tool.

## Decision

1. **A named key is looked for in the environment, then in env files**, in this order: the process
   environment, `$HEXA_ENV_FILE`, `~/.hexa/.env`, and the project's `.env`. The first non-empty
   value wins. An absent file is not an error.

2. **The file format is the ordinary one**: `KEY=value` per line, `export ` prefix tolerated,
   surrounding single or double quotes stripped, blank lines and `#` comments ignored. Anything
   else on a line is skipped rather than guessed at.

3. **A value read from a file is treated exactly as one read from the environment** — into the
   header and nowhere else. The existing warning already names only the variable, never the value,
   and the same holds for the file: the path may be reported, the contents never.

4. **The search order is reported, not guessed at.** When no value is found the warning names the
   variable and the places looked, so the operator learns where to put it instead of discovering
   the list by reading source.

## Consequences

- The endpoint on this machine works by pointing `~/.hexa/.env` at the file that already holds the
  key — no copy of the secret is made, and no shell has to be primed before hexa runs.
- `.env` files are read by hexa in process. They are still not read by the agent, and the deny rule
  that stops that stays exactly as it is.
- A machine with a key in neither the environment nor any of those files behaves as before: a
  warning naming the variable, and an unauthenticated request that fails honestly.

## Gate

`cargo test -p hexa-infer env_file`: `KEY=value`, `export KEY=value`, quoted values, comments, blank
lines and malformed lines parse as expected; the environment wins over a file; an earlier file wins
over a later one; a missing file is not an error; and no value anywhere yields an empty key rather
than a panic.
