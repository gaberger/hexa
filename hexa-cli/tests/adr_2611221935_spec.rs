#![allow(unused, clippy::all)]
mod part_1 {
    #![allow(dead_code)]

    use std::io::Read;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    const ADR_ID: &str = "ADR-2601010000";

    const ADR_BODY: &str = "# ADR-2601010000: A demo decision\n\n\
**Status:** Proposed\n\
**Date:** 2026-01-01\n\n\
## Context\n\nA demo project.\n\n\
## Decision\n\n1. **The demo does a thing.** It does it.\n\n\
## Consequences\n\n- The thing is done.\n";

    struct Project {
        dir: tempfile::TempDir,
    }

    impl Project {
        fn new() -> Self {
            let dir = tempfile::tempdir().expect("tempdir");
            let p = Project { dir };
            let _ = Command::new("git")
                .args(["init", "-q"])
                .current_dir(p.root())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            for d in ["docs/adr", "docs/adrs", "docs/decisions", "adr", "adrs"] {
                p.write(&format!("{d}/{ADR_ID}-demo.md"), ADR_BODY);
            }
            p
        }

        fn root(&self) -> &Path {
            self.dir.path()
        }

        fn write(&self, rel: &str, content: &str) {
            let path: PathBuf = self.root().join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("mkdir");
            }
            std::fs::write(path, content).expect("write");
        }

        fn with_runners(&self) {
            self.write(
                ".hexa/project.json",
                r#"{"test":{"runners":{"py":"python -m pytest {file}::{name}"}}}"#,
            );
        }

