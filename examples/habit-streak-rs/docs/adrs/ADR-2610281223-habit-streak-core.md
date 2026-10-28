---
id: ADR-2610281223
status: proposed
date: 2026-10-28
---
# ADR-2610281223: habit-streak core: days, names, the habit book, streaks and the use cases

**Status:** Proposed
**Date:** 2026-10-28
**Drivers:** The design `docs/designs/design_draft.md`, reviewed by the goldfish at sha256 `a2f1422f3638a353a433fd112a2ac768c7e012c6d87d97b2a29108f5ae26cd10` (`docs/designs/goldfish_review_report.md`), with no Blocker in its second review.
**Design:** docs/designs/design_draft.md

## Context

A person keeps a daily habit and wants two numbers per habit: the current streak and the longest streak. The design splits the tool into a pure core and its edges. This ADR decides the core: the domain, the ports the use cases need, and the use cases. The JSON file store, the calendar adapter and the command line are a later ADR. The scaffold's counter example is removed.

## Decision

1. **A `Day` is a day count.** `Day` in `src/domain/day.rs` is a copyable, ordered newtype over an `i64` count of days since 1970-01-01. `Day::from_days(n)` builds one, `days()` reads it back, and `pred()` is the day before. The domain has no month or year logic: two days are consecutive when their counts differ by 1.
2. **A `HabitName` is parsed, never assumed.** `HabitName::parse(raw)` in `src/domain/habit_name.rs` trims surrounding whitespace. It returns `Err(NameError::Empty)` when nothing is left, and `Err(NameError::ControlCharacter)` when the trimmed text holds any control character. A parsed name keeps its trimmed spelling, `display()`, and a comparison key, `key()`, which is the spelling lowercased with `str::to_lowercase`. Two names are equal when their keys are equal.
3. **The `HabitBook` holds every habit, at most one per key.** `HabitBook` in `src/domain/habit_book.rs` maps a name key to a `Habit { name: HabitName, days: BTreeSet<Day> }`.
   - `add(name)` returns `true` when the habit is new and `false` when one with the same key exists; it never changes an existing habit.
   - `find(&name)` returns the habit with that key, or `None`.
   - `check(&name, day)` returns `Some(true)` when the day is new, `Some(false)` when it was already recorded, and `None` when there is no such habit. A day is recorded at most once.
   - `uncheck(&name, day)` returns `Some(true)` when it removed the day, `Some(false)` when the day was not recorded, and `None` when there is no such habit.
   - `remove(&name)` returns whether the habit existed, and removes it.
   - `habits()` returns the habits in key order.
   - `HabitBook::from_entries(entries)` builds a book from `(HabitName, Vec<Day>)` pairs. It fails with `DuplicateName { first, second }`, naming both spellings, when two entries share a key; repeated days within one entry collapse to one.
4. **`streaks` is a pure function of the recorded days and today.** `streaks(days, today)` in `src/domain/streaks.rs` returns `Streaks { current, longest, checked_today }`. Days after `today` are ignored by all three. `longest` is the length of the longest run of consecutive days. `current` is the length of the run of consecutive days that ends at `today` when `today` is recorded, else at `today.pred()` when that is recorded (the grace rule: a streak stays alive until the end of the day after the last check-in), else 0. `checked_today` says whether `today` is recorded. With no days, all three are zero or false.
5. **The ports are the store and the clock.** `src/ports/mod.rs` declares `HabitStore` with `fn load(&self) -> Result<HabitBook, StoreError>` and `fn save(&self, book: &HabitBook) -> Result<(), StoreError>`, and `Clock` with `fn today(&self) -> Day`. Every method takes `&self`. `StoreError` is `NotFound`, `Malformed(String)`, `UnknownVersion(u64)` or `Io(String)`, each displayed as one line. The ports module re-exports `Day`, `HabitName`, `NameError`, `Habit`, `HabitBook`, `DuplicateName` and `Streaks`, so an adapter imports nothing but the port.
6. **The use cases are one `Habits` value over the ports.** The driving port `HabitCommands` in `src/ports/mod.rs` declares one method per command, with its outcome and error types beside it. `Habits<'a> { store: &'a dyn HabitStore, clock: &'a dyn Clock }` in `src/usecases/mod.rs` implements it; it is built as `Habits::new(&store, &clock)`, borrowing a store and a clock the caller owns, or with the struct literal. Tests reach it as `habit_streak::usecases::Habits` and the port as `habit_streak::ports::HabitCommands`. Each loads the book first and returns the first error; only a method that changed the book saves it, once.
   - `add(raw)`: `Added { created: bool }` or `AlreadyExists`. A `NotFound` store is an empty book, and then `created` is `true`.
   - `check(raw, day: Option<Day>)`: `Recorded(day)` or `AlreadyRecorded(day)`; `None` means `clock.today()`. A day after today is `UseCaseError::FutureDate { day, today }`, and nothing is saved.
   - `uncheck(raw, day)`: `Removed(day)` or `NotRecorded(day)`.
   - `remove(raw)`: `RemovedHabit`.
   - `status(raw: Option<&str>)`: a `Vec<HabitStatus { name, current, longest, checked_today }>`, one per habit in key order, or the one habit named.
   - Errors: `UseCaseError::InvalidName(NameError)`, `UnknownHabit { name, known: Vec<String> }` for every method but `add` when the key is absent (`known` lists display names in key order), `FutureDate`, and `Store(StoreError)`. Every method other than `add` treats `NotFound` as an empty book, so naming a habit then gives `UnknownHabit`.
7. **The scaffold's counter is gone.** `Count`, `CounterStore`, `CountValue`, `InMemoryCounterStore`, `increment`, `counter()`, `increment_once()` and `tests/counter.rs` are deleted. `src/lib.rs` stays the composition root and declares the modules; it names no adapter until the edges ADR adds them.

## Consequences

- The core is testable with in-memory doubles for `HabitStore` and `Clock`; no file, clock or terminal is needed.
- Calendar dates, the JSON format and the command line are decided by the edges ADR, against these ports.
- Case-folding by `str::to_lowercase` treats `Read` and `read` as one habit; the book refuses a file that holds both.

## Implementation

`src/domain/day.rs`, `src/domain/habit_name.rs`, `src/domain/habit_book.rs`, `src/domain/streaks.rs`, `src/domain/mod.rs`, `src/ports/mod.rs`, `src/usecases/mod.rs`, `src/lib.rs`.

## Gate

`hexa spec verify ADR-2610281223`: the spec's sealed tests, each red before the code.

## References

- `docs/designs/design_draft.md`: the reviewed design, sections Domain, Ports and Use cases.
- `docs/designs/goldfish_review_report.md`: its second review.
