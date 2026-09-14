//! A fixed-capacity ring buffer.
//!
//! The buffer is a shelf with a fixed number of slots. You put values in at the
//! back and take them out from the front. When the shelf is full, a new value
//! pushes the oldest value off the shelf, and `push` hands that old value back
//! to you.
//!
//! # There is no concurrency here, and that is a decision
//!
//! `push` and `pop` take `&mut self`, so the borrow checker permits only one
//! caller at a time. The crate has `#![forbid(unsafe_code)]` and holds no
//! atomic, no `UnsafeCell`, no `RefCell` and no `Mutex`. A data race cannot
//! compile.
//!
//! `RingBuffer<T>` is `Send` when `T` is `Send`, and `Sync` when `T` is `Sync`.
//! A caller who wants to share one buffer between threads writes
//! `Mutex<RingBuffer<T>>` in their own code. This crate does not ship that
//! wrapper.
//!
//! # Example
//!
//! ```
//! use ringbuf::RingBuffer;
//!
//! let mut buf = RingBuffer::new(3).unwrap();
//! assert_eq!(buf.push(1), None);
//! assert_eq!(buf.push(2), None);
//! assert_eq!(buf.push(3), None);
//! assert!(buf.is_full());
//!
//! // The shelf is full, so the oldest value comes back out.
//! assert_eq!(buf.push(4), Some(1));
//! assert_eq!(buf.pop(), Some(2));
//! ```

#![forbid(unsafe_code)]

pub mod domain;
pub mod ports;

pub use domain::ring::{CapacityError, RingBuffer};
