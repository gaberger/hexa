//! Integration tests for the habit-streak decisions (ADR-2610281223, ADR-2610281300).
//!
//! Only the port signatures in `habit_streak::ports` and the `habit-streak`
//! binary were provided. The adapter types the scenarios name
//! (`ChronoCalendar`, `JsonFileStore`, `resolve_data_path`, `DataPath`),
//! `Day::from_days`, `HabitBook` and the signature of `habit_streak::run` were
//! not shown, so no test calls them directly. Every scenario is checked through
//! the binary instead. Its exit code, stdout and stderr are what `run` returns
//! and writes. Where a scenario's `then` uses an unshown value, the test checks
//! the same behaviour as it is visible at the command line.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const BIN: &str = env!("CARGO_BIN_EXE_habit-streak");
const NONE: &[&str] = &[];

// ---------------------------------------------------------------- helpers

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let p = std::env::temp_dir().join(format!(
            "habit-streak-test-{}-{}-{}",
            std::process::id(),
            n,
            nanos
        ));
        fs::create_dir_all(&p).expect("create temp dir");
        TempDir(p)
    }
    fn path(&self) -> &Path {
        &self.0
    }
    fn join(&self, rel: &str) -> PathBuf {
        self.0.join(rel)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Makes a directory read-only and restores it when dropped.
struct ReadOnly(PathBuf);

impl ReadOnly {
    fn new(p: &Path) -> Self {
        fs::set_permissions(p, fs::Permissions::from_mode(0o555)).expect("chmod 555");
        ReadOnly(p.to_path_buf())
    }
}

impl Drop for ReadOnly {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
    }
}

/// True when the directory really refuses new files. This is false when the
/// tests run as root.
fn dir_refuses_new_files(dir: &Path) -> bool {
    let probe = dir.join(".probe-habit-streak");
    match fs::File::create(&probe) {
        Ok(_) => {
            let _ = fs::remove_file(&probe);
            false
        }
        Err(_) => true,
    }
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

/// Runs the binary with HABIT_STREAK_FILE, XDG_DATA_HOME and HOME removed,
/// then sets the given variables (`None` leaves a variable unset).
fn run_with(args: &[&str], env: &[(&str, Option<&str>)]) -> Run {
    let mut c = Command::new(BIN);
    c.args(args);
    for v in ["HABIT_STREAK_FILE", "XDG_DATA_HOME", "HOME"] {
        c.env_remove(v);
    }
    for (k, v) in env {
        if let Some(v) = v {
            c.env(k, v);
        }
    }
    let out = c.output().expect("spawn habit-streak");
    Run {
        code: out.status.code().expect("habit-streak exited by signal"),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn run_file(file: &Path, args: &[&str]) -> Run {
    let f = file.to_str().expect("utf-8 path");
    let mut a = vec!["--file", f];
    a.extend_from_slice(args);
    run_with(&a, &[])
}

fn one_line(s: &str) -> &str {
    let t = s.strip_suffix('\n').unwrap_or(s);
    assert!(!t.contains('\n'), "expected exactly one line, got {:?}", s);
    t
}

fn assert_error_line(r: &Run) {
    let line = one_line(&r.stderr);
    assert!(
        line.starts_with("error: "),
        "stderr should be one line starting \"error: \", got {:?}",
        r.stderr
    );
    assert!(line.len() > "error: ".len(), "error message is empty");
}

fn assert_one_stdout_line(r: &Run) -> String {
    assert!(!r.stdout.is_empty(), "stdout is empty");
    one_line(&r.stdout).to_string()
}

fn write(path: &Path, text: &str) {
    fs::write(path, text).expect("write file");
}

fn bytes(path: &Path) -> Vec<u8> {
    fs::read(path).expect("read file")
}

fn entries(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .expect("read dir")
        .map(|e| e.expect("dir entry").file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

fn book_json(habits: &[(&str, &[&str])]) -> String {
    let body: Vec<String> = habits
        .iter()
        .map(|(n, ds)| {
            let dates: Vec<String> = ds.iter().map(|d| format!("\"{}\"", d)).collect();
            format!("\"{}\": {{\"checkins\": [{}]}}", n, dates.join(", "))
        })
        .collect();
    format!("{{\"version\": 1, \"habits\": {{{}}}}}", body.join(", "))
}

fn date_cmd(spec: Option<&str>) -> String {
    let mut c = Command::new("date");
    if let Some(s) = spec {
        c.arg("-d").arg(s);
    }
    c.arg("+%F");
    let out = c.output().expect("run `date`");
    assert!(out.status.success(), "`date` failed");
    String::from_utf8(out.stdout).expect("utf-8").trim().to_string()
}

fn is_leap(y: u32) -> bool {
    y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
}

fn days_in_month(y: u32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(y) => 29,
        _ => 28,
    }
}

fn expected(habits: &[(&str, &[&str])]) -> Vec<(String, Vec<String>)> {
    let mut v: Vec<(String, Vec<String>)> = habits
        .iter()
        .map(|(n, ds)| {
            let mut d: Vec<String> = ds.iter().map(|s| s.to_string()).collect();
            d.sort();
            d.dedup();
            (n.to_string(), d)
        })
        .collect();
    v.sort();
    v
}

// ---- minimal JSON reader (structural comparison only)

#[derive(Debug, Clone, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Num(String),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    fn normalized(self) -> Json {
        match self {
            Json::Arr(a) => Json::Arr(a.into_iter().map(Json::normalized).collect()),
            Json::Obj(o) => {
                let mut o: Vec<(String, Json)> =
                    o.into_iter().map(|(k, v)| (k, v.normalized())).collect();
                o.sort_by(|a, b| a.0.cmp(&b.0));
                Json::Obj(o)
            }
            other => other,
        }
    }
}

struct P {
    c: Vec<char>,
    i: usize,
}

impl P {
    fn ws(&mut self) {
        while self.i < self.c.len() && self.c[self.i].is_whitespace() {
            self.i += 1;
        }
    }
    fn peek(&self) -> char {
        *self.c.get(self.i).expect("unexpected end of JSON")
    }
    fn expect(&mut self, ch: char) {
        self.ws();
        assert_eq!(self.peek(), ch, "bad JSON at {}", self.i);
        self.i += 1;
    }
    fn value(&mut self) -> Json {
        self.ws();
        match self.peek() {
            '{' => {
                self.i += 1;
                let mut v = Vec::new();
                self.ws();
                if self.peek() == '}' {
                    self.i += 1;
                    return Json::Obj(v);
                }
                loop {
                    self.ws();
                    let k = self.string();
                    self.expect(':');
                    let val = self.value();
                    v.push((k, val));
                    self.ws();
                    match self.peek() {
                        ',' => self.i += 1,
                        '}' => {
                            self.i += 1;
                            return Json::Obj(v);
                        }
                        c => panic!("bad JSON object char {:?}", c),
                    }
                }
            }
            '[' => {
                self.i += 1;
                let mut v = Vec::new();
                self.ws();
                if self.peek() == ']' {
                    self.i += 1;
                    return Json::Arr(v);
                }
                loop {
                    v.push(self.value());
                    self.ws();
                    match self.peek() {
                        ',' => self.i += 1,
                        ']' => {
                            self.i += 1;
                            return Json::Arr(v);
                        }
                        c => panic!("bad JSON array char {:?}", c),
                    }
                }
            }
            '"' => Json::Str(self.string()),
            't' => {
                self.i += 4;
                Json::Bool(true)
            }
            'f' => {
                self.i += 5;
                Json::Bool(false)
            }
            'n' => {
                self.i += 4;
                Json::Null
            }
            _ => {
                let st = self.i;
                while self.i < self.c.len()
                    && (self.c[self.i].is_ascii_digit() || "-+.eE".contains(self.c[self.i]))
                {
                    self.i += 1;
                }
                assert!(self.i > st, "bad JSON value at {}", st);
                Json::Num(self.c[st..self.i].iter().collect())
            }
        }
    }
    fn string(&mut self) -> String {
        assert_eq!(self.peek(), '"', "expected string");
        self.i += 1;
        let mut s = String::new();
        loop {
            let ch = self.peek();
            self.i += 1;
            match ch {
                '"' => return s,
                '\\' => {
                    let e = self.peek();
                    self.i += 1;
                    match e {
                        'n' => s.push('\n'),
                        't' => s.push('\t'),
                        'r' => s.push('\r'),
                        'b' => s.push('\u{8}'),
                        'f' => s.push('\u{c}'),
                        'u' => {
                            let hex: String = self.c[self.i..self.i + 4].iter().collect();
                            self.i += 4;
                            let n = u32::from_str_radix(&hex, 16).expect("hex");
                            s.push(char::from_u32(n).unwrap_or('\u{fffd}'));
                        }
                        other => s.push(other),
                    }
                }
                other => s.push(other),
            }
        }
    }
}

fn parse_json(s: &str) -> Json {
    let mut p = P {
        c: s.chars().collect(),
        i: 0,
    };
    let v = p.value();
    p.ws();
    assert_eq!(p.i, p.c.len(), "trailing text after JSON");
    v
}

fn field<'a>(j: &'a Json, k: &str) -> &'a Json {
    match j {
        Json::Obj(e) => &e
            .iter()
            .find(|(kk, _)| kk == k)
            .unwrap_or_else(|| panic!("missing field {:?}", k))
            .1,
        other => panic!("not an object: {:?}", other),
    }
}

