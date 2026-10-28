//! Sealed tests for ADR-2610281223: days, names, the habit book, streaks and the use cases.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use habit_streak::domain::day::Day;
use habit_streak::domain::habit_book::{DuplicateName, HabitBook};
use habit_streak::domain::habit_name::{HabitName, NameError};
use habit_streak::domain::streaks::{streaks, Streaks};
use habit_streak::ports::{
    AddOutcome, CheckOutcome, Clock, HabitCommands, HabitStatus, HabitStore, RemoveOutcome,
    StoreError, UncheckOutcome, UseCaseError,
};
use habit_streak::usecases::Habits;

// ---------------------------------------------------------------- helpers

fn d(n: i64) -> Day {
    Day::from_days(n)
}

fn nm(s: &str) -> HabitName {
    HabitName::parse(s).expect("test name parses")
}

fn days_of(ns: &[i64]) -> BTreeSet<Day> {
    ns.iter().map(|&n| d(n)).collect()
}

fn book_of(entries: &[(&str, &[i64])]) -> HabitBook {
    HabitBook::from_entries(
        entries
            .iter()
            .map(|(name, ds)| (nm(name), ds.iter().map(|&n| d(n)).collect()))
            .collect(),
    )
    .expect("test book builds")
}

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
}

struct RecStore {
    book: RefCell<Result<HabitBook, StoreError>>,
    save_result: Result<(), StoreError>,
    calls: RefCell<Vec<&'static str>>,
    saved: RefCell<Vec<HabitBook>>,
}

impl RecStore {
    fn with(book: HabitBook) -> RecStore {
        RecStore {
            book: RefCell::new(Ok(book)),
            save_result: Ok(()),
            calls: RefCell::new(Vec::new()),
            saved: RefCell::new(Vec::new()),
        }
    }
    fn failing_load(e: StoreError) -> RecStore {
        RecStore {
            book: RefCell::new(Err(e)),
            save_result: Ok(()),
            calls: RefCell::new(Vec::new()),
            saved: RefCell::new(Vec::new()),
        }
    }
    fn saves(&self) -> usize {
        self.calls.borrow().iter().filter(|c| **c == "save").count()
    }
    fn last_saved(&self) -> HabitBook {
        self.saved.borrow().last().cloned().expect("a book was saved")
    }
}

impl HabitStore for RecStore {
    fn load(&self) -> Result<HabitBook, StoreError> {
        self.calls.borrow_mut().push("load");
        self.book.borrow().clone()
    }
    fn save(&self, book: &HabitBook) -> Result<(), StoreError> {
        self.calls.borrow_mut().push("save");
        self.saved.borrow_mut().push(book.clone());
        if self.save_result.is_ok() {
            *self.book.borrow_mut() = Ok(book.clone());
        }
        self.save_result.clone()
    }
}

struct FixedClock(Day);

impl Clock for FixedClock {
    fn today(&self) -> Day {
        self.0
    }
}

fn displays(book: &HabitBook) -> Vec<String> {
    book.habits().iter().map(|h| h.name.display().to_string()).collect()
}

fn status_line(name: &str, current: u32, longest: u32, checked_today: bool) -> HabitStatus {
    HabitStatus {
        name: name.to_string(),
        current,
        longest,
        checked_today,
    }
}

fn run_ending(past: &BTreeSet<i64>, end: i64) -> u32 {
    let mut count = 0;
    let mut x = end;
    while past.contains(&x) {
        count += 1;
        x -= 1;
    }
    count
}

fn longest_run(past: &BTreeSet<i64>) -> u32 {
    let mut best = 0;
    for &x in past {
        if !past.contains(&(x - 1)) {
            let mut len = 0;
            let mut y = x;
            while past.contains(&y) {
                len += 1;
                y += 1;
            }
            best = best.max(len);
        }
    }
    best
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                rust_files(&p, out);
            } else if p.extension().map(|e| e == "rs").unwrap_or(false) {
                out.push(p);
            }
        }
    }
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn contains_word(text: &str, word: &str) -> bool {
    let mut start = 0;
    while let Some(pos) = text[start..].find(word) {
        let at = start + pos;
        let before_ok = text[..at].chars().next_back().map(|c| !is_ident_char(c)).unwrap_or(true);
        let after = at + word.len();
        let after_ok = text[after..].chars().next().map(|c| !is_ident_char(c)).unwrap_or(true);
        if before_ok && after_ok {
            return true;
        }
        start = at + word.len().max(1);
    }
    false
}

