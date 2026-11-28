# ADR-2611221933: A Go `deny` entry names one package

**Status:** Completed
> 2026-11-23 Amended (pre-PR review): §1 narrowed to standard-library paths. As first written it made every Go deny entry exact, so a module deny (`github.com/x/y`) stopped covering its subpackages, a check that loosened silently.
> 2026-11-23 Completed: landed on main at ff257f6; gate `cargo test -p hexa-analysis --test adr_2611221933_spec` passed (hexa loop stage done)
> 2026-11-23 Accepted: Accepted in craigjohnson's session: a bug fix under his rule (no new or changed functionality); he accepted the spec, and its sealed tests pass on main
**Date:** 2026-11-22
**Drivers:** a Go SDK's domain parses IP addresses with `net/netip` (`ParseAddr`, `ParsePrefix`), which does no I/O. The scaffolded domain policy denies `net`, and `covers` (`hexa-analysis/src/import_policy.rs`) matches an import below a prefix, so `net/netip` and `net/url` are denied as if they were `net`. A deny beats an allow (ADR-2609211430), so the policy's own advice, "add it to `allow` … deliberately", cannot work; the project lost 20 points for two pure imports. ADR-2609211430 §3 makes the default deny "the standard library's I/O surface", and lists `net` and `net/http` as separate entries, as Go names them.
**Supersedes:** none. A defect in how ADR-2609211430's Go `deny` entries are matched.

## Context

- A Go import path names one package. `net`, `net/netip` and `net/http` are three packages; Go has no package nesting, and `net/netip` shares nothing with `net` but the start of its path.
- `judge` (`hexa-analysis/src/import_policy.rs`) checks `deny` first, then lets the standard library through, then checks `allow`. Each check uses `covers`, which is true for the path itself and for any path below it on a `/` boundary. Below-the-path matching is right for Rust (`std::fs` covers `std::fs::File`, an item of that module) and for TypeScript (`node:fs` covers `node:fs/promises`). For Go it makes one entry deny every package whose path starts with it.
- The scaffolded rules file (`hexa-cli/assets/templates/ADR-rules.toml`) denies, for Go, `os`, `net`, `http`, `net/http`, `database/sql`, `os/exec`, `io/ioutil` and `plugin`. By prefix, these also deny I/O packages the list does not name: `os/signal`, `os/user`, `net/http/httptest`, `net/http/httputil`, `net/http/pprof`, `net/rpc`, `net/rpc/jsonrpc`, `net/smtp`, and pure ones: `net/netip`, `net/url`, `net/mail`, `net/textproto`.
- The rule is `pub fn judge(lang: Language, raw: &str, origin: Origin, allow: &[String], deny: &[String]) -> Verdict` in `hexa-analysis/src/import_policy.rs` (`hexa_analysis::import_policy`), with `Origin` and `Verdict` there; `Language` is `hexa_analysis::domain::Language` (`hexa-analysis/src/domain.rs`). The signature does not change.

## Decision

1. **A Go `deny` entry naming a standard-library package matches that package only.** For a Go import, a `deny` entry whose first path element has no dot (a standard-library path, by ADR-2609211430 §2's classification) denies an import path equal to it, and no other: `deny = ["net"]` denies `net` and not `net/netip` or `net/http`. A `deny` entry whose first element has a dot names a module, and covers the packages below it on a `/` boundary, as before: `deny = ["github.com/jackc/pgx/v5"]` still denies `github.com/jackc/pgx/v5/pgxpool`. Rust, TypeScript and Python `deny` entries match as before, and every language's `allow` entries match as before.
2. **The scaffolded Go deny names the I/O packages it used to reach by prefix.** The template's Go entries are `os`, `os/exec`, `os/signal`, `os/user`, `net`, `net/http`, `net/http/httptest`, `net/http/httputil`, `net/http/pprof`, `net/rpc`, `net/rpc/jsonrpc`, `net/smtp`, `database/sql`, `io/ioutil` and `plugin` (and `http`, kept for the bare Node spelling the list shares), so a scaffolded domain is denied every I/O package it was denied before, and `net/netip`, `net/url`, `net/mail` and `net/textproto` are standard library it may use.

## Consequences

- A Go domain may parse addresses and URLs with the standard library without losing points.
- An existing project's rules file that denies `net` or `os` alone no longer denies the I/O packages below them (`net/smtp`, `os/user`, …). Such a project names them, as the new template does; that is the price of a `deny` entry meaning what it names.
- Rust's `std::net` still covers `std::net::IpAddr`, a value type in an I/O module; that is how Rust names items, and is not changed here.

## Implementation

**Gate:** `cargo test -p hexa-analysis --test adr_2611221933_spec`

Every phase ends with `cargo test --workspace` and `hexa analyze .`.

## References

- ADR-2609211430: the domain imports only what it is allowed (§2 classification, §3 default deny)
- `hexa-analysis/src/import_policy.rs` (`judge`, `Origin`, `Verdict`), `hexa-cli/assets/templates/ADR-rules.toml`
