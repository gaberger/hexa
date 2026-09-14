//! The ring buffer and its capacity error.
//!
//! The buffer holds three fields and nothing more: the slots, the slot number
//! of the oldest value (`head`), and how many values are in the buffer now
//! (`len`).
//!
//! `head` and `len` are used instead of `head` and `tail` for one reason. With
//! a head and a tail, an empty shelf and a full shelf look the same, and the
//! classic off-by-one bug lives in the patch for that. With `head` and `len`,
//! `is_empty` is `len == 0` and `is_full` is `len == capacity`. Both are exact
//! at every boundary, and a capacity of 1 needs no special case.

use core::fmt;

/// A fixed-capacity buffer. A push onto a full buffer removes the oldest value.
#[derive(Debug)]
pub struct RingBuffer<T> {
    /// The shelf. Its length is the capacity, and the length never changes.
    slots: Box<[Option<T>]>,
    /// The slot number of the oldest value. Always less than the capacity.
    head: usize,
    /// How many values are in the buffer now. Always 0 to the capacity.
    len: usize,
}

/// The reason a buffer could not be made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapacityError {
    /// The caller asked for 0 slots. A buffer with 0 slots is empty and full at
    /// the same time, which is nonsense, so it is rejected at the door.
    Zero,
    /// The machine cannot hold that many slots.
    TooLarge,
}

impl fmt::Display for CapacityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CapacityError::Zero => f.write_str("capacity must be at least 1"),
            CapacityError::TooLarge => f.write_str("capacity is too large to allocate"),
        }
    }
}

impl std::error::Error for CapacityError {}

impl<T> RingBuffer<T> {
    /// Makes a buffer with exactly `capacity` slots.
    ///
    /// Returns [`CapacityError::Zero`] for a capacity of 0, and
    /// [`CapacityError::TooLarge`] if the machine cannot hold that many slots.
    /// It does not panic, and it does not stop the process.
    ///
    /// ```
    /// use ringbuf::{CapacityError, RingBuffer};
    ///
    /// assert_eq!(RingBuffer::<i32>::new(0).unwrap_err(), CapacityError::Zero);
    /// assert_eq!(RingBuffer::<i32>::new(4).unwrap().capacity(), 4);
    /// ```
    pub fn new(capacity: usize) -> Result<Self, CapacityError> {
        if capacity == 0 {
            return Err(CapacityError::Zero);
        }
        let mut slots: Vec<Option<T>> = Vec::new();
        slots
            .try_reserve_exact(capacity)
            .map_err(|_| CapacityError::TooLarge)?;
        // A successful reservation proves `capacity <= isize::MAX`, so
        // `head + len` can never overflow a `usize` later on.
        for _ in 0..capacity {
            slots.push(None);
        }
        Ok(Self {
            slots: slots.into_boxed_slice(),
            head: 0,
            len: 0,
        })
    }

    /// Adds a value at the back.
    ///
    /// Returns the evicted value: the oldest value that this push removed
    /// because the buffer was full. Returns `None` if the buffer had a free
    /// slot. The evicted value is never lost in silence.
    pub fn push(&mut self, value: T) -> Option<T> {
        if self.len == self.slots.len() {
            // The buffer is full, so `back()` is the same slot as `head`. One
            // write takes the old value out and puts the new value in.
            let evicted = self.slots[self.head].replace(value);
            self.head = self.step(self.head);
            // `len` does not move. It is still the capacity.
            evicted
        } else {
            let back = self.back();
            self.slots[back] = Some(value);
            self.len += 1;
            None
        }
    }

    /// Removes and returns the oldest value. Returns `None` if the buffer is
    /// empty, and then nothing in the buffer changes.
    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        let value = self.slots[self.head].take();
        self.head = self.step(self.head);
        self.len -= 1;
        value
    }

    /// How many values are in the buffer now.
    pub fn len(&self) -> usize {
        self.len
    }

    /// How many slots the buffer has. This number never changes.
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// True when the buffer holds no value.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// True when every slot holds a value.
    pub fn is_full(&self) -> bool {
        self.len == self.slots.len()
    }

    /// The next slot number after `i`, around the circle.
    ///
    /// This does not use `%`. One compare is exact and cheaper.
    fn step(&self, i: usize) -> usize {
        let n = i + 1;
        if n == self.slots.len() {
            0
        } else {
            n
        }
    }

    /// The slot number of the first free slot at the back.
    fn back(&self) -> usize {
        let b = self.head + self.len;
        if b >= self.slots.len() {
            b - self.slots.len()
        } else {
            b
        }
    }
}

/// Test-only accessors. They exist only with `--features internals`.
///
/// They are behind a real Cargo feature, not behind `#[cfg(test)]`, because an
/// integration test in `tests/` cannot see a `cfg(test)` item of the library.
#[cfg(feature = "internals")]
impl<T> RingBuffer<T> {
    /// The slot number of the oldest value. A test reads this to prove that a
    /// wrap really happened. A test must never compute it.
    pub fn head(&self) -> usize {
        self.head
    }

    /// Checks the four rules that must be true after every operation. Panics
    /// with a clear message when one of them fails.
    ///
    /// 1. The slot count is the capacity, and it is at least 1.
    /// 2. `len` is 0 to the capacity.
    /// 3. `head` is less than the capacity.
    /// 4. Walk `len` slots forward from `head`. Every one holds `Some`. Every
    ///    other slot holds `None`.
    pub fn check_invariants(&self) {
        let cap = self.slots.len();
        assert!(cap >= 1, "invariant 1: capacity is {cap}, want at least 1");
        assert!(
            self.len <= cap,
            "invariant 2: len is {}, want 0 to {cap}",
            self.len
        );
        assert!(
            self.head < cap,
            "invariant 3: head is {}, want less than {cap}",
            self.head
        );

        let mut i = self.head;
        for n in 0..self.len {
            assert!(
                self.slots[i].is_some(),
                "invariant 4: slot {i} is {n} steps from head and must hold a value"
            );
            i = self.step(i);
        }
        for n in 0..(cap - self.len) {
            assert!(
                self.slots[i].is_none(),
                "invariant 4: slot {i} is {n} steps past the back and must be free"
            );
            i = self.step(i);
        }
    }
}
