//! The ports layer is empty on purpose.
//!
//! A port is a hole in the wall of the domain, for a thing the domain needs
//! from outside. A fixed-capacity ring buffer needs nothing from outside. It
//! needs no clock, no logger, no storage, no metrics hook and no allocator
//! hook.
//!
//! So this layer declares nothing. An empty port layer is the correct answer
//! here, not a gap. The test `ports_declares_nothing` in `tests/structure.rs`
//! holds this line.
//!
//! There is no adapters directory either, because there is nothing to adapt.
