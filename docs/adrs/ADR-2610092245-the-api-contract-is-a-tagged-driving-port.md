---
id: ADR-2610092245
status: accepted
date: 2026-10-09
---
# ADR-2610092245: The API contract is a tagged driving port

**Status:** Accepted
**Date:** 2026-10-09
**Drivers:** A hexa project's primary adapters are its API boundary, and nothing hexa produces describes that boundary to a client. `examples/linkstore-svc` serves four routes, and the only record of their shape is an axum router and a hand-written `wire.rs`. Encore shows that tagging the code produces a spec that cannot fall behind it. Encore does this for Go only, and it tags the handler. hexa scaffolds Rust, Go and TypeScript, and in a hexagonal project the handler is not where the contract lives.

## Context

Encore tags the function that *is* the endpoint (`//encore:api public method=GET path=/x`), derives schemas from that function's Go types, and generates the server around it. That works because in Encore the handler is the application.

In a ports-and-adapters project, the handler belongs to an adapter. It turns HTTP into a port call and a port answer into a status code (`linkstore-svc/src/adapters/primary/http.rs` says this in its first comment). What the outside world may ask the application to do is already written down once, as a **driving port**: `BookmarkApi` in `ports/`, with `ApiError` as a typed enum *so that* the adapter never picks a status code by reading a message. If we tag the handler, the contract ends up in the layer that is meant to be replaceable, and in a different framework idiom for each language. If we tag the port, it is one place, in one layer, written the same way in all three languages.

hexa has three things it can build on:

- **A comment annotation already exists.** `@hexa:public`, which the tree-sitter adapter reads from the preceding comment (`has_hex_public_annotation`). A comment tag works the same way in Rust, Go and TypeScript. It needs no macro crate, no decorator runtime and no build step in the user's project, and it adds no dependency to the code hexa scaffolds.
- **The parser already reads ports.** `AstPort::extract_members` returns each trait's or interface's method names, and the port re-exports its value types (`pub use crate::domain::… as …Value`, `type Count = domain.Count`, `export { Count }`). That is the path a schema resolver follows.
- **The grade cannot tell a driving port from a driven one.** `HexLayer` has a single `Ports` layer. A tag would give the analyzer the primary/secondary distinction it does not have today.

## Decision

1. **The tag goes on the driving port, never on a handler.** One comment tag, `@hexa:api`, in the doc comment of a trait or interface in the `ports` layer and of each of its methods:

   ```rust
   /// @hexa:api service=bookmarks version=1.0.0
   pub trait BookmarkApi: Send + Sync {
       /// Save a link, or merge into the one saved under the same URL.
       /// @hexa:api POST /bookmarks 201
       fn create(&self, req: NewBookmark) -> Result<BookmarkValue, ApiError>;
       /// @hexa:api GET /bookmarks/{id}
       fn get(&self, id: &str) -> Result<BookmarkValue, ApiError>;
   }

   pub enum ApiError {
       /// @hexa:status 400
       Invalid(String),
       /// @hexa:status 404
       NotFound,
       Unavailable,            // untagged: 500
   }
   ```

   ```go
   // @hexa:api service=bookmarks version=1.0.0
   type BookmarkAPI interface {
       // @hexa:api GET /bookmarks/{id}
       Get(ctx context.Context, id string) (Bookmark, error)
   }
   ```

   ```ts
   /** @hexa:api service=bookmarks version=1.0.0 */
   export interface BookmarkApi {
     /** @hexa:api GET /bookmarks/{id} */
     get(id: string): Promise<Bookmark>;
   }
   ```

   The grammar is `@hexa:api <METHOD> <path> [<success-status>]` on a method and `@hexa:api key=value…` on the interface. The doc text above the tag becomes the operation's `description`. Methods without a tag are not exposed, so a port can be partly public.

2. **Parameters are bound by rule, not by annotation.** A parameter whose name matches a `{segment}` in the path is a path parameter. For `GET`, `DELETE` and `HEAD`, the remaining parameters are query parameters. For `POST`, `PUT` and `PATCH`, a single remaining parameter of a named type is the request body, and several remaining parameters are combined into one body object. A context or receiver parameter (`ctx context.Context`, `&self`) is not part of the API. A `{segment}` with no matching parameter, or a body on a `GET`, is an error that names the file and line.

