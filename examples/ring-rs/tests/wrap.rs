//! The wrap tests. Two of them read `head()`, so they need the `internals`
//! feature. Run them with `cargo test --features internals`.

use ringbuf::RingBuffer;

#[cfg(feature = "internals")]
#[derive(Debug)]
enum Op {
    Push(i32),
    Pop,
}

/// Test 6. The scripted run. This is the most important test in the suite.
///
/// The buffer has 3 slots. The script crosses the wrap point four times, and it
/// crosses in three different ways: while popping, while evicting on a full
/// buffer, and while emptying the buffer. Every column of the table is checked
/// after every step.
#[cfg(feature = "internals")]
#[test]
fn two_wrap_crossings() {
    // (operation, returns, head, len, is_empty, is_full)
    let script: [(Op, Option<i32>, usize, usize, bool, bool); 21] = [
        (Op::Push(1), None, 0, 1, false, false),
        (Op::Push(2), None, 0, 2, false, false),
        (Op::Push(3), None, 0, 3, false, true),
        (Op::Pop, Some(1), 1, 2, false, false),
        (Op::Pop, Some(2), 2, 1, false, false),
        (Op::Push(4), None, 2, 2, false, false),
        (Op::Push(5), None, 2, 3, false, true),
        (Op::Push(6), Some(3), 0, 3, false, true),
        (Op::Pop, Some(4), 1, 2, false, false),
        (Op::Pop, Some(5), 2, 1, false, false),
        (Op::Pop, Some(6), 0, 0, true, false),
        (Op::Pop, None, 0, 0, true, false),
        (Op::Push(7), None, 0, 1, false, false),
        (Op::Push(8), None, 0, 2, false, false),
        (Op::Push(9), None, 0, 3, false, true),
        (Op::Push(10), Some(7), 1, 3, false, true),
        (Op::Push(11), Some(8), 2, 3, false, true),
        (Op::Push(12), Some(9), 0, 3, false, true),
        (Op::Pop, Some(10), 1, 2, false, false),
        (Op::Pop, Some(11), 2, 1, false, false),
        (Op::Pop, Some(12), 0, 0, true, false),
    ];

    let mut buf = RingBuffer::new(3).unwrap();
    buf.check_invariants();

    let mut crossings = 0;
    let mut previous_head = buf.head();

    for (number, (op, returns, head, len, empty, full)) in script.into_iter().enumerate() {
        let step = number + 1;
        let got = match op {
            Op::Push(value) => buf.push(value),
            Op::Pop => buf.pop(),
        };

        assert_eq!(got, returns, "step {step} returned the wrong value");
        assert_eq!(buf.head(), head, "step {step} left head in the wrong slot");
        assert_eq!(buf.len(), len, "step {step} left the wrong len");
        assert_eq!(buf.is_empty(), empty, "step {step} got is_empty wrong");
        assert_eq!(buf.is_full(), full, "step {step} got is_full wrong");
        assert_eq!(buf.capacity(), 3, "step {step} moved the capacity");
        buf.check_invariants();

        // A wrap is the one moment when head moves back to a smaller slot.
        if buf.head() < previous_head {
            crossings += 1;
        }
        previous_head = buf.head();
    }

    assert_eq!(crossings, 4, "the script must cross the wrap point four times");
}

/// Test 8. A long overwrite run. The buffer holds 8 values, and the last 8
/// pushed values are the ones that come out.
#[test]
fn heavy_overwrite_order() {
    let mut buf = RingBuffer::new(8).unwrap();

    for value in 1..=1000 {
        buf.push(value);
        if value >= 8 {
            assert_eq!(buf.len(), 8, "len moved after push {value}");
            assert!(buf.is_full(), "buffer is not full after push {value}");
        }
    }

    let mut drained = Vec::new();
    while let Some(value) = buf.pop() {
        drained.push(value);
    }
    assert_eq!(drained, (993..=1000).collect::<Vec<i32>>());
}

/// Test 9. The four rules hold after every single push and pop, for four
/// different capacities.
#[cfg(feature = "internals")]
#[test]
fn invariants_hold_after_every_operation() {
    for capacity in [1usize, 2, 3, 7] {
        let mut buf = RingBuffer::new(capacity).unwrap();
        buf.check_invariants();

        // Fill past the capacity, drain part way, fill again, then drain all the
        // way. This crosses the wrap point for every capacity in the list.
        for value in 0..(capacity * 3 + 2) {
            buf.push(value);
            buf.check_invariants();
        }
        for _ in 0..(capacity + 1) {
            buf.pop();
            buf.check_invariants();
        }
        for value in 0..(capacity * 2) {
            buf.push(value);
            buf.check_invariants();
        }
        while buf.pop().is_some() {
            buf.check_invariants();
        }
        buf.check_invariants();
        assert!(buf.is_empty(), "capacity {capacity} did not end empty");
    }
}
