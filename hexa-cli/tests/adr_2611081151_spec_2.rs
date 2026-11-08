#![allow(unused, clippy::all)]
mod part_1 {
    #![allow(clippy::all)]

    use hexa_core::domain::spec::{judge_seal, Amendment, SealVerdict};

    fn amend(file: &str, from: &str, to: &str) -> Amendment {
        Amendment {
            file: file.to_string(),
            from: from.to_string(),
            to: to.to_string(),
            by: "tester".to_string(),
            reason: "test amendment".to_string(),
            at: "2026-11-08T00:00:00Z".to_string(),
            base_commit: "0000000000000000000000000000000000000000".to_string(),
        }
    }

    fn h(n: usize) -> String {
        format!("{:064x}", n as u128 * 0x9e37_79b9_7f4a_7c15u128 + 17)
    }

    /// S01: A sealed file whose bytes match its blob at the red commit holds
    #[test]
    fn s01_matching_blob_at_red_holds() {
        let h0 = h(0);
        let v = judge_seal("tests/a.rs", Some(&h0), &[], Some(&h0), Some(&h0));
        assert_eq!(v, SealVerdict::Holds);
    }

    /// S02: An amendment chain moves the expected hash
    #[test]
    fn s02_amendment_chain_moves_expected_hash() {
        let h0 = h(0);
        let h1 = h(1);
        let amendments = vec![amend("tests/a.rs", &h0, &h1)];
        let v = judge_seal("tests/a.rs", Some(&h0), &amendments, Some(&h1), Some(&h1));
        assert_eq!(v, SealVerdict::Holds);
    }

    /// S03: Amendments for other files do not apply
    #[test]
    fn s03_amendments_for_other_files_do_not_apply() {
        let h0 = h(0);
        let x = h(100);
        let y = h(101);
        let amendments = vec![amend("tests/b.rs", &x, &y)];
        let v = judge_seal("tests/a.rs", Some(&h0), &amendments, Some(&h0), Some(&h0));
        assert_eq!(v, SealVerdict::Holds);
    }

    /// S04: A loosened test whose new hash was written into the JSON by hand is refused as Rewritten
    #[test]
    fn s04_hand_written_hash_is_rewritten() {
        let h0 = h(0);
        let h1 = h(1);
        assert_ne!(h0, h1);
        let v = judge_seal("tests/a.rs", Some(&h0), &[], Some(&h1), Some(&h1));
        assert_ne!(v, SealVerdict::Holds);
        assert_eq!(v, SealVerdict::Rewritten);
    }

    /// S05: A file edited after sealing is Edited
    #[test]
    fn s05_file_edited_after_sealing_is_edited() {
        let h0 = h(0);
        let h2 = h(2);
        assert_ne!(h0, h2);
        let v = judge_seal("tests/a.rs", Some(&h0), &[], Some(&h0), Some(&h2));
        assert_eq!(v, SealVerdict::Edited);
    }

    /// S06: An amendment whose from does not continue the chain is a broken chain at its index
    #[test]
    fn s06_broken_chain_reports_index() {
        let h0 = h(0);
        let h1 = h(1);
        let h2 = h(2);
        let h9 = h(9);
        assert_ne!(h9, h1);
        let amendments = vec![amend("tests/a.rs", &h0, &h1), amend("tests/a.rs", &h9, &h2)];
        let v = judge_seal("tests/a.rs", Some(&h0), &amendments, Some(&h2), Some(&h2));
        assert_eq!(v, SealVerdict::BrokenChain(1));
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let d = Sha256::digest(bytes);
        d.iter().map(|b| format!("{:02x}", b)).collect()
    }

    fn git(dir: &std::path::Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "tester")
            .env("GIT_AUTHOR_EMAIL", "tester@example.invalid")
            .env("GIT_COMMITTER_NAME", "tester")
            .env("GIT_COMMITTER_EMAIL", "tester@example.invalid")
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// S07: hexa spec verify fails on an edited, rewritten or broken-chain seal
    #[test]
    fn s07_spec_verify_fails_on_rewritten_seal() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        git(root, &["init", "-q"]);
        git(root, &["config", "user.name", "tester"]);
        git(root, &["config", "user.email", "tester@example.invalid"]);

