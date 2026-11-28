#![allow(unused, clippy::all)]
mod part_1 {
    #![allow(dead_code, unused_imports, clippy::all)]

    use std::fs;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::{Arc, Mutex, MutexGuard};

    use hexa_infer::adapters::secondary::claude_code::{
        ClaudeCodeInferenceAdapter, ProcessSpawner, TokioProcessSpawner,
    };

    // ---------------------------------------------------------------------------
    // helpers
    // ---------------------------------------------------------------------------

    static PATH_LOCK: Mutex<()> = Mutex::new(());

    fn path_lock() -> MutexGuard<'static, ()> {
        PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn write_executable(path: &Path, body: &str) {
        fs::write(path, body).expect("write script");
        let mut perms = fs::metadata(path).expect("meta").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).expect("chmod");
    }

    /// Install a fake `claude` CLI first on PATH that exits with `code`, writes
    /// `stdout` to stdout and `stderr` to stderr; make a `claude-text` call and
    /// return the error message it fails with.
    fn claude_text_error(code: i32, stdout: &str, stderr: &str) -> String {
        let _guard = path_lock();
        let dir = tempfile::tempdir().expect("tempdir");
        let out_file = dir.path().join("stdout.txt");
        let err_file = dir.path().join("stderr.txt");
        fs::write(&out_file, stdout).unwrap();
        fs::write(&err_file, stderr).unwrap();
        let script = format!(
            "#!/bin/sh\ncat > /dev/null 2>&1\ncat '{}'\ncat '{}' >&2\nexit {}\n",
            out_file.display(),
            err_file.display(),
            code
        );
        write_executable(&dir.path().join("claude"), &script);

        let old_path = std::env::var_os("PATH");
        let mut paths = vec![dir.path().to_path_buf()];
        if let Some(p) = &old_path {
            paths.extend(std::env::split_paths(p));
        }
        std::env::set_var("PATH", std::env::join_paths(paths).unwrap());

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let result = rt.block_on(async {
            let spawner: Arc<dyn ProcessSpawner> = Arc::new(TokioProcessSpawner::new());
            let adapter = ClaudeCodeInferenceAdapter::text_with_spawner(None, spawner);
            adapter.complete_with_cost(Default::default()).await
        });

        match old_path {
            Some(p) => std::env::set_var("PATH", p),
            None => std::env::remove_var("PATH"),
        }

        match result {
            Ok(_) => panic!("claude-text call unexpectedly succeeded on a non-zero exit"),
            Err(e) => e.to_string(),
        }
    }

    /// Write one fixture in the corpus directory, both as `<id>.yaml` and as
    /// `<id>/fixture.yaml`.
    fn write_fixture(corpus: &Path, id: &str, evidence: &str) {
        let yaml = format!(
        "id: {id}\nstatus: verified\ntitle: {id}\nlanguage: typescript\nprompt: Make the failing test pass.\ntask: Make the failing test pass.\noracle:\n  command: {evidence}\n"
    );
        fs::write(corpus.join(format!("{id}.yaml")), &yaml).unwrap();
        let sub = corpus.join(id);
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("fixture.yaml"), &yaml).unwrap();
    }

    struct BenchRun {
        success: bool,
        stdout: String,
        stderr: String,
    }

    fn run_bench(project: &Path, corpus: &Path, bin_dir: &Path, home: &Path) -> BenchRun {
        let out = Command::new(env!("CARGO_BIN_EXE_hexa"))
            .args([
                "bench",
                "agentic",
                "--model",
                "test-model-does-not-exist",
                "--corpus",
            ])
            .arg(corpus)
            .args(["--arms", "react,fast"])
            .current_dir(project)
            .env("PATH", bin_dir)
            .env("HOME", home)
            .stdin(Stdio::null())
            .output()
            .expect("run hexa bench agentic");
        BenchRun {
            success: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    }

    fn marker_stub(bin_dir: &Path, name: &str, marker: &Path) {
        let body = format!("#!/bin/sh\n: > '{}'\nexit 0\n", marker.display());
        write_executable(&bin_dir.join(name), &body);
    }

    // ---------------------------------------------------------------------------
    // §1 claude-text says why
    // ---------------------------------------------------------------------------

    /// S01: A non-zero exit with a `result` on stdout surfaces that result
    #[test]
    fn s01_nonzero_exit_surfaces_result() {
        let msg = claude_text_error(
            1,
            r#"{"result":"AWS SSO token expired","terminal_reason":"auth"}"#,
            "",
        );
        assert!(msg.contains("exited with 1"), "error was: {msg}");
        assert!(msg.contains("AWS SSO token expired"), "error was: {msg}");
    }

    /// S02: With no `result`, the error falls back to `terminal_reason`
    #[test]
    fn s02_falls_back_to_terminal_reason() {
        let msg = claude_text_error(2, r#"{"terminal_reason":"rate_limited"}"#, "");
        assert!(msg.contains("exited with 2"), "error was: {msg}");
        assert!(msg.contains("rate_limited"), "error was: {msg}");
    }

    /// S03: Stderr is still reported alongside the stdout reason
    #[test]
    fn s03_stderr_reported_alongside_stdout_reason() {
        let msg = claude_text_error(
            1,
            r#"{"result":"model unavailable"}"#,
            "warning: retry exhausted",
        );
        assert!(msg.contains("model unavailable"), "error was: {msg}");
        assert!(msg.contains("warning: retry exhausted"), "error was: {msg}");
    }

    /// S04: A non-zero exit is never reported with nothing after the colon when the CLI said why
    #[test]
    fn s04_never_nothing_after_colon() {
        let msg = claude_text_error(1, r#"{"result":"AWS SSO token expired"}"#, "");
        assert_ne!(msg.trim(), "claude-text exited with 1:", "error was: {msg}");
        if let Some((_, rest)) = msg.split_once("exited with 1:") {
            assert!(
                !rest.trim().is_empty(),
                "nothing follows the colon in: {msg:?}"
            );
        }
        assert!(msg.contains("AWS SSO token expired"), "error was: {msg}");
    }

    /// S05: `result` takes precedence over `terminal_reason`
    #[test]
    fn s05_result_takes_precedence() {
        let msg = claude_text_error(
            1,
            r#"{"result":"token expired","terminal_reason":"error"}"#,
            "",
        );
        assert!(msg.contains("token expired"), "error was: {msg}");
    }

    /// S06: Every non-zero exit carries what the CLI wrote
    #[test]
    fn s06_every_nonzero_exit_carries_cli_output() {
        // (exit, result, terminal_reason, stderr)
        let cases: &[(i32, Option<&str>, Option<&str>, &str)] = &[
            (1, Some("quota exceeded for org"), None, ""),
            (3, None, Some("max_turns_reached"), ""),
            (
                7,
                Some("network unreachable"),
                Some("transport"),
                "curl: (6) host",
            ),
            (42, None, Some("cancelled_by_user"), "signal received"),
            (
                127,
                Some("login required"),
                Some("auth"),
                "hint: run claude login",
            ),
        ];
        for (code, result, reason, stderr) in cases {
            let mut obj = serde_json::Map::new();
            if let Some(r) = result {
                obj.insert("result".into(), serde_json::Value::String(r.to_string()));
            }
            if let Some(t) = reason {
                obj.insert(
                    "terminal_reason".into(),
                    serde_json::Value::String(t.to_string()),
                );
            }
            let stdout = serde_json::Value::Object(obj).to_string();
            let msg = claude_text_error(*code, &stdout, stderr);
            assert!(
                msg.contains(&code.to_string()),
                "exit {code}: error lacks status: {msg}"
            );
            let expected = result.or(*reason).unwrap();
            assert!(
                msg.contains(expected),
                "exit {code}: error lacks reason {expected:?}: {msg}"
            );
            if !stderr.is_empty() {
                assert!(
                    msg.contains(stderr),
                    "exit {code}: error lacks stderr {stderr:?}: {msg}"
                );
            }
        }
    }

    // ---------------------------------------------------------------------------
    // §2 bench checks evidence programs
    // ---------------------------------------------------------------------------

    
    
    
    // ---------------------------------------------------------------------------
    // §4 session context names Python
    // ---------------------------------------------------------------------------

    /// S16: `pyproject.toml` alone means Python
    #[test]
    fn s16_pyproject_alone_means_python() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("pyproj");
        let home = tmp.path().join("home");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&home).unwrap();
        fs::write(
            project.join("pyproject.toml"),
            "[project]\nname = \"demo\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::create_dir_all(project.join("demo")).unwrap();
        fs::write(project.join("demo").join("__init__.py"), "").unwrap();
        let _ = Command::new("git")
            .args(["init", "-q"])
            .current_dir(&project)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        let input = serde_json::json!({
            "session_id": "s16-test",
            "cwd": project.to_string_lossy(),
            "hook_event_name": "SessionStart",
            "source": "startup",
            "transcript_path": tmp.path().join("t.jsonl").to_string_lossy(),
        })
        .to_string();

        let mut child = Command::new(env!("CARGO_BIN_EXE_hexa"))
            .args(["hook", "session-start"])
            .current_dir(&project)
            .env("HOME", &home)
            .env("CLAUDE_PROJECT_DIR", &project)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn hexa hook session-start");
        {
            let mut stdin = child.stdin.take().unwrap();
            let _ = stdin.write_all(input.as_bytes());
        }
        let out = child.wait_with_output().expect("wait");
        let all = format!(
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let re = regex::Regex::new(r"(?i)language\W{0,6}python").unwrap();
        assert!(
            re.is_match(&all),
            "session context lacks Language: python:\n{all}"
        );
        let unknown = regex::Regex::new(r"(?i)language\W{0,6}unknown").unwrap();
        assert!(
            !unknown.is_match(&all),
            "session context says unknown:\n{all}"
        );
    }
}

mod part_2 {
    #![allow(dead_code, unused_imports)]

    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output, Stdio};

    const PY_MANIFESTS: [&str; 3] = ["pyproject.toml", "setup.py", "setup.cfg"];
    const OTHER_MANIFESTS: [&str; 3] = ["Cargo.toml", "go.mod", "package.json"];

    fn manifest_content(name: &str) -> &'static str {
        match name {
            "pyproject.toml" => "[project]\nname = \"demo\"\nversion = \"0.1.0\"\n",
            "setup.py" => "from setuptools import setup\nsetup(name='demo')\n",
            "setup.cfg" => "[metadata]\nname = demo\n",
            "Cargo.toml" => "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
            "go.mod" => "module example.com/demo\n\ngo 1.21\n",
            "package.json" => "{\n  \"name\": \"demo\",\n  \"version\": \"0.1.0\"\n}\n",
            _ => "",
        }
    }

    fn run_hook(event: &str, dir: &Path, home: &Path, input: &str) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_hexa"))
            .args(["hook", event])
            .current_dir(dir)
            .env("HOME", home)
            .env("CLAUDE_PROJECT_DIR", dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn hexa");
        {
            let mut stdin = child.stdin.take().expect("stdin");
            let _ = stdin.write_all(input.as_bytes());
        }
        child.wait_with_output().expect("wait hexa")
    }

    fn combined(out: &Output) -> String {
        format!(
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    }

    // ---------- session context (§4) ----------

    fn session_context(manifests: &[&str]) -> String {
        let proj = tempfile::tempdir().expect("tempdir");
        let home = tempfile::tempdir().expect("home");
        for m in manifests {
            std::fs::write(proj.path().join(m), manifest_content(m)).expect("write manifest");
        }
        let input = serde_json::json!({
            "session_id": "test-session",
            "transcript_path": "",
            "cwd": proj.path().to_string_lossy(),
            "hook_event_name": "SessionStart",
            "source": "startup"
        })
        .to_string();
        let out = run_hook("session-start", proj.path(), home.path(), &input);
        combined(&out)
    }

    fn shows_python(ctx: &str) -> bool {
        ctx.contains("Language: python")
    }

    /// S17: `setup.py` or `setup.cfg` alone also means Python
    #[test]
    fn s17_setup_py_or_setup_cfg_alone_means_python() {
        let ctx_py = session_context(&["setup.py"]);
        assert!(
            shows_python(&ctx_py),
            "a root with only setup.py should show Language: python; got:\n{ctx_py}"
        );
        let ctx_cfg = session_context(&["setup.cfg"]);
        assert!(
            shows_python(&ctx_cfg),
            "a root with only setup.cfg should show Language: python; got:\n{ctx_cfg}"
        );
    }

    /// S20: Python is shown exactly when a Python manifest is present and the others are absent
    #[test]
    fn s20_python_iff_python_manifest_and_no_others() {
        let all: Vec<&str> = PY_MANIFESTS
            .iter()
            .chain(OTHER_MANIFESTS.iter())
            .copied()
            .collect();
        let mut failures = Vec::new();
        for mask in 0u32..(1 << all.len()) {
            let subset: Vec<&str> = all
                .iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(_, m)| *m)
                .collect();
            let has_py = subset.iter().any(|m| PY_MANIFESTS.contains(m));
            let has_other = subset.iter().any(|m| OTHER_MANIFESTS.contains(m));
            let expected = has_py && !has_other;
            let ctx = session_context(&subset);
            if shows_python(&ctx) != expected {
                failures.push(format!(
                    "subset {:?}: expected python={}, got python={}",
                    subset,
                    expected,
                    shows_python(&ctx)
                ));
            }
        }
        assert!(failures.is_empty(), "mismatches:\n{}", failures.join("\n"));
    }

    // ---------- pre-agent hook (§5) ----------

    fn git(dir: &Path, args: &[&str]) {
        let st = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("run git");
        assert!(st.success(), "git {:?} failed", args);
    }

    struct Fixture {
        proj: tempfile::TempDir,
        home: tempfile::TempDir,
    }

    impl Fixture {
        fn new(branch: &str) -> Self {
            let proj = tempfile::tempdir().expect("tempdir");
            let home = tempfile::tempdir().expect("home");
            let p = proj.path();
            git(p, &["init", "-q", "-b", "main"]);
            git(p, &["config", "user.name", "t"]);
            git(p, &["config", "user.email", "t@example.com"]);
            std::fs::write(p.join("README.md"), "demo\n").unwrap();
            git(p, &["add", "."]);
            git(p, &["commit", "-q", "-m", "init"]);
            if branch != "main" {
                git(p, &["checkout", "-q", "-b", branch]);
            }
            Fixture { proj, home }
        }

        fn project_agents(&self) -> PathBuf {
            self.proj.path().join(".claude").join("agents")
        }

        fn home_agents(&self) -> PathBuf {
            self.home.path().join(".claude").join("agents")
        }

        fn pre_agent(&self, subagent_type: &str) -> Output {
            let input = serde_json::json!({
                "session_id": "test-session",
                "transcript_path": "",
                "cwd": self.proj.path().to_string_lossy(),
                "hook_event_name": "PreToolUse",
                "tool_name": "Agent",
                "tool_input": {
                    "subagent_type": subagent_type,
                    "description": "review the change",
                    "prompt": "Look over the code."
                }
            })
            .to_string();
            run_hook("pre-agent", self.proj.path(), self.home.path(), &input)
        }
    }

    #[derive(Clone, Copy)]
    enum Def<'a> {
        Absent,
        NoTools,
        Tools(&'a [&'a str]),
    }

    fn write_def(dir: &Path, name: &str, def: Def) {
        let body = match def {
        Def::Absent => return,
        Def::NoTools => format!(
            "---\nname: {name}\ndescription: A test agent.\n---\n\nYou are a test agent.\n"
        ),
        Def::Tools(tools) => format!(
            "---\nname: {name}\ndescription: A test agent.\ntools: {}\n---\n\nYou are a test agent.\n",
            tools.join(", ")
        ),
    };
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(format!("{name}.md")), body).unwrap();
    }

    fn is_blocked(out: &Output) -> bool {
        let stdout = String::from_utf8_lossy(&out.stdout);
        out.status.code() == Some(2) || stdout.contains("\"deny\"") || stdout.contains("\"block\"")
    }

    /// Find a branch on which an editing (undefined, non-read-only) agent is blocked.
    fn blocking_fixture() -> Fixture {
        for branch in ["feat/work", "main"] {
            let fx = Fixture::new(branch);
            if is_blocked(&fx.pre_agent("zz-unlisted-editor")) {
                return fx;
            }
        }
        panic!(
            "could not set up a branch on which an editing subagent without a worktree is blocked"
        );
    }

    /// S21: A project definition with only read tools runs in place
    #[test]
    fn s21_project_definition_with_read_tools_runs_in_place() {
        let fx = blocking_fixture();
        write_def(
            &fx.project_agents(),
            "code-reviewer",
            Def::Tools(&["Read", "Grep", "Glob"]),
        );
        let out = fx.pre_agent("code-reviewer");
        assert!(
            !is_blocked(&out),
            "code-reviewer with only Read, Grep, Glob should be allowed; got:\n{}",
            combined(&out)
        );
    }

    /// S22: The user's home definition is used when the project has none
    #[test]
    fn s22_home_definition_used_when_project_has_none() {
        let fx = blocking_fixture();
        write_def(&fx.home_agents(), "auditor", Def::Tools(&["Read", "Grep"]));
        assert!(!fx.project_agents().join("auditor.md").exists());
        let out = fx.pre_agent("auditor");
        assert!(
            !is_blocked(&out),
            "auditor defined in home with Read, Grep should be allowed; got:\n{}",
            combined(&out)
        );
    }

    /// S26: A definition is exempt exactly when it lists tools and none of them writes
    #[test]
    fn s26_exempt_iff_tools_listed_and_none_writes() {
        let fx = blocking_fixture();
        // Baseline: what the hook does for an unlisted type with no definition (before this ADR).
        let baseline_blocked = is_blocked(&fx.pre_agent("zz-unlisted-editor"));
        assert!(baseline_blocked);

        let tool_sets: Vec<&[&str]> = vec![
            &["Read"],
            &["Read", "Grep", "Glob"],
            &["Read", "Grep", "Glob", "WebFetch"],
            &["Edit"],
            &["Write"],
            &["MultiEdit"],
            &["NotebookEdit"],
            &["Bash"],
            &["Read", "Grep", "Bash"],
            &["Read", "Write"],
        ];
        let writers = ["Edit", "Write", "MultiEdit", "NotebookEdit", "Bash"];
        let mut states: Vec<Def> = vec![Def::Absent, Def::NoTools];
        for t in &tool_sets {
            states.push(Def::Tools(t));
        }

        let expect_allowed = |d: Def| match d {
            Def::Tools(t) => !t.iter().any(|x| writers.contains(x)),
            _ => false,
        };

        let mut failures = Vec::new();
        let mut n = 0;
        for (pi, &project) in states.iter().enumerate() {
            for (hi, &home) in states.iter().enumerate() {
                n += 1;
                let name = format!("zz-probe-{pi}-{hi}-{n}");
                write_def(&fx.project_agents(), &name, project);
                write_def(&fx.home_agents(), &name, home);
                let governing = match project {
                    Def::Absent => home,
                    other => other,
                };
                let allowed = expect_allowed(governing);
                let out = fx.pre_agent(&name);
                let blocked = is_blocked(&out);
                let ok = if allowed {
                    !blocked
                } else {
                    blocked == baseline_blocked
                };
                if !ok {
                    failures.push(format!(
                    "project state #{pi}, home state #{hi}: expected allowed={allowed}, got blocked={blocked}"
                ));
                }
            }
        }
        assert!(failures.is_empty(), "mismatches:\n{}", failures.join("\n"));
    }
}