// ---------------------------------------------------------------- §1 Day

/// S01: A Day round-trips its count, and pred is the day before.
#[test]
fn s01_day_round_trips_and_pred_is_day_before() {
    let day = Day::from_days(20000);
    assert_eq!(day.days(), 20000);
    assert_eq!(day.pred(), Day::from_days(19999));
}

/// S02: Days order by their count, including across what would be a month or year boundary.
#[test]
fn s02_days_order_by_count() {
    let a = Day::from_days(-1);
    let b = Day::from_days(0);
    let c = Day::from_days(365);
    assert!(a < b);
    assert!(b < c);
    assert!(a < c);
    assert_eq!(Day::from_days(0).pred(), Day::from_days(-1));
}

/// S03: Days whose counts differ by more than 1 are not consecutive.
#[test]
fn s03_days_two_apart_are_not_consecutive() {
    let s = streaks(&days_of(&[10, 12]), d(12));
    assert_eq!(s.longest, 1);
    assert_eq!(s.current, 1);
}

/// S04: A Day round-trips any count, and pred always subtracts exactly one.
#[test]
fn s04_day_round_trip_property() {
    let mut rng = Rng::new(4);
    let mut samples: Vec<i64> = vec![
        i64::MIN + 1,
        i64::MIN + 2,
        -1,
        0,
        1,
        365,
        i64::MAX,
        i64::MAX - 1,
    ];
    for _ in 0..2000 {
        let n = rng.next() as i64;
        if n > i64::MIN {
            samples.push(n);
        }
        let small = (rng.below(2001) as i64) - 1000;
        samples.push(small);
    }
    for (i, &n) in samples.iter().enumerate() {
        let day = Day::from_days(n);
        assert_eq!(day.days(), n);
        assert_eq!(day.pred().days(), n - 1);
        assert!(day.pred() < day);
        let m = samples[(i * 7 + 3) % samples.len()];
        assert_eq!(Day::from_days(n) == Day::from_days(m), n == m, "n={n} m={m}");
        assert_eq!(Day::from_days(n), Day::from_days(n));
        if n < i64::MAX {
            assert_ne!(Day::from_days(n), Day::from_days(n + 1));
        }
    }
}

// ---------------------------------------------------------------- §2 HabitName

/// S05: Parsing trims the raw name, keeps the trimmed spelling, and lowercases the key.
#[test]
fn s05_parse_trims_and_lowercases_key() {
    let name = HabitName::parse("  Read Book ").expect("parses");
    assert_eq!(name.display(), "Read Book");
    assert_eq!(name.key(), "read book");
}

/// S06: Names that differ only in case are equal.
#[test]
fn s06_names_differing_in_case_are_equal() {
    let a = HabitName::parse("Read").expect("parses");
    let b = HabitName::parse("rEAD").expect("parses");
    assert_eq!(a, b);
    assert_eq!(a.display(), "Read");
    assert_eq!(b.display(), "rEAD");
}

/// S07: An empty or whitespace-only name is refused.
#[test]
fn s07_empty_or_whitespace_name_is_refused() {
    assert_eq!(HabitName::parse("").unwrap_err(), NameError::Empty);
    assert_eq!(HabitName::parse(" \t ").unwrap_err(), NameError::Empty);
}

/// S08: A control character inside the trimmed name is refused.
#[test]
fn s08_control_character_is_refused() {
    for raw in ["re\u{0}ad", "a\u{7}b", "line\u{1b}x"] {
        assert_eq!(
            HabitName::parse(raw).unwrap_err(),
            NameError::ControlCharacter,
            "raw {raw:?}"
        );
    }
}

