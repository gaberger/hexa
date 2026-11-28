#![allow(unused, clippy::all)]
#![allow(unused, clippy::all)]
#![allow(dead_code, unused_imports)]

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn make_executable(path: &Path, body: &str) {
    fs::write(path, body).expect("write script");
    let mut perms = fs::metadata(path).expect("meta").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).expect("chmod");
}

fn language_for(program: &str) -> &'static str {
    match program {
        "cargo" => "rust",
        "pytest" | "python" | "python3" => "python",
        "go" => "go",
        _ => "typescript",
    }
}

fn target_for(program: &str) -> &'static str {
    match program {
        "cargo" => "src/lib.rs",
        "pytest" | "python" | "python3" => "demo/core.py",
        "go" => "main.go",
        _ => "src/index.ts",
    }
}

/// Write one fixture in the JSON format `hexa bench agentic` reads:
/// `<corpus>/<id>.json`, with its evidence as the oracle command.
fn write_json_fixture(corpus: &Path, id: &str, evidence: &str) {
    let program = evidence.split_whitespace().next().unwrap_or("");
    let fixture = serde_json::json!({
        "id": id,
        "status": "verified",
        "title": id,
        "tier": "T1",
        "language": language_for(program),
        "target_file": target_for(program),
        "instruction": "Make the failing test pass.",
        "oracle": {
            "command": evidence
        }
    });
    fs::write(
        corpus.join(format!("{id}.json")),
        serde_json::to_string_pretty(&fixture).unwrap(),
    )
    .unwrap();
}

/// A stub program on the bench PATH that records it was run.
fn recording_stub(bin_dir: &Path, name: &str, marker: &Path) {
    let body = format!("#!/bin/sh\n: > '{}'\nexit 0\n", marker.display());
    make_executable(&bin_dir.join(name), &body);
}

/// Locate a program on the test process's own PATH.
fn find_on_real_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

/// Link basic tools (git, sh) into the bench PATH so hexa itself can work.
fn link_basic_tools(bin_dir: &Path) {
    for tool in ["git", "sh"] {
        if let Some(real) = find_on_real_path(tool) {
            let _ = std::os::unix::fs::symlink(real, bin_dir.join(tool));
        }
    }
}

struct Workspace {
    _tmp: tempfile::TempDir,
    project: PathBuf,
    corpus: PathBuf,
    bin_dir: PathBuf,
    home: PathBuf,
    markers: PathBuf,
}

impl Workspace {
    fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let project = tmp.path().join("project");
        let corpus = tmp.path().join("corpus");
        let bin_dir = tmp.path().join("bin");
        let home = tmp.path().join("home");
        let markers = tmp.path().join("markers");
        for d in [&project, &corpus, &bin_dir, &home, &markers] {
            fs::create_dir_all(d).unwrap();
        }
        link_basic_tools(&bin_dir);
        // Model front-ends: running any of these means a model was invoked.
        for model_cli in ["claude", "ollama", "codex", "gemini"] {
            recording_stub(
                &bin_dir,
                model_cli,
                &markers.join(format!("model-{model_cli}")),
            );
        }
        Workspace {
            _tmp: tmp,
            project,
            corpus,
            bin_dir,
            home,
            markers,
        }
    }

    fn present_program(&self, name: &str) {
        recording_stub(
            &self.bin_dir,
            name,
            &self.markers.join(format!("prog-{name}")),
        );
    }

    fn program_ran(&self, name: &str) -> bool {
        self.markers.join(format!("prog-{name}")).exists()
    }

    fn model_invoked(&self) -> bool {
        fs::read_dir(&self.markers)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().starts_with("model-"))
    }
}

struct JsonBenchOutcome {
    success: bool,
    timed_out: bool,
    output: String,
}

