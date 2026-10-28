//! Domain — value objects and entities. Pure data, no I/O.
//!
//! Rule 1: `domain/` imports only `domain/`.

pub mod day;
pub mod habit_book;
pub mod habit_name;
pub mod streaks;

pub use day::Day;
pub use habit_book::{DuplicateName, Habit, HabitBook};
pub use habit_name::{HabitName, NameError};
pub use streaks::{streaks, Streaks};
