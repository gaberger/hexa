#![allow(unused, clippy::all)]
mod part_1 {
    #![allow(dead_code, unused_imports)]

    use std::path::{Path, PathBuf};
    use std::process::{Command, Output, Stdio};

    fn git(dir: &Path, args: &[&str]) -> Output {
        Command::new("git")
            .args(args)
            .current_dir(dir)
            .stdin(Stdio::null())
            .output()
            .expect("git runs")
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let _ = git(dir.path(), &["init", "-q"]);
        let _ = git(dir.path(), &["config", "user.name", "tester"]);
        let _ = git(dir.path(), &["config", "user.email", "tester@example.com"]);
        let _ = git(dir.path(), &["config", "commit.gpgsign", "false"]);
        std::fs::create_dir_all(dir.path().join("docs/adrs")).unwrap();
        dir
    }

    fn commit_all(dir: &Path, msg: &str) -> String {
        let _ = git(dir, &["add", "-A"]);
        let _ = git(dir, &["commit", "-q", "-m", msg, "--allow-empty"]);
        let out = git(dir, &["rev-parse", "HEAD"]);
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn hexa(dir: &Path, args: &[&str]) -> (bool, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_hexa"))
            .args(args)
            .current_dir(dir)
            .stdin(Stdio::null())
            .env("NO_COLOR", "1")
            .env("CLICOLOR", "0")
            .output()
            .expect("hexa runs");
        let mut text = String::from_utf8_lossy(&out.stdout).to_string();
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        (out.status.success(), text)
    }

    fn adr_md(id: &str, status: &str, gate: Option<&str>) -> String {
        let mut s = format!(
        "# {id}: A decision\n\n**Status:** {status}\n**Date:** 2026-11-01\n\n## Context\n\nSome context.\n\n## Decision\n\n1. **One.** Do the thing.\n\n## Consequences\n\nNone.\n"
    );
        if let Some(cmd) = gate {
            s.push_str(&format!("\n## Gate (uncounted)\n\n`{cmd}`\n"));
        }
        s
    }

    fn write_adr(dir: &Path, id: &str, status: &str, gate: Option<&str>) {
        let path = dir.join("docs/adrs").join(format!("{id}-a-decision.md"));
        std::fs::write(path, adr_md(id, status, gate)).unwrap();
    }

    fn write_unbuilt(dir: &Path, unbuilt: &[(&str, &str)]) {
        std::fs::create_dir_all(dir.join(".hexa")).unwrap();
        let mut map = serde_json::Map::new();
        for (id, why) in unbuilt {
            map.insert(id.to_string(), serde_json::Value::String(why.to_string()));
        }
        let v = serde_json::json!({ "adr": { "unbuilt": map } });
        std::fs::write(
            dir.join(".hexa/project.json"),
            serde_json::to_string_pretty(&v).unwrap(),
        )
        .unwrap();
    }

    fn says_nothing_checked(out: &str) -> bool {
        out.to_lowercase().contains("nothing was checked")
    }

    fn says_no_gate(out: &str) -> bool {
        out.to_lowercase().contains("no adr records a gate")
    }

    fn says_no_scenario(out: &str) -> bool {
        out.to_lowercase().contains("no spec holds a scenario")
    }

    /// S01: gates --ci in a repository with no ADR fails and says nothing was checked
    #[test]
    fn s01_gates_ci_with_no_adr_fails_and_says_nothing_checked() {
        let dir = repo();
        commit_all(dir.path(), "init");
        let (ok, out) = hexa(dir.path(), &["adr", "gates", "--ci"]);
        assert!(
            !ok,
            "gates --ci over no ADR must exit non-zero; output:\n{out}"
        );
        assert!(
            says_no_gate(&out),
            "output must say no ADR records a gate:\n{out}"
        );
        assert!(
            says_nothing_checked(&out),
            "output must say nothing was checked:\n{out}"
        );
    }

    /// S02: gates --ci where every ADR is without a gate fails
    #[test]
    fn s02_gates_ci_with_only_gateless_adrs_fails() {
        let dir = repo();
        write_adr(dir.path(), "ADR-2611010001", "Accepted", None);
        write_adr(dir.path(), "ADR-2611010002", "Accepted", None);
        write_adr(dir.path(), "ADR-2611010003", "Accepted", None);
        commit_all(dir.path(), "adrs");
        let (ok, out) = hexa(dir.path(), &["adr", "gates", "--ci"]);
        assert!(
            !ok,
            "gates --ci with no gate must exit non-zero; output:\n{out}"
        );
        assert!(
            says_no_gate(&out),
            "output must say no ADR records a gate:\n{out}"
        );
        assert!(
            says_nothing_checked(&out),
            "output must say nothing was checked:\n{out}"
        );
    }

    #[derive(Clone, Copy, Debug)]
    enum Case {
        Pass,
        Fail,
        FailUnbuilt,
        PassUnbuilt,
        NoGate,
    }

    /// S06: gates --ci reports an empty run exactly when no gate was judged
    #[test]
    fn s06_empty_run_reported_exactly_when_no_gate_judged() {
        let all = [
            Case::Pass,
            Case::Fail,
            Case::FailUnbuilt,
            Case::PassUnbuilt,
            Case::NoGate,
        ];
        let mut sets: Vec<Vec<Case>> = vec![vec![]];
        for a in all {
            sets.push(vec![a]);
        }
        for a in all {
            for b in all {
                sets.push(vec![a, b]);
            }
        }
        sets.push(vec![Case::NoGate, Case::NoGate, Case::NoGate]);
        for set in sets {
            let dir = repo();
            let mut unbuilt: Vec<(String, &str)> = Vec::new();
            for (i, c) in set.iter().enumerate() {
                let id = format!("ADR-26110100{:02}", i + 1);
                let gate = match c {
                    Case::Pass | Case::PassUnbuilt => Some("true"),
                    Case::Fail | Case::FailUnbuilt => Some("false"),
                    Case::NoGate => None,
                };
                write_adr(dir.path(), &id, "Accepted", gate);
                if matches!(c, Case::FailUnbuilt | Case::PassUnbuilt) {
                    unbuilt.push((id, "not built yet"));
                }
            }
            let refs: Vec<(&str, &str)> = unbuilt.iter().map(|(a, b)| (a.as_str(), *b)).collect();
            write_unbuilt(dir.path(), &refs);
            commit_all(dir.path(), "adrs");
            let (ok, out) = hexa(dir.path(), &["adr", "gates", "--ci"]);
            let empty = set.iter().all(|c| matches!(c, Case::NoGate));
            assert_eq!(
            says_nothing_checked(&out),
            empty,
            "case {set:?}: says-nothing-checked must equal whether no gate was judged; output:\n{out}"
        );
            if empty {
                assert!(
                    !ok,
                    "case {set:?}: an empty run must exit non-zero; output:\n{out}"
                );
            }
        }
    }

    /// S07: verify --ci with no spec fails and says nothing was checked
    #[test]
    fn s07_verify_ci_with_no_spec_fails() {
        let dir = repo();
        std::fs::create_dir_all(dir.path().join("docs/specs")).unwrap();
        std::fs::write(dir.path().join("docs/specs/.keep"), "").unwrap();
        commit_all(dir.path(), "init");
        let (ok, out) = hexa(dir.path(), &["spec", "verify", "--ci"]);
        assert!(
            !ok,
            "verify --ci with no spec must exit non-zero; output:\n{out}"
        );
        assert!(
            says_no_scenario(&out),
            "output must say no spec holds a scenario:\n{out}"
        );
        assert!(
            says_nothing_checked(&out),
            "output must say nothing was checked:\n{out}"
        );
    }

    /// S08: verify --ci where specs hold no scenarios fails
    #[test]
    fn s08_verify_ci_with_empty_specs_fails() {
        let dir = repo();
        std::fs::create_dir_all(dir.path().join("docs/specs")).unwrap();
        for id in ["ADR-2611010001", "ADR-2611010002"] {
            write_adr(dir.path(), id, "Accepted", None);
            let spec = serde_json::json!({ "adr": id, "feature": "f", "specs": [] });
            std::fs::write(
                dir.path().join(format!("docs/specs/{id}.json")),
                serde_json::to_string_pretty(&spec).unwrap(),
            )
            .unwrap();
        }
        commit_all(dir.path(), "specs");
        let (ok, out) = hexa(dir.path(), &["spec", "verify", "--ci"]);
        assert!(
            !ok,
            "verify --ci with empty specs must exit non-zero; output:\n{out}"
        );
        assert!(
            says_no_scenario(&out),
            "output must say no spec holds a scenario:\n{out}"
        );
        assert!(
            says_nothing_checked(&out),
            "output must say nothing was checked:\n{out}"
        );
    }

    /// S10: one passing scenario is not an empty run
    #[test]
    fn s10_one_passing_scenario_is_not_empty() {
        let dir = repo();
        let d = dir.path();
        let id = "ADR-2611010001";
        write_adr(d, id, "Accepted", None);
        std::fs::create_dir_all(d.join("tests")).unwrap();
        let script = d.join("tests/ok.sh");
        std::fs::write(&script, "#!/bin/sh\nexit 0\n").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            let mut p = std::fs::metadata(&script).unwrap().permissions();
            p.set_mode(0o755);
            std::fs::set_permissions(&script, p).unwrap();
        }
        // `spec accept` needs a test home (ADR-2611191120 §9); a manifest
        // gives one, and the .sh test keeps its built-in bash runner.
        std::fs::write(d.join("pyproject.toml"), "[project]\nname = \"fixture\"\n").unwrap();
        let red = commit_all(d, "test");
        std::fs::create_dir_all(d.join("docs/specs")).unwrap();
        let spec = serde_json::json!({
            "adr": id,
            "feature": "f",
            "red": red,
            "specs": [
                { "id": "S01", "decision": ["§1"], "kind": "example", "description": "d1",
                  "given": "g", "when": "w", "then": "t", "test": "tests/ok.sh::a" },
                { "id": "S02", "decision": ["§1"], "kind": "negative", "description": "d2",
                  "given": "g", "when": "w", "then": "t", "test": "tests/ok.sh::b" }
            ]
        });
        let spec_path = d.join(format!("docs/specs/{id}.json"));
        std::fs::write(&spec_path, serde_json::to_string_pretty(&spec).unwrap()).unwrap();
        commit_all(d, "spec");

        let (acc_ok, acc_out) = hexa(d, &["spec", "accept", id, "--by", "tester"]);
        if !acc_ok {
            let (ok2, out2) = hexa(d, &["spec", "accept", id]);
            assert!(ok2, "spec accept failed:\n{acc_out}\n{out2}");
        }
        let (seal_ok, seal_out) = hexa(d, &["spec", "seal", id]);
        assert!(seal_ok, "spec seal failed:\n{seal_out}");

        // Keep the red commit recorded, whatever accept/seal rewrote.
        let mut v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&spec_path).unwrap()).unwrap();
        if v.get("red")
            .and_then(|r| r.as_str())
            .map(|r| r.is_empty())
            .unwrap_or(true)
        {
            v["red"] = serde_json::Value::String(red.clone());
            std::fs::write(&spec_path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
        }
        commit_all(d, "sealed");

        let (ok, out) = hexa(d, &["spec", "verify", "--ci"]);
        assert!(
            ok,
            "one passing sealed scenario must exit 0; output:\n{out}"
        );
        assert!(
            !says_nothing_checked(&out),
            "must not say nothing was checked:\n{out}"
        );
    }
}

