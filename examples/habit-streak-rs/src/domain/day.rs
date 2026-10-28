//! A calendar day as a count of days since 1970-01-01 (ADR-2610281223 §1).

/// A day. Two days are consecutive when their counts differ by 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Day(i64);

impl Day {
    pub fn from_days(days: i64) -> Day {
        Day(days)
    }

    pub fn days(self) -> i64 {
        self.0
    }

    /// The day before.
    pub fn pred(self) -> Day {
        Day(self.0 - 1)
    }
}
