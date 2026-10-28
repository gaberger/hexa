//! Current and longest streaks (ADR-2610281223 §4).

use std::collections::BTreeSet;

use super::day::Day;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Streaks {
    pub current: u32,
    pub longest: u32,
    pub checked_today: bool,
}

/// Days after `today` are ignored. `current` ends at `today`, else at
/// `today.pred()` (the grace rule), else is 0.
pub fn streaks(days: &BTreeSet<Day>, today: Day) -> Streaks {
    let past = || days.range(..=today);
    let mut longest = 0u32;
    let mut run = 0u32;
    let mut prev: Option<Day> = None;
    for &d in past() {
        run = match prev {
            Some(p) if p.days().checked_add(1) == Some(d.days()) => run + 1,
            _ => 1,
        };
        longest = longest.max(run);
        prev = Some(d);
    }
    let checked_today = days.contains(&today);
    let start = if checked_today {
        Some(today)
    } else if days.contains(&today.pred()) {
        Some(today.pred())
    } else {
        None
    };
    let mut current = 0u32;
    if let Some(mut day) = start {
        while days.contains(&day) {
            current += 1;
            if day.days() == i64::MIN {
                break;
            }
            day = day.pred();
        }
    }
    Streaks { current, longest, checked_today }
}
