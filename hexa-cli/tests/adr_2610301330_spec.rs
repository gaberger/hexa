//! Integration tests for ADR-2610301330: `hexa loop line`, one line of loop
//! state for a status bar.

use std::path::Path;
use std::process::Command;

const SESSION: &str = "s1";

fn hexa(dir: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(args)
        .current_dir(dir)
        .env("HEXA_SESSION_ID", SESSION)
        .env("HEXA_SESSION_PID", std::process::id().to_string())
        .env("NO_COLOR", "1")
        .output()
        .expect("run hexa");
    (out.status.success(), String::from_utf8_lossy(&out.stdout).to_string())
}

fn line(dir: &Path) -> String {
    let (ok, out) = hexa(dir, &["loop", "line"]);
    assert!(ok, "hexa loop line exits 0:\n{out}");
    out
}

/// A project with `.hexa/` and docs/adrs/ADR-0001-first.md.
fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join(".hexa")).unwrap();
    std::fs::create_dir_all(d.path().join("docs/adrs")).unwrap();
    std::fs::write(d.path().join("docs/adrs/ADR-0001-first.md"), "# ADR-0001: first\n").unwrap();
    d
}

fn record(dir: &Path, args: &[&str]) {
    let (ok, out) = hexa(dir, args);
    assert!(ok, "hexa {args:?}:\n{out}");
}

/// Write this session's entry directly.
fn write_entry(dir: &Path, entry: serde_json::Value) {
    let file = serde_json::json!({ "sessions": { SESSION: entry } });
    std::fs::write(dir.join(".hexa/loop.json"), serde_json::to_string_pretty(&file).unwrap()).unwrap();
}

fn ago(mins: i64) -> String {
    (chrono::Utc::now() - chrono::Duration::minutes(mins)).to_rfc3339()
}

const LONG_GATE: &str = r#"cargo test --test domain compose_status 2>&1 | grep -E "test result: ok. [1-9]""#;

// ── §1 ──────────────────────────────────────────────────

#[test]
fn s01_a_recorded_loop_is_one_line() {
    let d = project();
    record(d.path(), &["loop", "adr", "ADR-0001"]);
    record(d.path(), &["loop", "gate", "cargo test"]);
    record(d.path(), &["loop", "task", "add", "one"]);
    record(d.path(), &["loop", "task", "add", "two"]);
    let out = line(d.path());
    assert!(out.ends_with('\n') && !out.ends_with("\n\n"), "{out:?}");
    assert_eq!(out.matches('\n').count(), 1, "{out:?}");
}

#[test]
fn s02_the_line_writes_nothing_and_never_fails() {
    let bare = tempfile::tempdir().unwrap();
    let out = line(bare.path());
    assert_eq!(out.matches('\n').count(), 1, "{out:?}");
    assert!(!bare.path().join(".hexa").exists(), "no .hexa/ is created");

    let d = project();
    write_entry(d.path(), serde_json::json!({ "adr": "ADR-0001", "stage": "build", "gate": "cargo test" }));
    let before = std::fs::read(d.path().join(".hexa/loop.json")).unwrap();
    let out = line(d.path());
    assert_eq!(out.matches('\n').count(), 1, "{out:?}");
    assert_eq!(std::fs::read(d.path().join(".hexa/loop.json")).unwrap(), before, "loop.json is untouched");
}

// ── §2 ──────────────────────────────────────────────────

#[test]
fn s03_nothing_recorded_is_idle() {
    let d = project();
    assert_eq!(line(d.path()), "▸ loop · idle\n");
}

#[test]
fn s04_idle_carries_no_advice() {
    let d = project();
    let out = line(d.path());
    for word in ["hexa loop", "Record", "nothing recorded", "`"] {
        assert!(!out.contains(word), "{word:?} in {out:?}");
    }
    let (_, full) = hexa(d.path(), &["loop"]);
    assert!(full.contains("nothing recorded") && full.contains("hexa loop gate"), "{full}");
}

// ── §3 ──────────────────────────────────────────────────

#[test]
fn s05_stage_is_a_position_then_adr_and_count() {
    let d = project();
    record(d.path(), &["loop", "adr", "ADR-0001"]);
    record(d.path(), &["loop", "gate", "cargo test"]);
    for t in ["a", "b", "c", "d", "e"] {
        record(d.path(), &["loop", "task", "add", t]);
    }
    record(d.path(), &["loop", "task", "done", "1"]);
    record(d.path(), &["loop", "task", "done", "2"]);
    assert_eq!(line(d.path()), "▸ loop · decide › GATE › build › harden · ADR-0001 · 2/5\n");
    record(d.path(), &["loop", "stage", "build"]);
    assert_eq!(line(d.path()), "▸ loop · decide › gate › BUILD › harden · ADR-0001 · 2/5\n");
}

#[test]
fn s06_done_and_a_harness_run() {
    let d = project();
    write_entry(d.path(), serde_json::json!({ "stage": "done", "adr": "ADR-0001" }));
    assert_eq!(line(d.path()), "▸ loop · done · ADR-0001\n");

    let running = |updated: String| {
        serde_json::json!({
            "stage": "harden",
            "running": { "verb": "harden", "phase": "verify", "message": "3 claims", "started": ago(2), "updated": updated }
        })
    };
    write_entry(d.path(), running(ago(0)));
    let out = line(d.path());
    assert!(out.starts_with("▸ loop · decide › gate › build › HARDEN · running harden/verify 2m"), "{out:?}");

    write_entry(d.path(), running(ago(10)));
    assert_eq!(line(d.path()), "▸ loop · decide › gate › build › HARDEN · stalled harden/verify\n");
}

