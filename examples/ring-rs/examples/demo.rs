//! A shelf with three slots, in front of your eyes.
//!
//! Run it with `./run.sh`.

use ringbuf::RingBuffer;

fn main() {
    let mut buf = RingBuffer::new(3).expect("3 is a valid capacity");
    println!("a shelf with {} slots\n", buf.capacity());

    for value in 1..=5 {
        let note = match buf.push(value) {
            None => "went into a free slot".to_string(),
            Some(old) => format!("{old} fell off the shelf"),
        };
        println!("push {value} -> {note:<22} len {}", buf.len());
    }

    println!();
    while let Some(value) = buf.pop() {
        println!("pop    -> {:<22} len {}", format!("{value} came out"), buf.len());
    }
    println!("pop    -> {:<22} empty: {}", "nothing left", buf.is_empty());
}
