//! Every habit, at most one per name key (ADR-2610281223 §3).

use std::collections::{BTreeMap, BTreeSet};

use super::day::Day;
use super::habit_name::HabitName;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Habit {
    pub name: HabitName,
    pub days: BTreeSet<Day>,
}

/// Two entries share a key; both spellings are named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateName {
    pub first: String,
    pub second: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HabitBook {
    habits: BTreeMap<String, Habit>,
}

impl HabitBook {
    pub fn from_entries(entries: Vec<(HabitName, Vec<Day>)>) -> Result<HabitBook, DuplicateName> {
        let mut habits: BTreeMap<String, Habit> = BTreeMap::new();
        for (name, days) in entries {
            if let Some(existing) = habits.get(name.key()) {
                return Err(DuplicateName {
                    first: existing.name.display().to_string(),
                    second: name.display().to_string(),
                });
            }
            let days: BTreeSet<Day> = days.into_iter().collect();
            habits.insert(name.key().to_string(), Habit { name, days });
        }
        Ok(HabitBook { habits })
    }

    /// `true` when the habit is new.
    pub fn add(&mut self, name: HabitName) -> bool {
        if self.habits.contains_key(name.key()) {
            return false;
        }
        self.habits.insert(name.key().to_string(), Habit { name, days: BTreeSet::new() });
        true
    }

    pub fn find(&self, name: &HabitName) -> Option<&Habit> {
        self.habits.get(name.key())
    }

    /// `Some(true)` new day, `Some(false)` already recorded, `None` no such habit.
    pub fn check(&mut self, name: &HabitName, day: Day) -> Option<bool> {
        self.habits.get_mut(name.key()).map(|h| h.days.insert(day))
    }

    /// `Some(true)` removed, `Some(false)` not recorded, `None` no such habit.
    pub fn uncheck(&mut self, name: &HabitName, day: Day) -> Option<bool> {
        self.habits.get_mut(name.key()).map(|h| h.days.remove(&day))
    }

    /// Whether the habit existed.
    pub fn remove(&mut self, name: &HabitName) -> bool {
        self.habits.remove(name.key()).is_some()
    }

    /// The habits in key order.
    pub fn habits(&self) -> Vec<&Habit> {
        self.habits.values().collect()
    }
}