/// S09: Parsing behaves the same way over every input string.
#[test]
fn s09_parse_property() {
    let alphabet: Vec<char> = vec![
        'a', 'Z', 'r', 'E', ' ', '\t', '\n', '\u{0}', '\u{7}', '\u{1b}', '\u{7f}', '\u{85}',
        '\u{a0}', '\u{2028}', 'İ', 'Σ', 'é', 'ß', '-', '1',
    ];
    let mut rng = Rng::new(9);
    let mut inputs: Vec<String> = vec![String::new(), " ".into(), "\u{85}x\u{85}".into()];
    for _ in 0..3000 {
        let len = rng.below(7) as usize;
        let s: String = (0..len).map(|_| *rng.pick(&alphabet)).collect();
        inputs.push(s);
    }
    for raw in &inputs {
        let trimmed = raw.trim();
        let got = HabitName::parse(raw);
        if trimmed.is_empty() {
            assert_eq!(got.unwrap_err(), NameError::Empty, "raw {raw:?}");
        } else if trimmed.chars().any(|c| c.is_control()) {
            assert_eq!(got.unwrap_err(), NameError::ControlCharacter, "raw {raw:?}");
        } else {
            let name = got.unwrap_or_else(|e| panic!("raw {raw:?} refused: {e:?}"));
            assert_eq!(name.display(), trimmed);
            assert_eq!(name.key(), trimmed.to_lowercase());
        }
    }
}

/// S10: Name equality is key equality.
#[test]
fn s10_name_equality_is_key_equality() {
    let alphabet: Vec<char> = vec!['a', 'A', 'b', 'B', ' ', 'İ', 'i', 'ß', 'Σ', 'σ'];
    let mut rng = Rng::new(10);
    let mut raws: Vec<String> = Vec::new();
    for _ in 0..400 {
        let len = 1 + rng.below(3) as usize;
        let s: String = (0..len).map(|_| *rng.pick(&alphabet)).collect();
        if HabitName::parse(&s).is_ok() {
            raws.push(s);
        }
    }
    let mut equal_pairs = 0;
    for a in &raws {
        for b in raws.iter().take(60) {
            let na = HabitName::parse(a).unwrap();
            let nb = HabitName::parse(b).unwrap();
            let expected = a.trim().to_lowercase() == b.trim().to_lowercase();
            if expected {
                equal_pairs += 1;
            }
            assert_eq!(na == nb, expected, "a={a:?} b={b:?}");
        }
    }
    assert!(equal_pairs > 0);
}

// ---------------------------------------------------------------- §3 HabitBook

/// S11: Adding a new habit and then finding it.
#[test]
fn s11_add_then_find() {
    let mut book = HabitBook::default();
    assert!(book.add(nm("Read")));
    let habit = book.find(&nm("read")).expect("found");
    assert_eq!(habit.name.display(), "Read");
    assert!(habit.days.is_empty());
    assert_eq!(book.habits().len(), 1);
}

/// S12: Checking, unchecking and removing report what happened.
#[test]
fn s12_check_uncheck_remove_report() {
    let mut book = HabitBook::default();
    assert!(book.add(nm("Read")));
    let read = nm("Read");
    assert_eq!(book.check(&read, d(5)), Some(true));
    assert_eq!(book.check(&read, d(5)), Some(false));
    assert_eq!(book.uncheck(&read, d(5)), Some(true));
    assert_eq!(book.uncheck(&read, d(5)), Some(false));
    assert!(book.remove(&read));
    assert!(book.find(&read).is_none());
}

/// S13: habits() returns the habits in key order, regardless of insertion order or case.
#[test]
fn s13_habits_in_key_order() {
    let mut book = HabitBook::default();
    assert!(book.add(nm("walk")));
    assert!(book.add(nm("Apple")));
    assert!(book.add(nm("banana")));
    assert_eq!(displays(&book), vec!["Apple", "banana", "walk"]);
}

/// S14: from_entries collapses repeated days within one entry.
#[test]
fn s14_from_entries_collapses_repeated_days() {
    let book = HabitBook::from_entries(vec![(nm("Read"), vec![d(3), d(3), d(1)])]).expect("ok");
    assert_eq!(book.find(&nm("Read")).expect("found").days, days_of(&[1, 3]));
}

