//! The boundary tests. They use only the public API, with no extra features.
//!
//! Rule for every test in this crate: a test may read `head()` to prove that a
//! wrap happened, but a test must never compute what a slot number should be.

use ringbuf::{CapacityError, RingBuffer};

/// Test 1. A capacity of 0 is nonsense, so `new` refuses it. It does not panic.
#[test]
fn capacity_zero_is_rejected() {
    let made = RingBuffer::<i32>::new(0);
    assert_eq!(made.unwrap_err(), CapacityError::Zero);
}

/// Test 2. One slot, the smallest buffer there is, through a full cycle.
#[test]
fn capacity_one_full_cycle() {
    let mut buf = RingBuffer::new(1).unwrap();

    assert_eq!(buf.push("a"), None);
    assert!(buf.is_full());
    assert_eq!(buf.len(), 1);

    assert_eq!(buf.push("b"), Some("a"));
    assert_eq!(buf.len(), 1);
    assert!(buf.is_full());

    assert_eq!(buf.pop(), Some("b"));
    assert_eq!(buf.pop(), None);
    assert!(buf.is_empty());
    assert_eq!(buf.len(), 0);
    assert_eq!(buf.capacity(), 1);
}

/// Test 3. A pop on an empty buffer moves no counter. The later order proves it.
#[test]
fn pop_on_empty_changes_nothing() {
    let mut buf = RingBuffer::<i32>::new(3).unwrap();

    assert_eq!(buf.pop(), None);
    assert_eq!(buf.pop(), None);
    assert_eq!(buf.pop(), None);

    assert_eq!(buf.len(), 0);
    assert!(buf.is_empty());
    assert!(!buf.is_full());

    assert_eq!(buf.push(1), None);
    assert_eq!(buf.push(2), None);
    assert_eq!(buf.push(3), None);

    assert_eq!(buf.pop(), Some(1));
    assert_eq!(buf.pop(), Some(2));
    assert_eq!(buf.pop(), Some(3));
    assert_eq!(buf.pop(), None);
}

/// Test 4. A push onto a full buffer hands the evicted value back.
#[test]
fn push_on_full_returns_the_evicted_value() {
    let mut buf = RingBuffer::new(3).unwrap();

    assert_eq!(buf.push(1), None);
    assert_eq!(buf.push(2), None);
    assert_eq!(buf.push(3), None);
    assert!(buf.is_full());

    assert_eq!(buf.push(4), Some(1));
    assert_eq!(buf.len(), 3);
    assert!(buf.is_full());

    assert_eq!(buf.pop(), Some(2));
    assert_eq!(buf.pop(), Some(3));
    assert_eq!(buf.pop(), Some(4));
    assert_eq!(buf.pop(), None);
}

/// Test 5. The order survives a wrap.
#[test]
fn wrap_around_keeps_order() {
    let mut buf = RingBuffer::new(4).unwrap();
    for value in 1..=6 {
        buf.push(value);
    }

    let mut drained = Vec::new();
    while let Some(value) = buf.pop() {
        drained.push(value);
    }
    assert_eq!(drained, vec![3, 4, 5, 6]);
}

/// Test 7. The four accessors are exact at every length, for every small
/// capacity. This covers length 0, length 1, one below the capacity, and the
/// capacity itself.
#[test]
fn boundary_exactness_table() {
    for capacity in 1..=5 {
        let mut buf = RingBuffer::new(capacity).unwrap();

        assert_eq!(buf.len(), 0, "capacity {capacity} at length 0");
        assert_eq!(buf.capacity(), capacity);
        assert!(buf.is_empty(), "capacity {capacity} at length 0");
        assert!(!buf.is_full(), "capacity {capacity} at length 0");

        for length in 1..=capacity {
            buf.push(length);

            assert_eq!(buf.len(), length, "capacity {capacity} at length {length}");
            assert_eq!(buf.capacity(), capacity);
            assert!(!buf.is_empty(), "capacity {capacity} at length {length}");
            assert_eq!(
                buf.is_full(),
                length == capacity,
                "capacity {capacity} at length {length}"
            );
        }
    }
}

/// Test 12. The capacity never changes, whatever the caller does.
#[test]
fn capacity_never_changes() {
    let mut buf = RingBuffer::new(5).unwrap();

    for step in 0..1000 {
        // A fixed mix of pushes and pops. Step 0 to 2 push, step 3 pops, and
        // the pattern repeats, so the buffer fills, wraps and drains many times.
        if step % 4 == 3 {
            buf.pop();
        } else {
            buf.push(step);
        }
        assert_eq!(buf.capacity(), 5, "capacity moved at step {step}");
        assert!(buf.len() <= 5, "len passed the capacity at step {step}");
    }
}

/// Test 13. A capacity the machine cannot hold is an error, not a dead process.
#[test]
fn huge_capacity_is_an_error_not_an_abort() {
    let made = RingBuffer::<u64>::new(usize::MAX);
    assert_eq!(made.unwrap_err(), CapacityError::TooLarge);
}
