# ringbuf

A fixed-capacity ring buffer, written in Rust in ports-and-adapters style.

## What it is

A shelf with a fixed number of slots. You put values in at the back and take
them out from the front, oldest first. When the shelf is full, a new value
pushes the oldest value off the shelf, and `push` hands that old value back to
you.

Picture a photo album with 5 pages. You can never add a page. You always take
the front photo out first. When you add photo 6, photo 1 comes out and you hold
it in your hand.

That is the whole product. Nothing else. There is no disk, no thread, no lock
and no dependency.

## How to run it

See the shelf at work:

```sh
./run.sh
```

Check that everything still works:

```sh
./gate.sh
```

## The API

```rust
use ringbuf::{CapacityError, RingBuffer};

let mut buf = RingBuffer::new(3).unwrap();   // 0 slots is an error, not a panic
assert_eq!(buf.push(1), None);               // a free slot was used
assert_eq!(buf.push(2), None);
assert_eq!(buf.push(3), None);
assert!(buf.is_full());
assert_eq!(buf.push(4), Some(1));            // the shelf was full, so 1 fell off
assert_eq!(buf.pop(), Some(2));              // the oldest value comes out first
assert_eq!(buf.len(), 2);
assert_eq!(buf.capacity(), 3);               // this number never changes
```

`new` returns a `Result`. A capacity of 0 gives `CapacityError::Zero`, because a
buffer with 0 slots is empty and full at the same time. A capacity the machine
cannot hold gives `CapacityError::TooLarge`. Neither one panics.

`push` returns the value that fell off, so nothing disappears in silence.

## How it works inside

The buffer holds three fields:

| Field | Meaning |
|---|---|
| `slots` | The shelf, a `Box<[Option<T>]>`. Its length is the capacity. |
| `head` | The slot number of the oldest value. |
| `len` | How many values are in the buffer now. |

A head and a tail would be the common choice, but then an empty shelf and a full
shelf look the same. People patch that with a wasted slot or a flag, and the
classic off-by-one bug lives in the patch. With `head` and `len`, `is_empty` is
`len == 0` and `is_full` is `len == capacity`. Both are exact at every boundary,
and a capacity of 1 needs no special case.

`Option<T>` in each slot means every live value dies exactly once when the
buffer dies. That is correct cleanup for free, with no `unsafe`.

## The layers

```
src/
  lib.rs            the composition root
  domain/ring.rs    the buffer and its rules
  ports/mod.rs      empty on purpose
```

The ports layer declares nothing. A port is a hole in the wall for a thing the
domain needs from outside, and this domain needs nothing from outside: no clock,
no logger, no storage, no metrics hook. An empty port layer is the correct
answer here, not a gap. The test `ports_declares_nothing` holds that line by
reading the file and failing on any declaration.

There is no adapters directory, because there is nothing to adapt.

## Threads

There are none, and that is a decision.

1. `push` and `pop` take `&mut self`, so only one caller can touch the buffer at
   a time. A data race cannot compile.
2. The crate has `#![forbid(unsafe_code)]` and holds no atomic, no `UnsafeCell`,
   no `RefCell` and no `Mutex`.
3. The four accessors take `&self`, so `len`, `capacity`, `is_empty` and
   `is_full` always agree with each other and with the slots.
4. `RingBuffer<T>` is `Send` when `T` is `Send`, and `Sync` when `T` is `Sync`.

If you want to share one buffer between threads, write `Mutex<RingBuffer<T>>` in
your own code. This crate does not ship that wrapper. Overwrite-on-full plus
many readers is a dangerous mix, and it needs its own decision record and its
own gate.

## The tests

15 tests, all in `tests/`, so they see only the public API.

| File | Covers |
|---|---|
| `boundaries.rs` | capacity 0, capacity 1, pop on empty, push on full, wrap, the exactness table, the fixed capacity, the huge capacity |
| `wrap.rs` | the 21-step scripted run, 1000 overwrites, the invariants after every operation |
| `model.rs` | 30000 random steps against a 4-line model written from the contract |
| `drops.rs` | 100 values die exactly once each |
| `structure.rs` | the ports layer is empty; the buffer is `Send` and `Sync` |

One rule for every test: a test may **read** `head()` to prove a wrap happened,
but a test may **never compute** what a slot number should be. A test that
copies the position maths only proves the code agrees with itself.

`head()` and `check_invariants()` exist only with `--features internals`. They
are behind a real Cargo feature, not behind `#[cfg(test)]`, because a test in
`tests/` cannot see a `cfg(test)` item of the library.

Three tests need that feature, so `cargo test` alone runs 13 of the 15. Run the
whole suite like this:

```sh
cargo test --features internals
```
