#![allow(unused, clippy::all)]
mod part_1 {
    #![allow(clippy::all)]

    use hexa_core::domain::evidence::{read_evidence, Evidence};
    use hexa_core::domain::test_runner::{judge_counted, Verdict};

    /// S01: A -v run of one passing test is one test
    #[test]
    fn s01_verbose_run_of_one_passing_test_is_one_test() {
        let output = "=== RUN   TestX\n--- PASS: TestX (0.00s)\nPASS\nok  \tdemo/pkg\t0.01s\n";
        assert_eq!(
            read_evidence(true, output, false),
            Evidence::Passed { tests: 1 }
        );
    }

    /// S02: A -v run of N passing tests in one package is N
    #[test]
    fn s02_verbose_run_of_n_passing_tests_is_n() {
        let output = "--- PASS: TestA (0.00s)\n--- PASS: TestB (0.00s)\n--- PASS: TestC (0.00s)\nPASS\nok  \tdemo/pkg\t0.02s\n";
        assert_eq!(
            read_evidence(true, output, false),
            Evidence::Passed { tests: 3 }
        );
    }

    /// S04: The tab-only form of the package line follows the same rule
    #[test]
    fn s04_tab_only_package_line_follows_same_rule() {
        let output = "--- PASS: TestX (0.00s)\nok\tdemo/pkg\t0.01s\n";
        assert_eq!(
            read_evidence(true, output, false),
            Evidence::Passed { tests: 1 }
        );
    }

    /// S05: A sealed Go test bound to a verbose configured runner is judged as one passing test
    #[test]
    fn s05_verbose_configured_runner_judged_as_one_passing_test() {
        let output = "=== RUN   TestX\n--- PASS: TestX (0.00s)\nPASS\nok  \tdemo/pkg\t0.01s\n";
        assert_eq!(judge_counted(true, output), Verdict::Passed);
    }

    /// S06: Each package is judged on its own lines since the previous package line
    #[test]
    fn s06_each_package_judged_on_its_own_lines() {
        let output =
            "--- PASS: TestA\n--- PASS: TestB\nok  \tdemo/p1\t0.01s\nok  \tdemo/p2\t0.01s\n";
        assert_eq!(
            read_evidence(true, output, false),
            Evidence::Passed { tests: 3 }
        );
    }

    /// S07: Two verbose packages each count only their listed tests
    #[test]
    fn s07_two_verbose_packages_count_only_listed_tests() {
        let output =
            "--- PASS: TestA\nok  \tdemo/p1\t0.01s\n--- PASS: TestB\nok  \tdemo/p2\t0.01s\n";
        assert_eq!(
            read_evidence(true, output, false),
            Evidence::Passed { tests: 2 }
        );
    }

    /// S08: The package line of a verbose run is not counted as another test
    #[test]
    fn s08_verbose_package_line_not_counted_as_another_test() {
        let output = "--- PASS: TestX (0.00s)\nok  \tdemo/pkg\t0.01s\n";
        let e = read_evidence(true, output, false);
        assert_ne!(e, Evidence::Passed { tests: 2 });
        assert_eq!(e, Evidence::Passed { tests: 1 });
    }

    /// S09: A verbose run of one correct test is not refused as matching two tests
    #[test]
    fn s09_verbose_run_of_one_test_not_ambiguous() {
        let output = "--- PASS: TestX (0.00s)\nPASS\nok  \tdemo/pkg\t0.01s\n";
        let v = judge_counted(true, output);
        assert!(!matches!(v, Verdict::Ambiguous(_)), "got {:?}", v);
        assert_eq!(v, Verdict::Passed);
    }

    /// S10: Lines listed for one package do not stop a later unlisted package line from counting
    #[test]
    fn s10_listed_lines_do_not_stop_later_unlisted_package_counting() {
        let output = "--- PASS: TestA\nok  \tdemo/p1\t0.01s\nok  \tdemo/p2\t0.01s\n";
        let e = read_evidence(true, output, false);
        assert_ne!(e, Evidence::Passed { tests: 1 });
        assert_eq!(e, Evidence::Passed { tests: 2 });
    }
}

mod part_2 {
    #![allow(clippy::all)]

    use hexa_core::domain::evidence::{read_evidence, Evidence};

    /// Every sequence of `len` items, each an index in `0..n`.
    fn sequences(n: usize, len: usize) -> Vec<Vec<usize>> {
        let mut out: Vec<Vec<usize>> = vec![Vec::new()];
        for _ in 0..len {
            let mut next = Vec::new();
            for s in &out {
                for i in 0..n {
                    let mut t = s.clone();
                    t.push(i);
                    next.push(t);
                }
            }
            out = next;
        }
        out
    }

    /// S11: Over all sequences of passing Go packages, each listed test counts once and each unlisted package line counts once
    #[test]
    fn s11_each_listed_test_counts_once_and_each_unlisted_package_line_once() {
        for packages in 1..=3usize {
            for ks in sequences(4, packages) {
                let mut output = String::new();
                let mut expected: u64 = 0;
                for (p, &k) in ks.iter().enumerate() {
                    for i in 0..k {
                        output.push_str(&format!("--- PASS: TestP{p}N{i} (0.00s)\n"));
                    }
                    output.push_str(&format!("ok  \tdemo/p{p}\t0.01s\n"));
                    expected += if k > 0 { k as u64 } else { 1 };
                }
                assert_eq!(
                    read_evidence(true, &output, false),
                    Evidence::Passed { tests: expected },
                    "packages with test counts {ks:?}, output:\n{output}"
                );
            }
        }
    }
}
