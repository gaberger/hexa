# ADR-2611230831: hexa says what is true (II)

**Status:** Completed
> 2026-11-23 Completed: landed on main at 9b29d89; gate `cargo test -p hexa-cli --test adr_2611230831_spec` passed (hexa loop stage done)
> 2026-11-23 Accepted: Accepted in craigjohnson's session: a bug fix under his rule (no architectural change); he accepted the spec, and its sealed tests pass on main
**Date:** 2026-11-23
**Drivers:** Four more places, found by projects adopting hexa, where hexa printed something untrue or crashed on a harmless use. ADR-2611221646 fixed five of the same kind. CLAUDE.md: "A gate that degrades silently is worse than no gate."
- `hexa doctor` warned ".claude/worktrees exists and is not in .gitignore" in a repository that ignores it through `.git/info/exclude`; `git check-ignore .claude/worktrees` says it is ignored.
- `hexa spec draft` ends "Read every scenario … Then `hexa spec accept <ADR>`", but `hexa spec accept` refuses a spec that is not committed (ADR-2611161600 §1), so following the instruction fails.
- A Python client library whose objective reads "a Python SDK for the Example API" was described in every session's context as a web API, with "Return JSON responses with appropriate HTTP status codes" injected as a constraint.
- `hexa analyze . | head` and `hexa spec show <ADR> | head` left a 75–100 MB core file in the working directory when `head` closed the pipe.
**Supersedes:** none. Defects in what hexa reports; no gate becomes looser.

## Context

- The doctor check (`hexa-cli/src/commands/doctor/claude_memory.rs`) reads the root `.gitignore` alone to decide whether `.claude/worktrees` is ignored.
- `draft` (`hexa-cli/src/commands/spec_ledger.rs`) writes the spec and prints the closing line; `accept` refuses an untracked or modified first acceptance and names `git add` and `git commit`.
- `infer_output_type` (`hexa-analysis/src/fingerprint_extractor.rs`) reads the project's objective: after a web-framework check and a CLI-word check, it tests the API words (`api`, `server`, `rest`, `http`, `endpoint`) before the library words (`library`, `crate`, `package`, `sdk`, `framework`), so an objective naming both an SDK and an API reads as a web API.
- `hexa` is built with `panic = "abort"`. Rust ignores `SIGPIPE`, so a write to a closed pipe fails, `println!` panics, and the abort leaves a core.

## Decision

1. **`hexa doctor` asks git whether `.claude/worktrees` is ignored.** The check is `git check-ignore`'s answer for the path, so `.gitignore`, `.git/info/exclude` and the global excludes file all count; outside a git repository it reads `.gitignore` as before.
2. **`hexa spec draft` says to commit the spec before accepting it.** Its closing line names the spec file, says to commit it, and then says `hexa spec accept <ADR>`.
3. **A library word in the objective outranks an API word.** An objective that names a library, crate, package, SDK or framework, and no web framework is detected, is a library even when it also names an API; the web-framework and CLI checks come first as today.
4. **A write to a closed pipe ends hexa quietly.** On Unix, `hexa` restores the default `SIGPIPE` disposition at start, so a reader that closes the pipe ends the process with no panic message and no core file.
5. **Nothing else changes.** What doctor checks besides this path, what `spec accept` refuses, the output types of objectives naming no library word, and every exit status other than a closed pipe's, are as before.

## Consequences

- A repository that keeps hexa's files out of its public `.gitignore` gets a true doctor report.
- The first `spec accept` after a draft no longer fails for an instruction hexa gave.
- A client library is described as a library, without HTTP-response constraints.
- Piping hexa's output into `head` leaves nothing in the working tree.

## Implementation

**Gate:** `cargo test -p hexa-cli --test adr_2611230831_spec`

Every phase ends with `cargo test --workspace` and `hexa analyze .`.

## References

- ADR-2611221646: hexa reports what it actually saw
- ADR-2611161600: a first acceptance is of a committed spec (§1)
- `hexa-cli/src/commands/doctor/claude_memory.rs`, `hexa-cli/src/commands/spec_ledger.rs`, `hexa-analysis/src/fingerprint_extractor.rs`, `hexa-cli/src/main.rs`
