#![allow(unused, clippy::all)]
mod part_1 {
    #![allow(dead_code, clippy::all)]

    use std::io::Read;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    const HEXA: &str = env!("CARGO_BIN_EXE_hexa");

    /// Run a command, draining stdout/stderr, killing it after `timeout`.
    /// Returns (exit success, stdout, stderr).
    fn run_split(mut cmd: Command, timeout: Duration) -> (bool, String, String) {
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().expect("spawn command");
        let mut out = child.stdout.take().unwrap();
        let mut err = child.stderr.take().unwrap();
        let t_out = std::thread::spawn(move || {
            let mut s = Vec::new();
            let _ = out.read_to_end(&mut s);
            s
        });
        let t_err = std::thread::spawn(move || {
            let mut s = Vec::new();
            let _ = err.read_to_end(&mut s);
            s
        });
        let deadline = Instant::now() + timeout;
        let status = loop {
            match child.try_wait().expect("wait") {
                Some(st) => break Some(st),
                None => {
                    if Instant::now() > deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        break None;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        };
        let o = t_out.join().unwrap_or_default();
        let e = t_err.join().unwrap_or_default();
        let so = String::from_utf8_lossy(&o).into_owned();
        let se = String::from_utf8_lossy(&e).into_owned();
        assert!(
            status.is_some(),
            "command timed out; output so far:\n{so}\n{se}"
        );
        (status.map(|s| s.success()).unwrap_or(false), so, se)
    }

    /// Run a command; returns (exit success, combined stdout+stderr).
    fn run_with_timeout(cmd: Command, timeout: Duration) -> (bool, String) {
        let (ok, o, e) = run_split(cmd, timeout);
        (ok, format!("{o}\n{e}"))
    }

    /// Whether doctor's output warns that .claude/worktrees is not ignored.
    fn warns_not_ignored(output: &str) -> bool {
        output.lines().any(|l| {
            let low = l.to_lowercase();
            low.contains("worktrees")
                && (low.contains("not in .gitignore")
                    || low.contains("not ignored")
                    || low.contains("isn't ignored")
                    || low.contains("is not git-ignored")
                    || low.contains("not git-ignored"))
        })
    }

    struct Fixture {
        root: tempfile::TempDir,
        home: tempfile::TempDir,
        global_cfg: PathBuf,
        git_repo: bool,
    }

    impl Fixture {
        fn new(git_repo: bool) -> Self {
            let root = tempfile::tempdir().expect("tempdir");
            let home = tempfile::tempdir().expect("home tempdir");
            let global_cfg = home.path().join("gitconfig");
            std::fs::write(
            &global_cfg,
            "[user]\n\tname = Test User\n\temail = test@example.com\n[init]\n\tdefaultBranch = main\n",
        )
        .unwrap();
            let f = Fixture {
                root,
                home,
                global_cfg,
                git_repo,
            };
            if git_repo {
                let (ok, out) = f.git(&["init", "-q"]);
                assert!(ok, "git init failed: {out}");
            }
            f
        }

        fn path(&self) -> &Path {
            self.root.path()
        }

        fn write(&self, rel: &str, content: &str) {
            let p = self.path().join(rel);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(p, content).unwrap();
        }

        fn mkdir(&self, rel: &str) {
            std::fs::create_dir_all(self.path().join(rel)).unwrap();
        }

        fn remove(&self, rel: &str) {
            let _ = std::fs::remove_file(self.path().join(rel));
        }

        fn set_global_excludes(&self, content: Option<&str>) {
            let mut cfg = String::from(
            "[user]\n\tname = Test User\n\temail = test@example.com\n[init]\n\tdefaultBranch = main\n",
        );
            if let Some(c) = content {
                let ex = self.home.path().join("global_excludes");
                std::fs::write(&ex, c).unwrap();
                cfg.push_str(&format!("[core]\n\texcludesFile = {}\n", ex.display()));
            }
            std::fs::write(&self.global_cfg, cfg).unwrap();
        }

        fn apply_env(&self, cmd: &mut Command) {
            cmd.current_dir(self.path())
                .env("HOME", self.home.path())
                .env("XDG_CONFIG_HOME", self.home.path().join(".config"))
                .env("CLAUDE_CONFIG_DIR", self.home.path().join(".claude"))
                .env("GIT_CONFIG_GLOBAL", &self.global_cfg)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("GIT_INDEX_FILE")
                .env("NO_COLOR", "1");
            if let Some(parent) = self.path().parent() {
                cmd.env("GIT_CEILING_DIRECTORIES", parent);
            }
        }

        fn git(&self, args: &[&str]) -> (bool, String) {
            let mut cmd = Command::new("git");
            cmd.args(args);
            self.apply_env(&mut cmd);
            run_with_timeout(cmd, Duration::from_secs(60))
        }

        fn check_ignore_says_ignored(&self) -> bool {
            let (ok, _) = self.git(&["check-ignore", "-q", ".claude/worktrees"]);
            ok
        }

        fn doctor(&self) -> String {
            let mut cmd = Command::new(HEXA);
            cmd.arg("doctor");
            self.apply_env(&mut cmd);
            let (_ok, out) = run_with_timeout(cmd, Duration::from_secs(180));
            out
        }
    }

    /// S01: An ignore entry in .git/info/exclude counts
    #[test]
    fn s01_git_info_exclude_counts() {
        let f = Fixture::new(true);
        f.mkdir(".claude/worktrees");
        f.write(".gitignore", "target/\n");

        let before = f.doctor();
        assert!(
            warns_not_ignored(&before),
            "control: doctor should warn when nothing ignores .claude/worktrees:\n{before}"
        );

        f.write(".git/info/exclude", ".claude/worktrees\n");
        assert!(
            f.check_ignore_says_ignored(),
            "git should report it ignored"
        );
        let out = f.doctor();
        assert!(
            !warns_not_ignored(&out),
            "doctor warned although .git/info/exclude ignores .claude/worktrees:\n{out}"
        );
    }

    /// S02: An ignore entry in the global excludes file counts
    #[test]
    fn s02_global_excludes_counts() {
        let f = Fixture::new(true);
        f.mkdir(".claude/worktrees");
        f.write(".gitignore", "target/\n");
        f.write(".git/info/exclude", "");

        let before = f.doctor();
        assert!(
            warns_not_ignored(&before),
            "control: doctor should warn when nothing ignores .claude/worktrees:\n{before}"
        );

        f.set_global_excludes(Some(".claude/worktrees\n"));
        assert!(
            f.check_ignore_says_ignored(),
            "git should report it ignored"
        );
        let out = f.doctor();
        assert!(
            !warns_not_ignored(&out),
            "doctor warned although the global excludes file ignores .claude/worktrees:\n{out}"
        );
    }

    /// S08: Doctor's verdict agrees with git
    #[test]
    fn s08_doctor_agrees_with_git() {
        let gitignores: [Option<&str>; 4] = [
            None,
            Some(".claude/worktrees\n"),
            Some(".claude/\n"),
            Some("!.claude/worktrees\n"),
        ];
        let excludes: [Option<&str>; 2] = [None, Some(".claude/worktrees\n")];
        let globals: [Option<&str>; 2] = [None, Some("/.claude/worktrees/\n")];

        let mut disagreements = Vec::new();
        for gi in gitignores.iter() {
            for ex in excludes.iter() {
                for gl in globals.iter() {
                    let f = Fixture::new(true);
                    f.mkdir(".claude/worktrees");
                    match gi {
                        Some(c) => f.write(".gitignore", c),
                        None => f.remove(".gitignore"),
                    }
                    f.write(".git/info/exclude", ex.unwrap_or(""));
                    f.set_global_excludes(*gl);

                    let ignored = f.check_ignore_says_ignored();
                    let out = f.doctor();
                    let warned = warns_not_ignored(&out);
                    if warned == ignored {
                        disagreements.push(format!(
                        "gitignore={gi:?} exclude={ex:?} global={gl:?}: git ignored={ignored}, doctor warned={warned}\n{out}"
                    ));
                    }
                }
            }
        }
        assert!(
            disagreements.is_empty(),
            "doctor disagreed with git check-ignore:\n{}",
            disagreements.join("\n----\n")
        );
    }

    // ---------- spec draft (§2) ----------

    const ADR_ID: &str = "ADR-2611230900";

    const ADR_TEXT: &str = "# ADR-2611230900: greeting is polite\n\n\
**Status:** Proposed\n\
**Date:** 2026-11-23\n\n\
## Context\n\n\
The `greet` command prints a greeting.\n\n\
## Decision\n\n\
1. **`greet` says hello.** Running `greet NAME` prints `Hello, NAME!` and exits 0.\n\
2. **An empty name is refused.** Running `greet` with an empty name prints an error and exits 2.\n\n\
## Consequences\n\n\
- Users are greeted politely.\n";

    struct SpecRepo {
        root: tempfile::TempDir,
        cfg_dir: tempfile::TempDir,
        global_cfg: PathBuf,
    }

    impl SpecRepo {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let cfg_dir = tempfile::tempdir().unwrap();
            let global_cfg = cfg_dir.path().join("gitconfig");
            std::fs::write(
            &global_cfg,
            "[user]\n\tname = Test User\n\temail = test@example.com\n[init]\n\tdefaultBranch = main\n[commit]\n\tgpgsign = false\n",
        )
        .unwrap();
            let r = SpecRepo {
                root,
                cfg_dir,
                global_cfg,
            };
            assert!(r.git(&["init", "-q"]).0);
            let adr_dir = r.root.path().join("docs/adrs");
            std::fs::create_dir_all(&adr_dir).unwrap();
            std::fs::write(adr_dir.join(format!("{ADR_ID}-greeting.md")), ADR_TEXT).unwrap();
            std::fs::write(r.root.path().join("README.md"), "# sample\n").unwrap();
            // `spec draft` needs a test home (ADR-2611191120 §9).
            std::fs::write(
                r.root.path().join("Cargo.toml"),
                "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
            )
            .unwrap();
            // A fake `claude` on PATH, as other spec draft tests use: the
            // draft is a model call, and a test must not depend on one.
            let bin = r.cfg_dir.path().join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            let scenarios = serde_json::json!([
                {"id":"S01","decision":["§1"],"kind":"example","description":"greet says hello","given":"the name Ada","when":"greet Ada runs","then":"it prints Hello, Ada! and exits 0"},
                {"id":"S02","decision":["§1"],"kind":"negative","description":"no other greeting","given":"the name Ada","when":"greet Ada runs","then":"it prints nothing else"},
                {"id":"S03","decision":["§2"],"kind":"example","description":"an empty name is refused","given":"an empty name","when":"greet runs","then":"it prints an error and exits 2"},
                {"id":"S04","decision":["§2"],"kind":"negative","description":"an empty name does not greet","given":"an empty name","when":"greet runs","then":"it prints no greeting"}
            ]);
            let envelope = serde_json::json!({
                "type":"result","subtype":"success","is_error":false,
                "result": scenarios.to_string(),
                "usage":{"input_tokens":1,"output_tokens":1,"cache_read_input_tokens":0},
                "total_cost_usd":0.0001
            });
            std::fs::write(r.cfg_dir.path().join("reply.json"), envelope.to_string()).unwrap();
            let stub = format!(
                "#!/bin/sh\ncase \" $* \" in *\" --version \"*) echo \"2.1.0 (Claude Code)\"; exit 0;; esac\ncat > /dev/null\ncat '{}'\n",
                r.cfg_dir.path().join("reply.json").display()
            );
            std::fs::write(bin.join("claude"), stub).unwrap();
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(bin.join("claude"), std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            assert!(r.git(&["add", "-A"]).0);
            let (ok, out) = r.git(&["commit", "-q", "-m", "initial"]);
            assert!(ok, "initial commit failed: {out}");
            r
        }

        fn apply_env(&self, cmd: &mut Command) {
            let path = format!(
                "{}:{}",
                self.cfg_dir.path().join("bin").display(),
                std::env::var("PATH").unwrap_or_default()
            );
            cmd.current_dir(self.root.path())
                .env("PATH", path)
                .env("HOME", self.cfg_dir.path())
                .env("GIT_CONFIG_GLOBAL", &self.global_cfg)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("GIT_INDEX_FILE")
                .env("NO_COLOR", "1");
            if let Some(parent) = self.root.path().parent() {
                cmd.env("GIT_CEILING_DIRECTORIES", parent);
            }
        }

        fn git(&self, args: &[&str]) -> (bool, String) {
            let mut cmd = Command::new("git");
            cmd.args(args);
            self.apply_env(&mut cmd);
            run_with_timeout(cmd, Duration::from_secs(60))
        }

        fn hexa(&self, args: &[&str]) -> (bool, String) {
            let mut cmd = Command::new(HEXA);
            cmd.args(args);
            self.apply_env(&mut cmd);
            run_with_timeout(cmd, Duration::from_secs(900))
        }

        /// Paths git sees as new or changed since the last commit.
        fn changed_paths(&self) -> Vec<String> {
            let (_, out) = self.git(&["status", "--porcelain", "--untracked-files=all"]);
            out.lines()
                .filter(|l| l.len() > 3)
                .map(|l| l[3..].trim().trim_matches('"').to_string())
                .collect()
        }

        fn draft(&self) -> (String, String) {
            let (ok, out) = self.hexa(&["spec", "draft", ADR_ID, "--model", "claude-text"]);
            assert!(ok, "`hexa spec draft {ADR_ID}` failed:\n{out}");
            let changed = self.changed_paths();
            let spec = changed
                .iter()
                .find(|p| p.contains(ADR_ID) && !p.ends_with(".md"))
                .or_else(|| changed.iter().find(|p| p.contains("spec")))
                .or_else(|| changed.iter().find(|p| p.contains(ADR_ID)))
                .cloned();
            assert!(
                spec.is_some(),
                "draft wrote no spec file git can see:\n{out}"
            );
            (out, spec.unwrap())
        }
    }

    /// S09: The draft's closing line names the file, says to commit, then says to accept
    #[test]
    fn s09_draft_closing_line_names_file_commit_accept() {
        let repo = SpecRepo::new();
        let (out, spec) = repo.draft();

        let accept = format!("hexa spec accept {ADR_ID}");
        let lines: Vec<&str> = out.lines().filter(|l| !l.trim().is_empty()).collect();
        let accept_line = lines.iter().rposition(|l| l.contains(&accept));
        assert!(
            accept_line.is_some(),
            "draft output does not name `{accept}`:\n{out}"
        );
        let idx = accept_line.unwrap();

        // The closing instruction: the line naming `hexa spec accept`, with up to
        // three lines before it in case the instruction wraps.
        let start = idx.saturating_sub(3);
        let window = lines[start..=idx].join("\n");
        let accept_pos = window.rfind(&accept).unwrap_or(0);
        let before_accept = &window[..accept_pos];
        let before_low = before_accept.to_lowercase();

        let file_name = Path::new(&spec)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| spec.clone());

        assert!(
        before_accept.contains(&spec) || before_accept.contains(&file_name),
        "the closing instruction does not name the spec file {spec} before `{accept}`:\n{window}"
    );
        assert!(
            before_low.contains("commit"),
            "the closing instruction does not say to commit the spec before `{accept}`:\n{window}"
        );
    }

    /// S10: Following the instruction succeeds
    #[test]
    fn s10_following_instruction_accept_not_refused_as_uncommitted() {
        let repo = SpecRepo::new();
        let (_out, spec) = repo.draft();

        let (ok, out) = repo.git(&["add", "--", &spec]);
        assert!(ok, "git add failed: {out}");
        let (ok, out) = repo.git(&["commit", "-q", "-m", "spec draft", "--", &spec]);
        assert!(ok, "git commit failed: {out}");

        let (_ok, out) = repo.hexa(&["spec", "accept", ADR_ID, "--by", "Test User"]);
        let low = out.to_lowercase();
        assert!(
            !low.contains("untracked")
                && !low.contains("not committed")
                && !low.contains("uncommitted")
                && !low.contains("not tracked")
                && !low.contains("git add"),
            "`hexa spec accept` refused a committed spec as untracked or modified:\n{out}"
        );
        assert!(
            !(low.contains("modified") && (low.contains("refuse") || low.contains("commit"))),
            "`hexa spec accept` refused a committed spec as modified:\n{out}"
        );
    }
}

mod part_2 {
    #![allow(dead_code, unused_imports, clippy::all)]

