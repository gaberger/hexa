---
id: ADR-2610092329
status: accepted
date: 2026-10-09
---
# ADR-2610092329: The contract is proven against the server

**Status:** Accepted
**Date:** 2026-10-09
**Drivers:** ADR-2610092245 derives an OpenAPI document from a tagged driving port, and it states what that cannot check: whether the server sends what the port declares. `examples/linkstore-svc` is the standing proof that the two can differ. Its port returns a domain `Bookmark`, while the adapter sends a `BookmarkResponse` with `created_at` in milliseconds. A document nobody runs against the server is a claim, and this project's rule for a claim is that it becomes a gate or it is prose.

## Context

The static grade sees that a primary adapter drives the port. It cannot see the bytes the adapter writes. The only witness to those is the running server.

Three things make a contract test easy to fake:

- **It can pass by not looking.** An operation that answered a declared error status (a `400` for an input the server refused, a `404` for an id that never existed) did nothing wrong, but it proved nothing about the success shape either. Counting it as a pass would make a server that refuses everything conform.
- **It can describe the server instead of judging it.** A property the server sends and the schema does not name is a schema that is wrong about the response. OpenAPI defaults `additionalProperties` to true, so such a property passes silently.
- **It needs inputs.** Path parameters need ids that exist, and bodies need values the domain accepts. Fuzzing these produces `400`s, which by the first point prove nothing.

## Decision

1. **`hexa api test --base-url <url> [path] [--examples <file>]`** exercises every operation of the contract against a running server and judges each answer against the contract.

2. **Operations run in an order that makes their inputs.** Creates first (a body method with no path parameter), then reads of collections, then reads by path parameter, then updates, then deletes. Each successful JSON answer teaches its scalar fields by wire name. A path parameter `{id}` takes the learned `id`, and an id the server minted is the one the next operation asks for. Deletes come last, so the run removes what it created.

3. **Inputs come from `--examples`, then from what was learned, then from the schema.** The examples file is a flat JSON object mapping a wire name to a value, used for any parameter or field of that name. Otherwise a value is synthesized from its type: a string is `hexa-contract-test` (an `https://example.com/…` URL when the name says url, uri or link), an integer `1`, a number `1.5`, a boolean `true`, a list one element, an object its fields, an enum its first variant. The same synthesized tag that a create sends is the one a list then asks for, so the list finds what the create made.

4. **Each answer gets one of four verdicts.**
   - **Proven:** the declared success status, with a body that validates against the response schema. A `204` (no response type) is proven by its status alone, and a body sent with it is a violation.
   - **Violation:** a status the operation does not declare, a body that does not parse as JSON where a schema is declared, or a body that does not validate. A body does not validate when a type differs, a required property is missing, a property the schema does not name is present, or an enum value is not one of the variants. Each violation names the JSON path (`$.tags[0]`).
   - **Unproven:** a declared error status, or an operation that could not be sent because a path parameter had no value. The verdict says which, and names `--examples` as the fix.
   - **Unreachable:** the request failed (connection refused, timeout).

5. **The exit code is the gate's vocabulary.** 0 when every operation is proven. 1 when any is a violation, unproven or unreachable. 2 when no operation was proven at all, or when nothing is tagged. A run that proved nothing is vacuous, whatever else it printed.

6. **The HTTP call is a port.** `HttpProbe` in `hexa-analysis/src/ports.rs` sends one request and returns the status and body. The run, the ordering, the input synthesis and the judging are a use case (`api_conformance`) and are pure apart from that port, so the gate's own unit tests need no network. The reqwest adapter lives in `hexa-cli/src/http_probe.rs` and is wired in `hexa-cli/src/lib.rs`. `reqwest` is already a dependency of `hexa-cli`, so this adds **no new dependency**.

## Consequences