        let adr = "ADR-2611081200";
        std::fs::create_dir_all(root.join(".hexa")).unwrap();
        std::fs::write(root.join(".hexa/project.json"), r#"{"name":"sealtest"}"#).unwrap();
        std::fs::create_dir_all(root.join("docs/adrs")).unwrap();
        std::fs::create_dir_all(root.join("docs/specs")).unwrap();
        std::fs::create_dir_all(root.join("tests")).unwrap();
        std::fs::write(
        root.join(format!("docs/adrs/{adr}-seal-test.md")),
        format!("# {adr}: seal test\n\n**Status:** Accepted\n\n## Decision\n\n1. Something.\n\n## Gate\n\n`hexa spec verify {adr}`\n"),
    )
    .unwrap();

        let strict = b"/// S01: strict\n#[test]\nfn s01() { assert_eq!(1 + 1, 3); }\n".to_vec();
        std::fs::write(root.join("tests/a.rs"), &strict).unwrap();
        git(root, &["add", "-A"]);
        git(root, &["commit", "-q", "-m", "red"]);
        let red = git(root, &["rev-parse", "HEAD"]);

        // Loosen the test and write its new hash into the JSON by hand.
        let loose = b"/// S01: strict\n#[test]\nfn s01() { assert!(true); }\n".to_vec();
        std::fs::write(root.join("tests/a.rs"), &loose).unwrap();
        let loose_hash = sha256_hex(&loose);
        let spec = serde_json::json!({
            "adr": adr,
            "scenarios": [{
                "id": "S01",
                "decision": ["§1"],
                "kind": "example",
                "description": "strict",
                "given": "g",
                "when": "w",
                "then": "t",
                "test": "tests/a.rs"
            }],
            "tests": { "S01": "tests/a.rs" },
            "red_commit": red,
            "red_at": red,
            "sealed": { "tests/a.rs": loose_hash },
            "amendments": []
        });
        std::fs::write(
            root.join(format!("docs/specs/{adr}.json")),
            serde_json::to_string_pretty(&spec).unwrap(),
        )
        .unwrap();
        git(root, &["add", "-A"]);
        git(root, &["commit", "-q", "-m", "loosen"]);

        let out = std::process::Command::new(env!("CARGO_BIN_EXE_hexa"))
            .args(["spec", "verify", adr])
            .current_dir(root)
            .env("NO_COLOR", "1")
            .output()
            .expect("hexa runs");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(!out.status.success(), "verify must fail:\n{text}");
        assert!(text.contains("tests/a.rs"), "must name the file:\n{text}");
        assert!(
            text.to_lowercase().contains("rewritten"),
            "must name the verdict:\n{text}"
        );
    }

    /// S08: The JSON record never moves the expected hash
    #[test]
    fn s08_record_never_moves_expected_hash() {
        for len in 0..5usize {
            let h0 = h(1000 + len);
            let mut amendments = Vec::new();
            let mut prev = h0.clone();
            for i in 0..len {
                let next = h(2000 + len * 10 + i);
                // interleave an unrelated file's amendment
                amendments.push(amend("tests/other.rs", &h(5000 + i), &h(6000 + i)));
                amendments.push(amend("tests/a.rs", &prev, &next));
                prev = next;
            }
            let e = prev;
            for r_seed in 0..4usize {
                let r = if r_seed == 0 {
                    h0.clone()
                } else {
                    h(9000 + r_seed)
                };
                if r == e {
                    continue;
                }
                let v = judge_seal("tests/a.rs", Some(&h0), &amendments, Some(&r), Some(&e));
                assert_ne!(
                    v,
                    SealVerdict::Holds,
                    "len={len} recorded={r} expected={e} must not hold"
                );
            }
        }
    }

    /// S09: An unbroken chain with matching record and file always holds
    #[test]
    fn s09_unbroken_chain_with_matching_record_holds() {
        for len in 0..6usize {
            let h0 = h(3000 + len);
            let mut amendments = Vec::new();
            let mut prev = h0.clone();
            for i in 0..len {
                let next = h(4000 + len * 10 + i);
                amendments.push(amend("tests/a.rs", &prev, &next));
                amendments.push(amend("tests/b.rs", &h(7000 + i), &h(8000 + i)));
                prev = next;
            }
            let e = prev;
            let v = judge_seal("tests/a.rs", Some(&h0), &amendments, Some(&e), Some(&e));
            assert_eq!(v, SealVerdict::Holds, "len={len}");
        }
    }

    /// S10: A file with no blob at the red commit is Unanchored
    #[test]
    fn s10_no_blob_at_red_is_unanchored() {
        let h0 = h(0);
        let v = judge_seal("tests/a.rs", None, &[], Some(&h0), Some(&h0));
        assert_eq!(v, SealVerdict::Unanchored);
    }
}

mod part_2 {
    #![allow(dead_code, unused_imports, clippy::all)]

    use std::path::Path;
    use std::process::Command;

    use hexa_core::domain::evidence::{read_evidence, Evidence};
    use hexa_core::domain::spec::{judge_seal, Amendment, SealVerdict};
    use sha2::{Digest, Sha256};

    fn sha256_hex(bytes: &[u8]) -> String {
        let digest = Sha256::digest(bytes);
        digest.iter().map(|b| format!("{:02x}", b)).collect()
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-c")
            .arg("user.name=hexa-test")
            .arg("-c")
            .arg("user.email=hexa-test@example.invalid")
            .arg("-c")
            .arg("commit.gpgsign=false")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    const SEALED_BODY: &str = "#[test]\nfn a() { assert_eq!(1 + 1, 2); }\n";

    /// A repository whose HEAD holds `docs/specs/ADR-X.json` sealing `tests/a.rs`
    /// at the hash of its committed bytes.
    fn sealed_repo() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        git(root, &["init", "-q"]);
        std::fs::create_dir_all(root.join("tests")).unwrap();
        std::fs::create_dir_all(root.join("docs/specs")).unwrap();
        std::fs::write(root.join("tests/a.rs"), SEALED_BODY).unwrap();
        let spec = serde_json::json!({
            "id": "ADR-X",
            "sealed": { "tests/a.rs": sha256_hex(SEALED_BODY.as_bytes()) }
        });
        std::fs::write(
            root.join("docs/specs/ADR-X.json"),
            serde_json::to_string_pretty(&spec).unwrap(),
        )
        .unwrap();
        git(root, &["add", "-A"]);
        git(root, &["commit", "-q", "-m", "seal tests/a.rs"]);
        tmp
    }

    fn names(list: &[String], needle: &str) -> bool {
        list.iter().any(|s| s.contains(needle))
    }

    fn amendment(file: &str, from: &str, to: &str) -> Amendment {
        Amendment {
            file: file.to_string(),
            from: from.to_string(),
            to: to.to_string(),
            by: "reviewer".to_string(),
            reason: "test".to_string(),
            at: "2026-11-08T00:00:00Z".to_string(),
            base_commit: "0000000".to_string(),
        }
    }