    use hexa_analysis::fingerprint_extractor::{ArchitectureFingerprint, FingerprintExtractor};
    use std::path::{Path, PathBuf};

    // ---------------------------------------------------------------------------
    // helpers
    // ---------------------------------------------------------------------------

    fn crate_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    fn workspace_root() -> PathBuf {
        crate_dir()
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(crate_dir)
    }

    fn spec_ledger_source() -> String {
        let path = crate_dir().join("src/commands/spec_ledger.rs");
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
    }

    /// The text of a function starting at `signature`, up to the next top-level
    /// `pub` item (or the end of the file).
    fn region<'a>(source: &'a str, signature: &str) -> &'a str {
        let start = source
            .find(signature)
            .unwrap_or_else(|| panic!("`{signature}` not found in spec_ledger.rs"));
        let rest = &source[start + signature.len()..];
        let end = rest
            .find("\npub ")
            .map(|i| start + signature.len() + i)
            .unwrap_or(source.len());
        &source[start..end]
    }

    /// The statement containing byte `at`, together with the three statements
    /// before it.
    fn statement_window(text: &str, at: usize) -> &str {
        let before = &text[..at];
        let mut start = 0;
        let mut count = 0;
        for (i, c) in before.char_indices().rev() {
            if c == ';' {
                count += 1;
                if count == 4 {
                    start = i + 1;
                    break;
                }
            }
        }
        let end = text[at..].find(';').map(|i| at + i).unwrap_or(text.len());
        &text[start..end]
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime")
    }

    fn write_files(root: &Path, files: &[(&str, &str)]) {
        for (rel, content) in files {
            let p = root.join(rel);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&p, content).unwrap();
        }
    }

    fn rust_lib_files() -> Vec<(&'static str, &'static str)> {
        vec![
        (
            "Cargo.toml",
            "[package]\nname = \"payments\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n",
        ),
        ("src/lib.rs", "pub fn add(a: u32, b: u32) -> u32 { a + b }\n"),
    ]
    }

    fn rust_web_files() -> Vec<(&'static str, &'static str)> {
        vec![
        (
            "Cargo.toml",
            "[package]\nname = \"payments\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\naxum = \"0.7\"\ntokio = { version = \"1\", features = [\"full\"] }\n",
        ),
        (
            "src/main.rs",
            "use axum::{routing::get, Router};\n\n#[tokio::main]\nasync fn main() {\n    let app = Router::new().route(\"/\", get(|| async { \"ok\" }));\n    let listener = tokio::net::TcpListener::bind(\"0.0.0.0:3000\").await.unwrap();\n    axum::serve(listener, app).await.unwrap();\n}\n",
        ),
    ]
    }

    fn python_sdk_files() -> Vec<(&'static str, &'static str)> {
        vec![
            (
                "pyproject.toml",
                "[project]\nname = \"client-sdk\"\nversion = \"0.1.0\"\ndependencies = []\n",
            ),
            ("client_sdk/__init__.py", "def ping():\n    return 1\n"),
        ]
    }

    fn infer(objective: &str, files: &[(&str, &str)]) -> ArchitectureFingerprint {
        let project = tempfile::tempdir().unwrap();
        write_files(project.path(), files);

        let wp_dir = tempfile::tempdir().unwrap();
        let wp_path = wp_dir.path().join("workplan.json");
        let wp = serde_json::json!({
            "id": "wp-test",
            "description": objective,
            "title": "test workplan",
            "status": "active",
            "phases": [],
            "steps": []
        });
        std::fs::write(&wp_path, serde_json::to_string_pretty(&wp).unwrap()).unwrap();

        let fp = runtime().block_on(FingerprintExtractor::extract(
            "00000000-0000-0000-0000-000000000000",
            project.path(),
            Some(&wp_path),
        ));
        assert!(
            fp.workplan_objective.contains(objective),
            "the fingerprint did not read the objective {objective:?}; got {:?}",
            fp.workplan_objective
        );
        fp
    }

    const LIBRARY_WORDS: [&str; 5] = ["library", "crate", "package", "SDK", "framework"];
    const API_WORDS: [&str; 5] = ["API", "server", "REST", "HTTP", "endpoint"];

    // ---------------------------------------------------------------------------
    // pipe helpers (Unix)
    // ---------------------------------------------------------------------------

    #[cfg(unix)]
    mod pipe {
        use std::io::{BufRead, BufReader};
        use std::os::unix::process::{CommandExt, ExitStatusExt};
        use std::path::Path;
        use std::process::{Command, ExitStatus, Stdio};

        pub fn command(args: &[&str], cwd: &Path, cores: bool) -> Command {
            let mut cmd = Command::new(env!("CARGO_BIN_EXE_hexa"));
            cmd.args(args)
                .current_dir(cwd)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .env("NO_COLOR", "1");
            unsafe {
                cmd.pre_exec(move || {
                    let mut lim = libc::rlimit {
                        rlim_cur: 0,
                        rlim_max: 0,
                    };
                    if cores {
                        if libc::getrlimit(libc::RLIMIT_CORE, &mut lim) == 0 {
                            lim.rlim_cur = lim.rlim_max;
                            let _ = libc::setrlimit(libc::RLIMIT_CORE, &lim);
                        }
                    } else {
                        let _ = libc::setrlimit(libc::RLIMIT_CORE, &lim);
                    }
                    Ok(())
                });
            }
            cmd
        }

        /// Like `| head`: read ten lines, then close the pipe.
        pub fn head(args: &[&str], cwd: &Path, cores: bool) -> (Vec<String>, ExitStatus, String) {
            let mut child = command(args, cwd, cores).spawn().expect("spawn hexa");
            let stdout = child.stdout.take().unwrap();
            let mut lines = Vec::new();
            {
                let mut reader = BufReader::new(stdout);
                for _ in 0..10 {
                    let mut l = String::new();
                    match reader.read_line(&mut l) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => lines.push(l),
                    }
                }
            }
            let out = child.wait_with_output().expect("wait hexa");
            (
                lines,
                out.status,
                String::from_utf8_lossy(&out.stderr).into_owned(),
            )
        }

        /// The reader closes the pipe before hexa writes anything.
        pub fn closed(args: &[&str], cwd: &Path, cores: bool) -> (ExitStatus, String) {
            let mut child = command(args, cwd, cores).spawn().expect("spawn hexa");
            drop(child.stdout.take());
            let out = child.wait_with_output().expect("wait hexa");
            (
                out.status,
                String::from_utf8_lossy(&out.stderr).into_owned(),
            )
        }

        pub fn full(args: &[&str], cwd: &Path) -> (ExitStatus, String, String) {
            let out = command(args, cwd, false).output().expect("run hexa");
            (
                out.status,
                String::from_utf8_lossy(&out.stdout).into_owned(),
                String::from_utf8_lossy(&out.stderr).into_owned(),
            )
        }

        pub fn assert_quiet(status: ExitStatus, stderr: &str, what: &str) {
            assert!(
                !stderr.contains("panicked") && !stderr.contains("failed printing"),
                "{what}: hexa printed a panic message on stderr:\n{stderr}"
            );
            assert_ne!(
                status.signal(),
                Some(libc::SIGABRT),
                "{what}: hexa aborted (status {status:?}); stderr:\n{stderr}"
            );
            assert!(
                !status.core_dumped(),
                "{what}: hexa dumped core (status {status:?})"
            );
        }

        pub fn core_files(dir: &Path) -> Vec<String> {
            std::fs::read_dir(dir)
                .map(|rd| {
                    rd.filter_map(|e| e.ok())
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .filter(|n| n == "core" || n.starts_with("core."))
                        .collect()
                })
                .unwrap_or_default()
        }
    }

    // ---------------------------------------------------------------------------
    // tests
    // ---------------------------------------------------------------------------

    /// S11: The draft no longer tells the user to accept an uncommitted spec directly
    #[test]
    fn s11_draft_closing_says_commit_before_accept() {
        let src = spec_ledger_source();
        let mut regions: Vec<&str> = vec![region(&src, "pub async fn draft(")];
        let mut idx = 0;
        while let Some(i) = src[idx..].find("Read every scenario") {
            let at = idx + i;
            let start = at.saturating_sub(600);
            let end = (at + 600).min(src.len());
            // keep to char boundaries
            let start = (0..=start)
                .rev()
                .find(|&s| src.is_char_boundary(s))
                .unwrap_or(0);
            let end = (end..=src.len())
                .find(|&e| src.is_char_boundary(e))
                .unwrap_or(src.len());
            regions.push(&src[start..end]);
            idx = at + 1;
        }

        for text in regions {
            let mut from = 0;
            while let Some(i) = text[from..].find("hexa spec accept") {
                let at = from + i;
                let window = statement_window(text, at);
                assert!(
                window.to_lowercase().contains("commit"),
                "draft's closing output tells the user to run `hexa spec accept` without saying to commit the spec first:\n{window}"
            );
                from = at + 1;
            }
        }
    }

    /// S13: An SDK for an API is a library
    #[test]
    fn s13_sdk_for_an_api_is_a_library() {
        let fp = infer("a Python SDK for the Example API", &python_sdk_files());
        assert_eq!(fp.output_type, "library", "fingerprint: {fp:?}");
        for c in &fp.constraints {
            assert!(
                !c.contains("HTTP status codes") && !c.contains("JSON responses"),
                "a web-API constraint was injected: {c:?}"
            );
        }
        let block = fp.to_injection_block();
        assert!(
            !block.contains("Return JSON responses with appropriate HTTP status codes"),
            "session context still carries the HTTP-response constraint:\n{block}"
        );
    }

    /// S14: Each library word outranks API words
    #[test]
    fn s14_each_library_word_outranks_each_api_word() {
        let files = rust_lib_files();
        for lib in LIBRARY_WORDS {
            for api in API_WORDS {
                let objective = format!("a {lib} for the payments {api}");
                let fp = infer(&objective, &files);
                assert_eq!(
                    fp.output_type, "library",
                    "objective {objective:?} was inferred as {:?}",
                    fp.output_type
                );
            }
        }
    }

    /// S15: A detected web framework still wins
    #[test]
    fn s15_detected_web_framework_still_wins() {
        let fp = infer("a client library for the HTTP API", &rust_web_files());
        assert_eq!(fp.output_type, "web-api", "fingerprint: {fp:?}");
        assert_ne!(fp.output_type, "library");
    }

    /// S16: The CLI check still comes first
    #[test]
    fn s16_cli_check_still_comes_first() {
        let fp = infer(
            "a command-line tool built on the payments SDK and its API",
            &rust_lib_files(),
        );
        assert_ne!(
            fp.output_type, "library",
            "the CLI check no longer comes before the library words: {fp:?}"
        );
        assert_eq!(fp.output_type, "cli", "fingerprint: {fp:?}");
    }

    /// S17: An API objective with no library word is still a web API
    #[test]
    fn s17_api_objective_without_library_word_is_web_api() {
        let fp = infer("a REST server exposing HTTP endpoints", &rust_lib_files());
        assert_eq!(fp.output_type, "web-api", "fingerprint: {fp:?}");
    }

    /// S18: Library words outrank API words for every objective
    #[test]
    fn s18_library_words_outrank_api_words_for_every_objective() {
        let files = rust_lib_files();
        let prefixes = ["a", "an async", "the official", "a typed", "a small"];
        let middles = ["for the", "wrapping the", "that talks to the", "around the"];
        let mut seed: u64 = 0x2611_2308_31;
        let mut next = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) as usize
        };
        for _ in 0..30 {
            let lib = LIBRARY_WORDS[next() % LIBRARY_WORDS.len()];
            let n_api = next() % 4; // zero to three API words
            let mut apis: Vec<&str> = Vec::new();
            for _ in 0..n_api {
                let w = API_WORDS[next() % API_WORDS.len()];
                if !apis.contains(&w) {
                    apis.push(w);
                }
            }
            let prefix = prefixes[next() % prefixes.len()];
            let middle = middles[next() % middles.len()];
            let objective = if apis.is_empty() {
                format!("{prefix} {lib} for payments")
            } else {
                format!("{prefix} {lib} {middle} payments {}", apis.join(" "))
            };
            let fp = infer(&objective, &files);
            assert_eq!(
                fp.output_type, "library",
                "objective {objective:?} was inferred as {:?}",
                fp.output_type
            );
        }
    }

    /// S19: Piping analyze into head leaves no core
    #[cfg(unix)]
    #[test]
    fn s19_analyze_into_head_leaves_no_core() {
        let cwd = tempfile::tempdir().unwrap();
        let root = workspace_root();
        let root_s = root.to_string_lossy().into_owned();
        let args = ["analyze", root_s.as_str()];

        let (lines, status, stderr) = pipe::head(&args, cwd.path(), true);
        assert!(
            !lines.is_empty(),
            "head printed nothing from `hexa analyze`; stderr:\n{stderr}"
        );
        pipe::assert_quiet(status, &stderr, "analyze | head");

        let (status, stderr) = pipe::closed(&args, cwd.path(), true);
        pipe::assert_quiet(status, &stderr, "analyze into a closed pipe");

        let cores = pipe::core_files(cwd.path());
        assert!(cores.is_empty(), "core files appeared: {cores:?}");
    }

    /// S20: Piping spec show into head leaves no core
    #[cfg(unix)]
    #[test]
    fn s20_spec_show_into_head_leaves_no_core() {
        let root = workspace_root();
        let adr = "ADR-2611230831";
        let args = ["spec", "show", adr];

        let (status, stdout, stderr) = pipe::full(&args, &root);
        assert!(
            status.success(),
            "`hexa spec show {adr}` failed: {status:?}\n{stderr}"
        );
        assert!(
            stdout.lines().count() > 10,
            "`hexa spec show {adr}` printed ten lines or fewer"
        );

        let before = pipe::core_files(&root);

        let (_lines, status, stderr) = pipe::head(&args, &root, false);
        pipe::assert_quiet(status, &stderr, "spec show | head");

        let (status, stderr) = pipe::closed(&args, &root, false);
        pipe::assert_quiet(status, &stderr, "spec show into a closed pipe");

        let after = pipe::core_files(&root);
        let new: Vec<_> = after.iter().filter(|c| !before.contains(c)).collect();
        assert!(new.is_empty(), "core files appeared: {new:?}");
    }
}

