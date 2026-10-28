<!-- hexa design brainstorm: the elephant (claude-text) on egm-brainstorm, from the Elephant-Goldfish Model (EGM, https://github.com/google/elephant-goldfish-model, Google LLC, Apache-2.0) at commit a8380ab5f0aba392c97e31c79f9a6666b3f09f14 -->

# Habit Streak CLI

* **Author:** gary (gary@sticksandstrings.org)
* **Date:** 2026-10-28

## Overview

`habit-streak` is a command-line tool for one person on one machine. The person records a check-in for a named habit on a calendar date. For each habit, the tool reports two numbers:

* **Current streak:** the number of consecutive days with a check-in, ending today or yesterday.
* **Longest streak:** the most consecutive days with a check-in at any point in the habit's history.

All data lives in one local JSON file. Several check-ins for the same habit on the same day count as one. A day with no check-in breaks the streak.

The intended user wants a private, offline record that can be read and backed up as plain text. They do not want an account, a sync service, or a phone app.

### Assumptions made without clarification

This brief was written in one pass, with no clarification turns and no Goldfish review yet. hexa runs the reviewers separately. These are the choices most likely to be challenged:

1. **"Today" means the local calendar date on the machine running the command, at the moment it runs.** The tool stores dates, not timestamps.
2. **Backfilling is allowed.** A check-in may target a past date. Future dates are rejected.
3. **Habits must be created before they can be checked in.** A check-in for an unknown name is an error, not a silent new habit.
4. **Every habit is daily.** Weekly habits, rest days, and "3 times a week" targets do not exist in v1.
5. **There is one user and one data file.** No multi-user support and no sync.
6. **The project also serves as a hexagonal-architecture exercise.** The repository enforces that layout, and the design follows it even where a single file would do.

## Analysis

This analysis is the elephant's own stress-test. It does not replace the Goldfish reviewers. It lists weak points first, then what holds up.

### 1. Why build this at all? (Contrarian / YAGNI)

**Objection:** This tool already exists in several forms. `habitctl` is a small CLI habit tracker. Loop Habit Tracker covers phones. A text file with one `habit date` line per check-in, plus about 30 lines of shell or Python, computes both streaks. If the only goal is to track habits, writing a Rust program with ports and adapters costs more than the problem is worth.

**Counter-argument:** This tool's value is control over the data format and the streak rules, plus practice with the repository's architecture. That holds only if the person accepts the opportunity cost. If they would be satisfied with `habitctl`, they should use it.

**Consequence for scope:** Every feature beyond record, report, and undo has to justify itself against "a text file and a script." Keep v1 small.

### 2. The definition of "today" is the hardest part

The core rule, "consecutive days ending today or yesterday," depends on what "today" means. That is where the bugs will be.

* **Time zones and travel.** A person checks in at 23:30 in New York and flies to London. On the local clock it is now the next day. If the tool stored UTC timestamps, the check-in could land on a different date than the person intended. **Decision:** store plain calendar dates (`YYYY-MM-DD`) computed from the local clock when the check-in is recorded. A date never changes after it is stored. The cost: a person who crosses time zones may see a streak gain or lose a day at the edges. That is accepted and documented.
* **Late-night check-ins.** A person who goes to bed at 01:00 and checks in "for yesterday" at 00:30 will record the wrong day. The `--date` flag covers this case. A configurable "day starts at 04:00" setting would handle it automatically, but it is out of scope for v1.
* **A wrong system clock** gives wrong streaks. The tool cannot detect this and will not try.
* **Testability.** If the streak function reads the system clock directly, it cannot be tested against fixed dates. **Decision:** the domain function receives "today" as an argument. The clock sits behind a port.

### 3. The "today or yesterday" grace rule has edge cases

* Last check-in yesterday, none today: current streak counts back from yesterday. The streak is still alive because the day is not over.
* Last check-in two or more days ago: current streak is 0.
* Check-ins today and yesterday: count back from today.
* No check-ins: current 0, longest 0.
* A backfilled date that fills a one-day gap merges two runs. The longest streak can then jump, for example from 5 and 6 to 12. That is the intended result, and the tests should cover it.
* Month ends, year ends, and 29 February must count as consecutive days. This is why date arithmetic should come from a date library and not from hand-written code.

### 4. Derived numbers must not be stored

It is tempting to store `current_streak` and `longest_streak` in the JSON file to save work. Don't. A stored streak is a second source of truth that can disagree with the dates, for example after a backfill, an undo, or a hand edit. The project's own lessons warn against this. A heavy user has about 365 dates per habit per year, so computing both streaks from the date set on every run costs nothing measurable. **Decision:** store only habits and their check-in dates. Compute everything else.

### 5. Data file safety (Migration & Backward Compatibility)

* **Corrupt or unreadable file.** The worst outcome is a parse error treated as "no data." The tool would report empty and then overwrite the user's history on the next write. **Decision:** any read or parse failure stops the command with a non-zero exit code. The message names the file path and the parse error. The tool never writes after a failed read.
* **Half-written file.** A crash during a write could truncate the file. **Decision:** write to a temporary file in the same directory, flush it, then rename it over the original. On Linux and macOS the rename replaces the file in one step.
* **Two commands at once** (for example, two terminals). With write-then-rename, the last writer wins and one check-in can be lost. For a single person this is rare. **Decision:** accept it in v1 and document it. A lock file is the fix if it ever happens in practice.
* **Format changes.** **Decision:** the file has a top-level `"version": 1`. A file with an unknown version is refused with a clear message and is never rewritten. Without this field, a later format change has no safe way to tell old files from new ones.
* **Write shape and read shape must match.** Dates are written as `"YYYY-MM-DD"` strings and must be read back as the same type. A round-trip test (write, read, compare) is part of the gate.
* **Hand edits.** People will edit the JSON by hand. Duplicate dates in the file are merged on read. Badly formatted dates are rejected with the habit name and the bad value.

Illustrative file shape (not production code):

```json
{
  "version": 1,
  "habits": {
    "read": { "created": "2026-10-01", "checkins": ["2026-10-01", "2026-10-02"] },
    "run":  { "created": "2026-10-05", "checkins": [] }
  }
}
```

Dates are written in sorted order with no duplicates, so diffs and backups stay readable.

### 6. Habit names (UX & Developer Ergonomics)

* **Typos.** If `check reed` silently creates a habit called `reed`, the person's real `read` streak breaks and they may not notice for days. **Decision:** habits are created with an explicit `add` command. `check` on an unknown name fails, and the message lists existing habit names. This costs one extra command per habit, once.
* **Case and spaces.** `Read`, `read`, and `read ` should not become three habits. **Decision:** trim surrounding whitespace and compare names case-insensitively. Keep the spelling the person used when they first added the habit for display. Reject empty names.
* **Undo.** Backfilling means mistakes will happen, such as the wrong date or the wrong habit. Without an undo command, the only fix is a hand edit of the JSON. **Decision:** include `uncheck <habit> --date <date>` in v1. Deleting a whole habit is also a small command with a confirmation flag. It is included because the alternative is a hand edit.

Illustrative command surface (not production code):

```
habit-streak add <habit>
habit-streak check <habit> [--date YYYY-MM-DD]
habit-streak uncheck <habit> --date YYYY-MM-DD
habit-streak status [<habit>]
habit-streak remove <habit> --yes
```

`check` on a date that already has a check-in succeeds and prints "already recorded." It does not fail, because the end state is what the person wanted.

### 7. Errors and exit codes (Observability)

A CLI has no tracing to inspect. Its error messages are all the diagnosis a user gets. Every error says what failed, which file or habit was involved, and what to do next. Exit codes: `0` for success, `1` for a user error (unknown habit, bad date, future date), `2` for a data file problem (unreadable, corrupt, unknown version). Scripts can rely on these codes.

### 8. Cost and scale

There is nothing here to worry about. The file is a few kilobytes per habit after several years. Each command loads the whole file, computes, and exits. There is no network, no quota, and no cache. A cache in front of a file read by a short-lived process would be a second source of truth, so do not add one.

### 9. Dependencies and maintenance (Long-Term Tech Debt)

* **JSON:** `serde` and `serde_json`. Writing a JSON parser by hand is not justified.
* **Dates:** Rust's standard library does not provide the local calendar date. A date crate is required. Pick one (`jiff` or `chrono`) in an ADR and use it only inside the clock adapter and for date arithmetic.
* **Argument parsing:** `clap` is the usual choice. A hand-written parser for five subcommands would also work and avoids a large dependency. Either is defensible. Record the choice in the same ADR.
* **Coupling risk.** If the domain uses the date crate's type directly, every layer depends on that crate. This is an accepted trade-off: dates are core to the domain, and wrapping them adds code with no benefit. Keep the domain free of `serde` and filesystem code.

### 10. Mapping to the enforced architecture

* **domain/** — `HabitName` (with normalization), the set of check-in dates for a habit, and a pure `streaks(dates, today) -> (current, longest)` function. No I/O and no clock access.
* **ports/** — `HabitStore` (load and save the whole data set) and `Clock` (`today()`). The ports re-export the domain types that adapters need.
* **usecases/** — add habit, record check-in, remove check-in, remove habit, report status. Each loads, changes, and saves through the ports.
* **adapters/primary/** — the CLI: parses arguments, calls use cases, formats output and exit codes.
* **adapters/secondary/** — the JSON file store (atomic write, version check, loud failure on bad data) and the system clock.
* **composition root** — `main.rs` picks the data file path and wires the adapters.

**Data file location:** by default, the platform data directory (on Linux, `$XDG_DATA_HOME/habit-streak/habits.json`, falling back to `~/.local/share/...`). The `HABIT_STREAK_FILE` environment variable or a `--file` flag overrides it. The override matters for tests and for people who keep the file in a synced folder.

### 11. Testing risk

The same model writes the streak code and the tests, so the tests may repeat the code's misunderstanding. **Decision:** check the streak function against an independent, plainly slow oracle. For example: walk backward one day at a time from today and count. Run both on randomly generated date sets with property tests. Add fixed cases for each edge in section 3. Add an end-to-end test that runs the built binary against a temporary data file and checks both output and exit code. "It compiles" is not evidence the tool works. A gate that runs zero tests counts as a failure.

### What holds up

The problem is small and well bounded. The data fits in memory. Streaks are computed from a plain set of dates, so there is no state machine to maintain. The one-file, offline design leaves no server or sync process to break. The real risks are about definitions (what "today" means, what counts as the same habit) and data safety (corrupt files, lost writes). Both can be handled with the decisions above.

## Goals vs. Non-Goals

* **In-Scope Goals:**
  * Create, list, and remove named daily habits.
  * Record a check-in for today or a given past date. Same-day repeats are accepted and stored once.
  * Remove a check-in for a given date.
  * Report current streak (ending today or yesterday) and longest streak for one habit or all habits.
  * Store data in one versioned, human-readable JSON file with atomic writes.
  * Fail loudly, without writing, on any unreadable or unknown-version data file.
  * Clear error messages and stable exit codes.
  * Use local calendar dates, with the clock behind a port so tests can control it.

* **Out-of-Scope Non-Goals:**
  * Weekly or "N times per week" habits, rest days, freeze days, or streak forgiveness.
  * A configurable start-of-day hour for late-night users.
  * Sync, multi-device use, multiple users, or any network access.
  * Locking against simultaneous commands. Last write wins, and this is documented.
  * Notifications, reminders, charts, calendars, or a GUI or TUI.
  * Notes, quantities, or durations attached to a check-in.
  * Importing from or exporting to other trackers.
  * Machine-readable (`--json`) output. Keep the status report as plain data inside the use case so it can be added later without changing the domain.
  * Time-zone-aware storage. Dates are fixed when recorded.

## Key Decisions & Staging for Next Steps

**Decisions to carry into `/egm-draft`:**

1. Store calendar dates only (`YYYY-MM-DD`). "Today" is the local system date, supplied through a `Clock` port.
2. Never store streak numbers. Compute them from the date set on every run.
3. The streak rule: current streak counts back from today if today has a check-in. Otherwise it counts back from yesterday if yesterday has one. Otherwise it is 0.
4. Future dates are rejected. Past dates are allowed.
5. Habits are created explicitly. Names are trimmed and compared case-insensitively. Unknown names are an error that lists the known names.
6. `uncheck` and `remove` are in v1.
7. JSON file with `"version": 1`. Writes go to a temp file, then rename. Any read failure stops the command with exit code 2 and no write.
8. Default path follows XDG, with `--file` and `HABIT_STREAK_FILE` overrides.
9. Dependencies: `serde`, `serde_json`, one date crate, and possibly `clap`. This needs an ADR before code, as the project requires.

**Gate to record before any code (`hexa loop gate`):** `cargo test` must pass with a non-zero test count. It must include (a) property tests comparing `streaks` against an independent day-by-day oracle, (b) a JSON write/read round-trip test, (c) a test that a corrupt file causes exit code 2 and leaves the file byte-for-byte unchanged, and (d) an end-to-end test that runs the built binary to `add`, `check`, and `status` a habit against a temporary data file.

**Open questions for the Goldfish reviewers and the user:**

* Is explicit `add` worth the friction, or would a "did you mean `read`?" prompt on unknown names be enough?
* Is "dates are fixed when recorded" acceptable for someone who travels often?
* Is the architecture practice a real goal? If not, the YAGNI objection in section 1 stands and an existing tool is the better choice.