    /// S11: verify --ci fails an unanchored seal by name
    #[test]
    fn s11_unanchored_seal_is_not_green() {
        // A red commit that is not an ancestor of HEAD gives no blob to anchor to.
        let h = sha256_hex(SEALED_BODY.as_bytes());
        let verdict = judge_seal("tests/a.rs", None, &[], Some(&h), Some(&h));
        assert_eq!(verdict, SealVerdict::Unanchored);
        assert_ne!(
            verdict,
            SealVerdict::Holds,
            "an unanchored seal must not be reported green"
        );
    }

    /// S12: Re-anchoring with --red-at restores trust
    #[test]
    fn s12_reanchored_seal_judges_against_blob_at_c() {
        let at_c = sha256_hex(SEALED_BODY.as_bytes());

        // Unchanged file, record matching the blob at C: holds, not Unanchored.
        let holds = judge_seal("tests/a.rs", Some(&at_c), &[], Some(&at_c), Some(&at_c));
        assert_eq!(holds, SealVerdict::Holds);
        assert_ne!(holds, SealVerdict::Unanchored);

        // The blob at C is the reference: a file differing from it is Edited.
        let other = sha256_hex(b"#[test]\nfn a() {}\n");
        let edited = judge_seal("tests/a.rs", Some(&at_c), &[], Some(&at_c), Some(&other));
        assert_eq!(edited, SealVerdict::Edited);
        assert_ne!(edited, SealVerdict::Unanchored);
    }

    /// S13: No anchor means never Holds
    #[test]
    fn s13_no_anchor_never_holds() {
        let h1 = sha256_hex(b"one");
        let h2 = sha256_hex(b"two");
        let h3 = sha256_hex(b"three");
        let files = ["tests/a.rs", "tests/b.rs", ""];
        let values: Vec<Option<&str>> = vec![
            None,
            Some(h1.as_str()),
            Some(h2.as_str()),
            Some(h3.as_str()),
        ];
        let amendment_sets: Vec<Vec<Amendment>> = vec![
            vec![],
            vec![amendment("tests/a.rs", &h1, &h2)],
            vec![
                amendment("tests/a.rs", &h1, &h2),
                amendment("tests/a.rs", &h2, &h3),
            ],
            vec![amendment("tests/b.rs", &h1, &h2)],
            vec![amendment("tests/a.rs", &h3, &h1)],
        ];
        for file in files {
            for amendments in &amendment_sets {
                for recorded in &values {
                    for current in &values {
                        let v = judge_seal(file, None, amendments, *recorded, *current);
                        assert_ne!(
                        v,
                        SealVerdict::Holds,
                        "Holds with no anchor: file={file:?} recorded={recorded:?} current={current:?} amendments={amendments:?}"
                    );
                    }
                }
            }
        }
    }

    /// S14: An agent run is checked against HEAD's seals
    #[test]
    fn s14_seals_are_read_from_head() {
        let tmp = sealed_repo();
        let root = tmp.path();

        // The run removes the working tree's sealed map, then edits the sealed test.
        std::fs::write(
            root.join("docs/specs/ADR-X.json"),
            serde_json::to_string_pretty(&serde_json::json!({ "id": "ADR-X" })).unwrap(),
        )
        .unwrap();
        std::fs::write(root.join("tests/a.rs"), "#[test]\nfn a() {}\n").unwrap();

        let broken = hexa_exec::direct_exec::broken_seals(root);
        assert!(
            names(&broken, "tests/a.rs"),
            "seals must come from HEAD, so the edit to tests/a.rs is caught; got {broken:?}"
        );
    }

    /// S15: A run that touches a spec JSON is refused and restored
    #[test]
    fn s15_touching_spec_json_is_refused_by_name() {
        let tmp = sealed_repo();
        let root = tmp.path();

        // The run rewrites the spec record (moving the seal to match its own edit).
        let loosened = "#[test]\nfn a() {}\n";
        std::fs::write(root.join("tests/a.rs"), loosened).unwrap();
        let spec = serde_json::json!({
            "id": "ADR-X",
            "sealed": { "tests/a.rs": sha256_hex(loosened.as_bytes()) }
        });
        std::fs::write(
            root.join("docs/specs/ADR-X.json"),
            serde_json::to_string_pretty(&spec).unwrap(),
        )
        .unwrap();

        let broken = hexa_exec::direct_exec::broken_seals(root);
        assert!(
            names(&broken, "docs/specs/ADR-X.json"),
            "a change to a spec JSON must be refused by name; got {broken:?}"
        );
        assert!(
            names(&broken, "tests/a.rs"),
            "the rewritten record must not move the seal; got {broken:?}"
        );
    }

    /// S16: A sealed file deleted by a run is an edit
    #[test]
    fn s16_deleted_sealed_file_is_gone() {
        let h = sha256_hex(SEALED_BODY.as_bytes());
        let verdict = judge_seal("tests/a.rs", Some(&h), &[], Some(&h), None);
        assert_eq!(verdict, SealVerdict::Gone);

        let tmp = sealed_repo();
        let root = tmp.path();
        std::fs::remove_file(root.join("tests/a.rs")).unwrap();
        let broken = hexa_exec::direct_exec::broken_seals(root);
        assert!(
            names(&broken, "tests/a.rs"),
            "a sealed file deleted by the run must be refused; got {broken:?}"
        );
    }

