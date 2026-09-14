//! Test 10. The property test.
//!
//! A model is a second, very simple version of the same rules. If the buffer
//! and the model ever disagree, one of them is wrong.
//!
//! The model is written from the English contract in the spec, not from the
//! buffer code. A model copied from the code would only repeat the mistakes of
//! the code.

use ringbuf::RingBuffer;
use std::collections::VecDeque;

/// A small random number generator with a fixed start value, so a failure
/// repeats in the same way every time.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

/// The model. Keep it this short so any reader can check it by eye.
///
/// `cap` is carried as a separate number on purpose. `VecDeque::capacity()` is
/// not the number you asked for, and a `VecDeque` grows.
fn model_push(m: &mut VecDeque<i32>, cap: usize, v: i32) -> Option<i32> {
    let evicted = if m.len() == cap { m.pop_front() } else { None };
    m.push_back(v);
    evicted
}

#[test]
fn model_matches_the_oracle() {
    for cap in 1..=6usize {
        let mut rng = Rng(0x5eed_1234_abcd_0001 ^ cap as u64);
        let mut buf = RingBuffer::new(cap).unwrap();
        let mut model: VecDeque<i32> = VecDeque::new();

        for step in 0..5000 {
            let roll = rng.next();
            if roll.is_multiple_of(3) {
                let got = buf.pop();
                let want = model.pop_front();
                assert_eq!(got, want, "cap {cap} step {step}: pop disagreed");
            } else {
                let value = (roll % 1_000_000) as i32;
                let got = buf.push(value);
                let want = model_push(&mut model, cap, value);
                assert_eq!(got, want, "cap {cap} step {step}: push disagreed");
            }

            assert_eq!(buf.len(), model.len(), "cap {cap} step {step}: len disagreed");
            assert_eq!(
                buf.is_empty(),
                model.is_empty(),
                "cap {cap} step {step}: is_empty disagreed"
            );
            assert_eq!(
                buf.is_full(),
                model.len() == cap,
                "cap {cap} step {step}: is_full disagreed"
            );
            assert_eq!(buf.capacity(), cap, "cap {cap} step {step}: capacity moved");
        }

        // Drain both and compare the whole order, not only the counts.
        let mut from_buf = Vec::new();
        while let Some(value) = buf.pop() {
            from_buf.push(value);
        }
        let from_model: Vec<i32> = model.into_iter().collect();
        assert_eq!(from_buf, from_model, "cap {cap}: the drained order disagreed");
    }
}
