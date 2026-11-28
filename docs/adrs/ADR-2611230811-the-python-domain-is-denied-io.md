# ADR-2611230811: The default domain policy denies Python's I/O, as it does the others'

**Status:** Completed
> 2026-11-23 Completed: landed on main at 91da205; gate `cargo test -p hexa-analysis --test adr_2611230811_spec` passed (hexa loop stage done)
> 2026-11-23 Accepted: Accepted in craigjohnson's session: Python brought up to what the other languages have, under his rule; he accepted the spec, and its sealed tests pass on main
**Date:** 2026-11-23
**Drivers:** A Python project adopting hexa found that the domain import policy `hexa init` writes denies nothing Python-specific: a domain module may import `subprocess`, `socket` or `sqlite3` and the grade holds. ADR-2609211430 §3 makes the scaffolded default deny "the standard library's I/O surface" of each language; it lists Rust's, TypeScript's and Go's. ADR-2611211324 added Python and did not extend the list.
**Supersedes:** none. Completes ADR-2609211430 §3 for the language ADR-2611211324 added.

## Context

- The policy is `[[import_policy]] domain-imports-only-what-it-is-allowed` in `hexa-cli/assets/templates/ADR-rules.toml`. For Python, a `deny` entry covers the module it names and every module below it on a `.` boundary (`covers`, `hexa-analysis/src/import_policy.rs`): `os` covers `os.path`.
- Two entries already listed for other languages reach Python by that rule: `os` (listed for Go) and `http` (listed as Node's bare spelling) deny Python's `os` and `http`, `http.client` and `http.server`. Nothing else in the list is a Python module.
- A standard-library import passes the policy unless a `deny` entry covers it (`judge`, `import_policy.rs`).

## Decision

1. **The scaffolded deny names Python's standard-library I/O.** The template's deny list adds `subprocess`, `socket`, `shutil`, `tempfile`, `glob`, `sqlite3`, `urllib.request`, `ftplib`, `smtplib`, `poplib`, `imaplib`, `ssl`, `signal`, `multiprocessing`, `ctypes` and `importlib` (it loads code at run time, as Go's `plugin`, ADR-2609211600 §6), under a `# Python` heading. `os` and `http` stay listed once, for every language that reads them.
2. **Pure modules stay usable.** `pathlib`, `io`, `urllib.parse`, `json`, `datetime`, `dataclasses`, `typing`, `enum`, `decimal`, `re` and the like are not listed: they compute values, and a domain may use them.
3. **Nothing else changes.** Third-party packages are not listed (no language's default lists one); Rust, TypeScript and Go entries are as before; a project whose rules file was written before this ADR keeps its own list.

## Consequences

- A new Python project's domain is denied the standard library's I/O, as a Rust, Go or TypeScript project's is.
- An existing Python project adds these entries itself to get the same policy.

## Implementation

**Gate:** `cargo test -p hexa-analysis --test adr_2611230811_spec`

Every phase ends with `cargo test --workspace` and `hexa analyze .`.

## References

- ADR-2609211430: the domain imports only what it is allowed (§3, the default deny)
- ADR-2611211324: hexa reads Python
- `hexa-analysis/src/import_policy.rs` (`judge`, `Origin`, `Verdict`; `Language` is `hexa_analysis::domain::Language`, `hexa-analysis/src/domain.rs`)