    /// S18: cargo test with passing tests reads as Passed
    #[test]
    fn s18_cargo_test_passed() {
        let output = "running 5 tests\n\
test a ... ok\ntest b ... ok\ntest c ... ok\ntest d ... ok\ntest e ... ok\n\n\
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n";
        let ev = read_evidence(true, output, false);
        assert_eq!(ev, Evidence::Passed { tests: 5 });
        assert!(ev.passes());
    }

    /// S19: The Node 23+ spec reporter is recognised
    #[test]
    fn s19_node_spec_reporter_passed() {
        let output = "✔ one (0.5ms)\n✔ two (0.2ms)\n✔ three (0.1ms)\n✔ four (0.1ms)\n\
ℹ tests 4\nℹ suites 0\nℹ pass 4\nℹ fail 0\nℹ cancelled 0\nℹ skipped 0\nℹ todo 0\nℹ duration_ms 42.1\n";
        let ev = read_evidence(true, output, false);
        assert_eq!(ev, Evidence::Passed { tests: 4 });
        assert!(ev.passes());
    }

    /// S20: The node --test TAP summary is recognised
    #[test]
    fn s20_node_tap_passed() {
        let output = "TAP version 13\nok 1 - one\nok 2 - two\nok 3 - three\n1..3\n\
# tests 3\n# suites 0\n# pass 3\n# fail 0\n# cancelled 0\n# skipped 0\n# todo 0\n# duration_ms 30.5\n";
        let ev = read_evidence(true, output, false);
        assert_eq!(ev, Evidence::Passed { tests: 3 });
        assert!(ev.passes());
    }

    /// S21: The hexa spec verify summary is recognised
    #[test]
    fn s21_spec_verify_summary_passed() {
        let output = "ADR-2611081151\n6 passed · 0 failed\n";
        let ev = read_evidence(true, output, false);
        assert_eq!(ev, Evidence::Passed { tests: 6 });
        assert!(ev.passes());
    }
}

mod part_3 {
    #![allow(dead_code)]

    use hexa_core::domain::evidence::{read_evidence, Evidence};
    use std::path::Path;
    use std::process::{Command, Output};

    /// Runner outputs used by the property scenarios, each paired with whether
    /// any recognised line in it reports a failed test.
    fn corpus() -> Vec<(&'static str, bool)> {
        vec![
        ("", false),
        ("all good", false),
        (
            "running 5 tests\n.....\ntest result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n",
            false,
        ),
        (
            "running 5 tests\ntest result: FAILED. 4 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out\n",
            true,
        ),
        (
            "running 0 tests\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n",
            false,
        ),
        (
            "running 3 tests\ntest result: ok. 3 passed; 0 failed; 0 ignored\n\nrunning 0 tests\ntest result: ok. 0 passed; 0 failed; 0 ignored\n",
            false,
        ),
        (
            "running 2 tests\ntest result: ok. 2 passed; 0 failed; 0 ignored\n\nrunning 1 test\ntest result: FAILED. 0 passed; 1 failed; 0 ignored\n",
            true,
        ),
        ("TAP version 13\n# tests 4\n# pass 4\n# fail 0\n", false),
        ("TAP version 13\n# tests 4\n# pass 3\n# fail 1\n", true),
        ("TAP version 13\n# tests 0\n# pass 0\n# fail 0\n", false),
        ("ℹ tests 4\nℹ suites 1\nℹ pass 4\nℹ fail 0\n", false),
        ("ℹ tests 4\nℹ suites 1\nℹ pass 2\nℹ fail 2\n", true),
        ("=== RUN   TestA\n--- PASS: TestA (0.00s)\nPASS\nok  \texample.com/a\t0.002s\n", false),
        ("=== RUN   TestA\n--- FAIL: TestA (0.00s)\nFAIL\nFAIL\texample.com/a\t0.002s\n", true),
        (" 3 pass\n 0 fail\nRan 3 tests across 1 files.\n", false),
        (" 2 pass\n 1 fail\nRan 3 tests across 1 files.\n", true),
        ("3 passed · 0 failed\n", false),
        ("2 passed · 1 failed\n", true),
        ("test result: ok. 5 passed; 0 failed\nall good\n", false),
    ]
    }

    /// S22: A reported failure fails even with exit 0
    #[test]
    fn s22_reported_failure_fails_even_with_exit_0() {
        let out = "running 5 tests\ntest result: FAILED. 4 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out\n";
        let ev = read_evidence(true, out, false);
        assert!(
            matches!(ev, Evidence::Failed { .. }),
            "expected Failed, got {ev:?}"
        );
    }

    /// S23: An empty node --test run is Vacuous
    #[test]
    fn s23_empty_node_test_run_is_vacuous() {
        let out = "TAP version 13\n1..0\n# tests 0\n# suites 0\n# pass 0\n# fail 0\n# cancelled 0\n# skipped 0\n# todo 0\n# duration_ms 12.3\n";
        let ev = read_evidence(true, out, false);
        assert_eq!(ev, Evidence::Vacuous);
    }

    /// S24: Silent or unknown output is Unrecognised
    #[test]
    fn s24_silent_or_unknown_output_is_unrecognised() {
        assert_eq!(read_evidence(true, "", false), Evidence::Unrecognised);
        assert_eq!(
            read_evidence(true, "all good", false),
            Evidence::Unrecognised
        );
    }

