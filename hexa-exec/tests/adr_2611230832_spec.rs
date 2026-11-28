#![allow(unused, clippy::all)]
mod part_1 {
    #![allow(clippy::all)]

    use hexa_exec::test_exec::{plan_test, TestConfig};
    use std::fs;
    use std::path::Path;

    fn write(p: &Path, s: &str) {
        if let Some(d) = p.parent() {
            fs::create_dir_all(d).unwrap();
        }
        fs::write(p, s).unwrap();
    }

    fn interp(p: &Path) {
        write(p, "#!/bin/sh\nexec python3 \"$@\"\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(p, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    fn test_file(p: &Path, name: &str) {
        write(p, &format!("def {name}():\n    assert True\n"));
    }

    fn plan(root: &Path, r: &str) -> String {
        match plan_test(root, r, &TestConfig::default(), "") {
            Ok(p) => p.cmd,
            Err(e) => panic!("plan_test failed for {r}: {e:?}"),
        }
    }

    /// S01: A uv project with uv.lock beside its pyproject.toml runs pytest through uv
    #[test]
    fn s01_uv_project_runs_pytest_through_uv() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        write(
            &root.join("sdk/pyproject.toml"),
            "[project]\nname = \"sdk\"\n",
        );
        write(&root.join("sdk/uv.lock"), "version = 1\n");
        test_file(&root.join("sdk/tests/test_client.py"), "test_connects");
        let cmd = plan(root, "sdk/tests/test_client.py::test_connects");
        assert!(cmd.contains("uv run python -m pytest"), "{cmd}");
        assert!(cmd.contains("test_client.py"), "{cmd}");
        assert!(cmd.contains("test_connects"), "{cmd}");
        assert!(cmd.contains("-q"), "{cmd}");
        assert!(cmd.contains("sdk"), "{cmd}");
        assert!(!cmd.contains("python3"), "{cmd}");
    }

