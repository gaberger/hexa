---
id: ADR-2610281300
status: proposed
date: 2026-10-28
---
# ADR-2610281300: habit-streak edges: dates, the JSON file, the data path and the command line

**Status:** Proposed
**Date:** 2026-10-28
**Drivers:** ADR-2610281223 decided the core against two ports. This decides the adapters behind them and the command line in front, so the tool runs. The decisions follow the reviewed design `docs/designs/design_draft.md`, sections JSON file store, Data path rules, Date conversion, and CLI and exit codes.
**Design:** docs/designs/design_draft.md

## Context

The core knows a `Day` only as a day count, and a book only through `HabitStore`. A person types and reads dates as `YYYY-MM-DD`, keeps the book in a file they can read and fix by hand, and runs the tool from a shell that reads its exit code.

## Decision

1. **Dates have one strict text form, and one adapter knows the calendar.** The port `DateText` in `src/ports/mod.rs` has `fn parse(&self, text: &str) -> Option<Day>` and `fn format(&self, day: Day) -> String`. `ChronoCalendar`, a unit struct in `src/adapters/secondary/chrono_calendar.rs`, implements `DateText` and `Clock`, and is the only file that names `chrono`. `parse` accepts exactly ten characters `YYYY-MM-DD` that form a real date and returns `None` for anything else (`2026-1-05`, `2026-02-30`, ` 2026-01-05`, `2026-01-05T00`). 1970-01-01 is `Day::from_days(0)`. `format` is the inverse of `parse` for every year 0000–9999. `today` is the local calendar date.
2. **The book is a version 1 JSON file.** `JsonFileStore::new(path: PathBuf, is_default: bool, dates: &dyn DateText)` in `src/adapters/secondary/json_file_store.rs` implements `HabitStore`; it reads and writes `{"version": 1, "habits": {"<name>": {"checkins": ["YYYY-MM-DD", …]}}}`. It writes habits in key order and dates ascending, pretty-printed with a trailing newline. A loaded book saved again is byte-identical to a file it wrote.
3. **A file that is not a valid version 1 book is refused, never rewritten.** `load` returns `Malformed` for invalid JSON, a top level that is not an object, a missing or non-integer `version`, an unknown field anywhere, a date `parse` refuses, a name `HabitName::parse` refuses, the same habit key twice, and two names that differ only in case. A `version` that is a non-negative integer other than 1 is `UnknownVersion(n)`. Each message names what is wrong.
4. **A missing file is a first run only at the default path.** At the default path, a missing file loads as an empty book and `save` creates the missing directories. At a path the person named, a missing file is `NotFound`, and a missing parent directory is an `Io` error from `save`: a typo must fail, not start a new history somewhere else.
5. **A write replaces the file whole or not at all.** `save` writes a temporary file in the same directory, flushes it with `sync_all`, and renames it over the target. On failure it removes the temporary file and returns `Io`; the original file is never opened for writing.
6. **The data path is chosen in a fixed order.** `resolve_data_path(flag: Option<PathBuf>, env: &dyn Fn(&str) -> Option<String>) -> Result<DataPath, NoDataPath>` in `src/lib.rs`, with `DataPath { path: PathBuf, is_default: bool }` and the unit struct `NoDataPath`, returns, in order: `--file <path>`; `HABIT_STREAK_FILE` when non-empty; `$XDG_DATA_HOME/habit-streak/habits.json` when non-empty; `$HOME/.local/share/habit-streak/habits.json` when non-empty; else an error. Only the last two are the default path.
7. **The command line.** `habit_streak::run(args: &[String], env: &dyn Fn(&str) -> Option<String>, out: &mut dyn std::io::Write, err: &mut dyn std::io::Write) -> i32` in `src/lib.rs` is the whole program, where `args` are the arguments after the program name; `src/main.rs` calls it with the process's arguments, environment, stdout and stderr, and exits with its result. The commands are `add <habit>`, `check <habit> [--date YYYY-MM-DD]`, `uncheck <habit> --date YYYY-MM-DD`, `status [<habit>]` and `remove <habit> --yes`, each after an optional global `--file <path>`.
   - `status` prints one line per habit in key order: the name padded to the longest name, `current <n>`, `longest <n>`, and ` (not checked today)` when the current streak is above 0 and today has no check-in. With no habits it prints `No habits yet. Add one with: habit-streak add <name>`.
   - `add`, `check`, `uncheck` and `remove` print one line saying what happened, naming the habit and, for a day, its `YYYY-MM-DD` form; a command whose end state already held says so and exits 0.
   - Every error goes to stderr as one line starting `error: `. An unknown habit lists the known ones.
8. **Exit codes say whose problem it is.** 0 on success, including help and a no-op. 1 for a usage error (an unknown command, a missing argument, `remove` without `--yes`, a `--date` that does not parse), an invalid name, an unknown habit and a future date. 2 for any store error and for no data path. A store error leaves the file byte-identical.

## Consequences

- The tool runs: `cargo run -- check Read` records today.
- Writing through a symlink, the temporary file's mode, and concurrent runs are not decided here; the design's rules for them remain to be decided in a later ADR. Two runs at once can lose one change.
- `clap`, `chrono`, `serde` and `serde_json` become dependencies, each used only in the adapter or entry point that needs it.

## Implementation

`src/ports/mod.rs` (`DateText`), `src/adapters/secondary/chrono_calendar.rs`, `src/adapters/secondary/json_file_store.rs`, `src/adapters/primary/cli.rs`, `src/lib.rs`, `src/main.rs`, `Cargo.toml`.

## Gate

`hexa spec verify ADR-2610281300`: the spec's sealed tests, each red before the code.

## References

- ADR-2610281223: the core these adapters serve.
- `docs/designs/design_draft.md`: the reviewed design.