fn habits_of(path: &Path) -> Vec<(String, Vec<String>)> {
    let text = fs::read_to_string(path).expect("read book");
    let json = parse_json(&text);
    match field(&json, "habits") {
        Json::Obj(entries) => {
            let mut v: Vec<(String, Vec<String>)> = entries
                .iter()
                .map(|(k, h)| {
                    let mut ds: Vec<String> = match field(h, "checkins") {
                        Json::Arr(items) => items
                            .iter()
                            .map(|i| match i {
                                Json::Str(s) => s.clone(),
                                o => panic!("checkin not a string: {:?}", o),
                            })
                            .collect(),
                        o => panic!("checkins not an array: {:?}", o),
                    };
                    ds.sort();
                    (k.clone(), ds)
                })
                .collect();
            v.sort();
            v
        }
        o => panic!("habits not an object: {:?}", o),
    }
}

fn file_has_habit(path: &Path, name: &str) -> bool {
    path.exists() && habits_of(path).iter().any(|(n, _)| n == name)
}

// ---------------------------------------------------------------- tests

/// S01: The epoch and an ordinary date parse to their day counts and format back
#[test]
fn s01_epoch_and_ordinary_date_round_trip() {
    let d = TempDir::new();
    let f = d.join("habits.json");
    write(&f, &book_json(&[("Read", NONE)]));
    for date in ["2026-01-05", "1970-01-01"] {
        let r = run_file(&f, &["check", "Read", "--date", date]);
        assert_eq!(r.code, 0, "check --date {} failed: {}", date, r.stderr);
        assert!(r.stdout.contains(date), "output should name {}: {:?}", date, r.stdout);
    }
    let text = fs::read_to_string(&f).unwrap();
    let a = text.find("1970-01-01").expect("1970-01-01 written back");
    let b = text.find("2026-01-05").expect("2026-01-05 written back");
    assert!(a < b, "the epoch must sort before 2026-01-05");
    assert_eq!(
        habits_of(&f),
        expected(&[("Read", &["1970-01-01", "2026-01-05"][..])])
    );
}