/// S15: Adding a habit whose key already exists is refused, and the existing habit is left unchanged.
#[test]
fn s15_duplicate_add_is_refused() {
    let mut book = book_of(&[("Read", &[1])]);
    assert!(!book.add(nm("READ")));
    let habits = book.habits();
    assert_eq!(habits.len(), 1);
    assert_eq!(habits[0].name.display(), "Read");
    assert_eq!(habits[0].days, days_of(&[1]));
}

/// S16: Operations on an absent habit return None or false and leave the book unchanged.
#[test]
fn s16_absent_habit_operations() {
    let mut book = book_of(&[("Read", &[])]);
    let before = book.clone();
    let walk = nm("Walk");
    assert_eq!(book.check(&walk, d(1)), None);
    assert_eq!(book.uncheck(&walk, d(1)), None);
    assert!(book.find(&walk).is_none());
    assert!(!book.remove(&walk));
    assert_eq!(book, before);
}

/// S17: from_entries refuses two entries with the same key and names both spellings.
#[test]
fn s17_from_entries_refuses_duplicate_key() {
    let got = HabitBook::from_entries(vec![(nm("Read"), vec![d(1)]), (nm("read"), vec![d(2)])]);
    assert_eq!(
        got.unwrap_err(),
        DuplicateName {
            first: "Read".to_string(),
            second: "read".to_string(),
        }
    );
}

/// S18: The book holds at most one habit per key, and each day at most once, under any sequence of operations.
#[test]
fn s18_book_invariants_property() {
    let names = ["a", "A", "b", "B", " c", "C", "Σ", "σ"];
    for seed in 0..60u64 {
        let mut rng = Rng::new(1000 + seed);
        let mut book = HabitBook::default();
        for _ in 0..80 {
            let name = nm(rng.pick(&names));
            let day = d(rng.below(6) as i64);
            match rng.below(4) {
                0 => {
                    let existed = book.find(&name).is_some();
                    assert_eq!(book.add(name.clone()), !existed);
                }
                1 => {
                    let before = book.find(&name).map(|h| h.days.contains(&day));
                    let got = book.check(&name, day);
                    if got == Some(true) {
                        assert_eq!(before, Some(false), "Some(true) for a day already present");
                    }
                    match before {
                        None => assert_eq!(got, None),
                        Some(true) => assert_eq!(got, Some(false)),
                        Some(false) => assert_eq!(got, Some(true)),
                    }
                    if got.is_some() {
                        assert!(book.find(&name).unwrap().days.contains(&day));
                    }
                }
                2 => {
                    let before = book.find(&name).map(|h| h.days.contains(&day));
                    assert_eq!(book.uncheck(&name, day), before);
                }
                _ => {
                    let existed = book.find(&name).is_some();
                    assert_eq!(book.remove(&name), existed);
                    assert!(book.find(&name).is_none());
                }
            }
            let keys: Vec<String> = book.habits().iter().map(|h| h.name.key().to_string()).collect();
            for w in keys.windows(2) {
                assert!(w[0] < w[1], "keys not strictly sorted: {keys:?}");
            }
        }
    }
}

// ---------------------------------------------------------------- §4 streaks

/// S19: Current and longest streaks when today is recorded.
#[test]
fn s19_streaks_when_today_recorded() {
    assert_eq!(
        streaks(&days_of(&[1, 2, 3, 7, 8]), d(8)),
        Streaks {
            current: 2,
            longest: 3,
            checked_today: true
        }
    );
}

/// S20: The grace rule keeps yesterday's run alive.
#[test]
fn s20_grace_rule() {
    assert_eq!(
        streaks(&days_of(&[5, 6, 7]), d(8)),
        Streaks {
            current: 3,
            longest: 3,
            checked_today: false
        }
    );
}

/// S21: No days give all zeros.
#[test]
fn s21_no_days_all_zero() {
    assert_eq!(
        streaks(&BTreeSet::new(), d(100)),
        Streaks {
            current: 0,
            longest: 0,
            checked_today: false
        }
    );
}

/// S22: A run that ended two or more days ago is not current.
#[test]
fn s22_old_run_not_current() {
    let s = streaks(&days_of(&[1, 2, 3]), d(5));
    assert_eq!(s.current, 0);
    assert_eq!(s.longest, 3);
    assert!(!s.checked_today);
}

