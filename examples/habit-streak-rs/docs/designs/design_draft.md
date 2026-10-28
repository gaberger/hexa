<!-- hexa design resolve: the elephant (claude-text) on egm-resolve applied S6-5, S7-3, S7-4, S8-5, S8-6, S9-1, S9-2, S9-6, S9-7, S9-8, S9-12, S9-22 from docs/designs/goldfish_review_report.md, from the Elephant-Goldfish Model (EGM, https://github.com/google/elephant-goldfish-model, Google LLC, Apache-2.0) at commit a8380ab5f0aba392c97e31c79f9a6666b3f09f14 -->

# Habit Streak CLI

*   **Author:** gary (gary@sticksandstrings.org)
*   **Status:** Draft
*   **Date:** 2026-10-28

--------------------------------------------------------------------------------

## Overview

`habit-streak` is a command-line tool for one person on one machine. The person creates named daily habits and records a check-in for a habit on a calendar date. For each habit, the tool reports two numbers:

*   **Current streak:** the number of consecutive days with a check-in, counting back from today if today has one, or from yesterday if yesterday has one. Otherwise it is 0.
    *   The second clause ("or from yesterday") is the **grace rule**. A streak that reached yesterday still counts during today, before today's check-in. A streak ends only when a whole calendar day passes with no check-in.
*   **Longest streak:** the largest number of consecutive days with a check-in, from the start of the habit's history up to today.

All data lives in one local, versioned JSON file. The tool stores only habit names and check-in dates. It computes both streak numbers from those dates every time it runs and never stores them.

The program follows the repository's enforced hexagonal layout (`domain/`, `ports/`, `usecases/`, `adapters/`, composition root). This is a stated goal of the project and is not only a means to an end. The layout costs more code than the problem needs. If architecture practice were not a goal, a text file and a short script would serve the person better (see Assumption 1).

### Assumptions made in this draft

This draft was written with no clarification turns. The Goldfish reviewers run separately. Each assumption below is a choice where another answer is defensible, and the reviewers or the owner should check it first.

1.  **The project goal includes architecture practice.** If it does not, the YAGNI objection stands. A text file with one line per check-in plus a short script, or an existing tool such as `habitctl`, would be the better choice, and this design should not be built.
2.  **The domain represents a date as a day number (days since 1970-01-01), not as a date-crate type.** The brief accepted using the date crate's type in the domain. That conflicts with the repository rule that `domain/` has zero external dependencies. A day number makes "consecutive" mean "differs by 1" with no calendar arithmetic in the domain. Converting between `YYYY-MM-DD` text and day numbers happens in one adapter, `chrono_calendar.rs`, which the other adapters reach through a port. See [Date conversion](#date-conversion), [Alternatives Considered](#alternatives-considered), and Open Question 4.
3.  **Date crate: `chrono`. Argument parser: `clap`.**
    *   `chrono` is the crate with the longest maintenance record for reading the local calendar date.
    *   `clap` removes the need to write and test help text and argument errors by hand. It also enforces the required `--yes` flag on `remove`.
    *   `clap` exits with code 2 on a usage error by default. That conflicts with this tool's rule that exit code 2 means a data file problem, so the CLI adapter must remap it (see Technical Plan, "CLI and exit codes").
4.  **The `created` field from the brief's example file is dropped.** Nothing in the goals reads it. Keeping it would raise a question the brief does not answer: whether a backfilled date before `created` is valid. Without the field, backfilling to any past date is allowed.
5.  **"List habits" is served by `status` with no argument.** No separate `list` command exists.
6.  **A command whose requested end state already holds succeeds and says so.** `add` on an existing name, `check` on a date already recorded, and `uncheck` on a date with no check-in all exit 0 with a message and do not write the file. An unknown habit name is always an error with exit 1 (except for `add`, which creates it).
7.  **Dates later than today are ignored when computing streaks.** The tool refuses to record them. If one is in the file (from a hand edit or a clock that was wrong), it stays in the file but does not count toward either number.
8.  **The default data path uses the XDG rule on every platform**, including macOS (`~/.local/share/habit-streak/habits.json`). This avoids an extra crate for platform directories. Windows is not tested in v1.
9.  **A missing file at an override path is an error for every command except `add`.** When `add` creates the file, it prints a note on stderr. The alternative, creating the file on any command with a warning, was rejected because the warning arrives after the history has already split in two (see Alternative I).
10. **The scaffold's `src/lib.rs` stays the composition root.** `src/main.rs` is a one-line wrapper that names no adapter. See [Composition root](#composition-root).

## The Problem

A person who wants to keep up a daily habit wants to answer two questions quickly: "Did I keep my streak going?" and "What is my best run?" Phone apps and hosted trackers answer these questions, but they require an account, keep the data in a format the person does not control, and need a network or a specific device.

The person wants a private, offline record that they can read, back up, and fix by hand as plain text, with streak rules they can see and check.

The weak points of this problem, carried forward from the brief, are:

*   **The value over a text file and a script is small.** The tool earns its place through clear streak rules, data safety, and undo. Every feature outside record, report, and undo must justify itself against that baseline.
*   **"Today" is the source of most bugs.** The current streak depends on the local calendar date. Time zones, late-night check-ins, and wrong clocks all affect it.
*   **The data file can be lost.** The worst failure is a read problem treated as "no data", followed by a write that replaces the person's history with a new file. Two routes lead there: a read error, and a file that is "not found" only because a synced folder is unmounted or a path was mistyped.

The proposed solution is a small Rust CLI. It stores calendar dates only. It computes streaks with a pure function that receives "today" as an argument. It writes the file with a temp-file-then-rename step. It refuses to write after any failed read, and it treats a missing file as an empty history only at the default path.

## Goals

*   Create a named daily habit with `add`, and remove one with `remove <habit> --yes`.
*   Record a check-in for today, or for a past date with `--date YYYY-MM-DD`. A repeat check-in on the same date is accepted, stored once, and reported as "already recorded".
*   Remove a check-in for a given date with `uncheck <habit> --date YYYY-MM-DD`.
*   Report the current streak, the longest streak, and whether today is checked, for one habit or for all habits, with `status [<habit>]`.
*   Store data in one human-readable JSON file with `"version": 1`, sorted habit names, and sorted dates with no duplicates.
*   Never lose existing data because of a crash during a write. The file on disk is always either the old contents or the new contents.
*   Stop with exit code 2, print the file path and the reason, and leave the file byte-for-byte unchanged in each of these cases:
    *   an unreadable file;
    *   a malformed file, including one with duplicate JSON keys;
    *   an unknown version;
    *   a symlink whose target is missing;
    *   a missing file at an override path, on any command other than `add`.
*   Use exit codes that scripts can rely on: `0` for success, `1` for a user error, `2` for a data file problem.
*   Read "today" through a `Clock` port so tests can fix the date.
*   Pass a gate that runs a non-zero number of tests. The tests include property tests against an independent oracle, fixed-point tests of the date conversion, a file round-trip test, a corrupt-file test, and an end-to-end test of the built binary.

## Non-Goals

*   Weekly or "N times per week" habits, rest days, freeze days, or streak forgiveness beyond the grace rule.
*   A configurable start-of-day hour for people who check in after midnight. The `--date` flag covers this case.
*   Time-zone-aware storage. A date is fixed when it is recorded.
*   Sync, multiple devices, multiple users, or any network access.
*   Locking against two commands running at once. The last write wins, and the README documents this.
*   Notes, quantities, or durations on a check-in.
*   Reminders, charts, calendars, a TUI, or a GUI.
*   Import from or export to other trackers.
*   Machine-readable (`--json`) output. The status report is a plain data structure inside the use case, so this can be added later without changing the domain.
*   Renaming a habit. A hand edit of the JSON key covers it.
*   Detecting a wrong system clock.
*   Windows support beyond "it may work".

## Open Questions

*   **Question 1 (from brief)**: Is explicit `add` worth one extra command per habit, compared with a "did you mean `read`?" prompt on unknown names? This draft keeps explicit `add`, because a silent new habit from a typo breaks the real streak without notice.
*   **Question 2 (from brief)**: Is "dates are fixed when recorded" acceptable for someone who travels often? This draft accepts it. A person who crosses time zones may see a streak gain or lose a day at the edges. The README documents this.
*   **Question 3 (from brief)**: Is architecture practice a real goal? This draft assumes yes (Assumption 1). If no, stop here and use an existing tool.
*   **Question 4**: Does `hexa analyze .` treat an external crate imported in `domain/` (for example `chrono`) as a boundary violation? If it does not, keeping `chrono::NaiveDate` in the domain is a valid alternative to the day-number type, and the owner should choose between them. Run `hexa analyze .` on the scaffold before writing the domain to find out.
*   **Question 5 (resolved in this draft)**: Which file is the composition root? The scaffold already answers this. Its doc comment designates `src/lib.rs` as the root, and that file already contains `pub mod adapters;` and imports an adapter. This design keeps `lib.rs` as the root, and `main.rs` names no adapter, so the analyzer's way of finding the root does not matter here. Step 1 of the order of work runs `hexa analyze .` on the unmodified scaffold. If that run flags the scaffold's own `lib.rs`, the analyzer disagrees with the scaffold it generates. Stop and report that as a `hexa` defect. Do not redesign the module layout.
*   **Question 6 (resolved in this draft)**: The scaffold uses `src/<layer>/mod.rs` (`src/domain/mod.rs`, `src/ports/mod.rs`, `src/usecases/mod.rs`, `src/adapters/mod.rs`, `src/adapters/secondary/mod.rs`). The file list below matches it. Step 1 still runs `hexa graph context src` to confirm the layout before any edit.

## Critical User Journeys (CUJs)

### CUJ 1: Start tracking a habit

*   **Given**: No data file exists at the default path. `$XDG_DATA_HOME` is unset and `$HOME` is `/home/gary`.
*   **When**: The person runs `habit-streak add Read`, then `habit-streak check read`, then `habit-streak status`.
*   **Then**: `add` creates `/home/gary/.local/share/habit-streak/` and `habits.json` containing `"version": 1` and a habit keyed `"Read"`. `check read` matches `Read` without regard to case and records today's date. `status` prints `Read` with current streak 1 and longest streak 1. All three commands exit 0.

### CUJ 2: Keep a streak alive overnight

*   **Given**: `read` has check-ins on each of the last 5 days up to yesterday, and none today.
*   **When**: The person runs `habit-streak status read`.
*   **Then**:
    *   The grace rule applies. The output shows current streak 5 and the marker `(not checked today)`.
    *   If the person then runs `check read`, `status` shows current 6 with no marker.
    *   If they skip today and run `status` tomorrow, the current streak is 0.

### CUJ 3: Backfill a missed day that joins two runs

*   **Given**: `read` has check-ins on 2026-10-01 to 2026-10-05 and 2026-10-07 to 2026-10-12. Today is 2026-10-12. Status shows current 6, longest 6.
*   **When**: The person runs `habit-streak check read --date 2026-10-06`.
*   **Then**: The command exits 0. `status read` shows current 12 and longest 12.

### CUJ 4: Undo a mistake

*   **Given**: The person meant to backfill `run` but typed `check read --date 2026-10-20`.
*   **When**: They run `habit-streak uncheck read --date 2026-10-20`.
*   **Then**: The date is removed from `read`, the file is rewritten, the command prints that the check-in was removed, and it exits 0. Running the same `uncheck` again prints that no check-in was recorded on that date, exits 0, and does not write the file.

### CUJ 5: Typo in a habit name

*   **Given**: Habits `Read` and `Run` exist.
*   **When**: The person runs `habit-streak check reed`.
*   **Then**: The command prints `unknown habit "reed"; known habits: Read, Run; create it with: habit-streak add reed`, exits 1, and does not write the file.

### CUJ 6: Corrupt data file

*   **Given**: The person hand-edited `habits.json` and left a trailing comma, or pasted a second `"Read"` entry.
*   **When**: They run `habit-streak check read`.
*   **Then**: The command prints the file path and the reason, says the file was not changed, and exits 2. For the trailing comma, the reason is the JSON parse error with line and column. For the second entry, it is `duplicate habit "Read"`. The file is byte-for-byte unchanged. Every other command, including `status`, behaves the same way until the file is fixed.

### CUJ 7: Data file in a synced folder

*   **Given**: `~/.local/share/habit-streak/habits.json` is a symlink to `~/Sync/habits.json`, or the person sets `HABIT_STREAK_FILE=~/Sync/habits.json`.
*   **When**: They run `habit-streak check read`.
*   **Then**: The new contents are written to `~/Sync/habits.json`. When the default path is a symlink, it stays a symlink and is not replaced by a regular file.
*   **And if `~/Sync` is not mounted**: the symlink's target is missing. Every command, including `add` and `status`, prints `data file /home/gary/.local/share/habit-streak/habits.json is a symlink to /home/gary/Sync/habits.json, which does not exist. Is the folder mounted? The file was not changed.` The command exits 2. No file is created, so the sync tool has nothing new to copy over the real history.

### CUJ 8: Mistyped override path

*   **Given**: The real history is at `~/Sync/habits.json`. The person sets `HABIT_STREAK_FILE=~/Sync/habit.json` (the `s` is missing). The directory `~/Sync` exists.
*   **When**: They run `habit-streak check read`.
*   **Then**: The command prints `data file /home/gary/Sync/habit.json does not exist. Check --file and HABIT_STREAK_FILE. To start a new file there, run: habit-streak add <habit>. No file was created.` It exits 2 and creates no file.
*   **And if they run `habit-streak add Read` with the same setting**: the file is created and the command prints `note: created new data file /home/gary/Sync/habit.json` on stderr. It exits 0.

## Technical Plan (Architecture)

### Layer map

```text
                   argv, env
                       |
                       v
   +---------------------------------------------+
   | main.rs                                     |   names no adapter
   |   exit(habit_streak::run(args, env, io))    |
   +---------------------------------------------+
                       |
                       v
   +---------------------------------------------+
   | lib.rs  (composition root)                  |   only file that names adapters
   |  - cli::parse(args)                         |
   |  - resolve_data_path(flag, env)             |
   |  - build ChronoCalendar, JsonFileStore      |
   |  - build Habits use case                    |
   |  - cli::execute(command, &habits, &cal, io) |
   |  - return exit code                         |
   +---------------------------------------------+
        |                          |
        v                          v
 adapters/primary/cli.rs      adapters/secondary/
  parse args (clap)            json_file_store.rs  (HabitStore; serde DTOs)
  format output                chrono_calendar.rs  (Clock, DateText;
  map errors -> exit code                           the only chrono user)
  dates via DateText port
        |                          ^ implements
        v                          |
   usecases/habits.rs  ----->  ports/
   add, check, uncheck,         HabitStore: load(&self), save(&self, &HabitBook)
   remove, status               Clock:      today(&self) -> Day
                                DateText:   parse(&self, &str) -> Option<Day>
                                            format(&self, Day) -> String
                                re-exports domain types
        |
        v
   domain/
   Day, HabitName, HabitBook, streaks(dates, today)
   (std only: no serde, no chrono, no I/O)
```

Dependency directions follow the enforced rules:

*   The CLI adapter imports `ports` and `usecases`.
*   Both secondary adapters import `ports` only.
*   Neither adapter imports the other. `JsonFileStore` and the CLI use date conversion through the `DateText` port. The composition root passes them the `ChronoCalendar` value.
*   When an adapter needs `Day`, `HabitName`, or `HabitBook`, it imports them from the `ports` re-export, not from `domain`.

### Composition root

`src/lib.rs` is the composition root, as the scaffold already declares. It keeps its module declarations. Its scaffold code is replaced (see [Scaffold code to remove](#scaffold-code-to-remove)) with:

*   `pub fn run(args, env, out, err) -> i32`. This builds and runs the application and returns the exit code.
*   A private `resolve_data_path` function and its `DataPath` and `NoDataPath` types (see [Data path rules](#data-path-rules)). Data path resolution is configuration, not a port implementation. It lives in the root, which is the only code that decides which file to open.

`src/main.rs` contains one call: `std::process::exit(habit_streak::run(...))`. It names no adapter.

The library target is needed for two reasons: integration tests in `tests/` link against it, and the end-to-end tests run the binary built from `main.rs`.

### Domain

*   **`Day`**: a copyable, ordered newtype around a signed day count since 1970-01-01. Operations: build from and read the day count, and `pred()` (the day before).
    *   The domain has no month or year logic. "Consecutive" means "the day counts differ by 1", so month ends, year ends, and 29 February need no special handling here.
    *   The calendar conversion lives in the `chrono_calendar` adapter (see [Date conversion](#date-conversion)).
*   **`HabitName`**: built only through a parse function.
    *   The parse function trims surrounding whitespace and rejects an empty result.
    *   It rejects any control character. This stops a hand-edited name from printing terminal escape codes.
    *   It keeps the trimmed spelling for display and a lowercase form (`str::to_lowercase`) as the key for comparison. Two names are equal when their keys are equal.
*   **`HabitBook`**: the whole data set. It maps a name key to a habit, where a habit holds the display name and an ordered set of `Day`s. Operations:
    *   `add(name)` returns whether the habit was new.
    *   `find(name)` returns the habit or nothing.
    *   `check(name, day)` returns "added" or "already recorded".
    *   `uncheck(name, day)` returns "removed" or "not recorded".
    *   `remove(name)` returns whether the habit existed.
    *   `names()` returns display names in key order.
    *   Building a book from raw entries fails if two entries share a key (for example `"Read"` and `"read"` in a hand-edited file).
*   **`streaks(dates, today) -> Streaks { current, longest, checked_today }`**: a pure function over the ordered set.
    *   Dates after `today` are ignored.
    *   `longest`: one pass in order, extending the run while each date is one day after the previous date, keeping the maximum.
    *   `current`: start at `today` if present. Otherwise start at `today.pred()` if present (the grace rule). Otherwise the result is 0. Count back one day at a time while the day is present.
    *   `checked_today`: whether `today` is present.

### Ports

*   **`HabitStore`**
    *   `fn load(&self) -> Result<HabitBook, StoreError>`. At the default path, a missing file gives an empty book. At an override path, a missing file gives `NotFound`.
    *   `fn save(&self, book: &HabitBook) -> Result<(), StoreError>`
*   **`Clock`**
    *   `fn today(&self) -> Day`
*   **`DateText`**: the strict `YYYY-MM-DD` text form of a `Day`.
    *   `fn parse(&self, text: &str) -> Option<Day>`. Returns `None` for any text that is not exactly a real calendar date in `YYYY-MM-DD` form.
    *   `fn format(&self, day: Day) -> String`
*   **Receivers.** Every method takes `&self`, because `Habits` holds shared references (`&dyn HabitStore`, `&dyn Clock`). This departs from the scaffold's `CounterStore::save(&mut self, …)`. That trait is deleted, and `JsonFileStore` keeps no state between calls, so nothing needs `&mut`. Test doubles that record calls use interior mutability (`Cell`, `RefCell`).
*   **`StoreError`**: a struct with `path: PathBuf` and `kind: StoreErrorKind`. Every `StoreError` has a path, because every one arises after a path has been resolved. The kinds are:
    *   `Unreadable(io detail)`: any I/O error on read, or a symlink whose target is missing.
    *   `NotFound`: no file at an override path.
    *   `Malformed(detail)`: invalid JSON, wrong shape, a duplicate key, a bad version value, a bad date, a bad or colliding name, or an unknown field.
    *   `UnknownVersion(u64)`: the version is a non-negative integer other than 1.
    *   `MissingParentDirectory`
    *   `WriteFailed(io detail)`
*   The ports module re-exports the domain types an adapter names: `Day`, `HabitName`, `NameError`, `HabitBook`, `Habit`, `DuplicateName`, and `Streaks`.

### Use cases

There is one `Habits` struct that holds a `&dyn HabitStore` and a `&dyn Clock`. It has one method per command.

*   Every method calls `load()` first and returns on the first error.
*   There is one exception. `add` treats a `NotFound` error as an empty book, and reports the path it will create.
*   Only methods that changed the book call `save()`.

| Method | Input | Writes when | Result |
|---|---|---|---|
| `add` | raw name | habit is new | `Added { created_file: Option<PathBuf> }` / `AlreadyExists` |
| `check` | raw name, optional `Day` | date is new | `Recorded(day)` / `AlreadyRecorded(day)` |
| `uncheck` | raw name, `Day` | date was present | `Removed(day)` / `NotRecorded(day)` |
| `remove` | raw name | habit existed | `Removed` |
| `status` | optional raw name | never | `StatusReport { habits: Vec<HabitStatus { name, current, longest, checked_today }> }` |

`created_file` is `Some(path)` only when `load` returned `NotFound`.

`check` uses `clock.today()` when no date is given. It rejects a date after today with `FutureDate { date, today }`.

`remove` takes no confirmation flag. The CLI enforces `--yes` before any use case runs (see [CLI and exit codes](#cli-and-exit-codes)).

`status` with no name lists every habit, ordered by name key.

`UseCaseError` variants:

*   `InvalidName(reason)`
*   `UnknownHabit { name, known }`
*   `FutureDate { date, today }`
*   `Store(StoreError)`

### JSON file store

File shape, version 1. This is an illustrative schema, not production code:

```json
{
  "version": 1,
  "habits": {
    "Read": { "checkins": ["2026-10-01", "2026-10-02"] },
    "Run":  { "checkins": [] }
  }
}
```

`JsonFileStore` holds the path, whether it is the default path, and a `&dyn DateText` for date conversion.

**Read**

1.  Inspect the path with `symlink_metadata`.
    *   **Nothing at the path:**
        *   At the default path, return an empty book. This is the first-run case (CUJ 1).
        *   At an override path, return `NotFound` (CUJ 8). A mistyped filename must fail, not start a new history.
    *   **The path is a symlink whose target does not exist:** return `Unreadable`, naming the target. This applies at default and override paths (CUJ 7). An unmounted synced folder looks exactly like this, and treating it as "no data" would let the next write replace the real history.
    *   **Any other I/O error:** return `Unreadable`.
2.  Read the whole file into memory. An I/O error is `Unreadable`.
3.  Parse the text as a generic `serde_json::Value`, for syntax and the top-level shape only.
    *   A syntax error is `Malformed`, with line and column.
    *   A top level that is not an object is `Malformed("top level must be a JSON object")`.
    *   The parsed `Value` is used only for these two checks. It keeps only the last of duplicate keys, so its contents are never used.
4.  Check the version before the full shape, so that a future version 2 file gets a clear "unknown version" message instead of a shape error.
    *   Deserialize the text into a probe struct `{ version: Option<serde_json::Value> }` that ignores other fields.
    *   A derived struct rejects a repeated field, so a file with two `version` keys is `Malformed`.
    *   Map the value as follows:

    | `version` in the file | Result |
    |---|---|
    | missing | `Malformed("missing \"version\"")` |
    | not a number (`"1"`, `true`, `null`, an array, an object) | `Malformed("\"version\" must be a non-negative integer")` |
    | a number that is not a non-negative integer (`1.0`, `1e0`, `-1`, a value above `u64::MAX`) | `Malformed("\"version\" must be a non-negative integer")` |
    | the integer `1` | continue |
    | any other non-negative integer (`0`, `2`) | `UnknownVersion(n)` |

    The test is `Value::as_u64()`. Values `1.0` and `1e0` parse as floats, and `-1` has no `u64` form, so all three fall into the third row.

5.  Deserialize the text into the version 1 data-transfer structs, which reject unknown fields. This is a deliberate choice: the tool must never rewrite a file containing fields it does not understand, because the rewrite would drop them without telling anyone.
    *   Rejecting unknown fields does not catch duplicate keys. `serde_json` deserializes a map into `BTreeMap<String, _>` by keeping the last of duplicate keys. A hand-edited file with two `"Read"` entries would lose one entry's check-ins on load, and the next write would make the loss permanent.
    *   So the `habits` field uses a custom map deserializer (`deserialize_with`). It inserts each key into a `BTreeMap` and fails with `duplicate habit "<key>"` when the key is already present.
    *   Derived structs already reject a repeated `version`, `habits`, or `checkins` field.
    *   Any deserialize error is `Malformed`. Together these checks make an exact duplicate key anywhere in the file `Malformed`.
6.  Convert each date with `DateText::parse`. A `None` result is `Malformed`, naming the habit and the bad value.
7.  Parse each key as a `HabitName`, then build the `HabitBook`.
    *   Duplicate dates within one habit are merged here.
    *   A collision between name keys that differ only in case (`"Read"` and `"read"`) is `Malformed`. Step 5 catches exact duplicates. This step catches case collisions.

**Write**

1.  Resolve the target path. If the path is a symlink, follow it to the real file, so that the temp file and the rename happen in the real file's directory.
2.  If the parent directory is missing:
    *   For the default path, create it.
    *   For a `--file` or `HABIT_STREAK_FILE` path, return `MissingParentDirectory`. A typo in an override path should fail, not start a new empty history somewhere else.
3.  Serialize the book as pretty-printed JSON with a trailing newline. Habits are ordered by name key. Dates are in ascending order, formatted with `DateText::format`.
4.  Write to `.<filename>.<pid>.tmp` in the same directory. On Unix, create the file with mode `0600`. Flush with `sync_all`, then rename it over the target.
5.  On any failure, try to delete the temp file and return `WriteFailed`. The original file is never opened for writing.
6.  The directory is not synced after the rename. After a power loss, the rename may be lost and the old file kept. The old file is intact, so this is accepted.

A temp file left behind by a crash is not cleaned up automatically. The README documents this.

### Data path rules

`resolve_data_path` lives in the composition root (`src/lib.rs`). It resolves the path in this order:

1.  The `--file <path>` flag.
2.  The `HABIT_STREAK_FILE` environment variable, if it is non-empty.
3.  `$XDG_DATA_HOME/habit-streak/habits.json`, if `$XDG_DATA_HOME` is set and non-empty.
4.  `$HOME/.local/share/habit-streak/habits.json`, if `$HOME` is set and non-empty.
5.  Otherwise, `NoDataPath`.

Signature: `fn resolve_data_path(flag: Option<PathBuf>, env: &dyn Fn(&str) -> Option<OsString>) -> Result<DataPath, NoDataPath>`.

*   `DataPath { path, is_default }`. Only cases 3 and 4 are the default path, because the read and write rules differ for it.
*   `NoDataPath` is its own unit error type, not a `StoreError`, because no path exists to report. `run` prints this exact message to stderr and returns exit code 2:

    ```text
    error: cannot choose a data file: neither XDG_DATA_HOME nor HOME is set. Pass --file <path> or set HABIT_STREAK_FILE.
    ```

*   The environment arrives through the lookup function, so the unit tests in `lib.rs` do not change the real process environment.

### Date conversion

All calendar logic lives in one adapter, `src/adapters/secondary/chrono_calendar.rs`. The adapter defines `ChronoCalendar`, a unit struct that implements both `Clock` and `DateText`. It is the only file that imports `chrono`.

*   **`pub fn day_from_naive(date: NaiveDate) -> Day`**: `Day::from_days_since_epoch(date.num_days_from_ce() - 719_163)`.
    *   `719_163` is `num_days_from_ce()` of 1970-01-01.
    *   `chrono`'s date range is about ±95 million days, so the result always fits in `i32`.
*   **`pub fn naive_from_day(day: Day) -> Option<NaiveDate>`**: `day.days_since_epoch().checked_add(719_163).and_then(NaiveDate::from_num_days_from_ce_opt)`.
    *   It returns `None` for a `Day` outside `chrono`'s range. Such a `Day` can only come from `pred()` at the range's lower edge.
*   **`DateText::parse`**: strict parsing.
    1.  The text must be exactly 10 bytes, with ASCII digits at positions 0–3, 5–6, and 8–9, and `-` at positions 4 and 7.
    2.  The year, month, and day must form a real date under `NaiveDate::from_ymd_opt`.
    3.  The result is `day_from_naive` of that date.
    *   Text such as `2026-1-05`, `2026-02-30`, ` 2026-01-05`, or `2026-01-05T00` returns `None`.
*   **`DateText::format`**: `naive_from_day` formatted as `%Y-%m-%d`, for a year in 0000–9999. For any other `Day`, it returns `day <n>` (the raw count) instead of panicking.
*   **`Clock::today`**: `day_from_naive(Local::now().date_naive())`. `chrono` reads the local time zone from `TZ` or `/etc/localtime`. This is the only place that reads the system time.

`day_from_naive` and `naive_from_day` are `pub` so that `tests/streak_oracle.rs` and `tests/date_codec.rs` use the exact conversion the tool uses.

### CLI and exit codes

Command surface (illustrative):

```text
habit-streak [--file <path>] add <habit>
habit-streak [--file <path>] check <habit> [--date YYYY-MM-DD]
habit-streak [--file <path>] uncheck <habit> --date YYYY-MM-DD
habit-streak [--file <path>] status [<habit>]
habit-streak [--file <path>] remove <habit> --yes
```

`uncheck` requires `--date`, so a command typed from shell history cannot remove today's check-in by accident.

`remove` declares `--yes` as a required `clap` flag. Without it, `clap` reports the missing argument and the command exits 1 before any file is read.

A habit name that starts with `-` must follow `--`. The help text says so.

**Parsing:** use `clap`'s non-exiting parse.

*   A help or version request prints to stdout and exits 0.
*   Any other parse error, including a missing `--yes`, prints `clap`'s message to stderr and exits **1**, not `clap`'s default of 2.
*   A `--date` value for which `DateText::parse` returns `None` is a user error. The CLI prints `invalid date "<value>"; expected YYYY-MM-DD` and exits 1.

**Output**

`status` prints one line per habit. The name column is padded to the longest name, measured in characters. Example:

```text
Read   current 6   longest 12
Run    current 3   longest 4   (not checked today)
```

The marker appears when the current streak is greater than 0 and today has no check-in. In that case, the grace rule alone is keeping the streak alive, and the streak ends unless the person checks in before the day ends. In every other case, a check-in today changes nothing the person needs to act on.

With no habits, `status` prints `No habits yet. Add one with: habit-streak add <name>`.

When `add` returns `created_file: Some(path)`, the CLI prints `note: created new data file <path>` to stderr.

Dates in messages are formatted with `DateText::format`.

Output goes through a writer. A broken pipe (for example `status | head -1`) ends the command quietly with exit 0 instead of a panic.

**Errors**

Every error message goes to stderr. It names what failed, the habit or file involved, and the next step.

`StoreError` messages have the form `error: data file <path>: <reason>`, followed by one closing sentence:

| Kind | Closing sentence |
|---|---|
| `Unreadable`, `Malformed`, `UnknownVersion` | `The file was not changed.` |
| `NotFound`, `MissingParentDirectory` | `No file was created.` |
| `WriteFailed` | `The original file was not changed, but this change was not saved.` |

For `NotFound`, the reason is `does not exist. Check --file and HABIT_STREAK_FILE. To start a new file there, run: habit-streak add <habit>` (CUJ 8).

| Error | Exit code |
|---|---|
| Argument error (including a missing `--yes`), invalid `--date`, `InvalidName`, `UnknownHabit`, `FutureDate` | 1 |
| Any `StoreError`, and `NoDataPath` | 2 |

### Concurrency

Two commands at once read the same old file. Both write, and the later rename wins. One change can be lost. This is accepted for v1 and documented in the README.

## Alternatives Considered

### Alternative A: Use `chrono::NaiveDate` directly in the domain

*   **Pros**: No conversion code. Date arithmetic comes from a tested library. This is the choice the brief made.
*   **Cons**: Breaks the repository rule that `domain/` has zero external dependencies, and may fail `hexa analyze .` (Open Question 4). Every layer would then depend on `chrono`.
*   **Reason for Rejection**: The streak rules need only "the day before" and ordering. A day number provides both with no calendar logic. The calendar conversion still comes from `chrono`, inside one adapter, so no date arithmetic is hand-written. This choice is reversible if Open Question 4 shows that the analyzer allows external crates in the domain.

### Alternative B: A hand-written date type with year, month, and day in the domain

*   **Pros**: The domain stays free of external crates and still shows readable dates.
*   **Cons**: Requires hand-written days-in-month and leap-year logic, which is where date bugs live.
*   **Reason for Rejection**: It adds the riskiest code to the layer meant to be simplest. The day number avoids it.

### Alternative C: Store current and longest streaks in the file

*   **Pros**: The numbers are visible in the file without running the tool.
*   **Cons**: Creates a second source of truth. Backfills, `uncheck`, hand edits, and a new "today" would all make the stored numbers wrong.
*   **Reason for Rejection**: Computing the streaks from about 365 dates per habit per year costs nothing measurable.

### Alternative D: Create a habit on first `check`

*   **Pros**: One fewer command.
*   **Cons**: A typo creates a new habit and breaks the real streak without notice.
*   **Reason for Rejection**: An explicit `add` costs one command per habit, once. Open Question 1 keeps this open for the reviewers.

### Alternative E: A hand-written argument parser instead of `clap`

*   **Pros**: No large dependency and faster compiles. The exit-code mapping is fully under the tool's control.
*   **Cons**: Help text, usage errors, required flags, and `--` handling must be written and tested by hand for five subcommands.
*   **Reason for Rejection**: The help and error behavior is more test code than the remapping `clap` needs. This is a close call, and the ADR records it.

### Alternative F: Ignore unknown JSON fields

*   **Pros**: Hand-added fields do not stop the tool.
*   **Cons**: The next write deletes those fields without telling the person.
*   **Reason for Rejection**: Losing data without notice is worse than a clear error the person can fix. The same reasoning covers duplicate keys, which rejecting unknown fields does not catch. Read step 5 handles them separately.

### Alternative G: Lock file for concurrent commands

*   **Pros**: No lost check-ins when two commands run at once.
*   **Cons**: Needs handling for stale locks and adds code for a rare case for one person.
*   **Reason for Rejection**: Deferred until a lost write happens in practice (see Future Improvements).

### Alternative H: Date conversion duplicated in each adapter, or in a shared helper module under `adapters/`

*   **Pros**: No extra port. Each adapter calls `chrono` directly.
*   **Cons**: Duplication gives three copies of the epoch formula that can drift apart. A shared helper module is one adapter importing another (rule 6), or at best a module whose layer the analyzer cannot classify.
*   **Reason for Rejection**: The `DateText` port keeps one implementation, keeps `chrono` in one file, and needs no new import edges.

### Alternative I: Create a missing override file on any command, with a warning

*   **Pros**: `check` on a new path works without a prior `add`.
*   **Cons**: The warning appears after the new file exists. By then a sync tool may have propagated it, and later commands use the new file without further warnings.
*   **Reason for Rejection**: Only `add` signals that the person intends a new history, so only `add` may create the file.

--------------------------------------------------------------------------------

## Non-Functional Requirements

*   **Security & Privacy**: Habit data can be personal. The tool makes no network access. New files are written with mode `0600` on Unix. Names with control characters are rejected, so printing names from a hand-edited file cannot send escape codes to the terminal.
*   **Data integrity**: This is the critical requirement. The rules are:
    *   no write after a failed read;
    *   temp-file-then-rename writes;
    *   a version check before parsing the full shape;
    *   refusal of unknown fields and of duplicate keys;
    *   refusal of a symlink whose target is missing;
    *   no new file at an override path except through `add`;
    *   no write when nothing changed.

    Read-only commands (`status`, and no-op `add`, `check`, and `uncheck`) never write the file.
*   **Performance**: Not a risk. Each command reads a file of a few kilobytes, computes, and exits. There is no cache: a cache in front of a file read by a short-lived process would be a second source of truth.
*   **Rollout & Rollback**: This is a new tool with no earlier data format. The `"version": 1` field is the only compatibility mechanism. A future version 2 must add an explicit migration. A version 1 binary refuses a version 2 file with exit code 2 and does not change it.

--------------------------------------------------------------------------------

### Additional Resources

*   Concepts brief: Habit Streak CLI (gary, 2026-10-28). This document states every rationale it needs inline. The brief is listed for provenance only and is not needed to read this design.
*   `CLAUDE.md`: hexagonal rules, gate-first pipeline, and lessons. The design relies most on these lessons: "write shape and read shape must agree", "a cache in front of a file … is not a cache", "tests can mirror bugs", "a vacuous gate is a failed gate", and "documentation about a deleted feature never fails".
*   `habitctl` and Loop Habit Tracker, as the baseline the tool must beat.

### Future Improvements

*   **Start-of-day hour**: a setting such as "the day starts at 04:00" for people who check in after midnight.
*   **`--json` status output**: built from the existing `StatusReport` structure.
*   **Lock file**: if lost writes from concurrent commands happen in practice.
*   **Rename command**: if hand-editing JSON keys turns out to be common.
*   **Stale temp file cleanup**: if crashes leave temp files often enough to notice.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

### Order of work

This order follows the repository's gate-first pipeline.

1.  Run `hexa graph context src` and `hexa analyze .` on the unmodified scaffold. Record the grade.
    *   Resolve Open Question 4. If the analyzer allows `chrono` in `domain/`, bring Alternative A back to the owner before continuing.
    *   Confirm Open Questions 5 and 6 as described there.
2.  Write the ADR (file 1).
3.  Write `scripts/gate.sh` (file 2). Record it with `hexa loop gate './scripts/gate.sh'`. Add one `hexa loop task add` entry per file group below.
4.  Remove the scaffold code listed in [Scaffold code to remove](#scaffold-code-to-remove).
    1.  For each file in that table, run `hexa graph consumers <file>` over the whole workspace.
    2.  Grep for each symbol name across `src/`, `tests/`, `examples/`, `benches/`, and all Markdown files. Also grep for the two cases a graph cannot see: a `pub use` re-export at the crate root, and an import under `#[cfg(...)]`.
    3.  The only expected consumers are the other scaffold symbols in the same table. Any other consumer stops this step and goes to the owner.
    4.  Delete the symbols, then run `cargo build --all-targets`. It must pass with the now-empty modules. The gate is expected to fail at this point (no tests yet). The build is the check for this step.
5.  Write the tests listed in the Verification Plan against the port and use-case signatures, before writing the implementations. Write the streak oracle from the streak rule in the Overview, not from the domain code.
6.  Implement in this order: domain, ports, use cases, secondary adapters, primary adapter, composition root (`src/lib.rs`, then `src/main.rs`).
7.  Run `hexa harden src --gate './scripts/gate.sh'`. Then run `hexa analyze .` and clear every item it reports.

### Scaffold code to remove

The scaffold's counter example is deleted in full. Nothing from it is kept elsewhere.

| File | Delete | Keep |
|---|---|---|
| `src/domain/mod.rs` | `Count` and its `impl` (`ZERO`, `next`, `value`) | Module doc comment stating rule 1 |
| `src/ports/mod.rs` | `use crate::domain::Count;`, `pub use crate::domain::Count as CountValue;` and its doc comment, `CounterStore` | Module doc comment stating rule 2. Rewrite the re-export rationale to refer to the new re-exports. |
| `src/usecases/mod.rs` | `use` lines for `Count` and `CounterStore`, `increment` | Module doc comment stating rule 3 |
| `src/adapters/secondary/mod.rs` | `use crate::ports::{CounterStore, CountValue as Count};`, `InMemoryCounterStore` and its `CounterStore` impl | Module doc comment stating rules 4 and 5 |
| `src/lib.rs` | `use adapters::secondary::InMemoryCounterStore;`, `use domain::Count;`, `counter()`, `increment_once()`, and the doc-comment sentences that name `InMemoryCounterStore` | `pub mod` lines, the "composition root" doc comment, and its layer diagram |

### Target Files to Create/Modify

#### 1. `docs/adrs/ADR-NNN-habit-streak-dependencies-and-dates.md` (Create)

*   **Type / Scope**: Architecture Decision Record. Use the next free number in `docs/adrs/`.
*   **Proposed Changes**: Record these decisions, with the alternatives and reasons from this document:
    *   Dependencies: `serde` and `serde_json` (JSON store only), `chrono` (`chrono_calendar` adapter only), and `clap` (CLI only, with the exit-code remapping).
    *   Dev dependencies: `proptest` and `tempfile`.
    *   The `Day` day-number representation, the `DateText` port, and why the two keep `chrono` out of the domain and in one adapter.
    *   `src/lib.rs` stays the composition root, and `src/main.rs` is a thin wrapper.
    *   File format version 1, the unknown-field and duplicate-key policies, the missing-file and dangling-symlink policies, and the last-writer-wins policy.
    *   Deletion of the scaffold counter example.
*   **Why**: The repository requires an ADR before adding a port, an adapter, or a dependency.

#### 2. `scripts/gate.sh` (Create)

*   **Type / Scope**: Gate script (shell, `set -euo pipefail`).
*   **Proposed Changes**: Run these steps in order. Any failure exits non-zero.
    1.  `cargo fmt --check`
    2.  `cargo clippy --all-targets -- -D warnings`
    3.  `cargo test --all-targets`, with its output saved to a temp file.
    4.  Fail if the total passed count across all `test result:` lines is 0.
    5.  Fail unless each required test listed in the Verification Plan appears in the output as `... ok`.
    6.  `hexa analyze .`. If `hexa` is not installed, fail with a message. Do not skip this step.
*   **Why**: `cargo test` alone passes with zero tests. A gate that skips a missing analyzer reports a weaker claim than it prints.

#### 3. `Cargo.toml` (Modify)

*   **Type / Scope**: Build configuration.
*   **Proposed Changes**:
    *   Keep the existing `[workspace]` table and its comment.
    *   Add `[lib]` with `path = "src/lib.rs"`, and `[[bin]]` with `name = "habit-streak"` and `path = "src/main.rs"`.
    *   Add dependencies: `serde` 1 (`derive`), `serde_json` 1, `chrono` 0.4 (`default-features = false`, features `clock` and `std`), and `clap` 4 (`derive`).
    *   Add dev-dependencies: `proptest` 1 and `tempfile` 3.
    *   Commit `Cargo.lock`, because this is a binary.
*   **Why**: These are the dependencies approved in the ADR. `chrono`'s `clock` feature is needed for the local date.

#### 4. `src/lib.rs` (Modify)

*   **Type / Scope**: Composition root and library crate root.
*   **Proposed Changes**:
    *   Delete the scaffold symbols listed in [Scaffold code to remove](#scaffold-code-to-remove). Rewrite the doc comment so it describes the new wiring.
    *   Keep `pub mod adapters; pub mod domain; pub mod ports; pub mod usecases;`.
    *   Add `pub fn run(args: impl IntoIterator<Item = OsString>, env: &dyn Fn(&str) -> Option<OsString>, out: &mut dyn Write, err: &mut dyn Write) -> i32`. It does the following:
        1.  Call `cli::parse(args)`. On `Err(ParseExit { message, code })`, write the message to `out` if `code` is 0 and to `err` otherwise, then return `code`.
        2.  Call `resolve_data_path(command.file_override, env)`. On `Err(NoDataPath)`, write the message from [Data path rules](#data-path-rules) to `err` and return 2.
        3.  Build `ChronoCalendar`, then `JsonFileStore::new(path, is_default, &calendar)`, then `Habits::new(&store, &calendar)`.
        4.  Return `cli::execute(command, &habits, &calendar, out, err)`.
    *   Add the private `resolve_data_path`, `DataPath { path, is_default }`, and `NoDataPath` (with `Display`), as in [Data path rules](#data-path-rules).
    *   Add `#[cfg(test)]` unit tests using a fake lookup function. Required test names:
        *   `flag_wins_over_env`
        *   `env_file_wins_over_xdg`
        *   `empty_env_values_are_unset`
        *   `xdg_then_home_are_default`
        *   `no_home_is_no_data_path`
*   **Why**: This is the only file allowed to name adapters (rule 7), as the scaffold already declares. `run` takes its inputs as parameters so that `main.rs` stays trivial.

#### 5. `src/main.rs` (Create)

*   **Type / Scope**: Binary entry point. The scaffold has no `main.rs`.
*   **Proposed Changes**: One function body: `std::process::exit(habit_streak::run(std::env::args_os(), &|k| std::env::var_os(k), &mut std::io::stdout().lock(), &mut std::io::stderr().lock()))`. It imports nothing from `adapters`.
*   **Why**: The binary target for the end-to-end tests, with no composition logic of its own.

#### 6. `src/domain/mod.rs` (Modify)

*   **Type / Scope**: Domain module root.
*   **Proposed Changes**:
    *   Delete `Count` (see [Scaffold code to remove](#scaffold-code-to-remove)) and keep the rule 1 doc comment.
    *   Declare `day`, `habit_name`, `habit_book`, and `streaks`. Re-export their public types.
*   **Why**: A single place for domain exports.

#### 7. `src/domain/day.rs` (Create)

*   **Type / Scope**: Domain value type.
*   **Proposed Changes**: `Day` is a newtype over `i32`. It derives `Copy`, `Eq`, `Ord`, `Hash`, and `Debug`. Provide `Day::from_days_since_epoch(i32)`, `days_since_epoch() -> i32`, and `pred() -> Day`. Unit tests cover `pred` and ordering.
*   **Why**: Calendar-free consecutive-day logic (Assumption 2).

#### 8. `src/domain/habit_name.rs` (Create)

*   **Type / Scope**: Domain value type.
*   **Proposed Changes**:
    *   `HabitName::parse(&str) -> Result<HabitName, NameError>`, with `NameError` variants `Empty` and `ControlCharacter`.
    *   `display() -> &str` and `key() -> &str`.
    *   Equality and ordering use the key only.
    *   Unit tests:
        *   trimming;
        *   case folding, including a non-ASCII example such as `Ärger` and `ärger`;
        *   rejection of empty and whitespace-only names;
        *   rejection of a name containing `\n` or `\x1b`.
*   **Why**: A case-insensitive key makes `check read` find `Read` and stops `Read` and `read` from becoming two habits. Rejecting control characters keeps a hand-edited name from sending terminal escape codes.

#### 9. `src/domain/habit_book.rs` (Create)

*   **Type / Scope**: Domain aggregate.
*   **Proposed Changes**:
    *   A `HabitBook` holding an ordered map from key to `Habit { name: HabitName, days: BTreeSet<Day> }`.
    *   Operations and outcome enums as listed in the Technical Plan.
    *   `HabitBook::from_entries(Vec<(HabitName, Vec<Day>)>) -> Result<HabitBook, DuplicateName>` and `entries()` for the store.
    *   Unit tests for each outcome, the merging of duplicate days, and duplicate keys.
*   **Why**: Keeps every data rule in one pure place.

#### 10. `src/domain/streaks.rs` (Create)

*   **Type / Scope**: Pure function.
*   **Proposed Changes**: `streaks(&BTreeSet<Day>, today: Day) -> Streaks { current: u32, longest: u32, checked_today: bool }`, following the algorithm in the Technical Plan. Fixed-case unit tests:
    *   empty set
    *   today only
    *   yesterday only (the grace rule)
    *   last check-in two days ago
    *   today and yesterday
    *   a gap merged by a backfill (the CUJ 3 numbers)
    *   a date after today being ignored
    *   a longest run in the past alongside a shorter current run
*   **Why**: The streak rule is the core of the tool. Computing it from dates on every run, with today as an argument, keeps one source of truth and makes the rule testable without a clock.

#### 11. `src/ports/mod.rs` (Modify)

*   **Type / Scope**: Ports root.
*   **Proposed Changes**:
    *   Delete `CountValue` and `CounterStore` (see [Scaffold code to remove](#scaffold-code-to-remove)). Keep the rule 2 doc comment, and rewrite the re-export rationale to refer to the new re-exports.
    *   Declare `habit_store`, `clock`, and `date_text`.
    *   Re-export `Day`, `HabitName`, `NameError`, `HabitBook`, `Habit`, `DuplicateName`, and `Streaks` from the domain.
*   **Why**: Adapters must get domain types through the ports (rule 4 note).

#### 12. `src/ports/habit_store.rs` (Create)

*   **Type / Scope**: Port trait.
*   **Proposed Changes**:
    *   The `HabitStore` trait with `fn load(&self) -> Result<HabitBook, StoreError>` and `fn save(&self, book: &HabitBook) -> Result<(), StoreError>`.
    *   A `StoreError` struct with `path: PathBuf` and `kind: StoreErrorKind`. The enum has the variants `Unreadable`, `NotFound`, `Malformed`, `UnknownVersion(u64)`, `MissingParentDirectory`, and `WriteFailed`.
    *   `Display` follows the message form and closing sentences in [CLI and exit codes](#cli-and-exit-codes).
*   **Why**: The contract that every data-file rule hangs on.

#### 13. `src/ports/clock.rs` (Create)

*   **Type / Scope**: Port trait.
*   **Proposed Changes**: The `Clock` trait with `fn today(&self) -> Day`.
*   **Why**: "Today" causes most of the bugs (see The Problem). A port lets tests fix it.

#### 14. `src/ports/date_text.rs` (Create)

*   **Type / Scope**: Port trait.
*   **Proposed Changes**: The `DateText` trait with `fn parse(&self, text: &str) -> Option<Day>` and `fn format(&self, day: Day) -> String`. The doc comment states the strict `YYYY-MM-DD` contract from [Date conversion](#date-conversion).
*   **Why**: Lets the CLI and the JSON store share one date conversion without importing each other (rule 6).

#### 15. `src/usecases/mod.rs` (Modify)

*   **Type / Scope**: Use-case root.
*   **Proposed Changes**:
    *   Delete `increment` and its imports. Keep the rule 3 doc comment.
    *   Declare `habits` and `error`.
    *   Re-export `Habits`, the outcome enums, `StatusReport`, `HabitStatus`, and `UseCaseError`.
*   **Why**: A single import point for the CLI.

#### 16. `src/usecases/habits.rs` (Create)

*   **Type / Scope**: Application logic.
*   **Proposed Changes**:
    *   `Habits<'a> { store: &'a dyn HabitStore, clock: &'a dyn Clock }`, with the methods `add`, `check`, `uncheck`, `remove`, and `status`. Their inputs, write conditions, and results are in the Technical Plan's use-case table.
    *   `add` maps a `NotFound` load error to an empty book and returns `created_file: Some(path)`.
    *   Names are parsed with `HabitName::parse`.
    *   An unknown name gives `UnknownHabit` with the display names of all known habits.
*   **Why**: The load, change, and save sequence in one place, with "no write unless changed".

#### 17. `src/usecases/error.rs` (Create)

*   **Type / Scope**: Error type.
*   **Proposed Changes**: The `UseCaseError` enum (`InvalidName`, `UnknownHabit`, `FutureDate`, `Store`), with `From<StoreError>`.
*   **Why**: The CLI maps exit codes from this one type.

#### 18. `src/adapters/mod.rs` (Modify), `src/adapters/primary/mod.rs` (Create), `src/adapters/secondary/mod.rs` (Modify)

*   **Type / Scope**: Module roots.
*   **Proposed Changes**:
    *   `src/adapters/mod.rs`: add `pub mod primary;` next to the existing `pub mod secondary;`.
    *   `src/adapters/primary/mod.rs`: declare `cli`.
    *   `src/adapters/secondary/mod.rs`: delete `InMemoryCounterStore`, its impl, and its `use` line (see [Scaffold code to remove](#scaffold-code-to-remove)). Keep the doc comment. Declare `json_file_store` and `chrono_calendar`.
*   **Why**: Module wiring.

#### 19. `src/adapters/primary/cli.rs` (Create)

*   **Type / Scope**: Driving adapter.
*   **Proposed Changes**:
    *   `clap` derive structs for the five subcommands and the global `--file` flag. `remove` has a required `--yes` flag.
    *   `parse(args) -> Result<Command, ParseExit { message, code }>`, with help and version mapped to 0 and all other errors mapped to 1. Dates stay as raw text in `Command`.
    *   `execute(Command, &Habits, &dyn DateText, out, err) -> i32`. It does the following:
        *   converts `--date` text with `DateText::parse` (exit 1 on `None`);
        *   calls the use case;
        *   formats output, the `created_file` note, and errors as described in the Technical Plan, with dates formatted through `DateText::format`;
        *   maps errors to exit codes.
    *   Writes to `out` stop quietly on a broken pipe.
    *   Imports only `ports`, `usecases`, `clap`, and `std`.
*   **Why**: The tool's whole user interface and the exit-code contract.

#### 20. `src/adapters/secondary/json_file_store.rs` (Create)

*   **Type / Scope**: Driven adapter.
*   **Proposed Changes**:
    *   `JsonFileStore<'a> { path, is_default, dates: &'a dyn DateText }`, implementing `HabitStore`.
    *   A private version probe struct, as in Read step 4.
    *   Private serde structs `FileV1 { version, habits: BTreeMap<String, HabitDto { checkins: Vec<String> }> }`, marked to reject unknown fields. `habits` uses the duplicate-rejecting map deserializer from Read step 5.
    *   The read and write procedures from the Technical Plan:
        *   the missing-file and dangling-symlink rules;
        *   the syntax and object check;
        *   the version mapping table;
        *   duplicate-key rejection;
        *   date conversion through `DateText`;
        *   symlink resolution on write;
        *   the rules for missing parent directories;
        *   a `0600` temp file, `sync_all`, rename, and temp-file cleanup on failure.
    *   Imports only `ports`, `serde`, `serde_json`, and `std`.
*   **Why**: All the data-safety rules from The Problem and the Data integrity requirement live here.

#### 21. `src/adapters/secondary/chrono_calendar.rs` (Create)

*   **Type / Scope**: Driven adapter.
*   **Proposed Changes**:
    *   `ChronoCalendar`, a unit struct implementing `Clock` and `DateText`.
    *   `pub fn day_from_naive(NaiveDate) -> Day` and `pub fn naive_from_day(Day) -> Option<NaiveDate>`, using the formulas in [Date conversion](#date-conversion).
    *   Strict parsing, and formatting with the `day <n>` fallback.
    *   Imports only `ports`, `chrono`, and `std`.
*   **Why**: The only reader of system time and the only home of calendar conversion.

#### 22. `tests/streak_oracle.rs` (Create)

*   **Type / Scope**: Property tests.
*   **Proposed Changes**:
    *   The oracle works on `chrono::NaiveDate` values and is written from the streak rule in the Overview, including the grace rule.
        *   `current`: walk back one calendar day at a time from today, or from yesterday, using `pred_opt`.
        *   `longest`: for each date, walk back one day at a time to count its run, and take the maximum.
        *   It ignores dates after today.
    *   `proptest` generates sets of 0 to 60 dates within a 120-day window, with "today" picked inside or just after the window. The window starts are biased toward 2024-02-29, year ends, and month ends.
    *   Each test case converts the dates with `chrono_calendar::day_from_naive`, runs `streaks`, and compares the result with the oracle.
    *   This comparison checks that the conversion keeps calendar adjacency (no gaps or repeats at month ends, year ends, and leap days). It cannot detect a constant offset. File 23 checks absolute values.
    *   Required test names: `streaks_match_oracle` and `streaks_match_oracle_across_calendar_edges`.
*   **Why**: The same author writes the code and its tests, so tests can share the code's mistakes. An oracle written from the rule text, over calendar dates, is an independent check.

#### 23. `tests/date_codec.rs` (Create)

*   **Type / Scope**: Conversion tests against `ChronoCalendar`.
*   **Proposed Changes**: Required tests:
    *   `date_codec_fixed_points`: `1970-01-01` is day 0, `1969-12-31` is day −1, `2000-03-01` is day 11017, and `2024-02-29` is day 19782, in both directions through `DateText`.
    *   `date_codec_matches_signed_duration`: a property test that `day_from_naive(d).days_since_epoch()` equals `(d - NaiveDate::from_ymd_opt(1970, 1, 1).unwrap()).num_days()` over years 0000–9999. This is a second formula from the same library.
    *   `date_codec_round_trip`: a property test that `parse(format(day)) == Some(day)` for every day in years 0000–9999.
    *   `date_codec_rejects_non_strict_text`: `2026-1-05`, `2026-02-30`, ` 2026-01-05`, `2026-01-05T00`, `20260105`, and the empty string all return `None`.
    *   `date_codec_formats_out_of_range_day`: a `Day` before 0000-01-01 formats as `day <n>` and does not panic.
*   **Why**: Pins the absolute value of the conversion, which the streak oracle cannot see.

#### 24. `tests/json_store.rs` (Create)

*   **Type / Scope**: Adapter integration tests (using `tempfile` and `ChronoCalendar`).
*   **Proposed Changes**: Required tests:
    *   `round_trip_preserves_book`: save, load, and compare. Also check that the file text has sorted dates.
    *   `default_missing_file_loads_empty`
    *   `override_missing_file_is_not_found`
    *   `dangling_symlink_is_unreadable_and_not_written`: for a default path and for an override path.
    *   `corrupt_file_is_malformed_and_unchanged`
    *   `non_object_top_level_is_malformed`
    *   `version_mapping`: one case for each row of the Read step 4 table, including `"1"`, `1.0`, `-1`, and a missing field (`Malformed`), and `0` and `2` (`UnknownVersion`).
    *   `unknown_version_is_refused_and_unchanged`
    *   `unknown_field_is_refused`
    *   `duplicate_json_keys_are_malformed`: a repeated habit key, a repeated `version`, and a repeated `checkins`. Each one is `Malformed`, and the file bytes are unchanged.
    *   `bad_date_names_habit_and_value`
    *   `duplicate_dates_merge_on_load`
    *   `case_colliding_names_are_malformed`
    *   `override_with_missing_parent_fails`
    *   `default_path_creates_parent`
    *   `save_through_symlink_keeps_symlink`
    *   `saved_file_mode_is_0600` (Unix only)
    *   `no_temp_file_left_after_save`
*   **Why**: Covers each data-safety rule, including the read-shape and write-shape lesson.

#### 25. `tests/usecases.rs` (Create)

*   **Type / Scope**: Use-case tests with test doubles.
*   **Proposed Changes**:
    *   Test doubles:
        *   An in-memory `HabitStore` that holds its book in a `RefCell<HabitBook>`, counts `save` calls in a `Cell<usize>`, and can be set to fail `load` with a chosen `StoreErrorKind`.
        *   A fixed `Clock`.
    *   Tests cover:
        *   each outcome in the use-case table;
        *   no save on no-op results and on `status`;
        *   no save after a failed load;
        *   `add` after `NotFound` saving and returning `created_file`;
        *   every other method returning `NotFound` as an error, without saving;
        *   `FutureDate`;
        *   `UnknownHabit` listing the known names;
        *   case-insensitive matching.
*   **Why**: Checks the "write only when changed, never after a failed read" rules without the filesystem.

#### 26. `tests/cli_e2e.rs` (Create)

*   **Type / Scope**: End-to-end tests of the built binary (`env!("CARGO_BIN_EXE_habit-streak")`, with `--file` pointing into a temp directory).
*   **Proposed Changes**: Required tests:
    *   `add_check_status_flow`: `add Read` on a new file, then `check read`, then `status`. Assert:
        *   current 1, longest 1, and exit 0;
        *   the `created new data file` note on stderr from `add`.

        Do not assert the "not checked today" marker, so a run that crosses midnight does not fail.
    *   `backfill_and_uncheck`: past dates via `--date`, checking the longest streak after a merge.
    *   `unknown_habit_exits_1_and_lists_names`
    *   `future_date_exits_1`
    *   `bad_arguments_exit_1`
    *   `mistyped_override_exits_2_and_creates_nothing`: `check` and `status` against a missing file in an existing directory. Assert that no file exists afterwards.
    *   `corrupt_file_exits_2_and_file_unchanged`: compare the file bytes before and after, for `check`, `add`, and `status`.
    *   `remove_requires_yes`: `remove read` without `--yes` exits 1 and leaves the file bytes unchanged.
*   **Why**: "It compiles" is not evidence that the tool works. These tests check exit codes as a script would see them.

#### 27. `README.md` (Create or Modify)

*   **Type / Scope**: User documentation.
*   **Proposed Changes**: Document:
    *   the commands and examples;
    *   the streak rules, with the grace rule defined as in the Overview;
    *   that dates are fixed when recorded, and what that means when travelling;
    *   using `--date` for check-ins after midnight;
    *   the data file location and precedence, and that only `add` creates a file at an override path;
    *   the dangling-symlink error for synced folders;
    *   the file format, how to hand-edit it safely, and that duplicate keys are rejected;
    *   the exit codes;
    *   last-writer-wins for concurrent commands;
    *   leftover `.tmp` files after a crash;
    *   using `--` for names that start with `-`.

    Remove any text about the scaffold counter example (`increment_once`, `InMemoryCounterStore`).
*   **Why**: Accepted limitations must be written down where users will find them, and documentation about deleted code must go with it.

### Verification Plan

*   **Compilation/Build**: `cargo build --release`. Also run `cargo build --all-targets` after the scaffold deletion in step 4 of the order of work.
*   **Unit Tests**: `cargo test --all-targets`. This must report more than 0 passed tests. It must include every required test name in files 4 and 22 to 26, plus the domain unit tests. `scripts/gate.sh` enforces both.
*   **Static Analysis & Linting**: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `hexa analyze .` with no boundary violations. A missing `hexa` binary fails the gate.
*   **Test Coverage Requirement**: No numeric threshold. A percentage target would reward tests written from the code. The pass criterion has three parts:
    *   every rule in this document has a required named test;
    *   the streak function agrees with the independent oracle on generated inputs;
    *   the date conversion matches fixed calendar points.
*   **E2E/Integration**: `./scripts/gate.sh`, recorded with `hexa loop gate`. After that, run a manual check:
    1.  Build the binary.
    2.  Run `add`, `check`, `status`, and `uncheck` against `HABIT_STREAK_FILE=/tmp/hs-manual.json`.
    3.  Corrupt the file by hand and confirm exit code 2 with an unchanged file.
    4.  Point `HABIT_STREAK_FILE` at `/tmp/hs-manul.json` (a typo), run `status`, and confirm exit code 2 and that no file was created.

--------------------------------------------------------------------------------

<!-- Provenance: Indicates initial structural drafting via EGM. -->

*Drafted with EGM*
