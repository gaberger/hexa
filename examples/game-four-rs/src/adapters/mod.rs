//! The outside world. Each adapter imports `ports` only: never the domain
//! directly, and never another adapter.

pub mod primary;
pub mod secondary;
