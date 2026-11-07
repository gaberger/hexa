#![allow(unused, clippy::all)]
mod part_1 {
    #![allow(dead_code)]

    use std::fs;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output, Stdio};

    use sha2::{Digest, Sha256};

    const ADR: &str = "ADR-2601010000";

    const ORIGINAL_A: &str = "/// S01: first fixture scenario\n#[test]\nfn s01_first_fixture() {\n    let fixture = vec![1, 2, 3];\n    assert_eq!(fixture.len(), 3);\n}\n";
    const EDITED_A: &str = "/// S01: first fixture scenario\n#[test]\nfn s01_first_fixture() {\n    let fixture = vec![1, 2, 3, 4];\n    assert_eq!(fixture.len(), 4);\n}\n";
    const ORIGINAL_B: &str = "/// S02: second fixture scenario\n#[test]\nfn s02_second_fixture() {\n    let fixture = vec![7, 8, 9];\n    assert_eq!(fixture.len(), 3);\n}\n";
    const EDITED_B: &str = "/// S02: second fixture scenario\n#[test]\nfn s02_second_fixture() {\n    let fixture = vec![7, 8, 9, 10];\n    assert_eq!(fixture.len(), 4);\n}\n";

    fn sha256_hex(bytes: &[u8]) -> String {
        let digest = Sha256::digest(bytes);
        digest.iter().map(|b| format!("{:02x}", b)).collect()
    }

    fn combined(out: &Output) -> String {
        format!(
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    }

    fn confirm_hashes(text: &str) -> Vec<String> {
        let re = regex::Regex::new(r"--confirm\s+([0-9a-fA-F]{64})").unwrap();
        re.captures_iter(text)
            .map(|c| c[1].to_lowercase())
            .collect()
    }

    struct Repo {
        dir: tempfile::TempDir,
    }

    impl Repo {
        fn path(&self) -> &Path {
            self.dir.path()
        }

        fn git(&self, args: &[&str]) -> Output {
            let out = Command::new("git")
                .args(args)
                .current_dir(self.path())
                .env("HOME", self.path())
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .output()
                .expect("git runs");
            assert!(
                out.status.success(),
                "git {:?} failed: {}",
                args,
                combined(&out)
            );
            out
        }

        fn hexa_with_stdin(&self, args: &[&str], stdin: &str) -> Output {
            let mut child = Command::new(env!("CARGO_BIN_EXE_hexa"))
                .args(args)
                .current_dir(self.path())
                .env("HOME", self.path())
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("NO_COLOR", "1")
                .env("CLICOLOR", "0")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("hexa runs");
            {
                let mut handle = child.stdin.take().unwrap();
                let _ = handle.write_all(stdin.as_bytes());
            }
            child.wait_with_output().expect("hexa finishes")
        }

        fn hexa(&self, args: &[&str]) -> Output {
            self.hexa_with_stdin(args, "")
        }

        fn write(&self, rel: &str, content: &str) {
            let p = self.path().join(rel);
            if let Some(parent) = p.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(p, content).unwrap();
        }

        fn file_hash(&self, rel: &str) -> String {
            sha256_hex(&fs::read(self.path().join(rel)).unwrap())
        }

        fn commit_all(&self, msg: &str) {
            self.git(&["add", "-A"]);
            self.git(&["commit", "-q", "--allow-empty", "-m", msg]);
        }

        fn spec_path(&self) -> PathBuf {
            fn walk(dir: &Path, found: &mut Vec<PathBuf>) {
                for entry in fs::read_dir(dir).unwrap().flatten() {
                    let p = entry.path();
                    if p.file_name().map(|n| n == ".git").unwrap_or(false) {
                        continue;
                    }
                    if p.is_dir() {
                        walk(&p, found);
                    } else if p.extension().map(|e| e == "json").unwrap_or(false) {
                        found.push(p);
                    }
                }
            }
            let mut found = Vec::new();
            walk(self.path(), &mut found);
            let mut candidates: Vec<PathBuf> = found
                .into_iter()
                .filter(|p| {
                    let s = fs::read_to_string(p).unwrap_or_default();
                    s.contains("scenarios") && (s.contains(ADR) || s.contains("2601010000"))
                })
                .collect();
            candidates.sort();
            // Prefer the one carrying a seal.
            if let Some(p) = candidates
                .iter()
                .find(|p| fs::read_to_string(p).unwrap_or_default().contains("sealed"))
            {
                return p.clone();
            }
            candidates
                .into_iter()
                .next()
                .expect("a spec JSON for the fixture ADR exists in the repository")
        }

        fn spec_bytes(&self) -> Vec<u8> {
            fs::read(self.spec_path()).unwrap()
        }

        fn spec_json(&self) -> serde_json::Value {
            serde_json::from_slice(&self.spec_bytes()).expect("spec JSON parses")
        }
    }

    fn sealed_hash(spec: &serde_json::Value, file: &str) -> Option<String> {
        let sealed = spec.get("sealed")?.as_object()?;
        for (k, v) in sealed {
            if k == file || k.ends_with(&format!("/{}", file)) || file.ends_with(k.as_str()) {
                return v.as_str().map(|s| s.to_lowercase());
            }
        }
        None
    }

    fn amendments(spec: &serde_json::Value) -> Vec<serde_json::Value> {
        spec.get("amendments")
            .and_then(|a| a.as_array())
            .cloned()
            .unwrap_or_default()
    }

    fn file_matches(value: &serde_json::Value, file: &str) -> bool {
        value
            .as_str()
            .map(|s| s == file || s.ends_with(&format!("/{}", file)))
            .unwrap_or(false)
    }

    /// Build a temporary git repository with an accepted spec whose given files are sealed.
    /// If `sealed_in_history` is false, the sealed content of the files is never committed.
    fn setup(files: &[(&str, &str, &str, &str)], sealed_in_history: bool) -> Repo {
        // (path, scenario id, test fn, content)
        let repo = Repo {
            dir: tempfile::tempdir().unwrap(),
        };
        repo.git(&["init", "-q"]);
        repo.git(&["config", "user.name", "Ada"]);
        repo.git(&["config", "user.email", "ada@example.com"]);
        repo.git(&["config", "commit.gpgsign", "false"]);

        let adr_md = format!(
        "# {}: Fixture decision\n\n**Status:** Accepted\n**Date:** 2026-01-01\n\n## Context\n\nFixture.\n\n## Decision\n\n1. **Fixture.** Something holds.\n\n## Consequences\n\n- None.\n",
        ADR
    );
        repo.write(&format!("docs/adr/{}-fixture-decision.md", ADR), &adr_md);

        let scenarios: Vec<serde_json::Value> = files
            .iter()
            .map(|(path, id, func, _)| {
                serde_json::json!({
                    "id": id,
                    "decision": ["§1"],
                    "kind": "example",
                    "description": format!("fixture scenario {}", id),
                    "given": "a fixture",
                    "when": "the test runs",
                    "then": "it holds",
                    "test": func,
                    "file": path,
                })
            })
            .collect();
        let spec = serde_json::json!({
            "adr": ADR,
            "scenarios": scenarios,
        });
        repo.write(
            &format!("docs/adr/{}.spec.json", ADR),
            &serde_json::to_string_pretty(&spec).unwrap(),
        );

        if sealed_in_history {
            for (path, _, _, content) in files {
                repo.write(path, content);
            }
            repo.commit_all("fixture tests");
        } else {
            for (path, _, _, _) in files {
                repo.write(path, "// placeholder, never the sealed version\n");
            }
            repo.commit_all("placeholder tests");
            for (path, _, _, content) in files {
                repo.write(path, content);
            }
        }

        let accept = repo.hexa_with_stdin(&["spec", "accept", ADR], "y\ny\n");
        if !accept.status.success() {
            let accept2 = repo.hexa_with_stdin(&["spec", "accept", ADR, "--yes"], "y\n");
            assert!(
                accept2.status.success(),
                "setup: accept failed: {}\n{}",
                combined(&accept),
                combined(&accept2)
            );
        }

        let mut seal_args: Vec<&str> = vec!["spec", "seal", ADR];
        for (path, _, _, _) in files {
            seal_args.push(path);
        }
        let seal = repo.hexa_with_stdin(&seal_args, "y\n");
        if !seal.status.success() {
            let seal2 = repo.hexa_with_stdin(&["spec", "seal", ADR], "y\n");
            assert!(
                seal2.status.success(),
                "setup: seal failed: {}\n{}",
                combined(&seal),
                combined(&seal2)
            );
        }

        let spec = repo.spec_json();
        for (path, _, _, _) in files {
            assert_eq!(
                sealed_hash(&spec, path),
                Some(repo.file_hash(path)),
                "setup: {} is sealed at its current content",
                path
            );
        }

        if sealed_in_history {
            repo.commit_all("accept and seal");
        } else {
            let spec_path = repo.spec_path();
            let rel = spec_path
                .strip_prefix(repo.path())
                .unwrap()
                .to_string_lossy()
                .to_string();
            repo.git(&["add", "-A", "docs"]);
            repo.git(&["add", &rel]);
            repo.git(&["commit", "-q", "-m", "accept and seal (spec only)"]);
        }
        repo
    }

    fn one_file_repo(sealed_in_history: bool) -> Repo {
        setup(
            &[("tests/a.rs", "S01", "s01_first_fixture", ORIGINAL_A)],
            sealed_in_history,
        )
    }

    fn two_file_repo() -> Repo {
        setup(
            &[
                ("tests/a.rs", "S01", "s01_first_fixture", ORIGINAL_A),
                ("tests/b.rs", "S02", "s02_second_fixture", ORIGINAL_B),
            ],
            true,
        )
    }

    fn assert_diff_of_a(text: &str) {
        assert!(
            text.contains("@@"),
            "unified diff hunk header present:\n{}",
            text
        );
        assert!(
            text.contains("-    let fixture = vec![1, 2, 3];"),
            "diff shows the removed sealed line:\n{}",
            text
        );
        assert!(
            text.contains("+    let fixture = vec![1, 2, 3, 4];"),
            "diff shows the added current line:\n{}",
            text
        );
        assert!(
            text.contains("tests/a.rs"),
            "diff names tests/a.rs:\n{}",
            text
        );
    }

    /// S01: The amend verb exists and handles a fixture-only change to a sealed file of an accepted spec.
    #[test]
    fn s01_amend_verb_exists_and_handles_fixture_change() {
        let repo = one_file_repo(true);
        repo.write("tests/a.rs", EDITED_A);
        let out = repo.hexa(&["spec", "amend", ADR, "--reason", "schema change"]);
        let text = combined(&out);
        let lower = text.to_lowercase();
        assert!(
            !lower.contains("unrecognized subcommand") && !lower.contains("unknown command"),
            "amend is recognised:\n{}",
            text
        );
        assert!(
            !text.contains("not by re-sealing"),
            "amend does not refuse as a re-seal:\n{}",
            text
        );
        assert_diff_of_a(&text);
    }

    /// S02: The amend verb requires a --reason argument.
    #[test]
    fn s02_amend_requires_reason() {
        let repo = one_file_repo(true);
        repo.write("tests/a.rs", EDITED_A);
        let before = repo.spec_bytes();
        let out = repo.hexa(&["spec", "amend", ADR]);
        let text = combined(&out);
        assert!(
            !out.status.success(),
            "amend without --reason is refused:\n{}",
            text
        );
        assert!(
            text.contains("--reason"),
            "message names --reason:\n{}",
            text
        );
        assert!(
            !text.to_lowercase().contains("unrecognized subcommand"),
            "amend is a known verb:\n{}",
            text
        );
        assert_eq!(repo.spec_bytes(), before, "spec JSON is byte-identical");
    }

    /// S03: The first call prints a diff and the confirm command, and writes nothing.
    #[test]
    fn s03_first_call_prints_diff_and_confirm_command() {
        let repo = one_file_repo(true);
        repo.write("tests/a.rs", EDITED_A);
        let before = repo.spec_bytes();
        let out = repo.hexa(&["spec", "amend", ADR, "--reason", "fixtures"]);
        let text = combined(&out);
        assert_diff_of_a(&text);
        let current = repo.file_hash("tests/a.rs");
        let hashes = confirm_hashes(&text);
        assert!(
            hashes.contains(&current),
            "confirm command with the current hash {} is shown:\n{}",
            current,
            text
        );
        let diff_pos = text.find("+    let fixture = vec![1, 2, 3, 4];").unwrap();
        let confirm_pos = text.find(&current).unwrap();
        assert!(
            confirm_pos > diff_pos,
            "confirm command follows the diff:\n{}",
            text
        );
        assert_eq!(repo.spec_bytes(), before, "spec JSON is byte-identical");
    }

    /// S04: Amend refuses when the sealed version is not in git history.
    #[test]
    fn s04_refuses_when_sealed_version_not_in_history() {
        let repo = one_file_repo(false);
        repo.write("tests/a.rs", EDITED_A);
        let current = repo.file_hash("tests/a.rs");
        let before = repo.spec_bytes();

        let without = repo.hexa(&["spec", "amend", ADR, "--reason", "fixtures"]);
        let with = repo.hexa(&[
            "spec",
            "amend",
            ADR,
            "--reason",
            "fixtures",
            "--confirm",
            &current,
        ]);
        for out in [&without, &with] {
            let text = combined(out);
            assert!(!out.status.success(), "amend is refused:\n{}", text);
            assert!(
                text.contains(
                    "the sealed version is not in git history, so the edit cannot be shown"
                ),
                "refusal message present:\n{}",
                text
            );
            assert!(
                confirm_hashes(&text).is_empty(),
                "no confirmable command offered:\n{}",
                text
            );
            assert_eq!(repo.spec_bytes(), before, "spec JSON is byte-identical");
        }
        assert!(amendments(&repo.spec_json()).is_empty());
    }

    /// S05: A call without --confirm never writes to the spec JSON.
    #[test]
    fn s05_call_without_confirm_never_writes() {
        let mut checked = 0;

        // One changed file, and nothing changed.
        let repo = one_file_repo(true);
        {
            let before = repo.spec_bytes();
            let out = repo.hexa(&["spec", "amend", ADR, "--reason", "r"]);
            assert_eq!(repo.spec_bytes(), before, "unchanged file: no write");
            assert!(confirm_hashes(&combined(&out)).is_empty());
            checked += 1;
        }
        repo.write("tests/a.rs", EDITED_A);
        for args in [
            vec!["spec", "amend", ADR, "--reason", "r"],
            vec!["spec", "amend", ADR, "--reason", ""],
            vec!["spec", "amend", ADR],
            vec![
                "spec",
                "amend",
                ADR,
                "--reason",
                "r",
                "--file",
                "tests/a.rs",
            ],
            vec![
                "spec",
                "amend",
                ADR,
                "--reason",
                "r",
                "--file",
                "tests/zzz.rs",
            ],
        ] {
            let before = repo.spec_bytes();
            let out = repo.hexa(&args);
            let text = combined(&out);
            assert_eq!(repo.spec_bytes(), before, "no write for {:?}", args);
            let current = repo.file_hash("tests/a.rs");
            for h in confirm_hashes(&text) {
                assert_eq!(
                    h, current,
                    "printed hash is the current hash of the diffed file"
                );
            }
            checked += 1;
        }

        // Two changed files.
        let repo2 = two_file_repo();
        repo2.write("tests/a.rs", EDITED_A);
        repo2.write("tests/b.rs", EDITED_B);
        for (args, file) in [
            (vec!["spec", "amend", ADR, "--reason", "r"], None),
            (
                vec![
                    "spec",
                    "amend",
                    ADR,
                    "--reason",
                    "r",
                    "--file",
                    "tests/a.rs",
                ],
                Some("tests/a.rs"),
            ),
            (
                vec![
                    "spec",
                    "amend",
                    ADR,
                    "--reason",
                    "r",
                    "--file",
                    "tests/b.rs",
                ],
                Some("tests/b.rs"),
            ),
        ] {
            let before = repo2.spec_bytes();
            let out = repo2.hexa(&args);
            let text = combined(&out);
            assert_eq!(repo2.spec_bytes(), before, "no write for {:?}", args);
            let hashes = confirm_hashes(&text);
            match file {
                None => assert!(hashes.is_empty(), "no confirm without --file:\n{}", text),
                Some(f) => {
                    let current = repo2.file_hash(f);
                    assert!(
                        !hashes.is_empty(),
                        "a confirm command is shown for {}:\n{}",
                        f,
                        text
                    );
                    for h in hashes {
                        assert_eq!(h, current, "printed hash is that of {}", f);
                    }
                }
            }
            checked += 1;
        }
        assert!(checked > 0, "checked no cases");
    }

    /// S06: With two differing files, --file selects one, and only that file is diffed and amended.
    #[test]
    fn s06_file_selects_one_of_two() {
        let repo = two_file_repo();
        let sealed_b_before = sealed_hash(&repo.spec_json(), "tests/b.rs").unwrap();
        repo.write("tests/a.rs", EDITED_A);
        repo.write("tests/b.rs", EDITED_B);

        let first = repo.hexa(&[
            "spec",
            "amend",
            ADR,
            "--file",
            "tests/a.rs",
            "--reason",
            "r",
        ]);
        let text = combined(&first);
        assert_diff_of_a(&text);
        assert!(
            !text.contains("vec![7, 8, 9"),
            "diff does not cover tests/b.rs:\n{}",
            text
        );

        let hash_a = repo.file_hash("tests/a.rs");
        let confirm = repo.hexa(&[
            "spec",
            "amend",
            ADR,
            "--file",
            "tests/a.rs",
            "--reason",
            "r",
            "--confirm",
            &hash_a,
        ]);
        assert!(
            confirm.status.success(),
            "confirm succeeds:\n{}",
            combined(&confirm)
        );

        let spec = repo.spec_json();
        let am = amendments(&spec);
        assert_eq!(am.len(), 1, "one amendment entry");
        assert!(
            file_matches(&am[0]["file"], "tests/a.rs"),
            "entry is for tests/a.rs"
        );
        assert_eq!(sealed_hash(&spec, "tests/a.rs"), Some(hash_a));
        assert_eq!(sealed_hash(&spec, "tests/b.rs"), Some(sealed_b_before));

        let verify = repo.hexa(&["spec", "verify", ADR]);
        let vtext = combined(&verify);
        let line = vtext
            .lines()
            .find(|l| l.contains("tests/b.rs") && l.contains("changed since sealing"));
        assert!(
            line.is_some()
                || (vtext.contains("tests/b.rs") && vtext.contains("changed since sealing")),
            "verify reports tests/b.rs as changed since sealing:\n{}",
            vtext
        );
    }

    /// S07: Amend refuses when several files differ and --file is not given.
    #[test]
    fn s07_several_differ_requires_file() {
        let repo = two_file_repo();
        repo.write("tests/a.rs", EDITED_A);
        repo.write("tests/b.rs", EDITED_B);
        let hash_a = repo.file_hash("tests/a.rs");
        let before = repo.spec_bytes();
        for args in [
            vec!["spec", "amend", ADR, "--reason", "r"],
            vec![
                "spec",
                "amend",
                ADR,
                "--reason",
                "r",
                "--confirm",
                hash_a.as_str(),
            ],
        ] {
            let out = repo.hexa(&args);
            let text = combined(&out);
            assert!(!out.status.success(), "refused for {:?}:\n{}", args, text);
            assert!(text.contains("tests/a.rs"), "names tests/a.rs:\n{}", text);
            assert!(text.contains("tests/b.rs"), "names tests/b.rs:\n{}", text);
            assert!(
                text.contains("--file"),
                "says --file is required:\n{}",
                text
            );
            assert!(
                !text.to_lowercase().contains("unrecognized subcommand"),
                "amend is known:\n{}",
                text
            );
            assert_eq!(repo.spec_bytes(), before, "spec JSON is byte-identical");
        }
    }

    /// S08: --file is optional when exactly one file differs.
    #[test]
    fn s08_file_optional_with_one_differing() {
        let repo = two_file_repo();
        repo.write("tests/a.rs", EDITED_A);
        let out = repo.hexa(&["spec", "amend", ADR, "--reason", "r"]);
        let text = combined(&out);
        assert_diff_of_a(&text);
        let current = repo.file_hash("tests/a.rs");
        assert!(
            confirm_hashes(&text).contains(&current),
            "confirm command for the current hash of tests/a.rs:\n{}",
            text
        );
        let diff_pos = text.find("+    let fixture = vec![1, 2, 3, 4];").unwrap();
        assert!(
            text.rfind(&current).unwrap() > diff_pos,
            "confirm follows diff"
        );
    }

    /// S09: Each successful confirmation amends exactly one file.
    #[test]
    fn s09_each_confirmation_amends_one_file() {
        let repo = two_file_repo();
        repo.write("tests/a.rs", EDITED_A);
        repo.write("tests/b.rs", EDITED_B);
        let mut checked = 0;

        for file in ["tests/b.rs", "tests/a.rs"] {
            let before = repo.spec_json();
            let first = repo.hexa(&["spec", "amend", ADR, "--file", file, "--reason", "r"]);
            let shown = confirm_hashes(&combined(&first));
            let current = repo.file_hash(file);
            assert!(shown.contains(&current), "first call shows {}'s hash", file);

            let out = repo.hexa(&[
                "spec",
                "amend",
                ADR,
                "--file",
                file,
                "--reason",
                "r",
                "--confirm",
                &current,
            ]);
            assert!(
                out.status.success(),
                "confirm succeeds:\n{}",
                combined(&out)
            );
            let after = repo.spec_json();

            let am_before = amendments(&before);
            let am_after = amendments(&after);
            assert_eq!(
                am_after.len(),
                am_before.len() + 1,
                "exactly one entry appended"
            );
            assert_eq!(&am_after[..am_before.len()], &am_before[..], "append-only");
            let new = am_after.last().unwrap();
            assert!(file_matches(&new["file"], file), "entry refers to {}", file);

            let sb = before["sealed"].as_object().unwrap();
            let sa = after["sealed"].as_object().unwrap();
            assert_eq!(sb.len(), sa.len());
            let changed: Vec<&String> = sa
                .iter()
                .filter(|(k, v)| sb.get(*k) != Some(*v))
                .map(|(k, _)| k)
                .collect();
            assert_eq!(changed.len(), 1, "exactly one sealed value changes");
            assert!(file_matches(
                &serde_json::Value::String(changed[0].clone()),
                file
            ));
            assert_eq!(sealed_hash(&after, file), Some(current));
            checked += 1;
        }
        assert_eq!(checked, 2, "checked amendments");
    }

    /// S10: A correct --confirm appends an amendment and moves the seal.
    #[test]
    fn s10_correct_confirm_records_amendment() {
        let repo = one_file_repo(true);
        let h0 = sealed_hash(&repo.spec_json(), "tests/a.rs").unwrap();
        repo.write("tests/a.rs", EDITED_A);
        let h1 = repo.file_hash("tests/a.rs");
        assert_ne!(h0, h1);

        let out = repo.hexa(&[
            "spec",
            "amend",
            ADR,
            "--reason",
            "lifecycle/status required",
            "--confirm",
            &h1,
        ]);
        assert!(
            out.status.success(),
            "confirm succeeds:\n{}",
            combined(&out)
        );

        let spec = repo.spec_json();
        let am = amendments(&spec);
        assert_eq!(am.len(), 1, "one amendment recorded");
        let e = &am[0];
        assert!(file_matches(&e["file"], "tests/a.rs"), "file: {}", e);
        assert_eq!(
            e["from"].as_str().map(|s| s.to_lowercase()),
            Some(h0.clone())
        );
        assert_eq!(e["to"].as_str().map(|s| s.to_lowercase()), Some(h1.clone()));
        assert_eq!(e["by"].as_str(), Some("Ada"));
        assert_eq!(e["reason"].as_str(), Some("lifecycle/status required"));
        assert!(
            e["at"].as_str().map(|s| !s.is_empty()).unwrap_or(false),
            "at timestamp present: {}",
            e
        );
        assert!(
            e["base_commit"]
                .as_str()
                .map(|s| !s.is_empty())
                .unwrap_or(false),
            "base_commit present: {}",
            e
        );
        assert_eq!(sealed_hash(&spec, "tests/a.rs"), Some(h1));
    }
}