/// S23: Days after today count toward none of the three values.
#[test]
fn s23_future_days_ignored() {
    assert_eq!(
        streaks(&days_of(&[9, 10, 11, 12]), d(9)),
        Streaks {
            current: 1,
            longest: 1,
            checked_today: true
        }
    );
}

/// S24: streaks matches the rule for any days and any today.
#[test]
fn s24_streaks_property() {
    let mut rng = Rng::new(24);
    for _ in 0..3000 {
        let count = rng.below(25) as usize;
        let raw: BTreeSet<i64> = (0..count).map(|_| rng.below(30) as i64 - 5).collect();
        let today = rng.below(32) as i64 - 3;
        let days: BTreeSet<Day> = raw.iter().map(|&n| d(n)).collect();
        let past: BTreeSet<i64> = raw.iter().copied().filter(|&n| n <= today).collect();
        let past_days: BTreeSet<Day> = past.iter().map(|&n| d(n)).collect();

        let got = streaks(&days, d(today));
        assert_eq!(got.checked_today, past.contains(&today), "days {raw:?} today {today}");
        assert_eq!(got.longest, longest_run(&past), "days {raw:?} today {today}");
        let expected_current = if past.contains(&today) {
            run_ending(&past, today)
        } else if past.contains(&(today - 1)) {
            run_ending(&past, today - 1)
        } else {
            0
        };
        assert_eq!(got.current, expected_current, "days {raw:?} today {today}");
        assert!(got.current <= got.longest);
        assert_eq!(got, streaks(&past_days, d(today)));
    }
}

// ---------------------------------------------------------------- §5 ports

/// S25: In-memory doubles satisfy the ports through &self, using only types imported from the ports module.
#[test]
fn s25_in_memory_doubles_through_ports_only() {
    use habit_streak::ports::{
        Clock, Day, DuplicateName, Habit, HabitBook, HabitName, HabitStore, NameError,
        StoreError, Streaks,
    };
    use std::cell::RefCell;

    struct MemStore(RefCell<Option<HabitBook>>);
    impl HabitStore for MemStore {
        fn load(&self) -> Result<HabitBook, StoreError> {
            self.0.borrow().clone().ok_or(StoreError::NotFound)
        }
        fn save(&self, book: &HabitBook) -> Result<(), StoreError> {
            *self.0.borrow_mut() = Some(book.clone());
            Ok(())
        }
    }
    struct Fixed(Day);
    impl Clock for Fixed {
        fn today(&self) -> Day {
            self.0
        }
    }

    let mut book = HabitBook::default();
    let name: HabitName = HabitName::parse("Read").expect("parses");
    assert!(book.add(name.clone()));
    assert_eq!(book.check(&name, Day::from_days(3)), Some(true));

    let mem = MemStore(RefCell::new(None));
    let store: &dyn HabitStore = &mem;
    store.save(&book).expect("saves");
    assert_eq!(store.load().expect("loads"), book);

    let fixed = Fixed(Day::from_days(42));
    let clock: &dyn Clock = &fixed;
    assert_eq!(clock.today(), Day::from_days(42));

    let habit: &Habit = book.find(&name).expect("found");
    assert_eq!(habit.days.len(), 1);
    let _e: NameError = NameError::Empty;
    let _dup = DuplicateName {
        first: "a".to_string(),
        second: "A".to_string(),
    };
    let _s: Streaks = Streaks::default();
}

/// S26: Each StoreError variant displays as a non-empty line.
#[test]
fn s26_store_error_display() {
    let not_found = StoreError::NotFound.to_string();
    let malformed = StoreError::Malformed("bad json".into()).to_string();
    let version = StoreError::UnknownVersion(7).to_string();
    let io = StoreError::Io("denied".into()).to_string();
    for s in [&not_found, &malformed, &version, &io] {
        assert!(!s.is_empty());
    }
    assert!(malformed.contains("bad json"));
    assert!(version.contains('7'));
    assert!(io.contains("denied"));
}

