//! Every numbered decision is covered by an accepted spec, and every scenario
//! by a sealed test that runs (ADR-2610221540 phase 2: §1, §3, §4, §6 seal,
//! §8 without `--prove`, §9 ratchet).
//!
//! The spec is the ADR in behaviour: `given / when / then` per numbered
//! decision, each naming one test. `hexa spec check` judges coverage with no
//! model involved. `hexa spec verify` runs each test alone and refuses a
//! vacuous run, a skip, an edited seal and a spec changed since a person
//! accepted it.
//!
//! Ids are assembled at run time: the citation checker scans source, and a
//! literal id here would claim a decision exists.

use std::path::Path;
use std::process::Command;

fn hexa() -> std::path::PathBuf {
    let mut p = std::env::current_exe().expect("test exe");
    p.pop();
    p.pop();
    p.push("hexa");
    p
}

fn run(root: &Path, args: &[&str]) -> (i32, String) {
    let out = Command::new(hexa())
        .args(args)
        .current_dir(root)
        .env_remove("HEXA_NO_SKIP")
        .output()
        .expect("run hexa");
    (
        out.status.code().unwrap_or(-1),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

fn id() -> String {
    format!("ADR-{}", "2701010000")
}

/// A standalone crate with one test of each kind a spec can point at.
fn fixture_crate(root: &Path) {
    std::fs::create_dir_all(root.join("tests")).unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"fx\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[workspace]\n",
    )
    .unwrap();
    std::fs::write(root.join("src/lib.rs"), "").unwrap();
    std::fs::write(
        root.join("tests/inv.rs"),
        "#[test]\nfn passes() {}\n\n\
         #[test]\nfn also_passes() {}\n\n\
         #[test]\nfn fails() { panic!(\"the rule is broken\") }\n\n\
         #[test]\nfn skips() {\n    if std::env::var_os(\"HEXA_NO_SKIP\").is_some() {\n        panic!(\"not proven here: no frobnicator on PATH\");\n    }\n}\n\n\
         #[cfg(any())]\n#[test]\nfn never_runs() {}\n",
    )
    .unwrap();
}

/// An ADR with `status` and `n` numbered decisions.
fn write_adr(root: &Path, status: &str, n: usize) {
    let f = id();
    std::fs::create_dir_all(root.join("docs/adrs")).unwrap();
    let items: String = (1..=n).map(|i| format!("{i}. **Decision {i}.** It holds.\n")).collect();
    std::fs::write(
        root.join("docs/adrs").join(format!("{f}-a-decision.md")),
        format!(
            "---\nid: {f}\nstatus: {}\ndate: 2027-01-01\n---\n# {f}: A decision\n\n\
             **Status:** {status}\n**Date:** 2027-01-01\n**Drivers:** a fixture\n\n\
             ## Context\n\nx\n\n## Decision\n\n{items}\n## Consequences\n\nx\n\n\
             ## Implementation\n\nx\n\n## References\n\nnone\n",
            status.to_lowercase()
        ),
    )
    .unwrap();
}

/// One scenario as JSON.
fn sc(sid: &str, decisions: &[&str], kind: &str, test: Option<&str>) -> String {
    let ds: Vec<String> = decisions.iter().map(|d| format!("\"{d}\"")).collect();
    let test = test.map(|t| format!(",\"test\":\"{t}\"")).unwrap_or_default();
    format!(
        "{{\"id\":\"{sid}\",\"decision\":[{}],\"kind\":\"{kind}\",\"description\":\"d\",\
         \"given\":\"g\",\"when\":\"w\",\"then\":\"t\"{test}}}",
        ds.join(",")
    )
}

/// `docs/specs/<id>.json` with the scenarios and the extra top-level fields.
fn write_spec(root: &Path, scenarios: &[String], extra: &str) {
    std::fs::create_dir_all(root.join("docs/specs")).unwrap();
    std::fs::write(
        root.join("docs/specs").join(format!("{}.json", id())),
        format!(
            "{{\"adr\":\"{}\",\"feature\":\"fixture\",{extra}\"specs\":[{}]}}",
            id(),
            scenarios.join(",")
        ),
    )
    .unwrap();
}

fn project(status: &str, n: usize) -> tempfile::TempDir {
    let d = tempfile::tempdir().expect("tempdir");
    fixture_crate(d.path());
    write_adr(d.path(), status, n);
    d
}

/// Two decisions, each with an example and a negative, every test passing.
fn covered(root: &Path) {
    write_spec(
        root,
        &[
            sc("S1", &["§1"], "example", Some("tests/inv.rs::passes")),
            sc("S2", &["§1", "§2"], "negative", Some("tests/inv.rs::also_passes")),
            sc("S3", &["§2"], "example", Some("tests/inv.rs::passes")),
        ],
        "",
    );
}

// ── §3: coverage, with no model involved ──────────────────────────────────

#[test]
fn a_covered_spec_passes_check() {
    let d = project("Accepted", 2);
    covered(d.path());
    let (code, out) = run(d.path(), &["spec", "check", &id()]);
    assert_eq!(code, 0, "every decision has an example and a negative (control):\n{out}");
}

#[test]
fn an_uncovered_decision_is_named() {
    let d = project("Accepted", 2);
    write_spec(
        d.path(),
        &[
            sc("S1", &["§1"], "example", None),
            sc("S2", &["§1"], "negative", None),
        ],
        "",
    );
    let (code, out) = run(d.path(), &["spec", "check", &id()]);
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("§2") && out.contains("uncovered"), "the uncovered decision is named:\n{out}");
}

