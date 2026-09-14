//! Test 11. Every value dies exactly once.
//!
//! A value that falls off the shelf must not be lost twice and must not be lost
//! zero times. `Box<[Option<T>]>` gives this for free, and this test proves it.

use ringbuf::RingBuffer;
use std::cell::RefCell;
use std::rc::Rc;

/// A value that writes down its own id when it dies.
struct Tracked {
    id: usize,
    deaths: Rc<RefCell<Vec<usize>>>,
}

impl Drop for Tracked {
    fn drop(&mut self) {
        self.deaths.borrow_mut().push(self.id);
    }
}

#[test]
fn drops_are_exact_after_the_buffer_dies() {
    let deaths: Rc<RefCell<Vec<usize>>> = Rc::new(RefCell::new(Vec::new()));

    let mut buf = RingBuffer::new(8).unwrap();
    for id in 0..100 {
        let evicted = buf.push(Tracked {
            id,
            deaths: Rc::clone(&deaths),
        });
        // The 92 evicted values come back to the caller and die here.
        drop(evicted);
    }

    assert_eq!(buf.len(), 8);
    assert_eq!(deaths.borrow().len(), 92, "92 values must have been evicted");

    // The last 8 values die with the buffer. The count before this line proves
    // nothing about them.
    drop(buf);

    let died = deaths.borrow();
    assert_eq!(died.len(), 100, "every value must die exactly once");

    let mut sorted = died.clone();
    sorted.sort_unstable();
    let want: Vec<usize> = (0..100).collect();
    assert_eq!(sorted, want, "every id must appear exactly once");
}
