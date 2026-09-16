# Challenge: a URL shortener, ports and adapters

Build a working URL shortener in TypeScript, runnable with `bun`.

## Behaviour it must have

**HTTP API**, started by `bun run src/main.ts serve --port <n>`:
- `POST /shorten` with JSON `{"url":"https://example.com/x"}` returns
  `201` and JSON `{"code":"<short code>"}`.
- `GET /<code>` returns `302` with a `Location` header equal to the
  original URL.
- `GET /<unknown>` returns `404`.
- `POST /shorten` with a non-http(s) URL returns `400`.

**CLI**, same entry point:
- `bun run src/main.ts shorten <url>` prints the short code on stdout.
- `bun run src/main.ts resolve <code>` prints the original URL on stdout,
  and exits non-zero printing nothing to stdout if the code is unknown.

**Persistence**: codes survive a restart. State lives in a file under a
directory given by the `STORE_DIR` environment variable.

**Caching**: resolutions are served from an in-memory cache in front of the
file store. The cache must be a separate implementation behind the same
contract as the file store, not a field inside it.

## Architecture it must have

Ports and adapters (hexagonal). Every arm is held to this identically:

1. `src/domain/` imports only from `src/domain/`.
2. `src/ports/` imports only from `src/domain/`.
3. `src/usecases/` imports only from `src/domain/` and `src/ports/`.
4. `src/adapters/primary/` and `src/adapters/secondary/` import only from
   `src/ports/`. Not from `src/domain/`, not even a type-only import of a
   value object: re-export what an adapter needs through its port.
5. No adapter imports another adapter.
6. Exactly one composition root (`src/main.ts`) imports from adapters.
7. All relative imports use `.js` extensions (NodeNext).

The HTTP API and the CLI are two primary adapters. The file store and the
in-memory cache are two secondary adapters behind one port.

## Definition of done

`./gate.sh <your directory>` exits 0. That script is the ground truth. It is
black box: it only runs your program from the outside. It does not inspect
your source and does not care how you structure it. The architecture rules
above are checked separately and are not part of the gate.


---

## Correction, 2026-09-16

Rule 4 above originally read "`src/ports/` (and `src/domain/` value types)".
That parenthesis was wrong. `hexa analyze` forbids adapter-to-domain imports
outright, with no value-type exception, so the brief contradicted the
instrument that scores it and all three trial arms were marked down for
obeying the brief. The first run's architecture comparison was voided by it
(`docs/analysis/2609152230-build-trial-results.md`).

Rule 4 now matches the analyzer. Anyone rerunning this trial gets a brief that
agrees with the thing measuring them.