mod part_3 {
    #![allow(dead_code, unused_imports, clippy::all)]

    use std::fs;
    use std::io::Read;
    use std::path::{Path, PathBuf};
    use std::process::{Command, ExitStatus, Stdio};

    fn hexa_bin() -> &'static str {
        env!("CARGO_BIN_EXE_hexa")
    }

    fn manifest_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    fn core_files(dir: &Path) -> Vec<String> {
        let mut out = Vec::new();
        if let Ok(entries) = fs::read_dir(dir) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if name == "core" || name.starts_with("core.") {
                    out.push(name);
                }
            }
        }
        out
    }

    /// Run hexa with stdout attached to a pipe whose read end is closed before
    /// hexa writes anything. Returns the exit status and captured stderr.
    fn run_with_closed_pipe(args: &[&str], cwd: &Path) -> (ExitStatus, String) {
        let mut child = Command::new(hexa_bin())
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("NO_COLOR", "1")
            .spawn()
            .expect("spawn hexa");
        // Close the reader immediately.
        drop(child.stdout.take());
        let output = child.wait_with_output().expect("wait for hexa");
        (
            output.status,
            String::from_utf8_lossy(&output.stderr).to_string(),
        )
    }

    /// Run hexa with stdout attached to a pipe that is read to the end.
    fn run_fully_read(args: &[&str], cwd: &Path) -> (ExitStatus, Vec<u8>, String) {
        let output = Command::new(hexa_bin())
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("NO_COLOR", "1")
            .output()
            .expect("run hexa");
        (
            output.status,
            output.stdout,
            String::from_utf8_lossy(&output.stderr).to_string(),
        )
    }

    /// Run hexa with stdout written to a file (not a pipe).
    fn run_to_file(args: &[&str], cwd: &Path, out: &Path) -> (ExitStatus, String) {
        let file = fs::File::create(out).expect("create output file");
        let output = Command::new(hexa_bin())
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::from(file))
            .stderr(Stdio::piped())
            .env("NO_COLOR", "1")
            .output()
            .expect("run hexa");
        (
            output.status,
            String::from_utf8_lossy(&output.stderr).to_string(),
        )
    }

    #[cfg(unix)]
    fn signal_of(status: &ExitStatus) -> Option<i32> {
        use std::os::unix::process::ExitStatusExt;
        status.signal()
    }

    fn git(dir: &Path, args: &[&str]) {
        let st = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("run git");
        assert!(st.success(), "git {:?} failed", args);
    }

    /// S21: A closed pipe does not produce a panic
    #[cfg(unix)]
    #[test]
    fn s21_closed_pipe_does_not_panic() {
        let tmp = tempfile::tempdir().unwrap();
        let target = manifest_dir();
        let target_s = target.to_string_lossy().to_string();
        let before = core_files(tmp.path());
        let (status, stderr) = run_with_closed_pipe(&["analyze", &target_s], tmp.path());
        assert!(
            !stderr.contains("panicked"),
            "hexa reported a panic on a closed pipe: {stderr}"
        );
        assert_ne!(
            signal_of(&status),
            Some(libc::SIGABRT),
            "hexa aborted on a closed pipe: {stderr}"
        );
        assert_eq!(
            signal_of(&status),
            Some(libc::SIGPIPE),
            "hexa should end by the default SIGPIPE disposition, got {status:?}; stderr: {stderr}"
        );
        let after = core_files(tmp.path());
        assert_eq!(before, after, "a core file was left behind: {after:?}");
    }

    /// S23: Every hexa command ends quietly on a closed pipe
    #[cfg(unix)]
    #[test]
    fn s23_every_command_ends_quietly_on_closed_pipe() {
        let tmp = tempfile::tempdir().unwrap();
        let target = manifest_dir();
        let target_s = target.to_string_lossy().to_string();
        let invocations: Vec<Vec<&str>> = vec![
            vec!["--help"],
            vec!["--version"],
            vec!["spec", "--help"],
            vec!["analyze", target_s.as_str()],
        ];
        for args in invocations {
            let before = core_files(tmp.path());
            let (status, stderr) = run_with_closed_pipe(&args, tmp.path());
            assert!(
                !stderr.contains("panicked"),
                "`hexa {:?}` reported a panic: {stderr}",
                args
            );
            assert_ne!(
                signal_of(&status),
                Some(libc::SIGABRT),
                "`hexa {:?}` aborted",
                args
            );
            assert_eq!(
                signal_of(&status),
                Some(libc::SIGPIPE),
                "`hexa {:?}` should end by SIGPIPE, got {status:?}; stderr: {stderr}",
                args
            );
            let after = core_files(tmp.path());
            assert_eq!(before, after, "`hexa {:?}` left a core file", args);
        }
    }

    /// S24: Other doctor checks report as before
    #[test]
    fn s24_other_doctor_checks_report_as_before() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        git(root, &["init", "-q"]);
        // A CLAUDE.md far over the advised length: another doctor check fails.
        let long: String = (0..400).map(|i| format!("line {i}\n")).collect();
        fs::write(root.join("CLAUDE.md"), long).unwrap();
        // Worktrees ignored through .git/info/exclude, not .gitignore.
        fs::create_dir_all(root.join(".claude/worktrees")).unwrap();
        fs::create_dir_all(root.join(".git/info")).unwrap();
        fs::write(root.join(".git/info/exclude"), ".claude/worktrees/\n").unwrap();

        let home = tempfile::tempdir().unwrap();
        let output = Command::new(hexa_bin())
            .arg("doctor")
            .current_dir(root)
            .env("NO_COLOR", "1")
            .env("HOME", home.path())
            .stdin(Stdio::null())
            .output()
            .expect("run hexa doctor");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!text.contains("panicked"), "doctor panicked: {text}");
        assert!(
            text.contains("CLAUDE.md"),
            "doctor no longer reports on CLAUDE.md: {text}"
        );
        assert!(
            !text.contains("not in .gitignore"),
            "doctor still warns about .claude/worktrees ignored via .git/info/exclude: {text}"
        );
    }

    /// S26: Failing commands keep their exit status
    #[test]
    fn s26_failing_commands_keep_exit_status() {
        let tmp = tempfile::tempdir().unwrap();
        // `spec show` of a missing ADR exits 0 today; `spec verify` of one
        // fails, which is the failing command this scenario needs.
        let args = ["spec", "verify", "ADR-0001010000"];
        let (piped_status, _out, piped_err) = run_fully_read(&args, tmp.path());
        let out_file = tmp.path().join("show.out");
        let (file_status, file_err) = run_to_file(&args, tmp.path(), &out_file);

        assert!(!piped_err.contains("panicked"), "panic: {piped_err}");
        assert!(!file_err.contains("panicked"), "panic: {file_err}");
        let code = piped_status
            .code()
            .expect("a failing command should exit with a status, not a signal");
        assert_ne!(code, 0, "spec verify of a missing ADR should fail");
        assert_eq!(Some(code), file_status.code());
    }

    fn infer_output_type(
        objective: &str,
    ) -> hexa_analysis::fingerprint_extractor::ArchitectureFingerprint {
        let tmp = tempfile::tempdir().unwrap();
        let wp = tmp.path().join("workplan.json");
        let body = serde_json::json!({
            "id": "wp-test",
            "title": objective,
            "description": objective,
            "phases": [],
            "steps": []
        });
        fs::write(&wp, serde_json::to_string_pretty(&body).unwrap()).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(
            hexa_analysis::fingerprint_extractor::FingerprintExtractor::extract(
                "00000000-0000-0000-0000-000000000000",
                tmp.path(),
                Some(&wp),
            ),
        )
    }

    /// S27: Objectives with no library word keep their output type
    #[test]
    fn s27_objectives_without_library_word_keep_output_type() {
        let cases = [
            (
                "A REST server that returns JSON over HTTP endpoints",
                "web-api",
            ),
            ("An HTTP api server for orders", "web-api"),
            ("A cli that queries the Example api", "cli"),
        ];
        for (objective, expected) in cases {
            let fp = infer_output_type(objective);
            assert!(
                fp.workplan_objective.contains("Example")
                    || fp.workplan_objective.contains("server")
                    || fp.workplan_objective.contains("REST"),
                "objective was not read from the workplan: {:?}",
                fp.workplan_objective
            );
            assert_eq!(
                fp.output_type, expected,
                "output type changed for objective {objective:?}"
            );
        }
    }
}
