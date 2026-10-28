//! The calendar: the only file that names `chrono` (ADR-2610281300 §1).

use chrono::{Datelike, Local, NaiveDate};

use crate::ports::{Clock, DateText, Day};

/// Local dates through `chrono`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ChronoCalendar;

fn epoch() -> NaiveDate {
    NaiveDate::from_ymd_opt(1970, 1, 1).expect("1970-01-01 is a date")
}

fn day_from_naive(date: NaiveDate) -> Day {
    Day::from_days(date.signed_duration_since(epoch()).num_days())
}

impl DateText for ChronoCalendar {
    fn parse(&self, text: &str) -> Option<Day> {
        let b = text.as_bytes();
        if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
            return None;
        }
        let digits = |r: std::ops::Range<usize>| -> Option<u32> {
            b[r.clone()].iter().all(u8::is_ascii_digit).then(|| text[r].parse().ok()).flatten()
        };
        let (y, m, d) = (digits(0..4)?, digits(5..7)?, digits(8..10)?);
        NaiveDate::from_ymd_opt(y as i32, m, d).map(day_from_naive)
    }

    fn format(&self, day: Day) -> String {
        match epoch().checked_add_signed(chrono::Duration::days(day.days())) {
            Some(date) if (0..=9999).contains(&date.year()) => {
                format!("{:04}-{:02}-{:02}", date.year(), date.month(), date.day())
            }
            _ => format!("day {}", day.days()),
        }
    }
}

impl Clock for ChronoCalendar {
    fn today(&self) -> Day {
        day_from_naive(Local::now().date_naive())
    }
}