    /// S25: A non-zero exit fails regardless of output
    #[test]
    fn s25_non_zero_exit_fails_regardless_of_output() {
        let out = "running 5 tests\ntest result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n";
        let ev = read_evidence(false, out, false);
        assert!(
            matches!(ev, Evidence::Failed { .. }),
            "expected Failed, got {ev:?}"
        );
    }

    /// S26: Passed requires exit 0, at least one test, and no reported failure
    #[test]
    fn s26_passed_requires_exit_0_tests_and_no_failure() {
        let mut saw_passed = false;
        for (out, reports_failure) in corpus() {
            for exit_ok in [true, false] {
                let ev = read_evidence(exit_ok, out, false);
                if let Evidence::Passed { tests } = ev {
                    saw_passed = true;
                    assert!(exit_ok, "Passed with non-zero exit for output {out:?}");
                    assert!(tests >= 1, "Passed with {tests} tests for output {out:?}");
                    assert!(
                        !reports_failure,
                        "Passed although output reports a failed test: {out:?}"
                    );
                }
            }
        }
        // The property must be exercised: a clean cargo run is Passed (§4).
        assert!(saw_passed, "no input in the corpus read as Passed");
    }

    /// S27: A non-zero exit is always Failed
    #[test]
    fn s27_non_zero_exit_is_always_failed() {
        for (out, _) in corpus() {
            let ev = read_evidence(false, out, false);
            assert!(
                matches!(ev, Evidence::Failed { .. }),
                "expected Failed for non-zero exit with output {out:?}, got {ev:?}"
            );
        }
    }

    /// S28: Only Passed (or a declared Uncounted) passes
    #[test]
    fn s28_passed_and_uncounted_pass() {
        assert!(Evidence::Passed { tests: 1 }.passes());
        assert!(Evidence::Uncounted.passes());
    }

    /// S29: Vacuous, Unrecognised and Failed do not pass
    #[test]
    fn s29_vacuous_unrecognised_failed_do_not_pass() {
        assert!(!Evidence::Vacuous.passes());
        assert!(!Evidence::Unrecognised.passes());
        assert!(!Evidence::Failed {
            reason: "x".to_string()
        }
        .passes());
    }

    // ---- CLI helpers ----------------------------------------------------------

    const ADR_ID: &str = "ADR-2611080001";

    fn git(dir: &Path, args: &[&str]) {
        let st = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .expect("git runs");
        assert!(st.success(), "git {args:?} failed");
    }

    fn project(gate_heading_body: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        git(root, &["init", "-q"]);
        git(root, &["config", "user.email", "test@example.com"]);
        git(root, &["config", "user.name", "Test"]);
        std::fs::create_dir_all(root.join(".hexa")).unwrap();
        std::fs::write(
            root.join(".hexa/project.json"),
            r#"{"name":"evidence-test"}"#,
        )
        .unwrap();
        std::fs::create_dir_all(root.join("docs/adrs")).unwrap();
        let adr = format!(
        "# {ADR_ID}: A test decision\n\n**Status:** Accepted\n**Date:** 2026-11-08\n\n## Context\n\nA test.\n\n## Decision\n\nA test.\n\n## Gate\n\n{gate_heading_body}\n"
    );
        std::fs::write(
            root.join(format!("docs/adrs/{ADR_ID}-a-test-decision.md")),
            adr,
        )
        .unwrap();
        std::fs::write(root.join("README.md"), "test\n").unwrap();
        git(root, &["add", "-A"]);
        git(root, &["commit", "-q", "-m", "init"]);
        dir
    }

    fn hexa(dir: &Path, home: &Path, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_hexa"))
            .args(args)
            .current_dir(dir)
            .env("HOME", home)
            .env("HEXA_SESSION_ID", "evidence-part3-session")
            .env("CLAUDE_SESSION_ID", "evidence-part3-session")
            .env("NO_COLOR", "1")
            .output()
            .expect("hexa runs")
    }

    fn text(o: &Output) -> String {
        format!(
            "{}\n{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        )
    }

    fn assert_unrecognised_refusal(what: &str, o: &Output) {
        let t = text(o);
        let lower = t.to_lowercase();
        assert!(!o.status.success(), "{what}: a gate of `true` passed:\n{t}");
        assert!(
            lower.contains("unrecognised") || lower.contains("unrecognized"),
            "{what}: output does not report unrecognised:\n{t}"
        );
        assert!(
            lower.contains("cargo test") && lower.contains("go test") && lower.contains("node"),
            "{what}: output does not list the runners the reader knows:\n{t}"
        );
    }

    /// S30: A gate of `true` is refused by every gate consumer
    #[test]
    fn s30_gate_true_is_refused_by_every_gate_consumer() {
        // `hexa do` needs a configured model to reach its gate, so it is covered
        // through the two consumers runnable without one.

        // hexa adr gates
        let p = project("`true`");
        let home = tempfile::tempdir().unwrap();
        let o = hexa(p.path(), home.path(), &["adr", "gates"]);
        assert_unrecognised_refusal("hexa adr gates", &o);

        // hexa loop stage done
        let p = project("`true`");
        let home = tempfile::tempdir().unwrap();
        let o = hexa(p.path(), home.path(), &["loop", "adr", ADR_ID]);
        assert!(o.status.success(), "loop adr failed:\n{}", text(&o));
        let o = hexa(p.path(), home.path(), &["loop", "gate", "true"]);
        assert!(o.status.success(), "loop gate failed:\n{}", text(&o));
        let o = hexa(p.path(), home.path(), &["loop", "stage", "done"]);
        assert_unrecognised_refusal("hexa loop stage done", &o);
    }

    /// S31: Closing a stage with no recorded gate fails
    #[test]
    fn s31_closing_stage_without_gate_fails() {
        let p = project("`cargo test`");
        let home = tempfile::tempdir().unwrap();
        let o = hexa(p.path(), home.path(), &["loop", "adr", ADR_ID]);
        assert!(o.status.success(), "loop adr failed:\n{}", text(&o));
        let o = hexa(p.path(), home.path(), &["loop", "stage", "done"]);
        let t = text(&o);
        assert!(
            !o.status.success(),
            "loop stage done succeeded with no gate:\n{t}"
        );
        assert!(
            t.to_lowercase().contains("no gate"),
            "output does not say that no gate is recorded:\n{t}"
        );
    }
}

mod part_4 {
    #![allow(dead_code, unused_imports)]