#[test]
fn s07_what_is_not_recorded_is_not_shown() {
    let d = project();
    record(d.path(), &["loop", "gate", "cargo test"]);
    let out = line(d.path());
    assert_eq!(out, "▸ loop · decide › GATE › build › harden\n");
    for word in ["ADR", "none", "/", "running"] {
        assert!(!out.contains(word), "{word:?} in {out:?}");
    }
}

// ── §4 ──────────────────────────────────────────────────

fn s08_state(dir: &Path) {
    write_entry(
        dir,
        serde_json::json!({
            "stage": "build",
            "gate": LONG_GATE,
            "evidence": "echo EVIDENCE-MARK",
            "tasks": [{ "title": "TITLE-MARK wire the parser", "status": "doing" }],
            "running": { "verb": "harden", "phase": "hunt", "message": "MESSAGE-MARK", "started": ago(1), "updated": ago(0) }
        }),
    );
}

#[test]
fn s08_recorded_text_does_not_appear() {
    let d = project();
    s08_state(d.path());
    let out = line(d.path());
    for word in ["cargo", "grep", "EVIDENCE-MARK", "TITLE-MARK", "MESSAGE-MARK"] {
        assert!(!out.contains(word), "{word:?} in {out:?}");
    }
    assert!(out.trim_end().chars().count() <= 80, "{out:?}");
}

#[test]
fn s09_the_full_report_keeps_the_detail() {
    let d = project();
    s08_state(d.path());
    let (_, full) = hexa(d.path(), &["loop"]);
    assert!(full.contains("cargo test --test domain compose_status"), "{full}");
    assert!(full.contains("TITLE-MARK wire the parser"), "{full}");
    // The detail moved to the full report; it was not lost on the way.
    let short = line(d.path());
    assert!(!short.contains("TITLE-MARK") && !short.contains("cargo"), "{short:?}");
}

/// xorshift64*: a fixed seed, so a failure reproduces.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn word(&mut self, len: usize) -> String {
        const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        (0..len).map(|_| CHARS[self.below(CHARS.len() as u64) as usize] as char).collect()
    }
    /// 12 to 400 characters of shell-ish text with `marker` somewhere in it.
    fn text(&mut self, marker: &str) -> String {
        const PIECES: &[&str] = &[" ", " | ", " && ", " 2>&1 ", "\"", "'", "·", " › ", "/", "-", "\n"];
        let target = 12 + self.below(389) as usize;
        let mut s = String::new();
        while s.chars().count() < target {
            if self.below(3) == 0 {
                s.push_str(PIECES[self.below(PIECES.len() as u64) as usize]);
            } else {
                let n = 1 + self.below(8) as usize;
                s.push_str(&self.word(n));
            }
        }
        let at = s.char_indices().nth(self.below(s.chars().count() as u64) as usize).map_or(0, |(i, _)| i);
        s.insert_str(at, marker);
        s
    }
}

#[test]
fn s10_any_recorded_text_stays_out_and_short() {
    let d = project();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let stages = ["decide", "gate", "build", "harden", "done"];
    let statuses = ["todo", "doing", "done"];
    for case in 0..48 {
        let mut markers = Vec::new();
        let mut mark = |rng: &mut Rng| {
            let m = rng.word(12);
            markers.push(m.clone());
            m
        };
        let gate_m = mark(&mut rng);
        let ev_m = mark(&mut rng);
        let msg_m = mark(&mut rng);
        let mut entry = serde_json::json!({
            "stage": stages[rng.below(5) as usize],
            "gate": rng.text(&gate_m),
            "evidence": rng.text(&ev_m),
        });
        if rng.below(2) == 0 {
            entry["adr"] = format!("ADR-{:010}", rng.below(10_000_000_000)).into();
        }
        let tasks: Vec<serde_json::Value> = (0..rng.below(31))
            .map(|_| {
                let m = mark(&mut rng);
                serde_json::json!({ "title": rng.text(&m), "status": statuses[rng.below(3) as usize] })
            })
            .collect();
        entry["tasks"] = tasks.into();
        if rng.below(2) == 0 {
            let started = rng.below(600) as i64;
            entry["running"] = serde_json::json!({
                "verb": "harden", "phase": "verify", "message": rng.text(&msg_m),
                "started": ago(started), "updated": ago(rng.below(started as u64 + 1) as i64),
            });
        }
        write_entry(d.path(), entry.clone());
        let out = line(d.path());
        for m in &markers {
            assert!(!out.contains(m.as_str()), "case {case}: marker {m} in {out:?}\nentry {entry}");
        }
        assert!(out.trim_end().chars().count() <= 80, "case {case}: {} chars: {out:?}", out.trim_end().chars().count());
        assert_eq!(out.matches('\n').count(), 1, "case {case}: {out:?}");
    }
}

// ── §5 ──────────────────────────────────────────────────

fn template_command() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/templates/hexa-claude-settings.json");
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    v["statusLine"]["command"].as_str().expect("statusLine.command").to_string()
}

#[test]
fn s11_the_template_runs_loop_line() {
    assert_eq!(template_command(), "hexa loop line 2>/dev/null");
}

#[test]
fn s12_the_template_does_not_cut_the_report() {
    let cmd = template_command();
    assert!(!cmd.contains("head"), "{cmd}");
    let bare = cmd.trim_end_matches("2>/dev/null").trim_end();
    assert!(!bare.ends_with("hexa loop") && !bare.ends_with("hexa loop show"), "{cmd}");
}
