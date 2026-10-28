//! Use cases — the application's verbs (ADR-2610281223 §6).
//!
//! Rule 3: `usecases/` imports `domain/` and `ports/` only.

use crate::domain::{streaks, Day, HabitBook, HabitName};
use crate::ports::{
    AddOutcome, CheckOutcome, Clock, HabitCommands, HabitStatus, HabitStore, RemoveOutcome, StoreError,
    UncheckOutcome, UseCaseError,
};

/// The commands, over the store and the clock.
pub struct Habits<'a> {
    pub store: &'a dyn HabitStore,
    pub clock: &'a dyn Clock,
}

impl<'a> Habits<'a> {
    pub fn new(store: &'a dyn HabitStore, clock: &'a dyn Clock) -> Habits<'a> {
        Habits { store, clock }
    }

    /// The book, and whether the store had none (`NotFound` is an empty book).
    fn load(&self) -> Result<(HabitBook, bool), UseCaseError> {
        match self.store.load() {
            Ok(book) => Ok((book, false)),
            Err(StoreError::NotFound) => Ok((HabitBook::default(), true)),
            Err(e) => Err(UseCaseError::Store(e)),
        }
    }

    fn save(&self, book: &HabitBook) -> Result<(), UseCaseError> {
        self.store.save(book).map_err(UseCaseError::Store)
    }
}

fn parse(raw: &str) -> Result<HabitName, UseCaseError> {
    HabitName::parse(raw).map_err(UseCaseError::InvalidName)
}

/// The book's habit for `name`, or `UnknownHabit` listing the known ones.
fn unknown(book: &HabitBook, name: &HabitName) -> UseCaseError {
    UseCaseError::UnknownHabit {
        name: name.display().to_string(),
        known: book.habits().iter().map(|h| h.name.display().to_string()).collect(),
    }
}

impl HabitCommands for Habits<'_> {
    fn add(&self, raw: &str) -> Result<AddOutcome, UseCaseError> {
        let (mut book, _) = self.load()?;
        let name = parse(raw)?;
        if !book.add(name) {
            return Ok(AddOutcome::AlreadyExists);
        }
        self.save(&book)?;
        // `created`: a new habit was created and saved. A missing store is
        // an empty book, so it is true there too (ADR-2610281223 §6).
        Ok(AddOutcome::Added { created: true })
    }

    fn check(&self, raw: &str, day: Option<Day>) -> Result<CheckOutcome, UseCaseError> {
        let (mut book, _) = self.load()?;
        let name = parse(raw)?;
        if book.find(&name).is_none() {
            return Err(unknown(&book, &name));
        }
        let today = self.clock.today();
        let day = day.unwrap_or(today);
        if day > today {
            return Err(UseCaseError::FutureDate { day, today });
        }
        if book.check(&name, day) == Some(true) {
            self.save(&book)?;
            Ok(CheckOutcome::Recorded(day))
        } else {
            Ok(CheckOutcome::AlreadyRecorded(day))
        }
    }

    fn uncheck(&self, raw: &str, day: Day) -> Result<UncheckOutcome, UseCaseError> {
        let (mut book, _) = self.load()?;
        let name = parse(raw)?;
        match book.uncheck(&name, day) {
            None => Err(unknown(&book, &name)),
            Some(true) => {
                self.save(&book)?;
                Ok(UncheckOutcome::Removed(day))
            }
            Some(false) => Ok(UncheckOutcome::NotRecorded(day)),
        }
    }

    fn remove(&self, raw: &str) -> Result<RemoveOutcome, UseCaseError> {
        let (mut book, _) = self.load()?;
        let name = parse(raw)?;
        if !book.remove(&name) {
            return Err(unknown(&book, &name));
        }
        self.save(&book)?;
        Ok(RemoveOutcome::RemovedHabit)
    }

    fn status(&self, raw: Option<&str>) -> Result<Vec<HabitStatus>, UseCaseError> {
        let (book, _) = self.load()?;
        let name = raw.map(parse).transpose()?;
        let today = self.clock.today();
        let line = |h: &crate::domain::Habit| {
            let s = streaks(&h.days, today);
            HabitStatus {
                name: h.name.display().to_string(),
                current: s.current,
                longest: s.longest,
                checked_today: s.checked_today,
            }
        };
        match name {
            None => Ok(book.habits().into_iter().map(line).collect()),
            Some(n) => book.find(&n).map(|h| vec![line(h)]).ok_or_else(|| unknown(&book, &n)),
        }
    }
}