        fn run(&self, args: &[&str]) -> String {
            let home = self.root().join(".home");
            std::fs::create_dir_all(&home).expect("home");
            let mut child = Command::new(env!("CARGO_BIN_EXE_hexa"))
                .args(args)
                .current_dir(self.root())
                .env("HOME", &home)
                .env("NO_COLOR", "1")
                .env_remove("ANTHROPIC_API_KEY")
                .env_remove("OPENAI_API_KEY")
                .env_remove("GEMINI_API_KEY")
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("spawn hexa");
            let mut stdout = child.stdout.take().unwrap();
            let mut stderr = child.stderr.take().unwrap();
            let t_out = std::thread::spawn(move || {
                let mut s = String::new();
                let _ = stdout.read_to_string(&mut s);
                s
            });
            let t_err = std::thread::spawn(move || {
                let mut s = String::new();
                let _ = stderr.read_to_string(&mut s);
                s
            });
            let start = Instant::now();
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) => {
                        if start.elapsed() > Duration::from_secs(60) {
                            let _ = child.kill();
                            let _ = child.wait();
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    Err(_) => break,
                }
            }
            let out = t_out.join().unwrap_or_default();
            let err = t_err.join().unwrap_or_default();
            format!("{out}\n{err}")
        }

        fn draft(&self) -> String {
            self.run(&["spec", "draft", ADR_ID])
        }

        fn accept(&self) -> String {
            // `spec accept` reads the drafted spec before it looks for a test
            // home, so the spec is drafted first, as a person accepts one.
            self.write(
                &format!("docs/specs/{ADR_ID}.json"),
                &format!(r#"{{"adr":"{ADR_ID}","feature":"demo","specs":[]}}"#),
            );
            self.run(&["spec", "accept", ADR_ID, "--by", "tester"])
        }
    }

    /// Whether the output is the refusal for lacking a test home: it names the
    /// test homes Cargo.toml, package.json and test.runners together.
    fn refused_for_test_home(out: &str) -> bool {
        let l = out.to_lowercase();
        l.contains("cargo.toml") && l.contains("package.json") && l.contains("test.runners")
    }

    fn names_python_manifest(out: &str) -> bool {
        let l = out.to_lowercase();
        l.contains("pyproject.toml") || l.contains("setup.py") || l.contains("setup.cfg")
    }

    fn names_pytest(out: &str) -> bool {
        out.to_lowercase().contains("pytest")
    }

    fn assert_full_refusal(out: &str, what: &str) {
        assert!(
            refused_for_test_home(out),
            "{what}: expected a test-home refusal, got:\n{out}"
        );
        assert!(
            names_python_manifest(out),
            "{what}: refusal does not name a Python manifest:\n{out}"
        );
        assert!(
            names_pytest(out),
            "{what}: refusal does not name pytest:\n{out}"
        );
    }

    const PYTEST: &str = "def test_x():\n    assert True\n";

    /// S01: A root pyproject.toml lets spec draft proceed
    #[test]
    fn s01_pyproject_lets_spec_draft_proceed() {
        let p = Project::new();
        p.write(
            "pyproject.toml",
            "[project]\nname = \"demo\"\nversion = \"0.1.0\"\n",
        );
        p.write("tests/test_x.py", PYTEST);
        let out = p.draft();
        assert!(
            !refused_for_test_home(&out),
            "spec draft refused under pyproject.toml:\n{out}"
        );
    }

    /// S03: A root setup.cfg alone is a test home
    #[test]
    fn s03_setup_cfg_alone_is_a_test_home() {
        let p = Project::new();
        p.write("setup.cfg", "[metadata]\nname = demo\n");
        let out = p.draft();
        assert!(
            !refused_for_test_home(&out),
            "spec draft refused under setup.cfg:\n{out}"
        );
    }

    /// S04: A Python manifest below the root is not a test home
    #[test]
    fn s04_python_manifest_below_root_is_not_a_test_home() {
        let p = Project::new();
        p.write("sub/pyproject.toml", "[project]\nname = \"sub\"\n");
        let out = p.draft();
        assert_full_refusal(&out, "sub/pyproject.toml only");
    }

    /// S05: A Python file that is not one of the three manifests does not make a test home
    #[test]
    fn s05_requirements_txt_is_not_a_test_home() {
        let p = Project::new();
        p.write("requirements.txt", "pytest\n");
        p.write("tests/test_x.py", PYTEST);
        let out = p.accept();
        assert_full_refusal(&out, "requirements.txt only");
    }

    /// S06: Any one of the three Python manifests at the root suffices
    #[test]
    fn s06_any_python_manifest_combination_suffices() {
        let manifests: [(&str, &str); 3] = [
            ("pyproject.toml", "[project]\nname = \"demo\"\n"),
            (
                "setup.py",
                "from setuptools import setup\nsetup(name='demo')\n",
            ),
            ("setup.cfg", "[metadata]\nname = demo\n"),
        ];
        let extras: [&[(&str, &str)]; 2] = [
            &[],
            &[
                ("requirements.txt", "pytest\n"),
                ("tests/test_x.py", PYTEST),
                ("README.md", "# demo\n"),
            ],
        ];
        for mask in 1u8..8 {
            for extra in extras.iter() {
                for cmd in ["draft", "accept"] {
                    let p = Project::new();
                    let mut names = Vec::new();
                    for (i, (name, body)) in manifests.iter().enumerate() {
                        if mask & (1 << i) != 0 {
                            p.write(name, body);
                            names.push(*name);
                        }
                    }
                    for (name, body) in extra.iter() {
                        p.write(name, body);
                    }
                    let out = if cmd == "draft" {
                        p.draft()
                    } else {
                        p.accept()
                    };
                    assert!(
                        !refused_for_test_home(&out),
                        "spec {cmd} refused with manifests {names:?} (extras {}):\n{out}",
                        extra.len()
                    );
                }
            }
        }
    }

    /// S07: The refusal message names a Python manifest and pytest
    #[test]
    fn s07_refusal_names_python_manifest_and_pytest() {
        let p = Project::new();
        let out = p.draft();
        assert_full_refusal(&out, "empty project, spec draft");
    }

    /// S08: The refusal no longer names only the three old test homes
    #[test]
    fn s08_refusal_not_only_old_test_homes() {
        let p = Project::new();
        let out = p.accept();
        assert!(
            refused_for_test_home(&out),
            "expected a test-home refusal, got:\n{out}"
        );
        assert!(
            names_python_manifest(&out) && names_pytest(&out),
            "refusal names only the old test homes:\n{out}"
        );
    }

    /// S09: Every refusal from either command names a Python manifest and pytest
    #[test]
    fn s09_every_refusal_names_python_manifest_and_pytest() {
        let layouts: [&[(&str, &str)]; 4] = [
            &[],
            &[("requirements.txt", "pytest\n")],
            &[("tests/test_x.py", PYTEST), ("README.md", "# demo\n")],
            &[
                ("sub/pyproject.toml", "[project]\nname = \"sub\"\n"),
                ("src/lib.py", "x = 1\n"),
            ],
        ];
        for (i, layout) in layouts.iter().enumerate() {
            for cmd in ["draft", "accept"] {
                let p = Project::new();
                for (name, body) in layout.iter() {
                    p.write(name, body);
                }
                let out = if cmd == "draft" {
                    p.draft()
                } else {
                    p.accept()
                };
                assert_full_refusal(&out, &format!("layout {i}, spec {cmd}"));
            }
        }
    }
}