    /// S02: A uv.lock above the pytest directory, but inside the repository, still selects uv
    #[test]
    fn s02_uv_lock_above_pytest_dir_within_repo_selects_uv() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        write(&root.join("uv.lock"), "version = 1\n");
        write(
            &root.join("packages/sdk/pyproject.toml"),
            "[project]\nname = \"sdk\"\n",
        );
        test_file(&root.join("packages/sdk/tests/test_a.py"), "test_a");
        let cmd = plan(root, "packages/sdk/tests/test_a.py::test_a");
        assert!(cmd.contains("uv run python -m pytest"), "{cmd}");
        assert!(
            cmd.contains("packages/sdk"),
            "pytest should run from packages/sdk: {cmd}"
        );
        assert!(!cmd.contains("python3"), "{cmd}");
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Uv {
        None,
        Root,
        Pkg,
        Other,
        Parent,
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Venv {
        None,
        RootDot,
        RootPlain,
        PkgDot,
        PkgPlain,
        TestsDot,
        ParentDot,
        EmptyDot,
    }

    struct Case {
        _t: tempfile::TempDir,
        root: std::path::PathBuf,
        r: String,
        uv_expected: bool,
        venv_expected: bool,
    }

    fn build(nested: bool, uv: Uv, venv: Venv) -> Case {
        let t = tempfile::tempdir().unwrap();
        let outer = t.path().to_path_buf();
        let root = outer.join("repo");
        fs::create_dir_all(&root).unwrap();
        let pkg = root.join("pkg");
        fs::create_dir_all(&pkg).unwrap();
        let (pydir, r) = if nested {
            write(&pkg.join("pyproject.toml"), "[project]\nname = \"p\"\n");
            (pkg.clone(), "pkg/tests/test_a.py::test_a".to_string())
        } else {
            write(&root.join("pyproject.toml"), "[project]\nname = \"p\"\n");
            (root.clone(), "tests/test_a.py::test_a".to_string())
        };
        test_file(&pydir.join("tests/test_a.py"), "test_a");
        match uv {
            Uv::None => {}
            Uv::Root => write(&root.join("uv.lock"), "version = 1\n"),
            Uv::Pkg => write(&pkg.join("uv.lock"), "version = 1\n"),
            Uv::Other => write(&root.join("other/uv.lock"), "version = 1\n"),
            Uv::Parent => write(&outer.join("uv.lock"), "version = 1\n"),
        }
        match venv {
            Venv::None => {}
            Venv::RootDot => interp(&root.join(".venv/bin/python")),
            Venv::RootPlain => interp(&root.join("venv/bin/python")),
            Venv::PkgDot => interp(&pkg.join(".venv/bin/python")),
            Venv::PkgPlain => interp(&pkg.join("venv/bin/python")),
            Venv::TestsDot => interp(&pydir.join("tests/.venv/bin/python")),
            Venv::ParentDot => interp(&outer.join(".venv/bin/python")),
            Venv::EmptyDot => fs::create_dir_all(root.join(".venv")).unwrap(),
        }
        let uv_expected = uv == Uv::Root || (uv == Uv::Pkg && nested);
        let venv_found = matches!(venv, Venv::RootDot | Venv::RootPlain)
            || (nested && matches!(venv, Venv::PkgDot | Venv::PkgPlain));
        Case {
            _t: t,
            root,
            r,
            uv_expected,
            venv_expected: !uv_expected && venv_found,
        }
    }

    const UVS: [Uv; 5] = [Uv::None, Uv::Root, Uv::Pkg, Uv::Other, Uv::Parent];
    const VENVS: [Venv; 8] = [
        Venv::None,
        Venv::RootDot,
        Venv::RootPlain,
        Venv::PkgDot,
        Venv::PkgPlain,
        Venv::TestsDot,
        Venv::ParentDot,
        Venv::EmptyDot,
    ];

    /// S05: uv is used exactly when a uv.lock exists at or above the pytest directory, within the repository
    #[test]
    fn s05_uv_iff_uv_lock_in_range() {
        for nested in [false, true] {
            for uv in UVS {
                for venv in VENVS {
                    let c = build(nested, uv, venv);
                    let cmd = plan(&c.root, &c.r);
                    let ctx = format!("nested={nested} uv={uv:?} venv={venv:?}: {cmd}");
                    assert_eq!(
                        cmd.contains("uv run python -m pytest"),
                        c.uv_expected,
                        "{ctx}"
                    );
                    if nested {
                        assert!(cmd.contains("pkg"), "pytest should run from pkg: {ctx}");
                    }
                    assert!(cmd.contains("test_a"), "{ctx}");
                    assert!(cmd.contains("-q"), "{ctx}");
                }
            }
        }
    }

    /// S06: A project with .venv/bin/python runs pytest on that interpreter
    #[test]
    fn s06_dot_venv_interpreter_used() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("repo");
        write(&root.join("pyproject.toml"), "[project]\nname = \"x\"\n");
        interp(&root.join(".venv/bin/python"));
        test_file(&root.join("tests/test_a.py"), "test_a");
        let cmd = plan(&root, "tests/test_a.py::test_a");
        assert!(cmd.contains(".venv/bin/python"), "{cmd}");
        assert!(cmd.contains("-m pytest"), "{cmd}");
        assert!(cmd.contains("test_a.py"), "{cmd}");
        assert!(cmd.contains("test_a"), "{cmd}");
        assert!(cmd.contains("-q"), "{cmd}");
        assert!(!cmd.contains("python3"), "{cmd}");
        assert!(!cmd.contains("uv run"), "{cmd}");
    }

    /// S07: A venv/bin/python above the pytest directory selects that interpreter
    #[test]
    fn s07_root_venv_above_pytest_dir() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("repo");
        interp(&root.join("venv/bin/python"));
        write(
            &root.join("pkg/pyproject.toml"),
            "[project]\nname = \"x\"\n",
        );
        test_file(&root.join("pkg/tests/test_b.py"), "test_b");
        let cmd = plan(&root, "pkg/tests/test_b.py::test_b");
        assert!(cmd.contains("venv/bin/python"), "{cmd}");
        assert!(!cmd.contains(".venv"), "{cmd}");
        assert!(!cmd.contains("pkg/venv"), "{cmd}");
        assert!(cmd.contains("-m pytest"), "{cmd}");
        assert!(cmd.contains("pkg"), "{cmd}");
        assert!(!cmd.contains("python3"), "{cmd}");
    }

    /// S08: uv.lock wins over a virtualenv
    #[test]
    fn s08_uv_lock_wins_over_venv() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("repo");
        write(&root.join("pyproject.toml"), "[project]\nname = \"x\"\n");
        write(&root.join("uv.lock"), "version = 1\n");
        interp(&root.join(".venv/bin/python"));
        test_file(&root.join("tests/test_a.py"), "test_a");
        let cmd = plan(&root, "tests/test_a.py::test_a");
        assert!(cmd.contains("uv run python -m pytest"), "{cmd}");
        assert!(!cmd.contains(".venv/bin/python"), "{cmd}");
    }

    /// S10: A virtualenv interpreter is used exactly when there is no uv.lock and a .venv/bin/python or venv/bin/python exists at or above the pytest directory, within the repository
    #[test]
    fn s10_venv_iff_no_uv_and_interpreter_in_range() {
        for nested in [false, true] {
            for uv in UVS {
                for venv in VENVS {
                    let c = build(nested, uv, venv);
                    let cmd = plan(&c.root, &c.r);
                    let ctx = format!("nested={nested} uv={uv:?} venv={venv:?}: {cmd}");
                    let uses_venv = cmd.contains("bin/python -m pytest");
                    assert_eq!(uses_venv, c.venv_expected, "{ctx}");
                    if c.venv_expected {
                        assert!(!cmd.contains("python3"), "{ctx}");
                        assert!(!cmd.contains("tests/.venv"), "{ctx}");
                    }
                    if !c.uv_expected && !c.venv_expected {
                        assert!(cmd.contains("python3 -m pytest"), "{ctx}");
                    }
                }
            }
        }
    }
}