3. **Returns and errors come from the signature.** The success type is unwrapped from `Result<T, E>` (Rust), `(T, error)` (Go), or `Promise<T>` / `T` (TypeScript). `()`, a bare `error` and `void` become `204` unless the tag gives another status. Each variant of the port's error enum declares its status with `@hexa:status`. An untagged variant becomes `500`. Go and TypeScript have no closed error enum, so there `@hexa:status` is written as a line on the method (`@hexa:status 404 not found`) instead.

4. **Schemas are resolved from the types the port speaks, and an unresolvable type is an error.** The resolver follows the port's re-exports to each named struct or interface, its fields and their wire names (serde `rename`/`rename_all`, Go `json:"…"` tags, and TypeScript property names as written), and emits them as `components/schemas`. A newtype (`struct Title(String)`) is emitted as the type it wraps. `Option<T>`, `*T` and `T | undefined` / `?:` make a field optional, and sequences become arrays. A type the resolver cannot reach becomes a hard error naming where it is used. It is never emitted as `{}`, because a schema that silently accepts anything is a gate that has degraded quietly.

5. **One intermediate model, one renderer.** Parsing produces a language-neutral `ApiContract` (interfaces, operations, parameters, type declarations). OpenAPI 3.1 is rendered from that model by a pure function. The cross-language claim then becomes testable: one contract written in three languages must produce the same document.

6. **The tag is architecture, so the grade reads it.**
   - A `@hexa:api` tag outside the `ports` layer is a rule error. The contract belongs to the port, and a tag on a handler is the Encore habit this decision rejects.
   - A tagged port that no primary adapter names is a finding: the API is declared but not served. A driving port is *implemented* by the application and *driven* by a primary adapter, so the check is whether some file in `adapters/primary` refers to the port. It counts with the unused ports, under the same cap.
   - A malformed tag or an unresolvable type costs grade the same way a rule error does (ADR-2609211430). A contract that cannot be read is not a contract.
   - A tagged port is a **driving** port. This is the first place the analyzer can tell a driving port from a driven one, and later detectors may rely on it.

7. **The verb is `hexa api`.**
   - `hexa api spec [path] [--out <file>] [--format json|yaml]` writes the document. The default is `openapi.json` at the project root. YAML uses the `serde_yaml` that `hexa-cli` already depends on, so this adds **no new dependency**.
   - `hexa api check [path]` exits 0 when the committed spec matches what the tags produce now and 1 when it has drifted, printing the operations that differ. It exits 2 when there are no tagged operations. An empty contract is a vacuous gate, and a vacuous gate is a failed gate. The comparison leaves out `info.x-generated-by`, which names the hexa version that wrote the file, so upgrading hexa is not drift.
   - `hexa ci` gains an API contract gate: skipped when nothing is tagged, failed when the contract cannot be read, and failed when a committed document that hexa wrote no longer matches the tags. A document another tool wrote is not hexa's to check.
   - `hexa api list` prints one line per operation: method, path, port, method name and file:line.

## Consequences

- **The contract is written once, in the layer that does not change when the framework does.** Moving from axum to actix, or from `net/http` to chi, does not touch the spec.
- **The spec cannot drift silently.** If a port changes without the spec being regenerated, `hexa api check` fails in `hexa ci`.
- **Numbers are as precise as the language.** Rust and Go integers become `integer`; TypeScript has only `number`, so a TypeScript port says `number`. The same port in three languages agrees exactly only where the languages do.
- **Ports become slightly richer.** A port whose methods take `&str` because the adapter should not build domain values (the `linkstore-svc` choice) gets a schema of plain strings, which is accurate. A port that wants a request body names a request type in `ports/`. That type is a value type, so rule 2 already allows it.
- **What this does not check: that the adapter serializes what the port declares.** Static analysis can see that a primary adapter drives the port. It cannot see that the JSON the adapter emits matches the schema. Closing that gap needs a generated contract test run against the live adapter. That is phase 2 below and needs its own gate.
- **`examples/linkstore-svc` is not tagged, and that is the gap above made concrete.** Its port returns the domain `Bookmark`, whose fields are private domain values (a `Timestamp`, a `NormalisedUrl`). What clients receive is `BookmarkResponse` in the adapter's `wire.rs`, with `created_at` as milliseconds. That split is deliberate there: the example keeps JSON names out of the domain. Tagging that port would publish a schema the server does not send. A project that wants its port to be its contract writes the port in wire-shaped value types, as the gate's fixtures do. One that keeps wire shapes in the adapter waits for phase 2.
- **No route check against the framework.** Reading axum, `net/http`, Express or Hono routers to confirm that each tagged route is mounted would mean one parser per framework. This decision does not do that, and says so in the output instead of implying it.

