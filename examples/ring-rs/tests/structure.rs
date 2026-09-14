//! Tests 14 and 15. The shape of the crate, checked by machine.

use ringbuf::RingBuffer;

/// Test 14. The ports layer declares nothing.
///
/// The challenge says the ports layer declares nothing the domain does not
/// need. The domain needs nothing from outside, so the file must hold only doc
/// lines and blank lines. This test turns that rule into a gate.
#[test]
fn ports_declares_nothing() {
    let source = include_str!("../src/ports/mod.rs");

    let banned = [
        "use ", "pub use ", "trait ", "pub trait ", "fn ", "pub fn ", "struct ", "pub struct ",
        "enum ", "pub enum ", "impl ", "mod ", "pub mod ", "type ", "const ", "static ",
    ];

    for (number, line) in source.lines().enumerate() {
        let text = line.trim_start();
        for word in banned {
            assert!(
                !text.starts_with(word),
                "src/ports/mod.rs line {} declares something: {line}",
                number + 1
            );
        }
        assert!(
            text.is_empty() || text.starts_with("//"),
            "src/ports/mod.rs line {} is not a doc line or a blank line: {line}",
            number + 1
        );
    }
}

/// Test 15. The buffer can move between threads, and can be shared behind a
/// lock that the caller owns. Rust works this out on its own, and this test
/// fails to compile if that ever stops being true.
#[test]
fn ring_buffer_is_send_and_sync() {
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}

    assert_send::<RingBuffer<i32>>();
    assert_sync::<RingBuffer<i32>>();
}