/// S27: A Display string never spans more than one line, even when its payload holds a newline.
#[test]
fn s27_store_error_display_single_line() {
    let a = StoreError::Malformed("line one\nline two".into()).to_string();
    let b = StoreError::Io("a\r\nb".into()).to_string();
    for s in [&a, &b] {
        assert!(!s.contains('\n'), "{s:?}");
        assert!(!s.contains('\r'), "{s:?}");
    }
}

/// S28: Every StoreError displays as exactly one line.
#[test]
fn s28_store_error_display_property() {
    let alphabet = ['a', 'Z', ' ', '\n', '\r', '\t', '\u{0}', '\u{2028}', 'é', ':'];
    let mut rng = Rng::new(28);
    let mut errors = vec![
        StoreError::NotFound,
        StoreError::UnknownVersion(0),
        StoreError::UnknownVersion(u64::MAX),
        StoreError::Malformed(String::new()),
        StoreError::Io(String::new()),
        StoreError::Malformed("\n".into()),
        StoreError::Io("\r".into()),
    ];
    for _ in 0..1000 {
        let len = rng.below(10) as usize;
        let s: String = (0..len).map(|_| *rng.pick(&alphabet)).collect();
        errors.push(match rng.below(3) {
            0 => StoreError::Malformed(s),
            1 => StoreError::Io(s),
            _ => StoreError::UnknownVersion(rng.next()),
        });
    }
    for e in &errors {
        let s = e.to_string();
        assert!(!s.is_empty(), "{e:?}");
        assert!(!s.contains('\n') && !s.contains('\r'), "{e:?} -> {s:?}");
    }
}

// ---------------------------------------------------------------- §6 use cases

/// S29: add on a missing store creates the habit and saves once.
#[test]
fn s29_add_on_missing_store() {
    let store = RecStore::failing_load(StoreError::NotFound);
    let clock = FixedClock(d(50));
    let habits = Habits::new(&store, &clock);
    assert_eq!(habits.add("Read"), Ok(AddOutcome::Added { created: true }));
    assert_eq!(store.saves(), 1);
    let saved = store.last_saved();
    assert_eq!(displays(&saved), vec!["Read"]);
    assert!(saved.find(&nm("Read")).unwrap().days.is_empty());
}

/// S30: check with no day records today.
#[test]
fn s30_check_without_day_records_today() {
    let store = RecStore::with(book_of(&[("Read", &[])]));
    let clock = FixedClock(d(50));
    let habits = Habits::new(&store, &clock);
    assert_eq!(habits.check("read", None), Ok(CheckOutcome::Recorded(d(50))));
    assert_eq!(store.saves(), 1);
    assert_eq!(
        store.last_saved().find(&nm("Read")).unwrap().days,
        days_of(&[50])
    );
}

/// S31: Repeated check and uncheck report the outcome, and a call that changes nothing saves nothing.
#[test]
fn s31_no_change_no_save() {
    let store = RecStore::with(book_of(&[("Read", &[49])]));
    let clock = FixedClock(d(50));
    let habits = Habits::new(&store, &clock);
    assert_eq!(
        habits.check("Read", Some(d(49))),
        Ok(CheckOutcome::AlreadyRecorded(d(49)))
    );
    assert_eq!(
        habits.uncheck("Read", d(48)),
        Ok(UncheckOutcome::NotRecorded(d(48)))
    );
    assert_eq!(store.saves(), 0);
}

/// S32: status reports every habit in key order, or the one named, without saving.
#[test]
fn s32_status_all_and_one() {
    let store = RecStore::with(book_of(&[("walk", &[49, 50]), ("Apple", &[10])]));
    let clock = FixedClock(d(50));
    let habits = Habits::new(&store, &clock);
    assert_eq!(
        habits.status(None),
        Ok(vec![
            status_line("Apple", 0, 1, false),
            status_line("walk", 2, 2, true),
        ])
    );
    assert_eq!(
        habits.status(Some("WALK")),
        Ok(vec![status_line("walk", 2, 2, true)])
    );
    assert_eq!(store.saves(), 0);
}