mod part_2 {
    #![allow(dead_code, clippy::all)]

    use std::path::{Path, PathBuf};
    use std::process::{Command, Output, Stdio};

    use sha2::{Digest, Sha256};

    const ADR: &str = "ADR-2611069001";
    const ADR_DOC_REL: &str = "docs/adr/ADR-2611069001-fixture.md";
    const SPEC_REL: &str = ".hexa/specs/ADR-2611069001.json";

    fn sha256_hex(bytes: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(bytes);
        h.finalize().iter().map(|b| format!("{:02x}", b)).collect()
    }

    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .args([
                "-c",
                "user.name=Fixture Tester",
                "-c",
                "user.email=tester@example.com",
            ])
            .args(args)
            .current_dir(dir)
            .stdin(Stdio::null())
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn hexa(dir: &Path, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_hexa"))
            .args(args)
            .current_dir(dir)
            .stdin(Stdio::null())
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

    /// A sealed test file whose fixture data carries a status value.
    fn test_file(fns: &[(&str, &str)], status: &str) -> String {
        let mut s = String::new();
        for (id, f) in fns {
            s.push_str(&format!(
            "/// {id}: scenario {id}\n#[test]\nfn {f}() {{\n    let status = \"{status}\";\n    assert!(!status.is_empty());\n}}\n\n"
        ));
        }
        s
    }

    struct Fx {
        dir: tempfile::TempDir,
    }

    impl Fx {
        fn root(&self) -> &Path {
            self.dir.path()
        }
        fn path(&self, rel: &str) -> PathBuf {
            self.dir.path().join(rel)
        }
        fn write(&self, rel: &str, content: &str) {
            let p = self.path(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, content).unwrap();
        }
        fn hash_of(&self, rel: &str) -> String {
            sha256_hex(&std::fs::read(self.path(rel)).unwrap())
        }
        fn spec_bytes(&self) -> Vec<u8> {
            std::fs::read(self.path(SPEC_REL)).expect("spec JSON exists")
        }
        fn spec_json(&self) -> serde_json::Value {
            serde_json::from_slice(&self.spec_bytes()).expect("spec JSON parses")
        }
        fn amendments(&self) -> Vec<serde_json::Value> {
            self.spec_json()
                .get("amendments")
                .and_then(|a| a.as_array().cloned())
                .unwrap_or_default()
        }
        fn sealed_of(&self, rel: &str) -> String {
            self.spec_json()["sealed"][rel]
                .as_str()
                .expect("sealed entry is a string")
                .to_string()
        }
        fn amend(&self, args: &[&str]) -> Output {
            let mut all = vec!["spec", "amend", ADR];
            all.extend_from_slice(args);
            hexa(self.root(), &all)
        }
    }

    /// files: (path, scenarios (id, fn) contained in that file)
    fn build(accept: bool, files: &[(&str, Vec<(&'static str, &'static str)>)]) -> Fx {
        let fx = Fx {
            dir: tempfile::tempdir().unwrap(),
        };
        git(fx.root(), &["init", "-q"]);
        git(fx.root(), &["config", "user.name", "Fixture Tester"]);
        git(fx.root(), &["config", "user.email", "tester@example.com"]);

        fx.write(
        ADR_DOC_REL,
        &format!(
            "# {ADR}: fixture decision\n\n**Status:** Accepted\n**Date:** 2026-11-06\n\n## Context\n\nFixture.\n\n## Decision\n\n1. **Fixture.** Something.\n\n## Consequences\n\n- None.\n"
        ),
    );

        let mut scenarios = Vec::new();
        for (path, fns) in files {
            fx.write(path, &test_file(fns, "active"));
            for (id, f) in fns {
                scenarios.push(serde_json::json!({
                    "id": id,
                    "decision": ["§1"],
                    "kind": "example",
                    "description": format!("Scenario {id}."),
                    "given": "A fixture.",
                    "when": "It runs.",
                    "then": "It passes.",
                    "test": f,
                }));
            }
        }
        let spec = serde_json::json!({ "adr": ADR, "scenarios": scenarios });
        fx.write(SPEC_REL, &serde_json::to_string_pretty(&spec).unwrap());
        git(fx.root(), &["add", "-A"]);
        git(fx.root(), &["commit", "-q", "-m", "fixture"]);

        if accept {
            let out = hexa(fx.root(), &["spec", "accept", ADR]);
            assert!(out.status.success(), "accept failed: {}", text(&out));
        }

        let mut spec = fx.spec_json();
        let mut sealed = serde_json::Map::new();
        for (path, _) in files {
            sealed.insert(
                path.to_string(),
                serde_json::Value::String(fx.hash_of(path)),
            );
        }
        spec["sealed"] = serde_json::Value::Object(sealed);
        fx.write(SPEC_REL, &serde_json::to_string_pretty(&spec).unwrap());
        git(fx.root(), &["add", "-A"]);
        git(fx.root(), &["commit", "-q", "-m", "seal"]);
        fx
    }

    fn a_fns() -> Vec<(&'static str, &'static str)> {
        vec![("S01", "t_s01"), ("S02", "t_s02"), ("S03", "t_s03")]
    }

    fn one_file(accept: bool) -> Fx {
        build(accept, &[("tests/a.rs", a_fns())])
    }

    fn fixture_edit(fx: &Fx, rel: &str, fns: &[(&str, &str)], status: &str) {
        fx.write(rel, &test_file(fns, status));
    }

    fn edit_scenarios(fx: &Fx) {
        let mut spec = fx.spec_json();
        spec["scenarios"][0]["then"] = serde_json::Value::String("It passes differently.".into());
        fx.write(SPEC_REL, &serde_json::to_string_pretty(&spec).unwrap());
    }

    /// S11: Amend refuses a --confirm hash that is not the file's current hash.
    #[test]
    fn s11_amend_refuses_wrong_confirm_hash() {
        let fx = one_file(true);
        fixture_edit(&fx, "tests/a.rs", &a_fns(), "planned");
        let h1 = fx.hash_of("tests/a.rs");
        let h2 = sha256_hex(b"some other content shown before a further edit");
        assert_ne!(h1, h2);
        let before = fx.spec_bytes();
        let out = fx.amend(&["--reason", "schema change", "--confirm", &h2]);
        assert!(
            !out.status.success(),
            "wrong confirm accepted: {}",
            text(&out)
        );
        assert_eq!(fx.spec_bytes(), before, "spec JSON changed on refusal");
    }

    /// S12: Amend refuses an empty reason.
    #[test]
    fn s12_amend_refuses_empty_reason() {
        let fx = one_file(true);
        fixture_edit(&fx, "tests/a.rs", &a_fns(), "planned");
        let h1 = fx.hash_of("tests/a.rs");
        let before = fx.spec_bytes();
        let out = fx.amend(&["--reason", "", "--confirm", &h1]);
        assert!(
            !out.status.success(),
            "empty reason accepted: {}",
            text(&out)
        );
        assert!(
            text(&out).to_lowercase().contains("reason"),
            "refusal does not mention the reason: {}",
            text(&out)
        );
        assert_eq!(fx.spec_bytes(), before, "spec JSON changed on refusal");
    }

    /// S13: The amendments list is append-only and records the hash that was shown.
    #[test]
    fn s13_amendments_append_only_and_record_shown_hash() {
        let b_fns: Vec<(&'static str, &'static str)> = vec![("S04", "t_s04")];
        let fx = build(
            true,
            &[("tests/a.rs", a_fns()), ("tests/b.rs", b_fns.clone())],
        );
        fixture_edit(&fx, "tests/a.rs", &a_fns(), "planned");
        fixture_edit(&fx, "tests/b.rs", &b_fns, "planned");
        let mut checked = 0usize;

        // A call whose --confirm differs from the current hash changes nothing.
        let before = fx.spec_bytes();
        let wrong = sha256_hex(b"not the file");
        let out = fx.amend(&["--file", "tests/a.rs", "--reason", "r", "--confirm", &wrong]);
        assert!(
            !out.status.success(),
            "wrong confirm accepted: {}",
            text(&out)
        );
        assert_eq!(fx.spec_bytes(), before);
        checked += 1;

        let mut step = |file: &str, fx: &Fx| {
            let prev_list = fx.amendments();
            let prev_sealed = fx.sealed_of(file);
            let current = fx.hash_of(file);
            let out = fx.amend(&[
                "--file",
                file,
                "--reason",
                "schema change",
                "--confirm",
                &current,
            ]);
            assert!(out.status.success(), "amend {file} failed: {}", text(&out));
            let after = fx.amendments();
            assert_eq!(
                after.len(),
                prev_list.len() + 1,
                "expected exactly one new entry"
            );
            assert_eq!(
                &after[..prev_list.len()],
                &prev_list[..],
                "list is not append-only"
            );
            let entry = &after[prev_list.len()];
            assert_eq!(entry["file"].as_str(), Some(file));
            assert_eq!(entry["to"].as_str(), Some(current.as_str()));
            assert_eq!(
                entry["to"].as_str().map(str::to_string),
                Some(fx.hash_of(file))
            );
            assert_eq!(entry["from"].as_str(), Some(prev_sealed.as_str()));
            assert_eq!(fx.sealed_of(file), current);
        };

        step("tests/a.rs", &fx);
        checked += 1;
        step("tests/b.rs", &fx);
        checked += 1;
        fixture_edit(&fx, "tests/a.rs", &a_fns(), "retired");
        git(fx.root(), &["add", "-A"]);
        git(fx.root(), &["commit", "-q", "-m", "amended"]);
        fixture_edit(&fx, "tests/a.rs", &a_fns(), "offline");
        step("tests/a.rs", &fx);
        checked += 1;

        assert!(checked > 0, "no amendments were checked");
    }

    /// S14: Amend refuses when the scenarios have changed since acceptance.
    #[test]
    fn s14_amend_refuses_changed_scenarios() {
        let fx = one_file(true);
        edit_scenarios(&fx);
        fixture_edit(&fx, "tests/a.rs", &a_fns(), "planned");
        let h = fx.hash_of("tests/a.rs");
        let before = fx.spec_bytes();

        for args in [
            vec!["--reason", "schema change"],
            vec!["--reason", "schema change", "--confirm", h.as_str()],
        ] {
            let out = fx.amend(&args);
            let t = text(&out).to_lowercase();
            assert!(!out.status.success(), "changed scenarios accepted: {t}");
            assert!(
                t.contains("accept") && t.contains("scenario"),
                "refusal does not point to the spec route: {t}"
            );
            assert_eq!(fx.spec_bytes(), before, "spec JSON changed on refusal");
        }
    }

    /// S15: Amend refuses an unaccepted spec.
    #[test]
    fn s15_amend_refuses_unaccepted_spec() {
        let control = one_file(true);
        fixture_edit(&control, "tests/a.rs", &a_fns(), "planned");
        let hc = control.hash_of("tests/a.rs");
        let out = control.amend(&["--reason", "schema change"]);
        let t = text(&out);
        assert!(out.status.success(), "control amend failed: {t}");
        assert!(
            t.contains("@@"),
            "control did not print a unified diff: {t}"
        );
        assert!(
            t.contains("--confirm"),
            "control did not print a confirm command: {t}"
        );
        assert!(
            t.contains(&hc),
            "confirm command does not carry the current hash: {t}"
        );

        let fx = one_file(false);
        fixture_edit(&fx, "tests/a.rs", &a_fns(), "planned");
        let before = fx.spec_bytes();
        let out = fx.amend(&["--reason", "schema change"]);
        let t = text(&out).to_lowercase();
        assert!(!out.status.success(), "unaccepted spec amended: {t}");
        assert!(
            t.contains("not accepted"),
            "message does not say not accepted: {t}"
        );
        assert_eq!(fx.spec_bytes(), before, "spec JSON changed on refusal");
    }

    /// S16: Amend refuses when a scenario's test function has been removed.
    #[test]
    fn s16_amend_refuses_removed_test_function() {
        let fx = one_file(true);
        fixture_edit(
            &fx,
            "tests/a.rs",
            &[("S01", "t_s01"), ("S02", "t_s02")],
            "planned",
        );
        let h = fx.hash_of("tests/a.rs");
        let before = fx.spec_bytes();

        for args in [
            vec!["--file", "tests/a.rs", "--reason", "schema change"],
            vec![
                "--file",
                "tests/a.rs",
                "--reason",
                "schema change",
                "--confirm",
                h.as_str(),
            ],
        ] {
            let out = fx.amend(&args);
            let t = text(&out);
            assert!(!out.status.success(), "missing test function accepted: {t}");
            assert!(t.contains("t_s03"), "refusal does not name t_s03: {t}");
            assert_eq!(fx.spec_bytes(), before, "spec JSON changed on refusal");
        }
    }

    /// S17: Amend refuses a file outside a tests/ directory.
    #[test]
    fn s17_amend_refuses_file_outside_tests_dir() {
        let fx = build(true, &[("src/a.rs", a_fns())]);
        fixture_edit(&fx, "src/a.rs", &a_fns(), "planned");
        let h = fx.hash_of("src/a.rs");
        let before = fx.spec_bytes();
        let out = fx.amend(&[
            "--file",
            "src/a.rs",
            "--reason",
            "schema change",
            "--confirm",
            &h,
        ]);
        let t = text(&out);
        assert!(!out.status.success(), "file outside tests/ amended: {t}");
        assert!(
            t.contains("tests/"),
            "refusal does not cite the tests/ rule: {t}"
        );
        assert_eq!(fx.spec_bytes(), before, "spec JSON changed on refusal");
    }

    /// S18: Amend refuses when nothing differs from the seal.
    #[test]
    fn s18_amend_refuses_when_nothing_differs() {
        let fx = one_file(true);
        let before = fx.spec_bytes();
        let out = fx.amend(&["--reason", "r"]);
        let t = text(&out).to_lowercase();
        assert!(
            !out.status.success(),
            "amend with nothing changed succeeded: {t}"
        );
        assert!(
            t.contains("nothing to amend"),
            "message lacks 'nothing to amend': {t}"
        );
        assert_eq!(fx.spec_bytes(), before, "spec JSON changed on refusal");
    }

    /// S19: Amend accepts a valid fixture edit that keeps every test function.
    #[test]
    fn s19_amend_accepts_valid_fixture_edit() {
        let fx = one_file(true);
        let old = fx.sealed_of("tests/a.rs");
        fixture_edit(&fx, "tests/a.rs", &a_fns(), "planned");
        let h = fx.hash_of("tests/a.rs");
        let out = fx.amend(&["--reason", "lifecycle/status now required", "--confirm", &h]);
        assert!(out.status.success(), "valid amend failed: {}", text(&out));
        let list = fx.amendments();
        assert_eq!(list.len(), 1, "expected one amendment: {:?}", list);
        let e = &list[0];
        assert_eq!(e["file"].as_str(), Some("tests/a.rs"));
        assert_eq!(e["from"].as_str(), Some(old.as_str()));
        assert_eq!(e["to"].as_str(), Some(h.as_str()));
        assert_eq!(e["reason"].as_str(), Some("lifecycle/status now required"));
        for k in ["by", "at", "base_commit"] {
            assert!(e.get(k).is_some(), "amendment lacks {k}: {e}");
        }
        assert_eq!(fx.sealed_of("tests/a.rs"), h);
    }

    /// S20: No refused call writes anything.
    #[test]
    fn s20_no_refused_call_writes_anything() {
        let mut checked = 0usize;
        let mut check = |fx: &Fx, file: &str| {
            let h = fx.hash_of(file);
            let before = fx.spec_bytes();
            for args in [
                vec!["--reason", "r"],
                vec!["--file", file, "--reason", "r"],
                vec!["--file", file, "--reason", "r", "--confirm", h.as_str()],
            ] {
                let out = fx.amend(&args);
                assert!(
                    !out.status.success(),
                    "refusal case succeeded with {:?}: {}",
                    args,
                    text(&out)
                );
                assert_eq!(fx.spec_bytes(), before, "spec JSON changed with {:?}", args);
            }
            checked += 1;
        };

        // not accepted
        let fx = one_file(false);
        fixture_edit(&fx, "tests/a.rs", &a_fns(), "planned");
        check(&fx, "tests/a.rs");

        // scenario hash mismatch
        let fx = one_file(true);
        edit_scenarios(&fx);
        fixture_edit(&fx, "tests/a.rs", &a_fns(), "planned");
        check(&fx, "tests/a.rs");

        // missing test function
        let fx = one_file(true);
        fixture_edit(
            &fx,
            "tests/a.rs",
            &[("S01", "t_s01"), ("S02", "t_s02")],
            "planned",
        );
        check(&fx, "tests/a.rs");

        // file outside tests/
        let fx = build(true, &[("src/a.rs", a_fns())]);
        fixture_edit(&fx, "src/a.rs", &a_fns(), "planned");
        check(&fx, "src/a.rs");

        // no difference
        let fx = one_file(true);
        check(&fx, "tests/a.rs");

        assert_eq!(checked, 5, "not every refusal condition was checked");
    }
}

mod part_3 {
    #![allow(dead_code, clippy::all)]