#[test]
fn a_decision_with_no_negative_is_named() {
    let d = project("Accepted", 1);
    write_spec(d.path(), &[sc("S1", &["§1"], "example", None)], "");
    let (code, out) = run(d.path(), &["spec", "check", &id()]);
    assert_ne!(code, 0, "a rule with no case that must be refused is half a rule:\n{out}");
    assert!(out.contains("§1") && out.contains("negative"), "{out}");
}

#[test]
fn a_universal_decision_needs_a_property() {
    let d = project("Accepted", 1);
    write_spec(
        d.path(),
        &[
            sc("S1", &["§1"], "example", None),
            sc("S2", &["§1"], "negative", None),
        ],
        "\"universal\":[\"§1\"],",
    );
    let (code, out) = run(d.path(), &["spec", "check", &id()]);
    assert_ne!(code, 0, "a rule over all inputs needs a property, not two examples:\n{out}");
    assert!(out.contains("property"), "{out}");
}

#[test]
fn an_exemption_covers_its_decision_and_is_printed() {
    let d = project("Accepted", 2);
    write_spec(
        d.path(),
        &[
            sc("S1", &["§1"], "example", None),
            sc("S2", &["§1"], "negative", None),
        ],
        "\"exempt\":[{\"decision\":\"§2\",\"reason\":\"it names a file layout\"}],",
    );
    let (code, out) = run(d.path(), &["spec", "check", &id()]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("exempt") && out.contains("it names a file layout"), "the reason is printed every run:\n{out}");
}

#[test]
fn an_exemption_without_a_reason_is_refused() {
    let d = project("Accepted", 2);
    write_spec(
        d.path(),
        &[
            sc("S1", &["§1"], "example", None),
            sc("S2", &["§1"], "negative", None),
        ],
        "\"exempt\":[{\"decision\":\"§2\",\"reason\":\"  \"}],",
    );
    let (code, out) = run(d.path(), &["spec", "check", &id()]);
    assert_ne!(code, 0, "an exemption is a claim, and a claim needs a reason:\n{out}");
    assert!(out.contains("§2") && out.contains("reason"), "{out}");
}

#[test]
fn a_scenario_citing_a_decision_the_adr_does_not_make_fails() {
    let d = project("Accepted", 1);
    write_spec(
        d.path(),
        &[
            sc("S1", &["§1"], "example", None),
            sc("S2", &["§1", "§7"], "negative", None),
        ],
        "",
    );
    let (code, out) = run(d.path(), &["spec", "check", &id()]);
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("§7"), "{out}");
}

// ── §4 and §6: a person accepts the spec; its tests are sealed ────────────