    use hexa_core::domain::evidence::{read_evidence, Evidence};
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    const CARGO_EMPTY: &str = "running 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n";
    const CARGO_ONE: &str = "running 1 test\ntest tests::it_works ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n";

    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn init_repo(dir: &Path) {
        git(dir, &["init", "-q"]);
        git(dir, &["config", "user.name", "t"]);
        git(dir, &["config", "user.email", "t@example.com"]);
        git(dir, &["config", "commit.gpgsign", "false"]);
    }

    fn commit_all(dir: &Path, msg: &str) {
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "-m", msg]);
    }

    fn hexa(dir: &Path, home: &Path, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_hexa"))
            .args(args)
            .current_dir(dir)
            .env("HOME", home)
            .env("NO_COLOR", "1")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .output()
            .expect("hexa binary runs")
    }

    fn text(o: &Output) -> String {
        format!(
            "{}\n{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        )
    }

    fn write(dir: &Path, rel: &str, body: &str) -> PathBuf {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&p, body).unwrap();
        p
    }

    fn shell_quote(s: &str) -> String {
        format!("'{}'", s.replace('\'', "'\\''"))
    }

    fn rust_project(dir: &Path) {
        init_repo(dir);
        write(
            dir,
            "Cargo.toml",
            "[package]\nname = \"cov\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        write(
        dir,
        "src/lib.rs",
        "pub fn add(a: i32, b: i32) -> i32 {\n    if a > 0 {\n        a + b\n    } else {\n        b + a\n    }\n}\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn it_works() {\n        assert_eq!(super::add(1, 2), 3);\n    }\n}\n",
    );
        write(dir, ".gitignore", "target/\n");
        commit_all(dir, "init");
    }

    fn sample_outputs() -> Vec<String> {
        vec![
            String::new(),
            "build finished".to_string(),
            CARGO_EMPTY.to_string(),
            CARGO_ONE.to_string(),
            "test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out\n"
                .to_string(),
            "ok  \tgithub.com/x/y\t0.012s\n".to_string(),
            "--- FAIL: TestX (0.00s)\nFAIL\tgithub.com/x/y\t0.01s\n".to_string(),
            "TAP version 13\n# tests 3\n# pass 3\n# fail 0\n".to_string(),
            "TAP version 13\n# tests 0\n# pass 0\n# fail 0\n".to_string(),
            "ℹ tests 4\nℹ suites 0\nℹ pass 4\nℹ fail 0\n".to_string(),
            "ℹ tests 4\nℹ pass 3\nℹ fail 1\n".to_string(),
            " 5 pass\n 0 fail\nRan 5 tests across 1 files.\n".to_string(),
            "12 passed · 0 failed\n".to_string(),
            "3 passed · 2 failed\n".to_string(),
            "lint clean\nwarning: something\n".to_string(),
            "ℹ tests 0\nℹ pass 0\nℹ fail 0\n".to_string(),
        ]
    }

    /// S32: A vacuous gate fails as measured nothing
    #[test]
    fn s32_vacuous_gate_fails_as_measured_nothing() {
        let ev = read_evidence(true, CARGO_EMPTY, false);
        assert_eq!(ev, Evidence::Vacuous);
        assert!(!ev.passes());

        let repo = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let dir = repo.path();
        init_repo(dir);
        write(
            dir,
            "docs/adrs/ADR-2611081151.md",
            "# ADR-2611081151: test\n\n**Status:** Accepted\n",
        );
        commit_all(dir, "init");

        let _ = hexa(dir, home.path(), &["loop", "adr", "ADR-2611081151"]);
        let gate = format!("printf {}", shell_quote(CARGO_EMPTY));
        let g = hexa(dir, home.path(), &["loop", "gate", &gate]);
        assert!(
            g.status.success(),
            "recording the gate failed: {}",
            text(&g)
        );

        let done = hexa(dir, home.path(), &["loop", "stage", "done"]);
        let out = text(&done);
        assert!(
            !done.status.success(),
            "stage done passed a vacuous gate: {out}"
        );
        assert!(
            out.to_lowercase().contains("measured nothing"),
            "failure does not say the gate measured nothing: {out}"
        );
    }

    /// S33: passes is true exactly for Passed and Uncounted
    #[test]
    fn s33_passes_exactly_for_passed_and_uncounted() {
        let cases = vec![
            (Evidence::Passed { tests: 1 }, true),
            (Evidence::Passed { tests: 0 }, true),
            (Evidence::Passed { tests: u64::MAX }, true),
            (Evidence::Uncounted, true),
            (
                Evidence::Failed {
                    reason: String::new(),
                },
                false,
            ),
            (
                Evidence::Failed {
                    reason: "1 failed".into(),
                },
                false,
            ),
            (Evidence::Vacuous, false),
            (Evidence::Unrecognised, false),
        ];
        for (e, expected) in cases {
            assert_eq!(e.passes(), expected, "passes() wrong for {e:?}");
        }
    }

    /// S34: A declared uncounted gate that exits 0 is Uncounted
    #[test]
    fn s34_declared_uncounted_exit_zero_is_uncounted() {
        assert_eq!(
            read_evidence(true, "build finished", true),
            Evidence::Uncounted
        );
    }

    /// S35: Uncounted results are labelled wherever they are reported
    #[test]
    fn s35_uncounted_labelled_in_report_and_commit() {
        let repo = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let dir = repo.path();
        init_repo(dir);
        write(dir, "notes.txt", "hello\n");
        commit_all(dir, "init");
        let head_before = String::from_utf8(
            Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(dir)
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap();

        // No model is reachable in a test: a completion command stands in for
        // one on the single-shot path, replying with the edit to make.
        let stub = home.path().join("stub-model.sh");
        std::fs::write(&stub, "#!/bin/sh\ncat >/dev/null\nprintf 'MODE: append\\n```\\nworld\\n```\\n'\n").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let run = Command::new(env!("CARGO_BIN_EXE_hexa"))
            .args([
                "do",
                "run",
                "Append the line 'world' to notes.txt",
                "--file",
                "notes.txt",
                "--evidence",
                "true",
                "--uncounted",
                "--fast",
                "--model",
                "stub",
            ])
            .current_dir(dir)
            .env("HOME", home.path())
            .env("NO_COLOR", "1")
            .env("HEXA_COMPLETION_COMMAND", &stub)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .output()
            .expect("hexa binary runs");
        let out = text(&run);
        assert!(run.status.success(), "hexa do run failed: {out}");
        let report_has = out.lines().any(|l| l.contains("uncounted"));
        assert!(report_has, "no report line carries 'uncounted': {out}");

        let log = Command::new("git")
            .args(["log", "-1", "--format=%H%n%B"])
            .current_dir(dir)
            .output()
            .unwrap();
        let log = String::from_utf8_lossy(&log.stdout).to_string();
        assert!(
            !log.starts_with(head_before.trim()),
            "hexa do wrote no commit: {out}"
        );
        assert!(
            log.contains("uncounted"),
            "commit message does not carry 'uncounted': {log}"
        );
    }

    /// S36: A declared uncounted gate that exits non-zero fails
    #[test]
    fn s36_declared_uncounted_nonzero_fails() {
        for output in ["build finished", "", CARGO_ONE] {
            let ev = read_evidence(false, output, true);
            assert!(
                matches!(ev, Evidence::Failed { .. }),
                "expected Failed for {output:?}, got {ev:?}"
            );
        }
    }

    /// S37: Unrecognised output is never treated as uncounted without the declaration
    #[test]
    fn s37_unrecognised_without_declaration() {
        let ev = read_evidence(true, "build finished", false);
        assert_eq!(ev, Evidence::Unrecognised);
        assert_ne!(ev, Evidence::Uncounted);
        assert!(!ev.passes());
    }

    /// S38: Uncounted is produced only by the declaration
    #[test]
    fn s38_uncounted_only_by_declaration() {
        for output in sample_outputs() {
            for exit_ok in [true, false] {
                let ev = read_evidence(exit_ok, &output, false);
                assert_ne!(
                    ev,
                    Evidence::Uncounted,
                    "Uncounted without declaration for exit_ok={exit_ok}, output={output:?}"
                );
            }
        }
    }

    /// S39: For a declared uncounted gate, the exit status alone decides
    #[test]
    fn s39_uncounted_exit_status_alone_decides() {
        for output in sample_outputs() {
            for exit_ok in [true, false] {
                let ev = read_evidence(exit_ok, &output, true);
                assert_eq!(
                    ev.passes(),
                    exit_ok,
                    "exit_ok={exit_ok} output={output:?} gave {ev:?}"
                );
            }
        }
    }

    /// S40: Coverage is reported from a gate that passes on the intact tree
    #[test]
    fn s40_coverage_from_gate_passing_on_intact_tree() {
        let repo = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let logdir = tempfile::tempdir().unwrap();
        let dir = repo.path();
        rust_project(dir);

        let original = std::fs::read(dir.join("src/lib.rs")).unwrap();
        let log = logdir.path().join("gate.log");
        let gate = format!(
            "cksum src/lib.rs >> {}; printf {}",
            shell_quote(&log.to_string_lossy()),
            shell_quote(CARGO_ONE)
        );
        let intact = Command::new("sh")
            .arg("-c")
            .arg("cksum src/lib.rs")
            .current_dir(dir)
            .output()
            .unwrap();
        let intact = String::from_utf8_lossy(&intact.stdout).trim().to_string();

        let run = hexa(dir, home.path(), &["gate", "coverage", "--gate", &gate]);
        let out = text(&run);
        assert!(
            run.status.success(),
            "gate coverage failed on a passing gate: {out}"
        );
        assert!(
            out.to_lowercase().contains("coverage"),
            "no coverage reported: {out}"
        );

        let logged = std::fs::read_to_string(&log).unwrap_or_default();
        let first = logged.lines().next().unwrap_or("").trim().to_string();
        assert_eq!(
            first, intact,
            "the gate's first run was not on the unmodified tree; log:\n{logged}"
        );
        assert_eq!(
            std::fs::read(dir.join("src/lib.rs")).unwrap(),
            original,
            "coverage left the tree modified"
        );
    }

    /// S41: A failing gate yields no coverage
    #[test]
    fn s41_failing_gate_yields_no_coverage() {
        let repo = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let dir = repo.path();
        rust_project(dir);

        let run = hexa(dir, home.path(), &["gate", "coverage", "--gate", "false"]);
        let out = text(&run);
        assert!(
            !run.status.success(),
            "gate coverage exited 0 for `false`: {out}"
        );

        let lower = out.to_lowercase();
        let holes = regex::Regex::new(r"(?i)\d+\s*(holes?|uncovered|survived|survivors?)").unwrap();
        assert!(!holes.is_match(&out), "a hole count was reported: {out}");
        let pct = regex::Regex::new(r"\d+(\.\d+)?\s*%").unwrap();
        assert!(!pct.is_match(&out), "a coverage figure was reported: {out}");
        assert!(
            !lower.contains("full coverage") && !lower.contains("fully covered"),
            "reported full coverage: {out}"
        );
        let reason =
            regex::Regex::new(r"(?i)fail|did not pass|not pass|exited|exit status|non-zero")
                .unwrap();
        assert!(reason.is_match(&out), "no reason given: {out}");
    }
}