    //! Batch 3 of the `hexa spec amend` scenarios (ADR-2611062141 §6, §7).
    //!
    //! Each test builds a temporary git repository holding an accepted ADR
    //! spec with a sealed test file. It then drives the `hexa` binary and
    //! checks what `verify` and `seal` report about amended or changed seals.

    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output, Stdio};

    use sha2::{Digest, Sha256};

    const HEXA: &str = env!("CARGO_BIN_EXE_hexa");

    struct Run {
        ok: bool,
        text: String,
    }

    fn run_in(dir: &Path, home: &Path, args: &[&str]) -> Run {
        let out: Output = Command::new(HEXA)
            .args(args)
            .current_dir(dir)
            .env("HOME", home)
            .env("NO_COLOR", "1")
            .env("CLICOLOR", "0")
            .env("GIT_AUTHOR_NAME", "Test Reviewer")
            .env("GIT_AUTHOR_EMAIL", "reviewer@example.com")
            .env("GIT_COMMITTER_NAME", "Test Reviewer")
            .env("GIT_COMMITTER_EMAIL", "reviewer@example.com")
            .stdin(Stdio::null())
            .output()
            .expect("failed to run hexa binary");
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        Run {
            ok: out.status.success(),
            text,
        }
    }

    fn git(dir: &Path, home: &Path, args: &[&str]) {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("HOME", home)
            .env("GIT_AUTHOR_NAME", "Test Reviewer")
            .env("GIT_AUTHOR_EMAIL", "reviewer@example.com")
            .env("GIT_COMMITTER_NAME", "Test Reviewer")
            .env("GIT_COMMITTER_EMAIL", "reviewer@example.com")
            .stdin(Stdio::null())
            .output()
            .expect("failed to run git");
        assert!(
            out.status.success(),
            "git {:?} failed: {}{}",
            args,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn sha256_hex(path: &Path) -> String {
        let bytes = fs::read(path).expect("read file to hash");
        let digest = Sha256::digest(&bytes);
        digest.iter().map(|b| format!("{:02x}", b)).collect()
    }

    fn test_file_body(fn_name: &str, fixture: &str) -> String {
        format!(
            "/// S1: The fixture builds a site.\n\
         #[test]\n\
         fn {fn_name}() {{\n\
         \x20   let fixture = \"{fixture}\";\n\
         \x20   assert!(!fixture.is_empty());\n\
         }}\n"
        )
    }

    struct AdrDef {
        id: &'static str,
        file: &'static str,
        fn_name: &'static str,
    }

    struct Fixture {
        _tmp: tempfile::TempDir,
        root: PathBuf,
        home: PathBuf,
    }

    impl Fixture {
        fn new(adrs: &[AdrDef]) -> Fixture {
            let tmp = tempfile::tempdir().expect("tempdir");
            let root = tmp.path().join("repo");
            let home = tmp.path().join("home");
            fs::create_dir_all(&root).unwrap();
            fs::create_dir_all(&home).unwrap();

            git(&root, &home, &["init", "-q"]);
            git(&root, &home, &["config", "user.name", "Test Reviewer"]);
            git(
                &root,
                &home,
                &["config", "user.email", "reviewer@example.com"],
            );
            git(&root, &home, &["config", "commit.gpgsign", "false"]);

            fs::create_dir_all(root.join("docs/adr")).unwrap();
            fs::create_dir_all(root.join(".hexa/specs")).unwrap();
            fs::create_dir_all(root.join("tests")).unwrap();

            for a in adrs {
                let adr_md = format!(
                    "# {id}: Fixture decision\n\n\
                 **Status:** Accepted\n\
                 **Date:** 2026-11-06\n\n\
                 ## Context\n\nA fixture.\n\n\
                 ## Decision\n\n1. **The fixture builds a site.**\n\n\
                 ## Consequences\n\n- None.\n\n\
                 ## Applies-To: {file}\n",
                    id = a.id,
                    file = a.file
                );
                fs::write(root.join(format!("docs/adr/{}.md", a.id)), adr_md).unwrap();

                let spec = serde_json::json!({
                    "adr": a.id,
                    "scenarios": [
                        {
                            "id": "S1",
                            "decision": ["§1"],
                            "kind": "example",
                            "description": "The fixture builds a site.",
                            "given": "An empty inventory.",
                            "when": "The fixture runs.",
                            "then": "A site exists.",
                            "test": a.fn_name
                        }
                    ]
                });
                fs::write(
                    root.join(format!(".hexa/specs/{}.json", a.id)),
                    serde_json::to_string_pretty(&spec).unwrap(),
                )
                .unwrap();

                fs::write(root.join(a.file), test_file_body(a.fn_name, "site-v1")).unwrap();
            }

            git(&root, &home, &["add", "-A"]);
            git(
                &root,
                &home,
                &["commit", "-q", "-m", "initial spec and tests"],
            );

            let fx = Fixture {
                _tmp: tmp,
                root,
                home,
            };

            for a in adrs {
                let acc = fx.hexa(&["spec", "accept", a.id]);
                assert!(acc.ok, "spec accept {} failed: {}", a.id, acc.text);
                let seal = fx.hexa(&["spec", "seal", a.id]);
                assert!(seal.ok, "spec seal {} failed: {}", a.id, seal.text);
            }
            git(&fx.root, &fx.home, &["add", "-A"]);
            git(
                &fx.root,
                &fx.home,
                &["commit", "-q", "-m", "accept and seal"],
            );

            for a in adrs {
                let sealed = fx.sealed_hash(a.id, a.file);
                assert_eq!(
                    sealed.as_deref(),
                    Some(sha256_hex(&fx.root.join(a.file)).as_str()),
                    "fixture: {} should be sealed at its current hash",
                    a.file
                );
            }
            fx
        }

        fn hexa(&self, args: &[&str]) -> Run {
            run_in(&self.root, &self.home, args)
        }

        fn commit_all(&self, msg: &str) {
            git(&self.root, &self.home, &["add", "-A"]);
            git(
                &self.root,
                &self.home,
                &["commit", "-q", "--allow-empty", "-m", msg],
            );
        }

        fn spec_path(&self, adr: &str) -> PathBuf {
            let expected = self.root.join(format!(".hexa/specs/{}.json", adr));
            if expected.exists() {
                return expected;
            }
            let mut stack = vec![self.root.clone()];
            while let Some(dir) = stack.pop() {
                for entry in fs::read_dir(&dir).unwrap() {
                    let p = entry.unwrap().path();
                    let name = p.file_name().unwrap().to_string_lossy().to_string();
                    if p.is_dir() {
                        if name != ".git" && name != "target" {
                            stack.push(p);
                        }
                    } else if name.ends_with(".json") {
                        if let Ok(s) = fs::read_to_string(&p) {
                            if s.contains(adr) && s.contains("scenarios") {
                                return p;
                            }
                        }
                    }
                }
            }
            panic!("no spec JSON found for {}", adr);
        }

        fn spec_bytes(&self, adr: &str) -> Vec<u8> {
            fs::read(self.spec_path(adr)).unwrap()
        }

        fn spec(&self, adr: &str) -> serde_json::Value {
            serde_json::from_slice(&self.spec_bytes(adr)).expect("spec JSON parses")
        }

        fn sealed_hash(&self, adr: &str, file: &str) -> Option<String> {
            let spec = self.spec(adr);
            let sealed = spec.get("sealed")?.as_object()?;
            sealed
                .iter()
                .find(|(k, _)| k.as_str() == file || k.ends_with(file))
                .and_then(|(_, v)| v.as_str().map(|s| s.to_string()))
        }

        fn amendments(&self, adr: &str) -> Vec<serde_json::Value> {
            self.spec(adr)
                .get("amendments")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default()
        }

        /// Edit the file, commit it, then run both calls of `amend`.
        /// Returns the hash that was confirmed.
        fn amend_once(
            &self,
            adr: &str,
            file: &str,
            fn_name: &str,
            fixture: &str,
            reason: &str,
        ) -> String {
            fs::write(self.root.join(file), test_file_body(fn_name, fixture)).unwrap();
            self.commit_all("fixture edit");

            let before = self.spec_bytes(adr);
            let show = self.hexa(&["spec", "amend", adr, "--reason", reason, "--file", file]);
            assert!(show.ok, "first amend call failed: {}", show.text);
            assert_eq!(
                before,
                self.spec_bytes(adr),
                "first amend call must write nothing"
            );

            let hash = sha256_hex(&self.root.join(file));
            assert!(
                show.text.contains(&hash),
                "first amend call should print the confirm hash {}: {}",
                hash,
                show.text
            );
            let confirm = self.hexa(&[
                "spec",
                "amend",
                adr,
                "--reason",
                reason,
                "--file",
                file,
                "--confirm",
                &hash,
            ]);
            assert!(confirm.ok, "confirmed amend failed: {}", confirm.text);
            self.commit_all("amend");
            hash
        }
    }

    const ADR_A: &str = "ADR-2611069901";
    const ADR_B: &str = "ADR-2611069902";

    fn adr_a() -> AdrDef {
        AdrDef {
            id: ADR_A,
            file: "tests/a.rs",
            fn_name: "s1_fixture_builds_site",
        }
    }

    fn adr_b() -> AdrDef {
        AdrDef {
            id: ADR_B,
            file: "tests/b.rs",
            fn_name: "s1_fixture_builds_rack",
        }
    }

    /// S21: Verify passes an amended seal and reports the amendment count.
    #[test]
    fn s21_verify_passes_amended_seal_and_reports_count() {
        let fx = Fixture::new(&[adr_a()]);
        let reason = "schema now requires lifecycle/status on sites";
        let hash = fx.amend_once(
            ADR_A,
            "tests/a.rs",
            "s1_fixture_builds_site",
            "site-v2",
            reason,
        );

        let amendments = fx.amendments(ADR_A);
        assert_eq!(amendments.len(), 1, "exactly one amendment recorded");
        let mut checked = 0usize;
        for a in &amendments {
            assert_eq!(a.get("to").and_then(|v| v.as_str()), Some(hash.as_str()));
            assert_eq!(a.get("reason").and_then(|v| v.as_str()), Some(reason));
            checked += 1;
        }
        assert!(checked > 0, "no amendments were checked");

        let plain = fx.hexa(&["spec", "verify", ADR_A]);
        assert!(plain.ok, "verify should pass: {}", plain.text);
        assert!(
            plain.text.contains("1 amendment"),
            "plain verify should show `1 amendment`: {}",
            plain.text
        );
        assert!(
            !plain.text.contains("1 amendments"),
            "singular count expected: {}",
            plain.text
        );
        assert!(
            plain.text.contains(ADR_A),
            "output names the ADR: {}",
            plain.text
        );

        let verbose = fx.hexa(&["spec", "verify", ADR_A, "--verbose"]);
        assert!(verbose.ok, "verify --verbose should pass: {}", verbose.text);
        assert!(
            verbose.text.contains(reason),
            "verbose verify should list the amendment's reason: {}",
            verbose.text
        );

        let ci = fx.hexa(&["spec", "verify", "--ci"]);
        assert!(
            ci.ok,
            "verify --ci should pass on an amended seal: {}",
            ci.text
        );
    }

    /// S22: Verify --ci fails when a file is edited after its amendment.
    #[test]
    fn s22_verify_ci_fails_when_file_edited_after_amendment() {
        let fx = Fixture::new(&[adr_a()]);
        let h1 = fx.amend_once(
            ADR_A,
            "tests/a.rs",
            "s1_fixture_builds_site",
            "site-v2",
            "schema requires lifecycle/status",
        );
        assert_eq!(fx.amendments(ADR_A).len(), 1, "one amendment checked");

        let pre = fx.hexa(&["spec", "verify", "--ci"]);
        assert!(
            pre.ok,
            "verify --ci passes right after amendment: {}",
            pre.text
        );

        fs::write(
            fx.root.join("tests/a.rs"),
            test_file_body("s1_fixture_builds_site", "site-v3-edited-again"),
        )
        .unwrap();
        fx.commit_all("edit after amendment");
        let h2 = sha256_hex(&fx.root.join("tests/a.rs"));
        assert_ne!(h1, h2);

        let ci = fx.hexa(&["spec", "verify", "--ci"]);
        assert!(
            !ci.ok,
            "verify --ci must fail after a post-amendment edit: {}",
            ci.text
        );
        assert!(
            ci.text.contains("tests/a.rs"),
            "failure should name tests/a.rs: {}",
            ci.text
        );
        let lower = ci.text.to_lowercase();
        assert!(
            lower.contains("changed"),
            "failure should report the file as changed: {}",
            ci.text
        );
    }

    /// S23: Verify --ci passes only when every amended file still matches its latest amendment.
    #[test]
    fn s23_verify_ci_passes_only_when_amended_files_match_latest_amendment() {
        let fx = Fixture::new(&[adr_a(), adr_b()]);

        // ADR A: two amendments; ADR B: one amendment.
        fx.amend_once(
            ADR_A,
            "tests/a.rs",
            "s1_fixture_builds_site",
            "site-v2",
            "first reason",
        );
        fx.amend_once(
            ADR_A,
            "tests/a.rs",
            "s1_fixture_builds_site",
            "site-v3",
            "second reason",
        );
        fx.amend_once(
            ADR_B,
            "tests/b.rs",
            "s1_fixture_builds_rack",
            "rack-v2",
            "rack reason",
        );

        let mut checked = 0usize;
        for (adr, file, expected_count) in
            [(ADR_A, "tests/a.rs", 2usize), (ADR_B, "tests/b.rs", 1usize)]
        {
            let amendments = fx.amendments(adr);
            assert_eq!(amendments.len(), expected_count, "{} amendment count", adr);
            let current = sha256_hex(&fx.root.join(file));
            let latest_to = amendments
                .last()
                .and_then(|a| a.get("to"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            assert_eq!(
                latest_to.as_deref(),
                Some(current.as_str()),
                "{} latest `to`",
                adr
            );
            assert_eq!(
                fx.sealed_hash(adr, file).as_deref(),
                Some(current.as_str()),
                "{} sealed",
                adr
            );
            checked += amendments.len();

            let out = fx.hexa(&["spec", "verify", adr]);
            assert!(out.ok, "verify {} should pass: {}", adr, out.text);
            let label = if expected_count == 1 {
                "1 amendment".to_string()
            } else {
                format!("{} amendments", expected_count)
            };
            assert!(
                out.text.contains(&label),
                "verify {} should print `{}`: {}",
                adr,
                label,
                out.text
            );
        }
        assert!(checked > 0, "no amendments were checked");

        let ci = fx.hexa(&["spec", "verify", "--ci"]);
        assert!(ci.ok, "all amended files match: --ci passes: {}", ci.text);

        // Break one: B drifts, so --ci must fail even though A still matches.
        fs::write(
            fx.root.join("tests/b.rs"),
            test_file_body("s1_fixture_builds_rack", "rack-drifted"),
        )
        .unwrap();
        fx.commit_all("drift b");
        let ci = fx.hexa(&["spec", "verify", "--ci"]);
        assert!(!ci.ok, "one drifted amended file fails --ci: {}", ci.text);
        assert!(
            ci.text.contains("tests/b.rs"),
            "names drifted file: {}",
            ci.text
        );

        // Restore B to its amended content: passes again.
        fs::write(
            fx.root.join("tests/b.rs"),
            test_file_body("s1_fixture_builds_rack", "rack-v2"),
        )
        .unwrap();
        fx.commit_all("restore b");
        let ci = fx.hexa(&["spec", "verify", "--ci"]);
        assert!(
            ci.ok,
            "restored content matches latest amendment: {}",
            ci.text
        );

        // An earlier amendment's `to` is not enough: revert A to its first amended content.
        fs::write(
            fx.root.join("tests/a.rs"),
            test_file_body("s1_fixture_builds_site", "site-v2"),
        )
        .unwrap();
        fx.commit_all("revert a to first amendment");
        let ci = fx.hexa(&["spec", "verify", "--ci"]);
        assert!(
            !ci.ok,
            "matching only an earlier amendment must fail --ci: {}",
            ci.text
        );
    }

    /// S24: Seal's refusal points to amend.
    #[test]
    fn s24_seal_refusal_points_to_amend() {
        let fx = Fixture::new(&[adr_a()]);
        fs::write(
            fx.root.join("tests/a.rs"),
            test_file_body("s1_fixture_builds_site", "site-v2"),
        )
        .unwrap();
        fx.commit_all("edit sealed file");

        let out = fx.hexa(&["spec", "seal", ADR_A]);
        assert!(
            !out.ok,
            "seal must refuse a changed sealed file: {}",
            out.text
        );
        assert!(
            out.text
                .contains("A sealed test changes through the spec, not by re-sealing"),
            "refusal keeps the original message: {}",
            out.text
        );
        assert!(
            out.text
                .contains("if the scenarios are unchanged and the edit is deliberate"),
            "refusal points to amend: {}",
            out.text
        );
        assert!(
            out.text.contains("hexa spec amend"),
            "refusal names `hexa spec amend`: {}",
            out.text
        );
    }

    /// S25: Seal still refuses to re-seal a changed file.
    #[test]
    fn s25_seal_still_refuses_to_reseal_changed_file() {
        let fx = Fixture::new(&[adr_a()]);
        let sealed_before = fx.sealed_hash(ADR_A, "tests/a.rs");
        assert!(sealed_before.is_some(), "fixture is sealed");
        let spec_before = fx.spec_bytes(ADR_A);

        fs::write(
            fx.root.join("tests/a.rs"),
            test_file_body("s1_fixture_builds_site", "site-v2"),
        )
        .unwrap();
        fx.commit_all("edit sealed file");
        let current = sha256_hex(&fx.root.join("tests/a.rs"));
        assert_ne!(sealed_before.as_deref(), Some(current.as_str()));

        let out = fx.hexa(&["spec", "seal", ADR_A]);
        assert!(!out.ok, "seal must refuse: {}", out.text);
        assert!(
            out.text.contains("hexa spec amend"),
            "refusal points to amend: {}",
            out.text
        );

        assert_eq!(
            fx.sealed_hash(ADR_A, "tests/a.rs"),
            sealed_before,
            "sealed[file] must be unchanged after a refused seal"
        );
        assert_eq!(
            fx.spec_bytes(ADR_A),
            spec_before,
            "spec JSON must be untouched by a refused seal"
        );
        assert!(
            fx.amendments(ADR_A).is_empty(),
            "seal must not record an amendment"
        );
    }
}