mod part_2 {
    #[allow(unused_imports)]
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    const ADR_ID: &str = "ADR-2611230001";

    fn bin() -> &'static str {
        env!("CARGO_BIN_EXE_hexa")
    }

    fn git(dir: &Path, args: &[&str]) {
        let _ = Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(dir)
            .output();
    }

    fn git_head(dir: &Path) -> String {
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    }

    /// A fresh repository with empty `docs/adrs/` and `docs/specs/`.
    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("docs/adrs")).unwrap();
        std::fs::create_dir_all(dir.path().join("docs/specs")).unwrap();
        std::fs::create_dir_all(dir.path().join(".hexa")).unwrap();
        git(dir.path(), &["init", "-q"]);
        dir
    }

    fn write(dir: &Path, rel: &str, content: &str) {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, content).unwrap();
    }

    fn adr_markdown(id: &str, gate: Option<&str>) -> String {
        let mut s = format!(
        "# {id}: A decision\n\n**Status:** Accepted\n**Date:** 2026-11-23\n\n## Context\n\nSome context.\n\n## Decision\n\n1. **One.** Something is decided.\n\n## Consequences\n\nNone.\n"
    );
        if let Some(g) = gate {
            s.push_str(&format!("\n## Gate (uncounted)\n\n`{g}`\n"));
        }
        s
    }

    fn write_adr(dir: &Path, id: &str, gate: Option<&str>) {
        write(
            dir,
            &format!("docs/adrs/{id}-a-decision.md"),
            &adr_markdown(id, gate),
        );
    }

    fn scenario(id: &str, test: Option<&str>) -> hexa_core::domain::spec::Scenario {
        hexa_core::domain::spec::Scenario {
            id: id.to_string(),
            decision: vec!["§1".to_string()],
            kind: hexa_core::domain::spec::Kind::Example,
            description: format!("scenario {id}"),
            given: "a given".to_string(),
            when: "a when".to_string(),
            then: "a then".to_string(),
            test: test.map(str::to_string),
            red: None,
        }
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        use sha2::Digest;
        let mut h = sha2::Sha256::new();
        h.update(bytes);
        h.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }

    fn write_spec(
        dir: &Path,
        adr: &str,
        scenarios: Vec<hexa_core::domain::spec::Scenario>,
        red: Option<String>,
    ) {
        let mut spec = hexa_core::domain::spec::Spec {
            adr: adr.to_string(),
            feature: "a feature".to_string(),
            description: String::new(),
            universal: vec![],
            exempt: vec![],
            accepted: None,
            sealed: std::collections::BTreeMap::new(),
            red,
            specs: scenarios,
        };
        let content = serde_json::to_string(&spec.accepted_content()).unwrap();
        spec.accepted = Some(hexa_core::domain::spec::Acceptance {
            by: "tester".to_string(),
            sha256: sha256_hex(content.as_bytes()),
        });
        for f in spec.test_files() {
            if let Ok(bytes) = std::fs::read(dir.join(&f)) {
                spec.sealed.insert(f.clone(), sha256_hex(&bytes));
            }
        }
        write(
            dir,
            &format!("docs/specs/{adr}.json"),
            &serde_json::to_string_pretty(&spec).unwrap(),
        );
    }

    fn run(dir: &Path, args: &[&str]) -> Output {
        Command::new(bin())
            .args(args)
            .current_dir(dir)
            .env("NO_COLOR", "1")
            .env("CLICOLOR", "0")
            .env("TERM", "dumb")
            .output()
            .expect("run hexa")
    }

    fn text(o: &Output) -> String {
        let mut s = String::from_utf8_lossy(&o.stdout).to_string();
        s.push('\n');
        s.push_str(&String::from_utf8_lossy(&o.stderr));
        s
    }

    fn says_nothing_checked(s: &str) -> bool {
        s.to_lowercase().contains("nothing was checked")
    }

    fn says_no_spec_holds_scenario(s: &str) -> bool {
        s.to_lowercase().contains("no spec holds a scenario")
    }

    /// S11: verify --ci reports an empty run exactly when there is no scenario
    #[test]
    fn s11_verify_ci_reports_empty_run_exactly_when_no_scenario() {
        // (number of specs' scenario lists, expected empty)
        let cases: Vec<(Vec<Vec<Option<&str>>>, bool)> = vec![
            (vec![], true),
            (vec![vec![]], true),
            (vec![vec![], vec![]], true),
            (vec![vec![None]], false),
            (vec![vec![], vec![None, None]], false),
            (vec![vec![Some("tests/fails.sh")]], false),
            (vec![vec![Some("tests/ok.sh"), None]], false),
        ];
        for (i, (specs, empty)) in cases.into_iter().enumerate() {
            let dir = repo();
            let d = dir.path();
            write(d, "tests/ok.sh", "#!/bin/sh\nexit 0\n");
            write(d, "tests/fails.sh", "#!/bin/sh\nexit 1\n");
            let mut total = 0;
            for (j, sc) in specs.iter().enumerate() {
                let id = format!("ADR-26112300{:02}", j + 1);
                write_adr(d, &id, None);
                let scenarios: Vec<_> = sc
                    .iter()
                    .enumerate()
                    .map(|(k, t)| scenario(&format!("S{}", k + 1), *t))
                    .collect();
                total += scenarios.len();
                write_spec(d, &id, scenarios, None);
            }
            assert_eq!(total == 0, empty);
            git(d, &["add", "-A"]);
            git(d, &["commit", "-q", "-m", "init"]);
            let o = run(d, &["spec", "verify", "--ci"]);
            let out = text(&o);
            if empty {
                assert!(
                    says_nothing_checked(&out),
                    "case {i}: an empty --ci run must say nothing was checked: {out}"
                );
                assert!(
                    !o.status.success(),
                    "case {i}: an empty --ci run must exit non-zero: {out}"
                );
            } else {
                assert!(
                    !says_nothing_checked(&out),
                    "case {i}: a run with {total} scenarios checked something: {out}"
                );
            }
        }
    }

    /// S14: the new guard must not turn a passing non-empty --ci run into a failure
    #[test]
    fn s14_non_empty_passing_ci_run_still_passes() {
        let dir = repo();
        let d = dir.path();
        write_adr(d, ADR_ID, Some("true"));
        write(d, "tests/ok.sh", "#!/bin/sh\nexit 0\n");
        git(d, &["add", "-A"]);
        git(d, &["commit", "-q", "-m", "red"]);
        let red = git_head(d);
        write_spec(
            d,
            ADR_ID,
            // A complete spec: §1 needs a negative scenario as well as an
            // example, or verify rejects the spec before running it.
            vec![
                scenario("S1", Some("tests/ok.sh")),
                hexa_core::domain::spec::Scenario {
                    kind: hexa_core::domain::spec::Kind::Negative,
                    ..scenario("S2", Some("tests/ok.sh"))
                },
            ],
            Some(red),
        );
        git(d, &["add", "-A"]);
        git(d, &["commit", "-q", "-m", "spec"]);

        let g = run(d, &["adr", "gates", "--ci"]);
        let gout = text(&g);
        assert!(
            g.status.success(),
            "gates --ci with a passing gate exits 0: {gout}"
        );
        assert!(
            gout.contains("1 passed") && gout.contains("0 failed"),
            "same summary as before: {gout}"
        );
        assert!(!says_nothing_checked(&gout), "{gout}");

        let v = run(d, &["spec", "verify", "--ci"]);
        let vout = text(&v);
        assert!(
            v.status.success(),
            "verify --ci with a passing scenario exits 0: {vout}"
        );
        assert!(
            vout.contains("2 passed · 0 failed"),
            "same summary as before (both scenarios of the complete spec): {vout}"
        );
        assert!(!says_nothing_checked(&vout), "{vout}");
    }
}
