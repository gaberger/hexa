# A URL shortener

Build a working URL shortener in TypeScript, runnable with `bun`, in this
directory.

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

**Caching**: resolutions are served from an in-memory cache, so a repeated
resolve does not read the file.

Use no package dependencies beyond what `bun` provides.

## Definition of done

`./gate.sh .` exits 0. It runs the program from the outside only.
