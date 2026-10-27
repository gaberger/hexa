//! Integration tests for ADR-2610271110: the gate ledger and the
//! `adr.unbuilt` declaration in `.hexa/project.json`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use hexa_core::domain::gate_ledger::{declaration_errors, judge, Outcome, Unbuilt, Verdict};

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn unbuilt(pairs: &[(&str, &str)]) -> Unbuilt {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn gated(pairs: &[(&str, bool)]) -> BTreeMap<String, bool> {
    pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

fn all_outcomes() -> Vec<Outcome> {
    vec![
        Outcome::Passed,
        Outcome::Failed,
        Outcome::Unrunnable,
        Outcome::CannotRunHere,
        Outcome::NotProvenHere,
        Outcome::NoGate,
    ]
}

fn non_passing_outcomes() -> Vec<Outcome> {
    vec![
        Outcome::Failed,
        Outcome::Unrunnable,
        Outcome::CannotRunHere,
        Outcome::NotProvenHere,
    ]
}

/// The two decisions hexa itself may declare unbuilt.
const ALLOWED_UNBUILT: [&str; 2] = ["ADR-2610161730", "ADR-2610221834"];

/// The ledger rule from §6: at most two entries, each one of the named ADRs.
fn ledger_check(keys: &[String]) -> Result<(), String> {
    let unexpected: Vec<&String> = keys
        .iter()
        .filter(|k| !ALLOWED_UNBUILT.contains(&k.as_str()))
        .collect();
    if !unexpected.is_empty() {
        return Err(format!(
            "adr.unbuilt may only hold {:?}; unexpected entries: {:?}",
            ALLOWED_UNBUILT, unexpected
        ));
    }
    if keys.len() > ALLOWED_UNBUILT.len() {
        return Err(format!(
            "adr.unbuilt holds {} entries, at most {} allowed: {:?}",
            keys.len(),
            ALLOWED_UNBUILT.len(),
            keys
        ));
    }
    Ok(())
}

fn find_project_json() -> PathBuf {
    let start = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut dir = Some(start);
    while let Some(d) = dir {
        let candidate = d.join(".hexa").join("project.json");
        if candidate.is_file() {
            return candidate;
        }
        dir = d.parent();
    }
    panic!(
        "no .hexa/project.json found at or above {}",
        start.display()
    );
}

/// Keys of `adr.unbuilt` in hexa's own `.hexa/project.json`.
fn read_hexa_unbuilt_keys() -> Vec<String> {
    let path = find_project_json();
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let value = Json::parse(&text)
        .unwrap_or_else(|e| panic!("{} is not valid JSON: {e}", path.display()));
    let adr = value
        .get("adr")
        .unwrap_or_else(|| panic!("{} has no `adr` section", path.display()));
    let list = adr
        .get("unbuilt")
        .unwrap_or_else(|| panic!("{} has no `adr.unbuilt`", path.display()));
    match list {
        Json::Obj(entries) => {
            for (k, v) in entries {
                assert!(
                    matches!(v, Json::Str(_)),
                    "adr.unbuilt[{k}] must be a string reason"
                );
            }
            entries.iter().map(|(k, _)| k.clone()).collect()
        }
        other => panic!("adr.unbuilt must be an object, got {other:?}"),
    }
}

// --- a minimal JSON reader, so the ledger test needs no extra dependency ---

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
    fn parse(text: &str) -> Result<Json, String> {
        let chars: Vec<char> = text.chars().collect();
        let mut p = Parser { s: chars, i: 0 };
        let v = p.value()?;
        p.ws();
        if p.i != p.s.len() {
            return Err(format!("trailing content at {}", p.i));
        }
        Ok(v)
    }

    fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
}

struct Parser {
    s: Vec<char>,
    i: usize,
}

impl Parser {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_whitespace() {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }

    fn expect(&mut self, c: char) -> Result<(), String> {
        self.ws();
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("expected '{c}' at {}", self.i))
        }
    }

    fn literal(&mut self, word: &str, v: Json) -> Result<Json, String> {
        for c in word.chars() {
            if self.peek() != Some(c) {
                return Err(format!("bad literal at {}", self.i));
            }
            self.i += 1;
        }
        Ok(v)
    }

    fn value(&mut self) -> Result<Json, String> {
        self.ws();
        match self.peek() {
            Some('{') => self.object(),
            Some('[') => self.array(),
            Some('"') => Ok(Json::Str(self.string()?)),
            Some('t') => self.literal("true", Json::Bool(true)),
            Some('f') => self.literal("false", Json::Bool(false)),
            Some('n') => self.literal("null", Json::Null),
            Some(c) if c == '-' || c.is_ascii_digit() => {
                let start = self.i;
                while let Some(c) = self.peek() {
                    if c.is_ascii_digit() || "+-.eE".contains(c) {
                        self.i += 1;
                    } else {
                        break;
                    }
                }
                Ok(Json::Num(self.s[start..self.i].iter().collect()))
            }
            other => Err(format!("unexpected {other:?} at {}", self.i)),
        }
    }

    fn object(&mut self) -> Result<Json, String> {
        self.expect('{')?;
        let mut entries = Vec::new();
        self.ws();
        if self.peek() == Some('}') {
            self.i += 1;
            return Ok(Json::Obj(entries));
        }
        loop {
            self.ws();
            let k = self.string()?;
            self.expect(':')?;
            let v = self.value()?;
            entries.push((k, v));
            self.ws();
            match self.peek() {
                Some(',') => self.i += 1,
                Some('}') => {
                    self.i += 1;
                    return Ok(Json::Obj(entries));
                }
                other => return Err(format!("unexpected {other:?} in object at {}", self.i)),
            }
        }
    }

    fn array(&mut self) -> Result<Json, String> {
        self.expect('[')?;
        let mut items = Vec::new();
        self.ws();
        if self.peek() == Some(']') {
            self.i += 1;
            return Ok(Json::Arr(items));
        }
        loop {
            items.push(self.value()?);
            self.ws();
            match self.peek() {
                Some(',') => self.i += 1,
                Some(']') => {
                    self.i += 1;
                    return Ok(Json::Arr(items));
                }
                other => return Err(format!("unexpected {other:?} in array at {}", self.i)),
            }
        }
    }

    fn string(&mut self) -> Result<String, String> {
        if self.peek() != Some('"') {
            return Err(format!("expected string at {}", self.i));
        }
        self.i += 1;
        let mut out = String::new();
        loop {
            let c = self.peek().ok_or("unterminated string")?;
            self.i += 1;
            match c {
                '"' => return Ok(out),
                '\\' => {
                    let e = self.peek().ok_or("bad escape")?;
                    self.i += 1;
                    match e {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        '/' => out.push('/'),
                        'b' => out.push('\u{8}'),
                        'f' => out.push('\u{c}'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'u' => {
                            let hex: String =
                                self.s.get(self.i..self.i + 4).ok_or("bad \\u")?.iter().collect();
                            self.i += 4;
                            let code = u32::from_str_radix(&hex, 16).map_err(|e| e.to_string())?;
                            out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                        }
                        other => return Err(format!("bad escape \\{other}")),
                    }
                }
                c => out.push(c),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// §1
// ---------------------------------------------------------------------------

/// S01: The declared reason is carried into the verdict verbatim
#[test]
fn s01_declared_reason_carried_verbatim() {
    let u = unbuilt(&[("ADR-2610161730", "delegate_scope test not written yet")]);
    assert_eq!(
        judge("ADR-2610161730", &Outcome::Failed, &u, true),
        Verdict::Unbuilt {
            reason: "delegate_scope test not written yet".to_string()
        }
    );
}

/// S02: A declaration covers only its own id; another entry's reason is never borrowed
#[test]
fn s02_declaration_covers_only_its_own_id() {
    let u = unbuilt(&[("ADR-2610161730", "not written")]);
    let v = judge("ADR-2610221834", &Outcome::Failed, &u, true);
    assert!(
        !matches!(v, Verdict::Unbuilt { .. }),
        "undeclared ADR must not be Unbuilt, got {v:?}"
    );
    assert!(matches!(v, Verdict::Fails(_)), "expected Fails, got {v:?}");
}

// ---------------------------------------------------------------------------
// §2
// ---------------------------------------------------------------------------

/// S03: Each failing outcome of a declared ADR becomes Unbuilt under --ci
#[test]
fn s03_each_failing_outcome_of_declared_adr_is_unbuilt_under_ci() {
    let u = unbuilt(&[("ADR-A", "why")]);
    for o in non_passing_outcomes() {
        assert_eq!(
            judge("ADR-A", &o, &u, true),
            Verdict::Unbuilt {
                reason: "why".to_string()
            },
            "outcome {o:?}"
        );
    }
}

/// S04: A declared ADR is reported as unbuilt outside --ci too
#[test]
fn s04_declared_adr_unbuilt_outside_ci() {
    let u = unbuilt(&[("ADR-A", "why")]);
    assert_eq!(
        judge("ADR-A", &Outcome::NotProvenHere, &u, false),
        Verdict::Unbuilt {
            reason: "why".to_string()
        }
    );
}

/// S05: A declared, still-unbuilt ADR is never counted as passed and never fails --ci
#[test]
fn s05_declared_unbuilt_never_passed_never_fails() {
    let u = unbuilt(&[("ADR-A", "why")]);
    let v = judge("ADR-A", &Outcome::Failed, &u, true);
    assert_ne!(v, Verdict::Passed);
    assert!(!matches!(v, Verdict::Fails(_)), "got {v:?}");
}

/// S06: For a declared ADR, every non-passing gate outcome is Unbuilt with the declared reason
#[test]
fn s06_property_declared_non_passing_is_unbuilt_with_reason() {
    let adrs = ["ADR-A", "ADR-2610161730", "ADR-x", "", "ADR-ünïcode"];
    let reasons = ["why", "", "multi\nline reason", "  padded  ", "ADR-OTHER"];
    for adr in adrs {
        for reason in reasons {
            let maps: Vec<Unbuilt> = vec![
                unbuilt(&[(adr, reason)]),
                unbuilt(&[(adr, reason), ("ADR-OTHER", "other reason")]),
                unbuilt(&[("ADR-0", "zero"), (adr, reason), ("ADR-Z", "zed")]),
            ];
            for u in &maps {
                for ci in [true, false] {
                    for o in non_passing_outcomes() {
                        assert_eq!(
                            judge(adr, &o, u, ci),
                            Verdict::Unbuilt {
                                reason: reason.to_string()
                            },
                            "adr={adr:?} reason={reason:?} o={o:?} ci={ci} U={u:?}"
                        );
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// §3
// ---------------------------------------------------------------------------

/// S07: A declared ADR whose gate passes fails the run and says to remove the declaration
#[test]
fn s07_declared_adr_passing_fails_and_says_remove() {
    let u = unbuilt(&[("ADR-A", "why")]);
    match judge("ADR-A", &Outcome::Passed, &u, true) {
        Verdict::Fails(msg) => {
            assert!(msg.contains("adr.unbuilt"), "msg must name adr.unbuilt: {msg}");
            assert!(msg.contains("ADR-A"), "msg must name ADR-A: {msg}");
            assert!(
                msg.to_lowercase().contains("remove"),
                "msg must tell the user to remove it: {msg}"
            );
        }
        other => panic!("expected Fails, got {other:?}"),
    }
}

/// S08: The list cannot hide a working gate, even outside --ci
#[test]
fn s08_list_cannot_hide_working_gate_outside_ci() {
    let u = unbuilt(&[("ADR-A", "why")]);
    let v = judge("ADR-A", &Outcome::Passed, &u, false);
    assert_ne!(v, Verdict::Passed);
    assert!(!matches!(v, Verdict::Unbuilt { .. }), "got {v:?}");
    assert!(matches!(v, Verdict::Fails(_)), "got {v:?}");
}

/// S09: A passing gate on a declared ADR always fails
#[test]
fn s09_property_passing_gate_on_declared_adr_fails() {
    let adrs = ["ADR-A", "ADR-2610221834", "ADR-q", ""];
    for adr in adrs {
        let maps: Vec<Unbuilt> = vec![
            unbuilt(&[(adr, "why")]),
            unbuilt(&[(adr, "")]),
            unbuilt(&[(adr, "r"), ("ADR-B", "b"), ("ADR-C", "c")]),
        ];
        for u in &maps {
            for ci in [true, false] {
                let v = judge(adr, &Outcome::Passed, u, ci);
                assert!(
                    matches!(v, Verdict::Fails(_)),
                    "adr={adr:?} ci={ci} U={u:?}: got {v:?}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// §4
// ---------------------------------------------------------------------------

/// S10: An id that names no ADR is reported
#[test]
fn s10_unknown_id_is_reported() {
    let u = unbuilt(&[("ADR-0000000000", "x")]);
    let g = gated(&[("ADR-A", true)]);
    let errs = declaration_errors(&u, &g);
    assert_eq!(errs.len(), 1, "errors: {errs:?}");
    assert!(errs[0].contains("ADR-0000000000"), "error: {}", errs[0]);
    assert!(
        errs[0].to_lowercase().contains("no adr"),
        "error must say it names no ADR: {}",
        errs[0]
    );
}

/// S11: An id that names an ADR with no gate is reported
#[test]
fn s11_ungated_id_is_reported() {
    let u = unbuilt(&[("ADR-B", "x")]);
    let g = gated(&[("ADR-B", false)]);
    let errs = declaration_errors(&u, &g);
    assert_eq!(errs.len(), 1, "errors: {errs:?}");
    assert!(errs[0].contains("ADR-B"), "error: {}", errs[0]);
    assert!(
        errs[0].to_lowercase().contains("no gate"),
        "error must say it records no gate: {}",
        errs[0]
    );
}

/// S12: A valid declaration yields no errors, and an empty one never does
#[test]
fn s12_valid_and_empty_declarations_have_no_errors() {
    let g = gated(&[("ADR-A", true), ("ADR-B", false)]);
    assert_eq!(
        declaration_errors(&unbuilt(&[("ADR-A", "x")]), &g),
        Vec::<String>::new()
    );
    assert_eq!(declaration_errors(&Unbuilt::new(), &g), Vec::<String>::new());
}

/// S13: Exactly the bad ids are reported
#[test]
fn s13_property_exactly_bad_ids_reported() {
    // Ids chosen so none is a substring of another.
    let ids = ["ADR-1001", "ADR-2002", "ADR-3003", "ADR-4004", "ADR-5005"];
    // Each id's status in G: 0 = absent, 1 = gated true, 2 = gated false.
    let n = ids.len();
    let mut cases = 0;
    for g_mask in 0..3u32.pow(n as u32) {
        let mut g = BTreeMap::new();
        let mut m = g_mask;
        for id in ids {
            match m % 3 {
                1 => {
                    g.insert(id.to_string(), true);
                }
                2 => {
                    g.insert(id.to_string(), false);
                }
                _ => {}
            }
            m /= 3;
        }
        for u_mask in 0..(1u32 << n) {
            if (g_mask + u_mask) % 7 != 0 && u_mask != 0 && u_mask != (1 << n) - 1 {
                continue; // sample the space
            }
            cases += 1;
            let u: Unbuilt = ids
                .iter()
                .enumerate()
                .filter(|(i, _)| u_mask & (1 << i) != 0)
                .map(|(_, id)| (id.to_string(), format!("reason for {id}")))
                .collect();
            let errs = declaration_errors(&u, &g);
            let mut all_good = true;
            for id in u.keys() {
                let naming = errs.iter().filter(|e| e.contains(id.as_str())).count();
                if g.get(id) == Some(&true) {
                    assert_eq!(naming, 0, "good id {id} named: U={u:?} G={g:?} errs={errs:?}");
                } else {
                    all_good = false;
                    assert_eq!(
                        naming, 1,
                        "bad id {id} must appear in exactly one error: U={u:?} G={g:?} errs={errs:?}"
                    );
                }
            }
            for (id, ok) in &g {
                if *ok {
                    assert!(
                        !errs.iter().any(|e| e.contains(id.as_str())),
                        "gated id {id} named: U={u:?} G={g:?} errs={errs:?}"
                    );
                }
            }
            assert_eq!(
                errs.is_empty(),
                all_good,
                "empty iff all ids gated: U={u:?} G={g:?} errs={errs:?}"
            );
        }
    }
    assert!(cases > 100);
}

// ---------------------------------------------------------------------------
// §5
// ---------------------------------------------------------------------------

/// S14: Undeclared ADRs keep today's verdicts
#[test]
fn s14_undeclared_adrs_keep_todays_verdicts() {
    let u = Unbuilt::new();
    assert_eq!(judge("ADR-A", &Outcome::Passed, &u, true), Verdict::Passed);
    assert!(matches!(
        judge("ADR-A", &Outcome::Failed, &u, true),
        Verdict::Fails(_)
    ));
    assert!(matches!(
        judge("ADR-A", &Outcome::NotProvenHere, &u, true),
        Verdict::Fails(_)
    ));
    assert_eq!(
        judge("ADR-A", &Outcome::NotProvenHere, &u, false),
        Verdict::Unproven
    );
    assert_eq!(judge("ADR-A", &Outcome::NoGate, &u, true), Verdict::NoGate);
}

/// S15: Declaring other ADRs does not soften an undeclared failure
#[test]
fn s15_other_declarations_do_not_soften_failure() {
    let u = unbuilt(&[("ADR-B", "why")]);
    let v = judge("ADR-A", &Outcome::Unrunnable, &u, true);
    assert!(matches!(v, Verdict::Fails(_)), "got {v:?}");
}

/// S16: The declaration has no effect on an ADR outside it
#[test]
fn s16_property_declaration_no_effect_outside_it() {
    let adrs = ["ADR-A", "ADR-2610271110", "ADR-NOPE"];
    let maps: Vec<Unbuilt> = vec![
        Unbuilt::new(),
        unbuilt(&[("ADR-B", "why")]),
        unbuilt(&[("ADR-2610161730", "x"), ("ADR-2610221834", "y")]),
        unbuilt(&[("ADR-A-suffix", "near miss"), ("adr-a", "case")]),
    ];
    for adr in adrs {
        for u in &maps {
            if u.contains_key(adr) {
                continue;
            }
            for o in all_outcomes() {
                for ci in [true, false] {
                    let with = judge(adr, &o, u, ci);
                    let without = judge(adr, &o, &Unbuilt::new(), ci);
                    assert_eq!(with, without, "adr={adr} o={o:?} ci={ci} U={u:?}");
                    assert!(
                        !matches!(with, Verdict::Unbuilt { .. }),
                        "adr={adr} o={o:?} ci={ci} U={u:?}: got {with:?}"
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// §6
// ---------------------------------------------------------------------------

/// S17: hexa's own list holds at most the two named ADRs
#[test]
fn s17_hexa_list_holds_at_most_two_named_adrs() {
    let keys = read_hexa_unbuilt_keys();
    assert!(keys.len() <= 2, "adr.unbuilt has {} entries: {keys:?}", keys.len());
    for k in &keys {
        assert!(
            ALLOWED_UNBUILT.contains(&k.as_str()),
            "unexpected adr.unbuilt entry {k}"
        );
    }
    ledger_check(&keys).unwrap();
}

/// S18: A new entry fails the ledger test
#[test]
fn s18_new_entry_fails_ledger_test() {
    let mut keys = read_hexa_unbuilt_keys();
    ledger_check(&keys).expect("hexa's own adr.unbuilt must pass the ledger");
    keys.push("ADR-2610271110".to_string());
    let err = ledger_check(&keys).expect_err("an added id must fail the ledger");
    assert!(err.contains("ADR-2610271110"), "error must name the added id: {err}");
}

/// S19: Removing a built decision is allowed
#[test]
fn s19_removing_built_decision_is_allowed() {
    let mut keys = read_hexa_unbuilt_keys();
    ledger_check(&keys).expect("hexa's own adr.unbuilt must pass the ledger");
    keys.retain(|k| k != "ADR-2610221834");
    if !keys.iter().any(|k| k == "ADR-2610161730") {
        keys.push("ADR-2610161730".to_string());
    }
    assert_eq!(keys, vec!["ADR-2610161730".to_string()]);
    assert_eq!(ledger_check(&keys), Ok(()));
}

// ---------------------------------------------------------------------------
// §7
// ---------------------------------------------------------------------------

/// S20: The judgement works from its arguments alone
#[test]
fn s20_judgement_works_from_arguments_alone() {
    let empty = std::env::temp_dir().join(format!(
        "hexa-gate-ledger-s20-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&empty).unwrap();
    assert!(!empty.join(".hexa").join("project.json").exists());
    let previous = std::env::current_dir().unwrap();
    std::env::set_current_dir(&empty).unwrap();

    let u = unbuilt(&[("ADR-X", "r")]);
    let v = judge("ADR-X", &Outcome::Failed, &u, true);
    let errs = declaration_errors(&u, &gated(&[("ADR-X", true)]));

    std::env::set_current_dir(previous).unwrap();
    let _ = std::fs::remove_dir_all(&empty);

    assert_eq!(
        v,
        Verdict::Unbuilt {
            reason: "r".to_string()
        }
    );
    assert_eq!(errs, Vec::<String>::new());
}

/// S21: judge does not run the gate itself or look ADRs up
#[test]
fn s21_judge_decides_from_outcome_only() {
    assert_eq!(
        judge("ADR-NOPE", &Outcome::Passed, &Unbuilt::new(), true),
        Verdict::Passed
    );
}

/// S22: Both functions are deterministic
#[test]
fn s22_property_both_functions_deterministic() {
    let maps: Vec<Unbuilt> = vec![
        Unbuilt::new(),
        unbuilt(&[("ADR-A", "why")]),
        unbuilt(&[("ADR-A", "why"), ("ADR-B", "b"), ("ADR-0000000000", "x")]),
    ];
    let gates: Vec<BTreeMap<String, bool>> = vec![
        BTreeMap::new(),
        gated(&[("ADR-A", true)]),
        gated(&[("ADR-A", true), ("ADR-B", false)]),
    ];
    for u in &maps {
        for adr in ["ADR-A", "ADR-B", "ADR-C"] {
            for o in all_outcomes() {
                for ci in [true, false] {
                    assert_eq!(
                        judge(adr, &o, u, ci),
                        judge(adr, &o, u, ci),
                        "adr={adr} o={o:?} ci={ci} U={u:?}"
                    );
                }
            }
        }
        for g in &gates {
            assert_eq!(
                declaration_errors(u, g),
                declaration_errors(u, g),
                "U={u:?} G={g:?}"
            );
        }
    }
}