mod part_2 {
    #![allow(dead_code)]

    use std::fs;
    use std::path::Path;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    const ADR_ID: &str = "ADR-2611221935";

    const ADR_BODY: &str = "# ADR-2611221935: Test fixture\n\n**Status:** Proposed\n**Date:** 2026-11-22\n\n## Context\n\nA fixture.\n\n## Decision\n\n1. **Something.** It does something.\n\n## Consequences\n\n- None.\n";

    /// A fresh project root: a git repository holding one ADR, with no manifest.
    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let _ = Command::new("git")
            .args(["init", "-q"])
            .current_dir(root)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        for sub in ["docs/adr", "docs/adrs"] {
            let d = root.join(sub);
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join(format!("{ADR_ID}-test-fixture.md")), ADR_BODY).unwrap();
        }
        fs::create_dir_all(root.join("home")).unwrap();
        dir
    }

    /// Run `hexa` in `root` with a timeout; returns the combined output.
    fn run(root: &Path, args: &[&str]) -> (Option<i32>, String) {
        // `spec accept` reads the drafted spec before it looks for a test
        // home, so a drafted spec is there first, as a person accepts one.
        if args.get(1) == Some(&"accept") {
            let specs = root.join("docs/specs");
            fs::create_dir_all(&specs).unwrap();
            fs::write(
                specs.join(format!("{ADR_ID}.json")),
                format!(r#"{{"adr":"{ADR_ID}","feature":"demo","specs":[]}}"#),
            )
            .unwrap();
        }
        let out_path = root
            .join("home")
            .join(format!("out-{}.txt", uuid::Uuid::new_v4()));
        let out_file = fs::File::create(&out_path).unwrap();
        let err_file = out_file.try_clone().unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_hexa"))
            .args(args)
            .current_dir(root)
            .env("HOME", root.join("home"))
            .env("XDG_CONFIG_HOME", root.join("home").join(".config"))
            .env("NO_COLOR", "1")
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("OPENAI_API_KEY")
            .env_remove("HEXA_MODEL")
            .stdin(Stdio::null())
            .stdout(Stdio::from(out_file))
            .stderr(Stdio::from(err_file))
            .spawn()
            .expect("spawn hexa");
        let start = Instant::now();
        let code = loop {
            match child.try_wait().expect("wait") {
                Some(status) => break status.code(),
                None if start.elapsed() > Duration::from_secs(90) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                None => std::thread::sleep(Duration::from_millis(100)),
            }
        };
        let text = fs::read_to_string(&out_path).unwrap_or_default();
        let _ = fs::remove_file(&out_path);
        (code, text)
    }

    /// Whether the output is the refusal for lacking a test home.
    fn refused_for_test_home(out: &str) -> bool {
        out.contains("Cargo.toml") && out.contains("package.json") && out.contains("test.runners")
    }

    fn names_python_manifest(out: &str) -> bool {
        out.contains("pyproject.toml") || out.contains("setup.py") || out.contains("setup.cfg")
    }

    fn py_files(root: &Path) -> Vec<String> {
        fn walk(dir: &Path, acc: &mut Vec<String>) {
            let Ok(rd) = fs::read_dir(dir) else { return };
            for e in rd.flatten() {
                let p = e.path();
                let name = e.file_name().to_string_lossy().to_string();
                if name == ".git" || name == "home" {
                    continue;
                }
                if p.is_dir() {
                    walk(&p, acc);
                } else if name.ends_with(".py") {
                    acc.push(p.to_string_lossy().to_string());
                }
            }
        }
        let mut acc = Vec::new();
        walk(root, &mut acc);
        acc.sort();
        acc
    }

    /// S11: A project with no test home is still refused
    #[test]
    fn s11_a_project_with_no_test_home_is_still_refused() {
        for cmd in [["spec", "draft", ADR_ID], ["spec", "accept", ADR_ID]] {
            let dir = project();
            let (code, out) = run(dir.path(), &cmd);
            assert_ne!(
                code,
                Some(0),
                "`hexa {}` should be refused with no test home:\n{out}",
                cmd.join(" ")
            );
            assert!(
                refused_for_test_home(&out),
                "`hexa {}` should refuse for lacking a test home:\n{out}",
                cmd.join(" ")
            );
            assert!(
            names_python_manifest(&out),
            "the refusal of `hexa {}` should list a Python manifest among the test homes:\n{out}",
            cmd.join(" ")
        );
            assert!(
                out.contains("pytest"),
                "the refusal of `hexa {}` should name pytest among the runners:\n{out}",
                cmd.join(" ")
            );
        }
    }

    /// S13: Outcomes are unchanged for every project without a Python manifest
    #[test]
    fn s13_outcomes_unchanged_without_a_python_manifest() {
        // (has Cargo.toml, has package.json, has test.runners)
        let mut cases = Vec::new();
        for cargo in [false, true] {
            for pkg in [false, true] {
                for runners in [false, true] {
                    cases.push((cargo, pkg, runners));
                }
            }
        }
        for (cargo, pkg, runners) in cases {
            let expect_proceed = cargo || pkg || runners;
            // draft may call a model when it proceeds; run it only where it must refuse.
            let cmds: Vec<[&str; 3]> = if expect_proceed {
                vec![["spec", "accept", ADR_ID]]
            } else {
                vec![["spec", "draft", ADR_ID], ["spec", "accept", ADR_ID]]
            };
            for cmd in cmds {
                let dir = project();
                let root = dir.path();
                if cargo {
                    fs::write(
                        root.join("Cargo.toml"),
                        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
                    )
                    .unwrap();
                }
                if pkg {
                    fs::write(
                        root.join("package.json"),
                        "{\"name\":\"demo\",\"version\":\"0.1.0\"}\n",
                    )
                    .unwrap();
                }
                if runners {
                    fs::create_dir_all(root.join(".hexa")).unwrap();
                    fs::write(
                        root.join(".hexa/project.json"),
                        serde_json::json!({"test": {"runners": {"sh": "bash {file}"}}}).to_string(),
                    )
                    .unwrap();
                }
                let (code, out) = run(root, &cmd);
                let refused = refused_for_test_home(&out);
                assert_eq!(
                    refused,
                    !expect_proceed,
                    "`hexa {}` with Cargo.toml={cargo} package.json={pkg} test.runners={runners}: \
                 refused for lacking a test home = {refused}, expected {}:\n{out}",
                    cmd.join(" "),
                    !expect_proceed
                );
                if !expect_proceed {
                    assert_ne!(code, Some(0), "a refusal must not exit 0:\n{out}");
                    assert!(
                        names_python_manifest(&out),
                        "the refusal should list a Python manifest among the test homes:\n{out}"
                    );
                }
            }
        }
    }
}
