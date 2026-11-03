//! Use cases — the application's verbs.
//!
//! as a parameter and never chooses which adapter fills it; that choice
//! belongs to the composition root alone.

pub mod bookmarks;

use crate::domain::Count;
use crate::ports::CounterStore;

/// Advance the count by one and return the new value.
pub fn increment(store: &mut dyn CounterStore) -> Count {
    let next = store.load().next();
    store.save(next);
    next
}