- **The test writes to the server.** It creates and deletes data. It is for a test instance, and it says so when it starts.
- **A strict server needs examples.** A domain that refuses `hexa-contract-test` as a title will leave the create unproven, and everything that needed its id with it. The output names the input and the flag. That is a cost paid once per project, in one file.
- **Extra properties fail.** That is stricter than OpenAPI's default and is the point: the document claims to describe the response.
- **Authentication is out of scope.** No headers beyond `Content-Type` and `Accept` are sent. A server behind auth is a later decision.
- **`linkstore-svc` stays untagged.** This verb is what would show its tagged port to be wrong, which is why it was not tagged. Tagging it is a change to that example's design, not to hexa.

## Implementation

- `hexa-analysis/src/ports.rs`: `HttpProbe`, `ProbeRequest`, `ProbeResponse`.
- `hexa-analysis/src/api_conformance.rs` (usecases): ordering, synthesis, learning, validation, verdicts, and `run(contract, probe, examples) -> ConformanceReport`.
- `hexa-cli/src/http_probe.rs` (adapters/secondary): the reqwest adapter. `hexa-cli/src/lib.rs`: `default_probe()`.
- `hexa-cli/src/commands/api.rs`: the `test` subcommand.

**Gate**, written before the code: `cargo test -p hexa-cli --test the_contract_is_proven_against_the_server`. Each case runs the Rust fixture's contract from ADR-2610092245 against an in-process HTTP server written in the test file with `std::net`:

- a conforming server: exit 0, four operations proven;
- `tags` sent as a string: exit 1, naming the operation and `$.tags`;
- an undeclared `418`: exit 1, naming the status;
- an extra `createdAt` property: exit 1, naming it;
- a server that refuses the synthesized URL: exit 1 with the create unproven and `--examples` named; with an examples file giving an accepted URL, exit 0;
- a server that answers `404` to everything: exit 2, nothing proven;
- nothing listening: a nonzero exit, never 0;
- a project with no tags: exit 2.

## References

- ADR-2610092245: the API contract is a tagged driving port. This is its phase 2.
- ADR-2609121400: the gate replaces the written spec. Here the server is the gate for the spec.
- ADR-2609132059: done means the gate ran. A run that proved nothing has not run.

## Evidence

`cargo test -p hexa-cli --test the_contract_is_proven_against_the_server 2>&1 | tail -3` at 6f6fcb7 with uncommitted changes on 2026-10-09 23:34 UTC:

```text

test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
```

## Amendment, 2026-10-10: an empty list proves nothing

Decision 4 called a list that validates *proven*. An empty list validates against any item schema, so that verdict claimed a check that never happened. A server whose create silently dropped the tags returned `[]` for the list by tag, and the list read *proven* beside the create's violation. An empty list for a declared array response is now **unproven**, and the verdict says so: there was nothing to check the items against. A non-empty list is judged item by item, as before.

One consequence for the gate: a server that refuses the synthesized create now proves nothing at all, because its list is empty too. That run exits 2, not 1. Gate case added: `an_empty_list_proves_nothing_about_its_items`.

## Amendment, 2026-10-10: only what the run created is reused

Decision 2 said each successful JSON answer teaches its scalar fields. `hexa harden` showed what that costs: a list or a read returns records that were on the server before the run, and an id learned from one replaced the id the run's own create returned. The closing delete then removed a record the run never made. A test that deletes production data is worse than no test.

So **only a create's answer is learned**, and each learned value is kept under the resource path that created it, so `/users` and `/posts` keep their own `id`. A read, update or delete takes its path parameter from the create of its own resource, from `--examples`, or not at all, in which case it is unproven. The same pass extended the empty-list amendment above: an empty or `null` answer is unproven wherever it checked nothing, including an optional list and a map. Gate cases: `a_delete_never_reaches_a_record_the_run_did_not_create` and `an_id_is_the_one_its_own_resource_created_and_a_read_never_replaces_it` (unit tests in `api_conformance`).
