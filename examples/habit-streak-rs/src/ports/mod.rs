//! Ports — the contracts between layers (ADR-2610281223 §5).
//!
//! Rule 2: `ports/` imports `domain/` only, for value types.

/// Re-exported so an adapter needs to import nothing but this module.
pub use crate::domain::{Day, DuplicateName, Habit, HabitBook, HabitName, NameError, Streaks};

/// Why the book could not be loaded or saved. Each displays as one line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    NotFound,
    Malformed(String),
    UnknownVersion(u64),
    Io(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            StoreError::NotFound => "the data file does not exist".to_string(),
            StoreError::Malformed(why) => format!("the data file is malformed: {why}"),
            StoreError::UnknownVersion(v) => format!("the data file has unknown version {v}"),
            StoreError::Io(why) => format!("the data file could not be read or written: {why}"),
        };
        // One line, whatever the payload holds.
        f.write_str(&text.replace(['\r', '\n'], " "))
    }
}

/// Where the book is kept.
pub trait HabitStore {
    fn load(&self) -> Result<HabitBook, StoreError>;
    fn save(&self, book: &HabitBook) -> Result<(), StoreError>;
}

/// What `add` did (ADR-2610281223 §6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddOutcome {
    Added { created: bool },
    AlreadyExists,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckOutcome {
    Recorded(Day),
    AlreadyRecorded(Day),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UncheckOutcome {
    Removed(Day),
    NotRecorded(Day),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoveOutcome {
    RemovedHabit,
}

/// One habit's line of `status`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HabitStatus {
    pub name: String,
    pub current: u32,
    pub longest: u32,
    pub checked_today: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UseCaseError {
    InvalidName(NameError),
    UnknownHabit { name: String, known: Vec<String> },
    FutureDate { day: Day, today: Day },
    Store(StoreError),
}

/// The driving port: one method per command. `usecases::Habits` implements it.
pub trait HabitCommands {
    fn add(&self, raw: &str) -> Result<AddOutcome, UseCaseError>;
    fn check(&self, raw: &str, day: Option<Day>) -> Result<CheckOutcome, UseCaseError>;
    fn uncheck(&self, raw: &str, day: Day) -> Result<UncheckOutcome, UseCaseError>;
    fn remove(&self, raw: &str) -> Result<RemoveOutcome, UseCaseError>;
    fn status(&self, raw: Option<&str>) -> Result<Vec<HabitStatus>, UseCaseError>;
}

/// What day it is.
pub trait Clock {
    fn today(&self) -> Day;
}

/// The strict `YYYY-MM-DD` text form of a day (ADR-2610281300 §1).
pub trait DateText {
    /// `None` for anything that is not exactly a real date as `YYYY-MM-DD`.
    fn parse(&self, text: &str) -> Option<Day>;
    fn format(&self, day: Day) -> String;
}