## Phases

1. **Contract and spec** (this decision's gate): the tag grammar, extraction for all three languages, schema resolution, the OpenAPI 3.1 renderer, and `hexa api spec | check | list`.
2. **Contract test**: `hexa api test --base-url <url>` exercises each operation against a running adapter and checks response status and shape against the spec. This closes the serialization gap above.
3. **Adapter from contract**: `hexa scaffold --api` and `hexa generate` emit a primary HTTP adapter skeleton that implements the tagged port (axum, `net/http`, Hono), which is Encore's generated server rebuilt as a replaceable adapter. Framework names live in the scaffold templates, the same way `hexa-cli/assets/scaffold` names tokio today. Choosing those frameworks needs its own ADR.

## Implementation

- `hexa-analysis/src/domain.rs`: what the parser reads (`ApiFacts`, `ApiPortDecl`, `ApiMethodDecl`, `TypeDecl`, `TypeRef`), the resolved contract (`ApiContract`, `ApiOperation`, `ApiParam`, `ApiSchema`), `ApiFindings` on `ArchAnalysisResult`, and `lower_camel`, the one spelling for names across languages.
- `hexa-analysis/src/ports.rs`: `AstPort::extract_api(path, source, lang) -> ApiFacts`, with an empty default so other adapters compile unchanged.
- `hexa-analysis/src/treesitter_api.rs` (adapters/secondary): tag reading, signature lowering per language, and wire names. A tag is a comment line that begins with the tag, so prose that mentions one is not a tag.
- `hexa-analysis/src/api_contract.rs` (usecases): resolves names across files, applies the binding rules, collects every error with its file and line, and reports unserved ports. It takes the file list from its caller, so it does not depend on the analyzer that calls it.
- `hexa-analysis/src/openapi.rs` (usecases): `render(&ApiContract, generator) -> serde_json::Value`, a pure function.
- `hexa-analysis/src/analyzer.rs`: the findings join the result; contract errors count with the rule errors, unserved ports with the unused ports.
- `hexa-cli/src/commands/api.rs`: the verb. `hexa analyze` prints and serializes the findings; `hexa ci` runs the gate.

**Gate**, written before the code: `cargo test -p hexa-cli --test a_tagged_port_is_the_api_contract`, with fixtures under `hexa-cli/tests/fixtures/api/{rust,go,ts}/` declaring the same four-operation bookmarks port, and a hand-written `expected.openapi.json` committed before any extractor exists.

- the Rust, Go and TypeScript fixtures each produce a document equal to `expected.openapi.json` (cross-language oracle, and not derived from the code);
- every `$ref` in the output resolves, and `openapi` is `3.1.0`;
- an untagged method in a tagged port is absent (control);
- a `{id}` with no `id` parameter fails and names the file and line;
- a field of an unresolvable type fails and names the file and line. No `{}` schema is emitted;
- a `@hexa:api` tag in `adapters/primary/` is a rule error in `hexa analyze --json`;
- a tagged port that no primary adapter names is reported, and stops being reported once one does;
- `hexa api check` exits 0 on a fresh spec, 1 after a method's path changes, and 2 on a project with no tags;
- `hexa api list` prints one line per operation with its file and line;
- `hexa ci` passes on a fresh committed document and fails after the port drifts;
- `hexa analyze .` on hexa itself still grades A+ with 100% coverage.

## References

- ADR-2609121400: the gate replaces the written spec. Here the OpenAPI document is generated output. The tag is the source, and `api check` is the gate.
- ADR-2609211430: a rule error costs grade. A malformed or misplaced tag is one.
- ADR-2609241707: a grade cannot exceed the code it could classify. A tag that cannot be resolved is code the grade could not read.
- ADR-2609221900: what generated this is written when it is generated. The spec's `info` carries the hexa version that produced it.

## Evidence

`cargo test -p hexa-cli --test a_tagged_port_is_the_api_contract 2>&1 | tail -3` at 4ba5501 with uncommitted changes on 2026-10-09 23:20 UTC:

```text

test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.09s
```
