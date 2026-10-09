---
id: ADR-2610100005
status: accepted
date: 2026-10-10
---
# ADR-2610100005: The HTTP adapter is generated from the contract

**Status:** Accepted
**Date:** 2026-10-10
**Drivers:** ADR-2610092245 makes a tagged driving port the API contract, and ADR-2610092329 proves a server against it. Between them sits the code a person still writes by hand: the primary adapter that turns HTTP into port calls. That is exactly what Encore generates, and it is mechanical. Each route, extractor, status code and error mapping is already decided by the contract. Writing it by hand is where a route gets mistyped (`:id` panics axum 0.8 at startup, as `linkstore-svc`'s own comments record).

## Context

Encore's generated server is the application's front door, owned by the framework. In a hexagonal project the front door is a primary adapter, one replaceable file that imports `ports/` only. So the generated code is an adapter. It drives the tagged port, the composition root hands it the port's implementation, and nothing else imports it.

A generated adapter has to be judged by something other than its generator. A generator test that compares output to a golden file can only say the generator still writes what it wrote. Phase 2 provides the independent judge: build the generated adapter into a real server, run it, and `hexa api test` it. The port's implementation in that server is written in the test, from the contract, not by hexa.

Framework choice is a dependency decision for the projects hexa generates into:

- **Go: `net/http`.** Go 1.22 added method and wildcard patterns (`GET /bookmarks/{id}`, `r.PathValue`) to the standard mux, so the adapter needs no dependency at all.
- **TypeScript: `node:http`.** The standard library again, which avoids choosing between Express, Fastify and Hono for every user. It costs a small route matcher written into the adapter.
- **Rust: axum 0.8.** Rust's standard library has no HTTP server. axum is what `examples/linkstore-svc` already uses, and its extractors map one to one onto path, query and body parameters.

## Decision

1. **`hexa api adapter [path] [--port <name>] [--out <file>] [--force]`** writes one primary adapter for one tagged port. The language is the port's. With more than one tagged port, `--port` is required. The default destination is the conventional primary-adapter folder: `src/adapters/primary/http.rs`, `adapters/primary/httpapi/handler.go`, or `src/adapters/primary/http-handler.ts`. An existing file is never overwritten without `--force`, and `--out` must resolve inside the project.

2. **The adapter drives the port and nothing else.** It imports the port's module only, exposes one constructor taking the port (`router(api)`, `NewHandler(api)`, `createHandler(api)`), and leaves listening to the composition root. It is generated to grade clean: an `adapters/primary` file importing `ports/` only.

3. **Errors map to the contract's statuses.**
   - **Rust:** each variant of the port's error enum maps to its `@hexa:status`, and every other variant to 500. The match uses `Variant { .. }` patterns, so tuple, struct and unit variants are all covered.
   - **Go:** an error reports its status by having an `HTTPStatus() int` method, found with `errors.As`; any other error is 500. Go has no closed error type to read statuses from, so this is the convention the generated adapter documents.
   - **TypeScript:** a thrown value with a numeric `status` property gives that status; anything else is 500.
   - Malformed input (a body that does not parse, a query value of the wrong type) is 400 in every language.

4. **The types are the port's.** Rust names the types as the port's signature writes them, and imports the port's module with a glob, so its re-exports resolve. A `&str` or `&[T]` parameter is extracted owned and passed by reference. Go qualifies the port package's types. TypeScript names nothing: it types each argument as `Parameters<Port['method']>[i]`, so it cannot drift from the port. Body types must deserialize: in Rust they derive `Deserialize` and responses `Serialize`, which is the port's business, as ADR-2610092245 already asks of wire-shaped ports.

5. **What it does not generate:** authentication, middleware, the server's `main`, and anything the contract cannot express. List-valued query parameters are refused for axum, whose query extractor cannot read repeated keys into a list, and the refusal names the parameter.

## Consequences

- **A project goes from a tagged port to a served, proven API with no hand-written HTTP code:** tag the port, `hexa api adapter`, wire it in the composition root, `hexa api test`.
- **The generated file belongs to the project.** It is written once and edited like any other adapter. Regenerating it is a deliberate `--force`, not a build step, so there is no generated code hidden from review.
- **Rust projects take on axum, serde and tokio.** The verb names them when it writes a Rust adapter. Go and TypeScript take on nothing.
- **Port methods may be sync or async.** A sync Rust method is called inside the async handler. That is right for in-memory work and wrong for blocking I/O; `linkstore-svc` shows the `spawn_blocking` answer, and the generated file says so where it calls the port.

## Implementation

- `hexa-analysis/src/domain.rs`: each operation gains its arguments as the port declares them (`ApiArg`: source name, type as written, and where it comes from: path, query, body, body field or context), whether the method is async, its port's file, and its error type's variants with their statuses. `VariantDecl` keeps the variant's identifier beside its wire name.
- `hexa-analysis/src/treesitter_api.rs`: records each parameter's type as written, `async`, and Go's context parameter.
- `hexa-analysis/src/api_adapter.rs` (usecases): `generate(contract, port, target) -> Result<String, String>`, a pure text function per language.
- `hexa-cli/src/commands/api.rs`: the `adapter` subcommand.
- `.github/workflows/ci.yml`: sets `HEXA_TEST_CARGO_NET` for the Rust leg below, as it already sets `HEXA_TEST_NPM`.

**Gate**, written before the code: `cargo test -p hexa-cli --test an_adapter_is_generated_from_the_contract`. In each language the test copies the fixture contract, runs `hexa api adapter`, writes a composition root and an in-memory port implementation itself, builds and starts the server, and requires `hexa api test` to exit 0 with four operations proven, and `hexa analyze --json` to report no boundary violation and no unserved port.

- **Go:** runs wherever `go` is on PATH, as the scaffold gate does.
- **TypeScript:** behind `HEXA_TEST_NPM=1`, for its `npm install`.
- **Rust:** behind `HEXA_TEST_CARGO_NET=1`, for fetching axum.
- **Always:** the verb refuses to overwrite without `--force`, refuses an `--out` outside the project, requires `--port` when two ports are tagged, and generates the same bytes twice.

## References

- ADR-2610092245: the API contract is a tagged driving port. This is its phase 3.
- ADR-2610092329: the contract is proven against the server. That proof is this decision's gate.
- ADR-2609132158: a gate that never runs on the branch is not a gate. Hence the CI variable.

## Evidence

`HEXA_TEST_NPM=1 HEXA_TEST_CARGO_NET=1 cargo test -p hexa-cli --test an_adapter_is_generated_from_the_contract 2>&1 | tail -3` at f6c9388 with uncommitted changes on 2026-10-09 23:48 UTC:

```text

test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.30s
```