/// S33: remove deletes the habit and saves once.
#[test]
fn s33_remove_saves_once() {
    let store = RecStore::with(book_of(&[("Read", &[]), ("Walk", &[])]));
    let clock = FixedClock(d(50));
    let habits = Habits::new(&store, &clock);
    assert_eq!(habits.remove("read"), Ok(RemoveOutcome::RemovedHabit));
    assert_eq!(store.saves(), 1);
    assert_eq!(displays(&store.last_saved()), vec!["Walk"]);
}

/// S34: A future day is refused and nothing is saved.
#[test]
fn s34_future_day_refused() {
    let store = RecStore::with(book_of(&[("Read", &[])]));
    let clock = FixedClock(d(50));
    let habits = Habits::new(&store, &clock);
    assert_eq!(
        habits.check("Read", Some(d(51))),
        Err(UseCaseError::FutureDate {
            day: d(51),
            today: d(50)
        })
    );
    assert_eq!(store.saves(), 0);
}

/// S35: Naming an absent habit returns UnknownHabit with the known names in key order.
#[test]
fn s35_unknown_habit_lists_known() {
    let store = RecStore::with(book_of(&[("walk", &[]), ("Apple", &[])]));
    let clock = FixedClock(d(50));
    let habits = Habits::new(&store, &clock);
    let expected = UseCaseError::UnknownHabit {
        name: "Run".to_string(),
        known: vec!["Apple".to_string(), "walk".to_string()],
    };
    assert_eq!(habits.check("Run", None), Err(expected.clone()));
    assert_eq!(habits.uncheck("Run", d(1)), Err(expected.clone()));
    assert_eq!(habits.remove("Run"), Err(expected.clone()));
    assert_eq!(habits.status(Some("Run")), Err(expected));
    assert_eq!(store.saves(), 0);
}

/// S36: A missing store makes every method other than add report UnknownHabit for a named habit.
#[test]
fn s36_missing_store_unknown_habit() {
    let store = RecStore::failing_load(StoreError::NotFound);
    let clock = FixedClock(d(50));
    let habits = Habits::new(&store, &clock);
    assert_eq!(
        habits.remove("Read"),
        Err(UseCaseError::UnknownHabit {
            name: "Read".to_string(),
            known: vec![],
        })
    );
    assert_eq!(store.saves(), 0);
    assert_eq!(habits.status(None), Ok(vec![]));
    assert_eq!(store.saves(), 0);
}

/// S37: Invalid names, duplicate adds and store failures are refused without saving.
#[test]
fn s37_refusals_without_saving() {
    let first = RecStore::with(book_of(&[("Read", &[])]));
    let second = RecStore::failing_load(StoreError::Io("denied".into()));
    let clock = FixedClock(d(50));

    let on_first = Habits::new(&first, &clock);
    assert_eq!(
        on_first.add("  "),
        Err(UseCaseError::InvalidName(NameError::Empty))
    );
    assert_eq!(on_first.add("READ"), Ok(AddOutcome::AlreadyExists));

    let on_second = Habits {
        store: &second,
        clock: &clock,
    };
    assert_eq!(
        on_second.add("Walk"),
        Err(UseCaseError::Store(StoreError::Io("denied".into())))
    );

    assert_eq!(first.saves(), 0);
    assert_eq!(second.saves(), 0);
}

/// S38: A save failure is returned as a Store error.
#[test]
fn s38_save_failure_is_store_error() {
    let mut store = RecStore::with(book_of(&[("Read", &[])]));
    store.save_result = Err(StoreError::Io("full".into()));
    let clock = FixedClock(d(50));
    let habits = Habits::new(&store, &clock);
    assert_eq!(
        habits.check("Read", None),
        Err(UseCaseError::Store(StoreError::Io("full".into())))
    );
}