fn accept_and_seal(root: &Path) {
    let (code, out) = run(root, &["spec", "accept", &id(), "--by", "a reviewer"]);
    assert_eq!(code, 0, "accept:\n{out}");
    let (code, out) = run(root, &["spec", "seal", &id()]);
    assert_eq!(code, 0, "seal:\n{out}");
}

#[test]
fn an_accepted_sealed_spec_whose_tests_pass_verifies() {
    let d = project("Accepted", 2);
    covered(d.path());
    accept_and_seal(d.path());
    let (code, out) = run(d.path(), &["spec", "verify", "--ci", &id()]);
    assert_eq!(code, 0, "the control: accepted, sealed, every test ran once and passed:\n{out}");
    assert!(out.contains("S1") && out.contains("S3"), "each scenario is reported:\n{out}");
}

#[test]
fn seal_refuses_a_spec_nobody_accepted() {
    let d = project("Accepted", 2);
    covered(d.path());
    let (code, out) = run(d.path(), &["spec", "seal", &id()]);
    assert_ne!(code, 0, "tests are sealed against an accepted spec, not a draft:\n{out}");
    assert!(out.contains("not accepted"), "{out}");
}

#[test]
fn seal_refuses_a_test_outside_a_tests_directory() {
    let d = project("Accepted", 1);
    std::fs::write(d.path().join("src/lib.rs"), "#[cfg(test)]\nmod t {\n    #[test]\n    fn inline() {}\n}\n").unwrap();
    write_spec(
        d.path(),
        &[
            sc("S1", &["§1"], "example", Some("tests/inv.rs::passes")),
            sc("S2", &["§1"], "negative", Some("src/lib.rs::t::inline")),
        ],
        "",
    );
    let (code, out) = run(d.path(), &["spec", "accept", &id(), "--by", "a reviewer"]);
    assert_eq!(code, 0, "{out}");
    let (code, out) = run(d.path(), &["spec", "seal", &id()]);
    assert_ne!(code, 0, "sealing src/lib.rs would freeze the code the test drives:\n{out}");
    assert!(out.contains("src/lib.rs") && out.contains("tests/"), "{out}");
}

#[test]
fn a_spec_changed_after_acceptance_fails_verify() {
    let d = project("Accepted", 2);
    covered(d.path());
    accept_and_seal(d.path());
    let p = d.path().join("docs/specs").join(format!("{}.json", id()));
    // `"t"` is only ever the `then` value in these fixtures, however the
    // file is formatted after acceptance rewrote it.
    let before = std::fs::read_to_string(&p).unwrap();
    let text = before.replacen("\"t\"", "\"something else\"", 1);
    assert_ne!(before, text, "the edit must change the spec, or this test proves nothing");
    std::fs::write(&p, text).unwrap();
    let (code, out) = run(d.path(), &["spec", "verify", &id()]);
    assert_ne!(code, 0, "what a person accepted is not what is here:\n{out}");
    assert!(out.contains("accepted"), "{out}");
}

#[test]
fn an_edited_sealed_test_fails_verify() {
    let d = project("Accepted", 2);
    covered(d.path());
    accept_and_seal(d.path());
    let p = d.path().join("tests/inv.rs");
    let text = std::fs::read_to_string(&p).unwrap() + "\n// loosened\n";
    std::fs::write(&p, text).unwrap();
    let (code, out) = run(d.path(), &["spec", "verify", &id()]);
    assert_ne!(code, 0, "a sealed test changes through the spec, never by hand:\n{out}");
    assert!(out.contains("seal") && out.contains("tests/inv.rs"), "{out}");
}

#[test]
fn seal_refuses_to_reseal_an_edited_sealed_test() {
    let d = project("Accepted", 2);
    covered(d.path());
    accept_and_seal(d.path());
    let p = d.path().join("tests/inv.rs");
    let text = std::fs::read_to_string(&p).unwrap() + "\n// loosened\n";
    std::fs::write(&p, text).unwrap();
    let (code, out) = run(d.path(), &["spec", "seal", &id()]);
    assert_ne!(code, 0, "re-sealing would launder the edit:\n{out}");
    assert!(out.contains("seal broken") && out.contains("tests/inv.rs"), "{out}");
    let (code, out) = run(d.path(), &["spec", "verify", "--ci", &id()]);
    assert_ne!(code, 0, "the edit is still a broken seal after the refused re-seal:\n{out}");
}