mod part_5 {
    #![allow(dead_code)]

    use std::path::Path;
    use std::process::{Command, Output};

    /// A small git repository with one committed source file, so `hexa gate
    /// coverage` has an unmodified tree to run the gate on.
    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
        root.join("src/lib.rs"),
        "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n\npub fn sub(a: i32, b: i32) -> i32 {\n    a - b\n}\n",
    )
    .unwrap();
        std::fs::create_dir_all(root.join(".hexa")).unwrap();
        std::fs::write(
            root.join(".hexa/project.json"),
            "{\"name\":\"coverage-fixture\"}\n",
        )
        .unwrap();
        git(root, &["init", "-q"]);
        git(root, &["add", "-A"]);
        git(
            root,
            &[
                "-c",
                "user.name=hexa-test",
                "-c",
                "user.email=hexa-test@example.invalid",
                "commit",
                "-q",
                "-m",
                "fixture",
            ],
        );
        dir
    }

    fn git(root: &Path, args: &[&str]) {
        let st = Command::new("git")
            .args(args)
            .current_dir(root)
            .status()
            .expect("git runs");
        assert!(st.success(), "git {:?} failed", args);
    }

    fn hexa(root: &Path, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_hexa"))
            .args(args)
            .current_dir(root)
            .env("HOME", root)
            .env("NO_COLOR", "1")
            .output()
            .expect("hexa runs")
    }

    fn text(out: &Output) -> String {
        format!(
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    }

    fn names_baseline_reason(lower: &str) -> bool {
        [
            "unrecognised",
            "unrecognized",
            "vacuous",
            "measured nothing",
            "zero tests",
            "0 tests",
            "runners",
        ]
        .iter()
        .any(|w| lower.contains(w))
    }

    fn reports_coverage_figure(lower: &str) -> bool {
        lower.contains('%')
            || lower.contains("full coverage")
            || lower.contains("holes")
            || lower.contains("hole count")
    }

    /// S42: A vacuous or unrecognised baseline yields no coverage
    #[test]
    fn s42_vacuous_or_unrecognised_baseline_yields_no_coverage() {
        let gates = [
        // Unrecognised: exits 0, no runner output at all.
        "true",
        // Vacuous: a recognised runner that reported zero tests.
        "printf 'running 0 tests\\n\\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\\n'",
    ];
        for gate in gates {
            let dir = repo();
            let out = hexa(dir.path(), &["gate", "coverage", "--gate", gate]);
            let all = text(&out);
            let lower = all.to_lowercase();
            assert!(
                !lower.contains("unexpected argument")
                    && !lower.contains("unrecognized subcommand"),
                "gate coverage must accept --gate; got:\n{all}"
            );
            assert!(
            !out.status.success(),
            "gate coverage must exit non-zero when the baseline run of `{gate}` is not Passed; got:\n{all}"
        );
            assert!(
                names_baseline_reason(&lower),
                "gate coverage must name why the baseline of `{gate}` did not pass; got:\n{all}"
            );
            assert!(
            !reports_coverage_figure(&lower),
            "gate coverage must report no coverage from a baseline that could not pass (`{gate}`); got:\n{all}"
        );
        }
    }

    /// S43: A declared uncounted gate that exits 0 on the intact tree is measured
    #[test]
    fn s43_declared_uncounted_gate_that_exits_zero_is_measured() {
        let dir = repo();
        let out = hexa(
            dir.path(),
            &["gate", "coverage", "--gate", "true", "--uncounted"],
        );
        let all = text(&out);
        let lower = all.to_lowercase();
        assert!(
            !lower.contains("unexpected argument") && !lower.contains("unrecognized subcommand"),
            "gate coverage must accept a declared uncounted gate; got:\n{all}"
        );
        assert!(
            !lower.contains("unrecognised") && !lower.contains("unrecognized"),
            "a declared uncounted gate must not be judged Unrecognised; got:\n{all}"
        );
        assert!(
            !lower.contains("vacuous") && !lower.contains("measured nothing"),
            "a declared uncounted gate must not be judged Vacuous; got:\n{all}"
        );
        assert!(
            !lower.contains("no coverage"),
            "a declared uncounted gate that exits 0 must proceed to measure coverage; got:\n{all}"
        );
        assert!(
        lower.contains("uncounted"),
        "the coverage report for an uncounted gate must carry the word `uncounted`; got:\n{all}"
    );
    }
}
