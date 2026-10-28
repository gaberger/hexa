//! Secondary adapters — driven by the application.
//!
//! Rule 4: an adapter imports `ports/` only, never another adapter.

pub mod chrono_calendar;
pub mod json_file_store;

pub use chrono_calendar::ChronoCalendar;
pub use json_file_store::JsonFileStore;