/// S39: Over every call, load happens first and save happens at most once, and only when the book changed.
#[test]
fn s39_load_first_save_at_most_once_property() {
    let names = ["Read", "read", "Walk", "apple", "  ", "x\u{1}y", "Run"];
    for seed in 0..600u64 {
        let mut rng = Rng::new(39_000 + seed);
        let today = 50i64;

        let start = if rng.below(5) == 0 {
            None
        } else {
            let mut book = HabitBook::default();
            for candidate in ["Read", "Walk", "apple"] {
                if rng.below(2) == 0 {
                    book.add(nm(candidate));
                    for _ in 0..rng.below(4) {
                        book.check(&nm(candidate), d(46 + rng.below(6) as i64));
                    }
                }
            }
            Some(book)
        };
        let store = match &start {
            Some(book) => RecStore::with(book.clone()),
            None => RecStore::failing_load(StoreError::NotFound),
        };
        let clock = FixedClock(d(today));
        let habits = Habits::new(&store, &clock);

        let raw = *rng.pick(&names);
        let day = d(46 + rng.below(7) as i64);
        let (ok, changed) = match rng.below(7) {
            0 => {
                let r = habits.add(raw);
                (r.is_ok(), matches!(r, Ok(AddOutcome::Added { created: true })))
            }
            1 => {
                let r = habits.check(raw, Some(day));
                (r.is_ok(), matches!(r, Ok(CheckOutcome::Recorded(_))))
            }
            2 => {
                let r = habits.check(raw, None);
                (r.is_ok(), matches!(r, Ok(CheckOutcome::Recorded(_))))
            }
            3 => {
                let r = habits.uncheck(raw, day);
                (r.is_ok(), matches!(r, Ok(UncheckOutcome::Removed(_))))
            }
            4 => {
                let r = habits.remove(raw);
                (r.is_ok(), matches!(r, Ok(RemoveOutcome::RemovedHabit)))
            }
            5 => {
                let r = habits.status(Some(raw));
                (r.is_ok(), false)
            }
            _ => {
                let r = habits.status(None);
                (r.is_ok(), false)
            }
        };

        let calls = store.calls.borrow().clone();
        assert_eq!(calls.first(), Some(&"load"), "seed {seed}: calls {calls:?}");
        let saves = store.saves();
        assert!(saves <= 1, "seed {seed}: saved {saves} times");
        assert_eq!(
            saves == 1,
            ok && changed,
            "seed {seed}: raw {raw:?} ok {ok} changed {changed} calls {calls:?}"
        );
        if saves == 1 {
            let loaded = start.clone().unwrap_or_default();
            assert_ne!(store.last_saved(), loaded, "seed {seed}: saved an unchanged book");
        }
    }
}

// ---------------------------------------------------------------- §7 the counter is gone

/// S40: The crate builds and its tests run without the counter.
#[test]
fn s40_crate_builds_without_counter() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    assert!(
        !root.join("tests").join("counter.rs").exists(),
        "tests/counter.rs still exists"
    );
    let lib = std::fs::read_to_string(root.join("src").join("lib.rs")).expect("src/lib.rs reads");
    for module in ["domain", "ports", "usecases"] {
        assert!(
            lib.contains(&format!("pub mod {module};")) || lib.contains(&format!("mod {module};")),
            "src/lib.rs does not declare {module}"
        );
    }
    // Compiling this file exercises the habit API.
    assert_eq!(Day::from_days(1).days(), 1);
}

/// S41: No counter item remains, and the composition root names no adapter.
#[test]
fn s41_no_counter_items_and_core_names_no_adapter() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    let counter_test = root.join("tests").join("counter.rs");
    if counter_test.exists() {
        files.push(counter_test);
    }
    assert!(!files.is_empty());

    let words = [
        "Count",
        "CounterStore",
        "CountValue",
        "InMemoryCounterStore",
        "increment",
        "increment_once",
    ];
    for file in &files {
        let text = std::fs::read_to_string(file).expect("source reads");
        for w in words {
            assert!(
                !contains_word(&text, w),
                "{} contains the counter identifier {w}",
                file.display()
            );
        }
        assert!(
            !text.contains("counter("),
            "{} contains counter(",
            file.display()
        );
    }

    for core in ["domain", "ports", "usecases"] {
        let mut core_files = Vec::new();
        rust_files(&root.join("src").join(core), &mut core_files);
        for file in &core_files {
            let text = std::fs::read_to_string(file).expect("source reads");
            assert!(
                !text.contains("adapters::") && !contains_word(&text, "adapters"),
                "{} names an adapter",
                file.display()
            );
        }
    }
}
