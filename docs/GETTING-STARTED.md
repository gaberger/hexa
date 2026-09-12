# Getting started

hexa is one binary. It needs a Rust toolchain to build and an inference server to
run. Nothing else has to be installed or started.

## Build

```bash
git clone https://github.com/gaberger/hexa.git
cd hexa
cargo build -p hexa-cli --release
export PATH="$PWD/target/release:$PATH"
```

`hexa --help` lists the verbs. `hexa --version` confirms the build.

## Set up inference

`hexa bootstrap` checks prerequisites, starts the local inference server if one is
installed, pulls the models the project configures, and writes `.hexa/project.json`.

```bash
hexa bootstrap --dry-run    # show what would happen
hexa bootstrap              # do it
```

| Flag | Effect |
|---|---|
| `--skip-models` | Do not pull models. They load on first use instead. |
| `--skip-prereq` | Skip OS checks. For CI. |
| `--force` | Restart the inference server even if it is running. |
| `--profile ci` | Use a frontier API instead of a local server. Needs an API key in the environment. |

The models it pulls come from `.hexa/project.json`. There is no built-in list. A
project that configures no models gets none, and `hexa bootstrap` says so.

A frontier model needs no server. `hexa scaffold`, `hexa build` and `hexa harden`
delegate to a logged-in `claude` CLI, so `claude --version` is the only check.
`hexa do` uses the local server first and the frontier path as a fallback.

[Inference](INFERENCE.md) covers providers, tiers, keys and benchmarking.

## Scaffold a project

The floor comes first. It is deterministic and needs no model.

```bash
hexa init ./myapp --scaffold --lang rust
cd myapp && cargo test
```

That gives you a manifest, the layer directories, four passing tests, and a
`.hexa/ADR-rules.toml` that `hexa analyze` will run from now on. Rust, Go and
TypeScript are supported.

Then build your project onto it. This step uses the frontier path.

```bash
hexa scaffold "A bookmark service: SQLite store, HTTP API, tag search" \
  --target ./myapp --lang rust --grade A
```

The command exits nonzero unless both gates pass: the test command, and an
architecture grade of A or better.

## Make one gated change

```bash
hexa do run "make add() return a + b, not a - b" \
  --file src/lib.rs --evidence "cargo test --test add"
```

hexa edits the file, runs the evidence command, and commits only if it exits 0.
Otherwise the edit is reverted. `hexa do runs` lists recent runs and their
verdicts.

## Check the shape

```bash
hexa analyze .                # grade, boundary violations, rule violations
hexa analyze . --exit-code    # nonzero on any violation, for CI
hexa graph build .            # build the code graph
hexa graph consumers <path>   # who depends on this, before you delete it
```

The scan covers every source file under the project. Vendored code, generated
output and template data go in `analyze.exclude` in `.hexa/project.json`, as
paths relative to the project root:

```json
{ "analyze": { "exclude": ["vendor", "gen"] } }
```

The analyzer has no built-in list of directory names to skip. What a project
excludes is the project's decision, and it is visible in the project's config.

## Read next

| Document | Covers |
|---|---|
| [README](../README.md) | The problem hexa addresses and the evidence |
| [ARCHITECTURE](../ARCHITECTURE.md) | The crates, the loop, the rules |
| [Development workflow](guides/development-workflow.md) | The gate-first pipeline, step by step |
| [Inference](INFERENCE.md) | Providers, tiers, benchmarking |
| [Evidence](EVIDENCE.md) | Every claim, with the command that checks it |
| [Glossary](reference/glossary.md) | The vocabulary, precisely |
