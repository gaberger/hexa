# habit-streak

A command-line habit tracker. Record a check-in for a habit on a day; see each
habit's current streak and its longest streak. The data is one JSON file you
can read and fix by hand.

```console
$ ./run.sh add Read
added habit "Read"
$ ./run.sh check Read --date 2026-10-27
checked in "Read" on 2026-10-27
$ ./run.sh status
Read  current 1  longest 1 (not checked today)
```

The streak survives until the end of the day after your last check-in:
`(not checked today)` means a check-in today keeps it alive.

| Command | |
|---|---|
| `add <habit>` | start tracking a habit |
| `check <habit> [--date YYYY-MM-DD]` | record a check-in (today by default; never a future day) |
| `uncheck <habit> --date YYYY-MM-DD` | remove a check-in |
| `status [<habit>]` | current and longest streak, per habit |
| `remove <habit> --yes` | stop tracking a habit and delete its history |

The data file is `--file <path>`, else `$HABIT_STREAK_FILE`, else
`$XDG_DATA_HOME/habit-streak/habits.json`, else
`~/.local/share/habit-streak/habits.json`. A file you name must exist; a typo
fails rather than starting a new history. A file that is not a valid version 1
book is refused and never rewritten. Exit codes: 0 done, 1 your input, 2 the
data file.

## How it was made

Every step ran through hexa, on real models, from one sentence of intent:

1. **Design** (`docs/designs/`). `hexa design brainstorm` wrote the concepts
   brief and the critic goldfish reviewed it; `hexa design draft` wrote a
   676-line design. `hexa design review` ran EGM's five goldfish stages: 132
   findings, the same few raised independently by every stage. Twelve were
   resolved with `hexa design resolve`, and the second review had no Blocker.
2. **Decide** (`docs/adrs/`). `hexa design adr` compiled the committed,
   reviewed design into an ADR, split into the core (ADR-2610281223) and the
   edges (ADR-2610281300).
3. **Spec** (`docs/specs/`). A spec author that saw only the ADRs and the port
   and domain signatures wrote 78 scenarios; a person accepted them.
4. **Gate**. A test writer that never saw the code sealed one test per
   scenario, each shown red before the code existed (`hexa spec verify --prove`).
5. **Build** to the sealed tests: 41 of 41 core, 37 of 37 edges; `hexa analyze .`
   grades A+, 100/100.

Running it found eight gaps in hexa itself, each fixed in hexa: a design
context without the build manifest, an ADR compiler that took a design's
framing for decisions, a grader false positive on child modules, sealed tests
that could never pass, a test writer blind to the API it tested, no way to
repair a sealed test after the code existed, a red check that shared a build
directory with HEAD, and that test writer reading HEAD's code after it.

## Layout

```
src/domain/      Day, HabitName, HabitBook, streaks — pure
src/ports/       HabitStore, Clock, DateText, and the driving port HabitCommands
src/usecases/    Habits: add, check, uncheck, remove, status
src/adapters/    secondary: ChronoCalendar, JsonFileStore · primary: cli
src/lib.rs       the composition root: resolve_data_path, run
tests/           the sealed spec tests, one per scenario
```

```bash
cargo test                       # 78 sealed tests
hexa spec verify --prove         # each passes, and was red before the code
hexa analyze .                   # the architecture grade
```