fn bench_agentic_json(ws: &Workspace, timeout: Duration) -> JsonBenchOutcome {
    let out_path = ws.markers.parent().unwrap().join("bench-stdout.txt");
    let err_path = ws.markers.parent().unwrap().join("bench-stderr.txt");
    let out_file = fs::File::create(&out_path).unwrap();
    let err_file = fs::File::create(&err_path).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args([
            "bench",
            "agentic",
            "--model",
            "test-model-does-not-exist",
            "--corpus",
        ])
        .arg(&ws.corpus)
        .args(["--arms", "react,fast"])
        .current_dir(&ws.project)
        .env("PATH", &ws.bin_dir)
        .env("HOME", &ws.home)
        .stdin(Stdio::null())
        .stdout(Stdio::from(out_file))
        .stderr(Stdio::from(err_file))
        .spawn()
        .expect("spawn hexa bench agentic");
    let start = Instant::now();
    let mut timed_out = false;
    let status = loop {
        if let Some(st) = child.try_wait().expect("try_wait") {
            break Some(st);
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            timed_out = true;
            break None;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let output = format!(
        "{}\n{}",
        fs::read_to_string(&out_path).unwrap_or_default(),
        fs::read_to_string(&err_path).unwrap_or_default()
    );
    JsonBenchOutcome {
        success: status.map(|s| s.success()).unwrap_or(false),
        timed_out,
        output,
    }
}

fn reports_pass_rate(output: &str) -> bool {
    let re = regex::Regex::new(r"(?i)(pass(ing)?\s*rate|\d+(\.\d+)?\s*%)").unwrap();
    re.is_match(output)
}

fn line_reports_missing(output: &str, program: &str) -> bool {
    output.lines().any(|l| {
        let low = l.to_lowercase();
        l.contains(program)
            && (low.contains("not found")
                || low.contains("missing")
                || low.contains("not on path")
                || low.contains("not installed")
                || low.contains("cannot find")
                || low.contains("no such"))
    })
}

// ---------------------------------------------------------------------------
// §2 bench checks evidence programs
// ---------------------------------------------------------------------------

/// S07: A missing evidence program stops the bench before any fixture runs
#[test]
fn s07_missing_bun_refuses_before_any_fixture() {
    let ws = Workspace::new();
    let ids = [
        "alpha-ts-fixture",
        "bravo-ts-fixture",
        "charlie-ts-fixture",
        "delta-ts-fixture",
        "echo-ts-fixture",
    ];
    for id in &ids {
        write_json_fixture(&ws.corpus, id, &format!("bun test tests/{id}.test.ts"));
    }
    assert!(!ws.bin_dir.join("bun").exists());

    let run = bench_agentic_json(&ws, Duration::from_secs(120));
    assert!(
        !run.timed_out,
        "bench did not stop; output:\n{}",
        run.output
    );
    assert!(!run.success, "bench should refuse; output:\n{}", run.output);
    assert!(
        run.output.contains("bun"),
        "refusal does not name bun:\n{}",
        run.output
    );
    for id in &ids {
        assert!(
            run.output.contains(id),
            "refusal does not name fixture {id}:\n{}",
            run.output
        );
    }
    assert!(!ws.model_invoked(), "a model was invoked:\n{}", run.output);
    assert!(
        !reports_pass_rate(&run.output),
        "a pass rate was reported:\n{}",
        run.output
    );
}

/// S09: One missing program refuses the whole run and names only the fixtures that need it
#[test]
fn s09_one_missing_program_refuses_whole_run() {
    let ws = Workspace::new();
    ws.present_program("cargo");
    let a = "alpha-rust-fixture";
    let b = "bravo-rust-fixture";
    let c = "charlie-python-fixture";
    write_json_fixture(&ws.corpus, a, "cargo test --quiet alpha");
    write_json_fixture(&ws.corpus, b, "cargo test --quiet bravo");
    write_json_fixture(&ws.corpus, c, "pytest tests/test_charlie.py");
    assert!(!ws.bin_dir.join("pytest").exists());

    let run = bench_agentic_json(&ws, Duration::from_secs(120));
    assert!(
        !run.timed_out,
        "bench did not stop; output:\n{}",
        run.output
    );
    assert!(!run.success, "bench should refuse; output:\n{}", run.output);
    assert!(
        run.output.contains("pytest"),
        "refusal does not name pytest:\n{}",
        run.output
    );
    assert!(
        run.output.contains(c),
        "refusal does not name fixture {c}:\n{}",
        run.output
    );
    assert!(
        !run.output.contains(a),
        "refusal names fixture {a}, which can run:\n{}",
        run.output
    );
    assert!(
        !run.output.contains(b),
        "refusal names fixture {b}, which can run:\n{}",
        run.output
    );
    assert!(
        !ws.program_ran("cargo"),
        "cargo evidence ran although the bench refused:\n{}",
        run.output
    );
    assert!(!ws.model_invoked(), "a model was invoked:\n{}", run.output);
    assert!(
        !reports_pass_rate(&run.output),
        "a pass rate was reported:\n{}",
        run.output
    );
}

/// S11: The bench runs nothing unless every evidence program resolves
#[test]
fn s11_runs_nothing_unless_every_program_resolves() {
    // (fixtures as (id, program), programs present on PATH)
    let cases: Vec<(Vec<(&str, &str)>, Vec<&str>)> = vec![
        (vec![("kilo-one", "bun")], vec![]),
        (
            vec![("kilo-one", "bun"), ("lima-two", "pytest")],
            vec!["bun"],
        ),
        (
            vec![
                ("kilo-one", "bun"),
                ("lima-two", "pytest"),
                ("mike-three", "go"),
            ],
            vec![],
        ),
        (
            vec![
                ("kilo-one", "cargo"),
                ("lima-two", "cargo"),
                ("mike-three", "deno"),
            ],
            vec!["cargo"],
        ),
        (
            vec![
                ("kilo-one", "npm"),
                ("lima-two", "go"),
                ("mike-three", "pytest"),
            ],
            vec!["npm", "pytest"],
        ),
        (
            vec![("kilo-one", "bun"), ("lima-two", "cargo")],
            vec!["bun", "cargo"],
        ),
    ];

    let mut failures = Vec::new();
    for (i, (fixtures, present)) in cases.iter().enumerate() {
        let ws = Workspace::new();
        for p in present {
            ws.present_program(p);
        }
        for (id, prog) in fixtures {
            write_json_fixture(&ws.corpus, id, &format!("{prog} test {id}"));
        }
        let missing: BTreeSet<&str> = fixtures
            .iter()
            .map(|(_, p)| *p)
            .filter(|p| !present.contains(p))
            .collect();

        if missing.is_empty() {
            let run = bench_agentic_json(&ws, Duration::from_secs(90));
            for p in present {
                if line_reports_missing(&run.output, p) {
                    failures.push(format!(
                        "case {i}: every program present but {p} reported missing:\n{}",
                        run.output
                    ));
                }
            }
            continue;
        }

        let run = bench_agentic_json(&ws, Duration::from_secs(120));
        if run.timed_out || run.success {
            failures.push(format!(
                "case {i}: expected a refusal (missing {missing:?}); success={} timed_out={}:\n{}",
                run.success, run.timed_out, run.output
            ));
            continue;
        }
        for m in &missing {
            if !run.output.contains(m) {
                failures.push(format!(
                    "case {i}: refusal does not name missing program {m}:\n{}",
                    run.output
                ));
            }
        }
        for (id, prog) in fixtures {
            if missing.contains(prog) && !run.output.contains(id) {
                failures.push(format!(
                    "case {i}: refusal does not name fixture {id} (needs {prog}):\n{}",
                    run.output
                ));
            }
        }
        for p in present {
            if ws.program_ran(p) {
                failures.push(format!(
                    "case {i}: evidence program {p} ran although the bench refused"
                ));
            }
        }
        if ws.model_invoked() {
            failures.push(format!(
                "case {i}: a model was invoked although the bench refused"
            ));
        }
        if reports_pass_rate(&run.output) {
            failures.push(format!(
                "case {i}: a pass rate was reported although the bench refused:\n{}",
                run.output
            ));
        }
    }
    assert!(failures.is_empty(), "mismatches:\n{}", failures.join("\n"));
}