/// S02: A leap day parses only in a leap year
#[test]
fn s02_leap_day_only_in_leap_year() {
    let d = TempDir::new();
    let f = d.join("habits.json");
    write(&f, &book_json(&[("Read", NONE)]));
    let r = run_file(&f, &["check", "Read", "--date", "2024-02-29"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(habits_of(&f), expected(&[("Read", &["2024-02-29"][..])]));

    let before = bytes(&f);
    let r = run_file(&f, &["check", "Read", "--date", "2023-02-29"]);
    assert_eq!(r.code, 1);
    assert_error_line(&r);
    assert_eq!(bytes(&f), before);
}

/// S03: Any text that is not exactly ten characters forming a real YYYY-MM-DD date is refused
#[test]
fn s03_non_dates_are_refused() {
    let d = TempDir::new();
    let f = d.join("habits.json");
    write(&f, &book_json(&[("Read", NONE)]));
    let before = bytes(&f);
    for bad in [
        "2026-1-05",
        "2026-02-30",
        " 2026-01-05",
        "2026-01-05 ",
        "2026-01-05T00",
        "2026/01/05",
        "20260105",
        "2026-13-01",
        "2026-00-10",
        "2026-01-00",
        "+026-01-05",
        "２０２６-01-05",
        "",
    ] {
        let r = run_file(&f, &["check", "Read", "--date", bad]);
        assert_eq!(r.code, 1, "date {:?} should be refused", bad);
        assert_error_line(&r);
        assert!(r.stdout.is_empty());
        assert_eq!(bytes(&f), before, "file changed for {:?}", bad);
    }
}

/// S04: format is the inverse of parse across years 0000 to 9999
#[test]
fn s04_format_inverts_parse() {
    let mut dates: Vec<String> = vec![
        "0000-01-01".into(),
        "0000-02-29".into(),
        "0001-01-01".into(),
        "0400-02-29".into(),
        "1900-02-28".into(),
        "1970-01-01".into(),
        "2000-02-29".into(),
        "9999-12-31".into(),
    ];
    let mut y = 0u32;
    while y <= 9999 {
        let m = (y % 12) + 1;
        let day = 1 + (y * 7) % days_in_month(y, m);
        dates.push(format!("{:04}-{:02}-{:02}", y, m, day));
        y += 97;
    }
    dates.sort();
    dates.dedup();
    let refs: Vec<&str> = dates.iter().map(|s| s.as_str()).collect();

    let d = TempDir::new();
    let f = d.join("habits.json");
    write(&f, &book_json(&[("Read", &refs[..])]));
    // The store parses every date on load and formats it on save.
    let r = run_file(&f, &["add", "Gym"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(
        habits_of(&f),
        expected(&[("Gym", NONE), ("Read", &refs[..])])
    );
    // parse(format(day)) == day: a second round trip gives the same dates.
    let r = run_file(&f, &["add", "Walk"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(
        habits_of(&f),
        expected(&[("Gym", NONE), ("Read", &refs[..]), ("Walk", NONE)])
    );
}

/// S05: parse accepts only text of exactly ten characters
#[test]
fn s05_only_ten_characters_parse() {
    let d = TempDir::new();
    let f = d.join("habits.json");
    write(&f, &book_json(&[("Read", NONE)]));
    let before = bytes(&f);
    let base = "2026-01-05";
    let mut cases: Vec<String> = Vec::new();
    for n in 0..10 {
        cases.push(base[..n].to_string());
    }
    for extra in ["0", "5", " ", "Z", "00", "T00:00", "-01"] {
        cases.push(format!("{}{}", base, extra));
        cases.push(format!("{}{}", extra, base));
    }
    cases.push("２０２６-01-055".into());
    cases.push("é".repeat(9));
    cases.push("é".repeat(11));
    for bad in &cases {
        assert_ne!(bad.chars().count(), 10, "test case {:?} is ten characters", bad);
        let r = run_file(&f, &["check", "Read", "--date", bad]);
        assert_eq!(r.code, 1, "date {:?} should be refused", bad);
        assert_error_line(&r);
        assert_eq!(bytes(&f), before);
    }
}

/// S06: today is the local calendar date
#[test]
fn s06_today_is_local_date() {
    let d = TempDir::new();
    let f = d.join("habits.json");
    write(&f, &book_json(&[("Read", NONE)]));
    let before = date_cmd(None);
    let r = run_file(&f, &["check", "Read"]);
    let after = date_cmd(None);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let h = habits_of(&f);
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].1.len(), 1);
    let got = &h[0].1[0];
    assert!(got == &before || got == &after, "today {} not {} or {}", got, before, after);
}

/// S07: A saved book is written as pretty-printed version 1 JSON, habits in key order and dates ascending, ending in a newline
#[test]
fn s07_saved_book_format() {
    let d = TempDir::new();
    let f = d.join("habits.json");
    write(&f, &book_json(&[]));
    for args in [
        &["add", "Read"][..],
        &["add", "Gym"][..],
        &["check", "Read", "--date", "2026-01-06"][..],
        &["check", "Read", "--date", "2026-01-05"][..],
    ] {
        let r = run_file(&f, args);
        assert_eq!(r.code, 0, "{:?}: {}", args, r.stderr);
    }
    let text = fs::read_to_string(&f).unwrap();
    let want = parse_json(
        r#"{"version":1,"habits":{"Gym":{"checkins":[]},"Read":{"checkins":["2026-01-05","2026-01-06"]}}}"#,
    )
    .normalized();
    assert_eq!(parse_json(&text).normalized(), want);
    assert!(text.find("Gym").unwrap() < text.find("Read").unwrap());
    assert!(text.find("2026-01-05").unwrap() < text.find("2026-01-06").unwrap());
    assert!(text.trim_end().lines().count() > 1, "not pretty-printed");
    assert!(text.ends_with('\n'));
}

/// S08: A hand-written valid file loads into the matching book
#[test]
fn s08_hand_written_file_loads() {
    let d = TempDir::new();
    let f = d.join("habits.json");
    write(
        &f,
        r#"{"version": 1, "habits": {"Read": {"checkins": ["2026-01-05", "2026-01-06"]}}}"#,
    );
    let r = run_file(&f, &["status"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let line = assert_one_stdout_line(&r);
    assert!(line.starts_with("Read"));
    // Uncheck reports the day as removed, so the loaded book holds it.
    let r = run_file(&f, &["uncheck", "Read", "--date", "2026-01-06"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(habits_of(&f), expected(&[("Read", &["2026-01-05"][..])]));
}

/// S09: A file in a different shape is not read as a book
#[test]
fn s09_other_shape_is_malformed() {
    let d = TempDir::new();
    let f = d.join("habits.json");
    write(&f, r#"{"version": 1, "habits": [{"name": "Read", "checkins": []}]}"#);
    let before = bytes(&f);
    let r = run_file(&f, &["status"]);
    assert_eq!(r.code, 2);
    assert_error_line(&r);
    assert!(r.stdout.is_empty());
    assert_eq!(bytes(&f), before);
}

/// S10: Saving a loaded book reproduces the file the store wrote, byte for byte
#[test]
fn s10_resave_is_byte_identical() {
    let books: Vec<Vec<(&str, Vec<&str>)>> = vec![
        vec![],
        vec![("Read", vec![])],
        vec![
            ("Read", vec!["0000-01-01", "2026-01-05", "9999-12-31"]),
            ("Gym", vec!["2024-02-29"]),
            ("Meditate", vec!["1970-01-01", "1999-12-31", "2000-01-01"]),
        ],
        vec![("Zz", vec!["0400-02-29"]), ("Aa", vec!["5000-06-15", "0042-03-01"])],
    ];
    for book in books {
        let d = TempDir::new();
        let a = d.join("a.json");
        let b = d.join("b.json");
        let spec: Vec<(&str, &[&str])> = book.iter().map(|(n, ds)| (*n, &ds[..])).collect();
        write(&a, &book_json(&spec));
        // Have the store write A once, then load and save it again as B.
        assert_eq!(run_file(&a, &["add", "Seed"]).code, 0);
        assert_eq!(run_file(&a, &["remove", "Seed", "--yes"]).code, 0);
        fs::copy(&a, &b).unwrap();
        assert_eq!(run_file(&b, &["add", "Seed"]).code, 0);
        assert_eq!(run_file(&b, &["remove", "Seed", "--yes"]).code, 0);
        assert_eq!(bytes(&a), bytes(&b), "re-saved file differs for {:?}", spec);
        assert_eq!(habits_of(&b), expected(&spec));
    }
}

/// S11: A version other than 1 is reported as an unknown version
#[test]
fn s11_version_two_is_unknown() {
    let d = TempDir::new();
    let f = d.join("habits.json");
    write(&f, r#"{"version": 2, "habits": {}}"#);
    let before = bytes(&f);
    let r = run_file(&f, &["status"]);
    assert_eq!(r.code, 2);
    assert_error_line(&r);
    assert!(r.stderr.contains('2'), "should name version 2: {:?}", r.stderr);
    assert_eq!(bytes(&f), before);
}

/// S12: Each kind of invalid book is refused as Malformed with a message naming the fault
#[test]
fn s12_each_invalid_book_is_malformed() {
    let cases: Vec<(&str, Vec<&str>)> = vec![
        ("this is not json", vec![]),
        ("[1]", vec![]),
        (r#"{"habits": {}}"#, vec!["version"]),
        (r#"{"version": "1", "habits": {}}"#, vec!["version"]),
        (r#"{"version": 1.5, "habits": {}}"#, vec!["version"]),
        (r#"{"version": -1, "habits": {}}"#, vec!["version"]),
        (r#"{"version": 1, "habits": {}, "extra": 0}"#, vec!["extra"]),
        (r#"{"version": 1, "habits": {"Read": {"checkins": [], "note": "x"}}}"#, vec!["note"]),
        (r#"{"version": 1, "habits": {"Read": {"checkins": ["2026-02-30"]}}}"#, vec!["2026-02-30"]),
        (r#"{"version": 1, "habits": {"": {"checkins": []}}}"#, vec![]),
        (
            r#"{"version": 1, "habits": {"Read": {"checkins": []}, "Read": {"checkins": []}}}"#,
            vec!["Read"],
        ),
        (
            r#"{"version": 1, "habits": {"Read": {"checkins": []}, "read": {"checkins": []}}}"#,
            vec!["Read", "read"],
        ),
    ];
    for (content, names) in cases {
        let d = TempDir::new();
        let f = d.join("habits.json");
        write(&f, content);
        let r = run_file(&f, &["status"]);
        assert_eq!(r.code, 2, "content {:?}", content);
        assert_error_line(&r);
        for n in names {
            assert!(r.stderr.contains(n), "message for {:?} should name {:?}: {:?}", content, n, r.stderr);
        }
        assert_eq!(bytes(&f), content.as_bytes());
    }
}

/// S13: A refused file is never rewritten by the command line
#[test]
fn s13_refused_file_not_rewritten() {
    let d = TempDir::new();
    let f = d.join("habits.json");
    write(&f, "{ not json");
    let r = run_file(&f, &["check", "Read", "--date", "2026-01-05"]);
    assert_eq!(r.code, 2);
    assert_error_line(&r);
    assert_eq!(bytes(&f), b"{ not json");
}

/// S14: No invalid book loads
#[test]
fn s14_no_invalid_book_loads() {
    let mutations = [
        r#"{"version": 1, "habits": {"Gym": {"checkins": ["2026-01-05"]}, "Read": {"checkins": []}}, "x": 1}"#,
        r#"{"version": 1, "habits": {"Gym": {"checkins": ["2026-01-05"], "x": null}, "Read": {"checkins": []}}}"#,
        r#"{"version": 1, "habits": {"Gym": {"checkins": ["2026-01-05"]}, "Read": {"checkins": [], "streak": 3}}}"#,
        r#"{"version": 1, "habits": {"Gym": {"checkins": ["2026-1-05"]}, "Read": {"checkins": []}}}"#,
        r#"{"version": 1, "habits": {"Gym": {"checkins": ["2026-02-30"]}, "Read": {"checkins": []}}}"#,
        r#"{"version": 1, "habits": {"Gym": {"checkins": ["yesterday"]}, "Read": {"checkins": []}}}"#,
        r#"{"version": 1, "habits": {"Gym": {"checkins": [" 2026-01-05"]}, "Read": {"checkins": []}}}"#,
        r#"{"version": 1, "habits": {"": {"checkins": ["2026-01-05"]}, "Read": {"checkins": []}}}"#,
        r#"{"version": 1, "habits": {"Gym": {"checkins": ["2026-01-05"]}, "Gym": {"checkins": []}}}"#,
        r#"{"version": 1, "habits": {"Gym": {"checkins": ["2026-01-05"]}, "GYM": {"checkins": []}}}"#,
        r#"{"version": 1, "habits": {"Gym": {"checkins": ["2026-01-05"]}, "Read": {"checkins": []}, "read": {"checkins": []}}}"#,
        r#"{"version": "1", "habits": {"Gym": {"checkins": ["2026-01-05"]}, "Read": {"checkins": []}}}"#,
        r#"{"version": 1.5, "habits": {"Gym": {"checkins": ["2026-01-05"]}, "Read": {"checkins": []}}}"#,
        r#"{"version": true, "habits": {"Gym": {"checkins": ["2026-01-05"]}, "Read": {"checkins": []}}}"#,
        r#"{"version": null, "habits": {"Gym": {"checkins": ["2026-01-05"]}, "Read": {"checkins": []}}}"#,
        r#"{"version": -1, "habits": {"Gym": {"checkins": ["2026-01-05"]}, "Read": {"checkins": []}}}"#,
    ];
    for content in mutations {
        for args in [&["status"][..], &["add", "New"][..]] {
            let d = TempDir::new();
            let f = d.join("habits.json");
            write(&f, content);
            let r = run_file(&f, args);
            assert_eq!(r.code, 2, "{:?} on {:?} should be refused", args, content);
            assert_error_line(&r);
            assert_eq!(bytes(&f), content.as_bytes());
        }
    }
}

/// S15: Any non-negative integer version other than 1 is an unknown version
#[test]
fn s15_any_other_version_is_unknown() {
    let versions: [u64; 9] = [0, 2, 3, 7, 10, 255, 65536, 4294967296, u64::MAX];
    for n in versions {
        let d = TempDir::new();
        let f = d.join("habits.json");
        let content = format!("{{\"version\": {}, \"habits\": {{}}}}", n);
        write(&f, &content);
        let r = run_file(&f, &["status"]);
        assert_eq!(r.code, 2, "version {}", n);
        assert_error_line(&r);
        assert!(r.stderr.contains(&n.to_string()), "should name version {}: {:?}", n, r.stderr);
        assert_eq!(bytes(&f), content.as_bytes());
    }
}

/// S16: At the default path a missing file is an empty book and save creates the directories
#[test]
fn s16_default_path_missing_is_empty_and_save_creates_dirs() {
    let d = TempDir::new();
    let xdg = d.join("a");
    let home = d.join("h");
    let xdg_s = xdg.to_str().unwrap();
    let home_s = home.to_str().unwrap();
    let env = [("XDG_DATA_HOME", Some(xdg_s)), ("HOME", Some(home_s))];
    let target = xdg.join("habit-streak").join("habits.json");

    let r = run_with(&["status"], &env);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(
        one_line(&r.stdout),
        "No habits yet. Add one with: habit-streak add <name>"
    );

    let r = run_with(&["add", "Read"], &env);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(target.exists(), "{} not created", target.display());
    assert_eq!(habits_of(&target), expected(&[("Read", NONE)]));
}

/// S17: At a named path a missing file is NotFound, not a new history
#[test]
fn s17_named_missing_file_is_not_found() {
    let d = TempDir::new();
    let f = d.join("habits.json");
    let r = run_file(&f, &["status"]);
    assert_eq!(r.code, 2);
    assert_error_line(&r);
    assert!(!f.exists());
}

/// S18: At a named path a missing parent directory is not created
#[test]
fn s18_named_missing_parent_not_created() {
    let d = TempDir::new();
    let typo = d.join("typo");
    let f = typo.join("habits.json");
    let r = run_file(&f, &["add", "Read"]);
    assert_eq!(r.code, 2);
    assert_error_line(&r);
    assert!(!typo.exists());
}

/// S19: A mistyped --file fails with exit 2 instead of starting a new history
#[test]
fn s19_mistyped_file_exits_2() {
    let d = TempDir::new();
    let f = d.join("habitz.json");
    let r = run_file(&f, &["check", "Read"]);
    assert_eq!(r.code, 2);
    assert_error_line(&r);
    assert!(!f.exists());
}

/// S20: A save replaces the file whole and leaves no temporary file
#[test]
fn s20_save_replaces_whole_file() {
    let d = TempDir::new();
    let f = d.join("habits.json");
    write(&f, &book_json(&[("Read", &["2026-01-05"][..])]));
    let r = run_file(&f, &["add", "Gym"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(
        habits_of(&f),
        expected(&[("Gym", NONE), ("Read", &["2026-01-05"][..])])
    );
    assert_eq!(entries(d.path()), vec!["habits.json".to_string()]);
}

/// S21: A failed save does not touch the original file and cleans up after itself
#[test]
fn s21_failed_save_leaves_original() {
    let d = TempDir::new();
    let dir = d.join("d");
    fs::create_dir(&dir).unwrap();
    let f = dir.join("habits.json");
    write(&f, &book_json(&[("Read", &["2026-01-05"][..])]));
    let before = bytes(&f);
    let _ro = ReadOnly::new(&dir);
    if !dir_refuses_new_files(&dir) {
        eprintln!("S21 skipped: directory permissions are not enforced (running as root?)");
        return;
    }
    let r = run_file(&f, &["add", "Gym"]);
    assert_eq!(r.code, 2);
    assert_error_line(&r);
    assert_eq!(bytes(&f), before);
    assert_eq!(entries(&dir), vec!["habits.json".to_string()]);
}

/// S22: After any save the target holds either the old content or the new, never a mix
#[test]
fn s22_save_is_all_or_nothing() {
    let olds: Vec<String> = vec![
        book_json(&[]),
        book_json(&[("Read", &["2026-01-05"][..])]),
        book_json(&[("Gym", &["0000-01-01", "9999-12-31"][..]), ("Read", NONE)]),
    ];
    for old in &olds {
        for fail in [false, true] {
            let d = TempDir::new();
            let dir = d.join("d");
            fs::create_dir(&dir).unwrap();
            let f = dir.join("habits.json");
            write(&f, old);
            let before_habits = habits_of(&f);
            let before = bytes(&f);
            let _ro = if fail { Some(ReadOnly::new(&dir)) } else { None };
            if fail && !dir_refuses_new_files(&dir) {
                continue;
            }
            let r = run_file(&f, &["add", "Newhabit"]);
            if fail {
                assert_eq!(r.code, 2);
                assert_error_line(&r);
                assert_eq!(bytes(&f), before);
            } else {
                assert_eq!(r.code, 0, "{}", r.stderr);
                let mut want = before_habits.clone();
                want.push(("Newhabit".to_string(), vec![]));
                want.sort();
                assert_eq!(habits_of(&f), want);
            }
            assert_eq!(entries(&dir), vec!["habits.json".to_string()]);
        }
    }
}

/// S23: The flag wins, then HABIT_STREAK_FILE, then XDG_DATA_HOME, then HOME
#[test]
fn s23_data_path_precedence() {
    let d = TempDir::new();
    let flag = d.join("f.json");
    let envf = d.join("e.json");
    let x = d.join("x");
    let h = d.join("h");
    write(&flag, &book_json(&[]));
    write(&envf, &book_json(&[]));
    let x_target = x.join("habit-streak").join("habits.json");
    let h_target = h.join(".local").join("share").join("habit-streak").join("habits.json");
    let (fs_, es, xs, hs) = (
        flag.to_str().unwrap(),
        envf.to_str().unwrap(),
        x.to_str().unwrap(),
        h.to_str().unwrap(),
    );

    let all = [
        ("HABIT_STREAK_FILE", Some(es)),
        ("XDG_DATA_HOME", Some(xs)),
        ("HOME", Some(hs)),
    ];
    assert_eq!(run_with(&["--file", fs_, "add", "One"], &all).code, 0);
    assert_eq!(run_with(&["add", "Two"], &all).code, 0);
    let no_env_file = [("XDG_DATA_HOME", Some(xs)), ("HOME", Some(hs))];
    assert_eq!(run_with(&["add", "Three"], &no_env_file).code, 0);
    let home_only = [("HOME", Some(hs))];
    assert_eq!(run_with(&["add", "Four"], &home_only).code, 0);

    assert_eq!(habits_of(&flag), expected(&[("One", NONE)]));
    assert_eq!(habits_of(&envf), expected(&[("Two", NONE)]));
    assert_eq!(habits_of(&x_target), expected(&[("Three", NONE)]));
    assert_eq!(habits_of(&h_target), expected(&[("Four", NONE)]));

    // The flag and HABIT_STREAK_FILE are not default paths: a missing file is an error.
    let missing = d.join("missing.json");
    let ms = missing.to_str().unwrap();
    let r = run_with(&["--file", ms, "add", "Five"], &all);
    assert_eq!(r.code, 2);
    let r = run_with(
        &["add", "Five"],
        &[("HABIT_STREAK_FILE", Some(ms)), ("XDG_DATA_HOME", Some(xs)), ("HOME", Some(hs))],
    );
    assert_eq!(r.code, 2);
    assert_error_line(&r);
    assert!(!missing.exists());
}

/// S24: Empty variables are skipped and no source at all is an error
#[test]
fn s24_empty_or_unset_everything_is_no_data_path() {
    let empty = [
        ("HABIT_STREAK_FILE", Some("")),
        ("XDG_DATA_HOME", Some("")),
        ("HOME", Some("")),
    ];
    let r = run_with(&["status"], &empty);
    assert_eq!(r.code, 2);
    assert_error_line(&r);
    assert!(r.stdout.is_empty());

    let r = run_with(&["status"], &[]);
    assert_eq!(r.code, 2);
    assert_error_line(&r);
    assert!(r.stdout.is_empty());
}

/// S25: An empty HABIT_STREAK_FILE does not override the default path
#[test]
fn s25_empty_env_file_falls_back_to_home() {
    let d = TempDir::new();
    let h = d.join("h");
    let target = h.join(".local").join("share").join("habit-streak").join("habits.json");
    let r = run_with(
        &["add", "Read"],
        &[("HABIT_STREAK_FILE", Some("")), ("HOME", Some(h.to_str().unwrap()))],
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(habits_of(&target), expected(&[("Read", NONE)]));
}

/// S26: The first non-empty source in the fixed order is chosen, and only the last two are default
#[test]
fn s26_first_non_empty_source_chosen() {
    #[derive(Clone, Copy, Debug)]
    enum V {
        Unset,
        Empty,
        Set,
    }
    let states = [V::Unset, V::Empty, V::Set];
    for flag_present in [false, true] {
        for e in states {
            for x in states {
                for h in states {
                    for pre_create in [true, false] {
                        let d = TempDir::new();
                        let flag = d.join("f.json");
                        let envf = d.join("e.json");
                        let xd = d.join("x");
                        let hd = d.join("h");
                        if pre_create {
                            write(&flag, &book_json(&[]));
                            write(&envf, &book_json(&[]));
                        }
                        let x_target = xd.join("habit-streak").join("habits.json");
                        let h_target =
                            hd.join(".local").join("share").join("habit-streak").join("habits.json");
                        let pick = |v: V, s: &str| -> Option<String> {
                            match v {
                                V::Unset => None,
                                V::Empty => Some(String::new()),
                                V::Set => Some(s.to_string()),
                            }
                        };
                        let ev = pick(e, envf.to_str().unwrap());
                        let xv = pick(x, xd.to_str().unwrap());
                        let hv = pick(h, hd.to_str().unwrap());
                        let env = [
                            ("HABIT_STREAK_FILE", ev.as_deref()),
                            ("XDG_DATA_HOME", xv.as_deref()),
                            ("HOME", hv.as_deref()),
                        ];
                        let mut args: Vec<&str> = Vec::new();
                        let fstr = flag.to_str().unwrap().to_string();
                        if flag_present {
                            args.push("--file");
                            args.push(&fstr);
                        }
                        args.extend_from_slice(&["add", "Read"]);
                        let r = run_with(&args, &env);

                        let ctx = format!(
                            "flag={} env={:?} xdg={:?} home={:?} pre_create={}",
                            flag_present, e, x, h, pre_create
                        );
                        // (target, is_default)
                        let chosen: Option<(&Path, bool)> = if flag_present {
                            Some((&flag, false))
                        } else if matches!(e, V::Set) {
                            Some((&envf, false))
                        } else if matches!(x, V::Set) {
                            Some((&x_target, true))
                        } else if matches!(h, V::Set) {
                            Some((&h_target, true))
                        } else {
                            None
                        };
                        match chosen {
                            None => {
                                assert_eq!(r.code, 2, "{}", ctx);
                                assert_error_line(&r);
                            }
                            Some((_, false)) if !pre_create => {
                                assert_eq!(r.code, 2, "non-default missing must fail: {}", ctx);
                                assert_error_line(&r);
                            }
                            Some((target, _)) => {
                                assert_eq!(r.code, 0, "{}: {}", ctx, r.stderr);
                                assert!(file_has_habit(target, "Read"), "{}", ctx);
                            }
                        }
                        let winner = match chosen {
                            Some((t, _)) if r.code == 0 => Some(t.to_path_buf()),
                            _ => None,
                        };
                        for other in [&flag, &envf, &x_target, &h_target] {
                            if Some(other.clone()) != winner {
                                assert!(
                                    !file_has_habit(other, "Read"),
                                    "{} written unexpectedly: {}",
                                    other.display(),
                                    ctx
                                );
                            }
                        }
                        if !pre_create {
                            assert!(!flag.exists() && !envf.exists(), "{}", ctx);
                        }
                    }
                }
            }
        }
    }
}

/// S27: status prints aligned lines in key order with streaks and a not-checked-today mark
#[test]
fn s27_status_lines() {
    let today = date_cmd(None);
    let d1 = date_cmd(Some("1 day ago"));
    let d2 = date_cmd(Some("2 days ago"));
    let d = TempDir::new();
    let f = d.join("habits.json");
    write(
        &f,
        &book_json(&[("Read", &[today.as_str()][..]), ("Gym", &[d2.as_str(), d1.as_str()][..])]),
    );
    let r = run_file(&f, &["status"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let lines: Vec<&str> = r.stdout.lines().collect();
    assert_eq!(lines.len(), 2, "{:?}", r.stdout);
    assert!(lines[0].starts_with("Gym"), "{:?}", lines[0]);
    assert!(lines[1].starts_with("Read"), "{:?}", lines[1]);
    let col = |line: &str, n: usize| -> usize {
        n + line[n..].find(|c: char| c != ' ').expect("text after name")
    };
    assert_eq!(col(lines[0], 3), col(lines[1], 4), "columns differ: {:?}", lines);
    assert!(lines[0].as_bytes()[3] == b' ', "Gym should be padded: {:?}", lines[0]);
    assert!(lines[0].contains("current 2"));
    assert!(lines[0].contains("longest 2"));
    assert!(lines[0].ends_with(" (not checked today)"));
    assert!(lines[1].contains("current 1"));
    assert!(lines[1].contains("longest 1"));
    assert!(!lines[1].contains("not checked today"));
}

/// S28: status on an empty book tells the person how to start
#[test]
fn s28_status_empty_book() {
    let d = TempDir::new();
    let f = d.join("habits.json");
    write(&f, r#"{"version": 1, "habits": {}}"#);
    let r = run_file(&f, &["status"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(
        one_line(&r.stdout),
        "No habits yet. Add one with: habit-streak add <name>"
    );
}

/// S29: Commands report what happened, naming the habit and the day
#[test]
fn s29_commands_report() {
    let d = TempDir::new();
    let f = d.join("habits.json");
    write(&f, &book_json(&[]));
    let steps: [(&[&str], bool); 4] = [
        (&["add", "Read"], false),
        (&["check", "Read", "--date", "2026-01-05"], true),
        (&["uncheck", "Read", "--date", "2026-01-05"], true),
        (&["remove", "Read", "--yes"], false),
    ];
    for (args, names_day) in steps {
        let r = run_file(&f, args);
        assert_eq!(r.code, 0, "{:?}: {}", args, r.stderr);
        let line = assert_one_stdout_line(&r);
        assert!(line.contains("Read"), "{:?}: {:?}", args, line);
        if names_day {
            assert!(line.contains("2026-01-05"), "{:?}: {:?}", args, line);
        }
    }
    assert!(habits_of(&f).is_empty());
}

/// S30: A command whose end state already held says so and exits 0
#[test]
fn s30_already_held_exits_0() {
    let d = TempDir::new();
    let f = d.join("habits.json");
    write(&f, &book_json(&[("Read", &["2026-01-05"][..])]));
    let before = habits_of(&f);
    for args in [&["check", "Read", "--date", "2026-01-05"][..], &["add", "Read"][..]] {
        let r = run_file(&f, args);
        assert_eq!(r.code, 0, "{:?}: {}", args, r.stderr);
        let line = assert_one_stdout_line(&r);
        assert!(line.contains("Read"));
        assert_eq!(habits_of(&f), before);
    }
}

/// S31: An unknown habit is an error that lists the known habits
#[test]
fn s31_unknown_habit_lists_known() {
    let d = TempDir::new();
    let f = d.join("habits.json");
    write(&f, &book_json(&[("Gym", NONE), ("Read", NONE)]));
    let r = run_file(&f, &["check", "Run"]);
    assert_ne!(r.code, 0);
    assert!(r.stdout.is_empty());
    assert_error_line(&r);
    for n in ["Run", "Gym", "Read"] {
        assert!(r.stderr.contains(n), "should contain {:?}: {:?}", n, r.stderr);
    }
}

/// S32: uncheck without --date is refused
#[test]
fn s32_uncheck_needs_date() {
    let today = date_cmd(None);
    let d = TempDir::new();
    let f = d.join("habits.json");
    write(&f, &book_json(&[("Read", &[today.as_str()][..])]));
    let before = bytes(&f);
    let r = run_file(&f, &["uncheck", "Read"]);
    assert_eq!(r.code, 1);
    assert_error_line(&r);
    assert_eq!(bytes(&f), before);
}

/// S33: Help exits 0
#[test]
fn s33_help_exits_0() {
    let r = run_with(&["--help"], &[]);
    assert_eq!(r.code, 0);
    assert!(!r.stdout.trim().is_empty(), "help should go to stdout");
}

/// S34: Usage errors, invalid names, unknown habits and future dates exit 1 without changing the file
#[test]
fn s34_user_errors_exit_1() {
    let tomorrow = date_cmd(Some("tomorrow"));
    let d = TempDir::new();
    let f = d.join("habits.json");
    write(&f, &book_json(&[("Read", NONE)]));
    let before = bytes(&f);
    let cases: Vec<Vec<&str>> = vec![
        vec!["frobnicate"],
        vec!["check"],
        vec!["remove", "Read"],
        vec!["check", "Read", "--date", "2026-1-05"],
        vec!["add", ""],
        vec!["check", "Run"],
        vec!["check", "Read", "--date", &tomorrow],
    ];
    for args in cases {
        let r = run_file(&f, &args);
        assert_eq!(r.code, 1, "{:?}: {}", args, r.stderr);
        assert_error_line(&r);
        assert_eq!(bytes(&f), before, "{:?} changed the file", args);
    }
    assert_eq!(habits_of(&f), expected(&[("Read", NONE)]));
}

/// S35: No data path exits 2
#[test]
fn s35_no_data_path_exits_2() {
    let r = run_with(&["status"], &[]);
    assert_eq!(r.code, 2);
    assert_error_line(&r);
}

/// S36: Any store error exits 2 and leaves the file byte-identical
#[test]
fn s36_store_errors_exit_2() {
    let all: Vec<Vec<&str>> = vec![
        vec!["status"],
        vec!["check", "Read"],
        vec!["check", "Read", "--date", "2026-01-05"],
        vec!["uncheck", "Read", "--date", "2026-01-05"],
        vec!["remove", "Read", "--yes"],
    ];
    let check_case = |content: Option<&str>, args: &[&str]| {
        let d = TempDir::new();
        let f = d.join("habits.json");
        if let Some(c) = content {
            write(&f, c);
        }
        let r = run_file(&f, args);
        assert_eq!(r.code, 2, "{:?} on {:?}: {}", args, content, r.stderr);
        assert_error_line(&r);
        match content {
            Some(c) => assert_eq!(bytes(&f), c.as_bytes()),
            None => assert!(!f.exists()),
        }
    };
    for content in ["{ not json", r#"{"version": 2, "habits": {}}"#] {
        for args in &all {
            check_case(Some(content), args);
        }
        check_case(Some(content), &["add", "Gym"]);
    }
    for args in &all {
        check_case(None, args);
    }

    let writes: Vec<Vec<&str>> = vec![
        vec!["check", "Read"],
        vec!["check", "Read", "--date", "2026-01-05"],
        vec!["uncheck", "Read", "--date", "2026-01-04"],
        vec!["remove", "Read", "--yes"],
    ];
    for args in writes {
        let d = TempDir::new();
        let dir = d.join("d");
        fs::create_dir(&dir).unwrap();
        let f = dir.join("habits.json");
        write(&f, &book_json(&[("Read", &["2026-01-04"][..])]));
        let before = bytes(&f);
        let _ro = ReadOnly::new(&dir);
        if !dir_refuses_new_files(&dir) {
            continue;
        }
        let r = run_file(&f, &args);
        assert_eq!(r.code, 2, "{:?} in read-only dir: {}", args, r.stderr);
        assert_error_line(&r);
        assert_eq!(bytes(&f), before);
        assert_eq!(entries(&dir), vec!["habits.json".to_string()]);
    }
}

/// S37: Every non-zero exit writes exactly one error line and nothing to stdout
#[test]
fn s37_non_zero_exit_shape() {
    let tomorrow = date_cmd(Some("tomorrow"));
    let d = TempDir::new();
    let good = d.join("good.json");
    write(&good, &book_json(&[("Read", &["2026-01-05"][..])]));
    let bad = d.join("bad.json");
    write(&bad, "{ not json");
    let unknown = d.join("v9.json");
    write(&unknown, r#"{"version": 9, "habits": {}}"#);
    let missing = d.join("missing.json");
    let (g, b, u, m) = (
        good.to_str().unwrap(),
        bad.to_str().unwrap(),
        unknown.to_str().unwrap(),
        missing.to_str().unwrap(),
    );
    let cases: Vec<Vec<&str>> = vec![
        vec!["--file", g, "frobnicate"],
        vec!["--file", g, "--bogus", "status"],
        vec!["--file", g, "check"],
        vec!["--file", g, "remove", "Read"],
        vec!["--file", g, "uncheck", "Read"],
        vec!["--file", g, "check", "Read", "--date", "2026-02-30"],
        vec!["--file", g, "check", "Read", "--date", &tomorrow],
        vec!["--file", g, "check", "Run"],
        vec!["--file", g, "uncheck", "Run", "--date", "2026-01-05"],
        vec!["--file", g, "add", ""],
        vec!["--file", b, "status"],
        vec!["--file", u, "add", "Gym"],
        vec!["--file", m, "check", "Read"],
        vec!["status"],
        vec![],
    ];
    for args in cases {
        let r = run_with(&args, &[]);
        if r.code == 0 {
            continue;
        }
        assert!(r.code == 1 || r.code == 2, "{:?} exited {}", args, r.code);
        assert_error_line(&r);
        assert!(r.stdout.is_empty(), "{:?} wrote to stdout: {:?}", args, r.stdout);
    }
}
