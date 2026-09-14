# url-shortener-rs

A hexagonal skeleton in Rust: a URL shortener behind a port.

This is a shape, not a product. It exists to show where each kind of code goes
and to be graded by `hexa analyze`.

## What is here

| Directory | Holds |
|---|---|
| `src/domain/` | the value types. No dependencies. |
| `src/ports/` | the traits the use cases need. |
| `src/adapters/` | the implementations of those traits. |
| `src/usecases/` | the operations, written against ports only. |
| `src/lib.rs` | the composition root — the only file that names an adapter. |

## Test it

```bash
cargo test
```

There is no binary. The library and its tests are the whole example.

## Grade it

```bash
hexa analyze .
```