mod part_2 {
    #![allow(clippy::all)]

    use std::fs;
    use std::path::Path;

    use hexa_exec::test_exec::{
        env_prefix, judge, plan_test, run_plans, Planned, TestConfig, TestRun, Where,
    };

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        if let Some(d) = p.parent() {
            fs::create_dir_all(d).unwrap();
        }
        fs::write(p, body).unwrap();
    }

    fn write_exec(root: &Path, rel: &str, body: &str) {
        write(root, rel, body);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(root.join(rel), fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    fn py_test(name: &str) -> String {
        format!("def {name}():\n    assert True\n")
    }

    fn plan(root: &Path, r: &str, cfg: &TestConfig) -> Planned {
        match plan_test(root, r, cfg, &env_prefix(None, None)) {
            Ok(p) => p,
            Err(e) => panic!("plan_test refused {r}: {e:?}"),
        }
    }

    /// The part of the command from `-m pytest` on: the selector and flags.
    fn tail_from_pytest(cmd: &str) -> String {
        let i = cmd
            .find("-m pytest")
            .unwrap_or_else(|| panic!("no `-m pytest` in `{cmd}`"));
        cmd[i..].to_string()
    }

    /// The part of the command before the interpreter `interp`.
    fn head_before(cmd: &str, interp: &str) -> String {
        let i = cmd
            .find("-m pytest")
            .unwrap_or_else(|| panic!("no `-m pytest` in `{cmd}`"));
        let head = &cmd[..i];
        let head = head
            .strip_suffix(&format!("{interp} "))
            .unwrap_or_else(|| panic!("`{cmd}` does not run `{interp}` before `-m pytest`"));
        head.to_string()
    }

    fn verdict(p: &Planned, ok: bool, out: &str) -> String {
        format!("{:?}", judge(p, ok, out))
    }

    /// S12: python3 is not used when the project has its own environment
    #[test]
    fn s12_venv_does_not_use_python3() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        write(root, "pyproject.toml", "[project]\nname = \"a\"\n");
        write_exec(root, ".venv/bin/python", "#!/bin/sh\nexit 0\n");
        write(root, "tests/test_a.py", &py_test("test_a"));
        let p = plan(root, "tests/test_a.py::test_a", &TestConfig::default());
        assert!(!p.cmd.contains("python3 -m pytest"), "cmd: {}", p.cmd);
        assert!(p.cmd.contains(".venv/bin/python"), "cmd: {}", p.cmd);
        assert!(p.cmd.contains("-m pytest"), "cmd: {}", p.cmd);
    }

    /// S15: A missing uv fails the run with the command's own error, without falling back to python3
    #[test]
    fn s15_missing_uv_fails_without_fallback() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        write(root, "pyproject.toml", "[project]\nname = \"a\"\n");
        write(root, "uv.lock", "version = 1\n");
        write(root, "tests/test_a.py", &py_test("test_a"));

        let bin = tempfile::tempdir().unwrap();
        let marker = bin.path().join("python3-was-run");
        write_exec(
            bin.path(),
            "python3",
            &format!(
                "#!/bin/sh\ntouch '{}'\necho '1 passed in 0.01s'\nexit 0\n",
                marker.display()
            ),
        );
        let path_env = bin.path().to_str().unwrap().to_string();
        let prefix = env_prefix(Some(&path_env), None);
        let p = match plan_test(
            root,
            "tests/test_a.py::test_a",
            &TestConfig::default(),
            &prefix,
        ) {
            Ok(p) => p,
            Err(e) => panic!("plan_test refused: {e:?}"),
        };
        assert!(p.cmd.contains("uv run python -m pytest"), "cmd: {}", p.cmd);

        let rt = tokio::runtime::Runtime::new().unwrap();
        let runs = rt.block_on(run_plans(root, Where::Here, std::slice::from_ref(&p)));
        assert_eq!(runs.len(), 1);
        let run = &runs[0];
        assert!(!matches!(run, TestRun::Passed), "run passed: {run:?}");
        let shown = format!("{run:?}");
        assert!(shown.contains("uv"), "run: {shown}");
        assert!(shown.contains("not found"), "run: {shown}");
        assert!(!marker.exists(), "python3 was invoked");
    }

    /// S17: Judging is unchanged: two tests passing under uv is not a pass
    #[test]
    fn s17_two_passed_under_uv_is_not_pass() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        write(root, "pyproject.toml", "[project]\nname = \"a\"\n");
        write(root, "uv.lock", "version = 1\n");
        write(root, "tests/test_a.py", &py_test("test_a"));
        let p = plan(root, "tests/test_a.py::test_a", &TestConfig::default());
        assert!(p.cmd.contains("uv run python -m pytest"), "cmd: {}", p.cmd);
        let out = "..                                                                       [100%]\n2 passed in 0.01s\n";
        let v = judge(&p, true, out);
        assert!(!matches!(v, TestRun::Passed), "verdict: {v:?}");
    }

    /// S18: Only the interpreter prefix depends on the environment; the directory, the selector, the judge and override precedence do not
    #[test]
    fn s18_only_interpreter_depends_on_environment() {
        let outputs: [(bool, &str); 4] = [
        (true, ".                                                                        [100%]\n1 passed in 0.01s\n"),
        (true, "..                                                                       [100%]\n2 passed in 0.01s\n"),
        (false, "F                                                                        [100%]\n1 failed in 0.01s\n"),
        (true, "\nno tests ran in 0.01s\n"),
    ];
        for env in ["uv", "venv_dot", "venv"] {
            for nested in [false, true] {
                let d = tempfile::tempdir().unwrap();
                let root = d.path();
                let base = if nested { "pkg/" } else { "" };
                write(
                    root,
                    &format!("{base}pyproject.toml"),
                    "[project]\nname = \"a\"\n",
                );
                // The environment sits at the repository root, above the pytest directory.
                match env {
                    "uv" => write(root, "uv.lock", "version = 1\n"),
                    "venv_dot" => write_exec(root, ".venv/bin/python", "#!/bin/sh\nexit 0\n"),
                    _ => write_exec(root, "venv/bin/python", "#!/bin/sh\nexit 0\n"),
                }
                write(root, &format!("{base}tests/test_c.py"), &py_test("test_c"));
                let r = format!("{base}tests/test_c.py::test_c");
                let cfg = TestConfig::default();

                let with_env = plan(root, &r, &cfg);
                let mut cfg_t = TestConfig::default();
                cfg_t
                    .runners
                    .insert("py".into(), "my-runner {file} {name}".into());
                let with_env_t = plan(root, &r, &cfg_t);

                let _ = fs::remove_file(root.join("uv.lock"));
                let _ = fs::remove_dir_all(root.join(".venv"));
                let _ = fs::remove_dir_all(root.join("venv"));

                let without = plan(root, &r, &cfg);
                let without_t = plan(root, &r, &cfg_t);

                assert!(
                    without.cmd.contains("python3 -m pytest"),
                    "{env}: {}",
                    without.cmd
                );
                assert!(
                    !with_env.cmd.contains("python3 -m pytest"),
                    "{env}: {}",
                    with_env.cmd
                );
                assert_eq!(
                    tail_from_pytest(&with_env.cmd),
                    tail_from_pytest(&without.cmd),
                    "{env} nested={nested}"
                );
                if env == "uv" {
                    assert_eq!(
                        head_before(&with_env.cmd, "uv run python"),
                        head_before(&without.cmd, "python3"),
                        "uv nested={nested}"
                    );
                } else {
                    let dir = if env == "venv_dot" {
                        ".venv/bin/python"
                    } else {
                        "venv/bin/python"
                    };
                    assert!(with_env.cmd.contains(dir), "{env}: {}", with_env.cmd);
                    if nested {
                        assert!(
                            head_before(&without.cmd, "python3").contains("pkg"),
                            "{}",
                            without.cmd
                        );
                    }
                }
                for (ok, out) in outputs {
                    assert_eq!(
                        verdict(&with_env, ok, out),
                        verdict(&without, ok, out),
                        "{env} nested={nested}: {out}"
                    );
                }
                assert_eq!(with_env_t.cmd, without_t.cmd, "{env} nested={nested}");
                assert!(with_env_t.cmd.contains("my-runner"), "{}", with_env_t.cmd);
                assert!(!with_env_t.cmd.contains("-m pytest"), "{}", with_env_t.cmd);
            }
        }
    }
}