// ── §8: each test runs alone and must prove something ─────────────────────

fn verify_one(test: &str) -> (i32, String, i32, String) {
    let d = project("Accepted", 1);
    write_spec(
        d.path(),
        &[
            sc("S1", &["§1"], "example", Some("tests/inv.rs::passes")),
            sc("S2", &["§1"], "negative", Some(test)),
        ],
        "",
    );
    accept_and_seal(d.path());
    let (code, out) = run(d.path(), &["spec", "verify", &id()]);
    let (ci, ci_out) = run(d.path(), &["spec", "verify", "--ci", &id()]);
    (code, out, ci, ci_out)
}

#[test]
fn a_failing_test_fails_verify() {
    let (code, out, _, _) = verify_one("tests/inv.rs::fails");
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("S2") && out.contains("failed"), "{out}");
}

#[test]
fn a_ref_to_a_missing_test_fails_verify() {
    let (code, out, _, _) = verify_one("tests/inv.rs::no_such_test");
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("no_such_test") && out.contains("not found"), "{out}");
}

#[test]
fn a_test_that_runs_nothing_is_vacuous() {
    // In the file, so a text search finds it; compiled out, so `--exact`
    // runs zero tests. Only running it tells the difference.
    let (code, out, _, _) = verify_one("tests/inv.rs::never_runs");
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("vacuous"), "{out}");
}

#[test]
fn a_skip_is_not_proven_here_and_fails_under_ci() {
    let (code, out, ci, ci_out) = verify_one("tests/inv.rs::skips");
    assert!(out.contains("not proven here") && out.contains("frobnicator"), "{out}");
    assert_eq!(code, 0, "without --ci a case this machine cannot run is reported:\n{out}");
    assert_ne!(ci, 0, "under --ci it fails:\n{ci_out}");
}

#[test]
fn a_scenario_with_no_test_is_pending_and_a_completed_adr_may_not_have_one() {
    let d = project("Accepted", 1);
    write_spec(
        d.path(),
        &[
            sc("S1", &["§1"], "example", Some("tests/inv.rs::passes")),
            sc("S2", &["§1"], "negative", None),
        ],
        "",
    );
    accept_and_seal(d.path());
    let (code, out) = run(d.path(), &["spec", "verify", "--ci", &id()]);
    assert!(out.contains("pending"), "a scenario not yet generated is pending:\n{out}");
    assert_eq!(code, 0, "work in progress on an Accepted ADR is visible, not red:\n{out}");

    write_adr(d.path(), "Completed", 1);
    let (code, out) = run(d.path(), &["spec", "verify", "--ci", &id()]);
    assert_ne!(code, 0, "a Completed decision with an untested scenario is not complete:\n{out}");
}

// ── §9: the ledger, ratcheted ─────────────────────────────────────────────

/// ADRs accepted before ADR-2610221540 carry no spec. The count may fall as
/// they are backfilled; it may never rise.
const LEGACY_CEILING: usize = 45;

#[test]
fn this_ledger_checks_and_its_legacy_only_shrinks() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let (code, out) = run(root, &["spec", "check"]);
    assert_eq!(code, 0, "every spec here covers its ADR, and every ADR from ADR-2610221540 on has one:\n{out}");
    let legacy = out
        .lines()
        .find_map(|l| {
            let head = l.split(" legacy").next()?;
            (head != l).then(|| head.rsplit(' ').next()?.parse::<usize>().ok()).flatten()
        })
        .unwrap_or_else(|| panic!("a summary with the legacy count:\n{out}"));
    assert!(legacy <= LEGACY_CEILING, "{legacy} legacy ADRs > ceiling {LEGACY_CEILING}");
}

#[test]
fn an_adr_after_this_decision_without_a_spec_fails() {
    let d = tempfile::tempdir().unwrap();
    write_adr(d.path(), "Accepted", 1);
    let (code, out) = run(d.path(), &["spec", "check"]);
    assert_ne!(code, 0, "a decision made under ADR-2610221540 must have a spec:\n{out}");
    assert!(out.contains(&id()) && out.contains("no spec"), "{out}");
}
