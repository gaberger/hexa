#![allow(unused, clippy::all)]
mod part_1 {
    use hexa_core::ports::spec::{
        binding_anchor, judge_binding, BindingVerdict, Kind, Move, Scenario, Spec,
    };

    fn scenario(id: &str, test: Option<&str>) -> Scenario {
        Scenario {
            id: id.to_string(),
            decision: vec!["§1".to_string()],
            kind: Kind::Example,
            description: "the description".to_string(),
            given: "the given".to_string(),
            when: "the when".to_string(),
            then: "the then".to_string(),
            test: test.map(str::to_string),
            red: None,
        }
    }

    fn x(test: Option<&str>) -> Scenario {
        scenario("X", test)
    }

    fn spec_of(specs: Vec<Scenario>) -> Spec {
        Spec {
            adr: "ADR-1".to_string(),
            feature: "feature".to_string(),
            description: String::new(),
            universal: Vec::new(),
            exempt: Vec::new(),
            accepted: None,
            sealed: Default::default(),
            red: None,
            specs,
        }
    }

    /// A revision holding another scenario, `Y`, ahead of `X`.
    fn revision(holds: bool, s: Scenario) -> (bool, Spec) {
        (holds, spec_of(vec![scenario("Y", Some("ci/other.sh")), s]))
    }

    fn mv(scenario: &str, from: &str, to: &str) -> Move {
        Move {
            scenario: scenario.to_string(),
            from: from.to_string(),
            to: to.to_string(),
            by: "someone".to_string(),
            reason: "a reason".to_string(),
            at: "2026-11-25T00:00:00Z".to_string(),
            base_commit: "abc1234".to_string(),
        }
    }

    /// S01: The anchor is the oldest test ref recorded under the current accepted text.
    #[test]
    fn s01_anchor_is_oldest_ref_under_current_text() {
        let history = vec![
            revision(true, x(None)),
            revision(true, x(Some("ci/a.sh"))),
            revision(true, x(Some("ci/b.sh"))),
        ];
        let got = binding_anchor(&x(Some("ci/c.sh")), &history);
        assert_eq!(got, Some("ci/a.sh".to_string()));
    }

    /// S02: A revision in which the scenario is pending does not end the search.
    #[test]
    fn s02_pending_revision_does_not_end_the_search() {
        let history = vec![
            revision(true, x(Some("ci/a.sh"))),
            revision(true, x(None)),
            revision(true, x(Some("ci/b.sh"))),
        ];
        let got = binding_anchor(&x(Some("ci/b.sh")), &history);
        assert_eq!(got, Some("ci/a.sh".to_string()));
    }

    /// S03: The red commit is not part of a scenario's text.
    #[test]
    fn s03_red_commit_is_not_part_of_the_text() {
        let mut older = x(Some("ci/a.sh"));
        older.red = Some("1111111".to_string());
        let mut newer = x(Some("ci/b.sh"));
        newer.red = Some("2222222".to_string());
        let mut current = x(Some("ci/b.sh"));
        current.red = Some("3333333".to_string());
        let history = vec![revision(true, older), revision(true, newer)];
        let got = binding_anchor(&current, &history);
        assert_eq!(got, Some("ci/a.sh".to_string()));
    }

    /// S04: A ref recorded before the scenario last took its current accepted text is not the anchor.
    #[test]
    fn s04_ref_before_the_current_text_was_last_taken_is_not_the_anchor() {
        let mut different = x(Some("ci/b.sh"));
        different.then = "another then".to_string();
        let history = vec![
            revision(true, x(Some("ci/a.sh"))),
            revision(true, different),
            revision(true, x(Some("ci/c.sh"))),
        ];
        let got = binding_anchor(&x(Some("ci/c.sh")), &history);
        assert_eq!(got, Some("ci/c.sh".to_string()));
    }

    /// S05: An unaccepted text edit that is later reverted does not move the anchor.
    #[test]
    fn s05_reverted_unaccepted_edit_does_not_move_the_anchor() {
        let mut edited = x(Some("ci/a.sh"));
        edited.then = "another then".to_string();
        let history = vec![
            revision(true, x(Some("ci/a.sh"))),
            revision(false, edited),
            revision(true, x(Some("ci/b.sh"))),
        ];
        let got = binding_anchor(&x(Some("ci/b.sh")), &history);
        assert_eq!(got, Some("ci/a.sh".to_string()));
    }

    /// S06: A scenario with no bound revision under its current text has no anchor.
    #[test]
    fn s06_no_bound_revision_under_current_text_means_no_anchor() {
        let current = x(Some("ci/current.sh"));

        assert_eq!(binding_anchor(&current, &[]), None, "empty history");

        let always_pending = vec![revision(true, x(None)), revision(true, x(None))];
        assert_eq!(
            binding_anchor(&current, &always_pending),
            None,
            "always pending"
        );

        let mut different = x(Some("ci/old.sh"));
        different.then = "another then".to_string();
        let only_old_text_bound = vec![revision(true, different), revision(true, x(None))];
        assert_eq!(
            binding_anchor(&current, &only_old_text_bound),
            None,
            "bound only under other text"
        );
    }

    /// S07: Only a change to a text field in an accepted revision ends the anchor search.
    #[test]
    fn s07_only_an_accepted_text_change_ends_the_search() {
        let edits: Vec<fn(&mut Scenario)> = vec![
            |s| s.decision = vec!["§2".to_string()],
            |s| s.kind = Kind::Negative,
            |s| s.description = "another description".to_string(),
            |s| s.given = "another given".to_string(),
            |s| s.when = "another when".to_string(),
            |s| s.then = "another then".to_string(),
        ];
        let changed = |edit: fn(&mut Scenario), test: Option<&str>| {
            let mut s = x(test);
            edit(&mut s);
            s
        };

        let a = "ci/a.sh";

        // Revisions that may follow the one binding X to A.
        let mut followers: Vec<(bool, Spec)> = vec![
            revision(true, x(None)),
            revision(true, x(Some(a))),
            revision(true, x(Some("ci/b.sh"))),
            revision(true, x(Some("ci/c.sh::case"))),
            revision(false, x(None)),
            revision(false, x(Some("ci/u.sh"))),
        ];
        for edit in &edits {
            followers.push(revision(false, changed(*edit, None)));
            followers.push(revision(false, changed(*edit, Some("ci/forged.sh"))));
        }

        let mut tails: Vec<Vec<(bool, Spec)>> = vec![Vec::new()];
        let mut last: Vec<Vec<(bool, Spec)>> = vec![Vec::new()];
        for _ in 0..3 {
            let mut next = Vec::new();
            for tail in &last {
                for f in &followers {
                    let mut t = tail.clone();
                    t.push(f.clone());
                    next.push(t);
                }
            }
            tails.extend(next.iter().cloned());
            last = next;
        }

        // What may come before it without holding an older ref under this text.
        let prefixes: Vec<Vec<(bool, Spec)>> = vec![
            vec![],
            vec![revision(true, x(None))],
            vec![revision(false, x(Some("ci/unaccepted.sh")))],
            vec![
                revision(true, x(Some("ci/ancient.sh"))),
                revision(true, changed(edits[5], Some("ci/old.sh"))),
            ],
        ];

        let currents = [None, Some(a), Some("ci/b.sh"), Some("ci/never-recorded.sh")];

        for (i, tail) in tails.iter().enumerate() {
            let prefix = &prefixes[i % prefixes.len()];
            let mut history = prefix.clone();
            history.push(revision(true, x(Some(a))));
            history.extend(tail.iter().cloned());
            let current = x(currents[(i / prefixes.len()) % currents.len()]);

            assert_eq!(
                binding_anchor(&current, &history),
                Some(a.to_string()),
                "current {:?}, history {:?}",
                current,
                history
            );

            let edit = edits[i % edits.len()];
            for bound in [None, Some("ci/z.sh")] {
                for pending_after in 0..3 {
                    let mut ended = history.clone();
                    ended.push(revision(true, changed(edit, bound)));
                    for _ in 0..pending_after {
                        ended.push(revision(true, x(None)));
                    }
                    assert_eq!(
                        binding_anchor(&current, &ended),
                        None,
                        "current {:?}, history {:?}",
                        current,
                        ended
                    );
                }
            }
        }

        // Every prefix, current ref and text field, over the shortest history.
        for prefix in &prefixes {
            for test in currents {
                for edit in &edits {
                    let current = x(test);
                    let mut history = prefix.clone();
                    history.push(revision(true, x(Some(a))));
                    assert_eq!(binding_anchor(&current, &history), Some(a.to_string()));
                    history.push(revision(true, changed(*edit, Some(a))));
                    history.push(revision(true, x(None)));
                    assert_eq!(binding_anchor(&current, &history), None);
                }
            }
        }
    }

    /// S08: Recorded moves carry the anchor to the current ref, oldest first.
    #[test]
    fn s08_recorded_moves_carry_the_anchor_to_the_current_ref() {
        let moves = vec![
            mv("Y", "a", "y1"),
            mv("X", "a", "b"),
            mv("Z", "b", "z1"),
            mv("X", "b", "c"),
            mv("Y", "c", "y2"),
        ];
        assert_eq!(
            judge_binding("X", Some("a"), &moves, Some("c")),
            BindingVerdict::Holds
        );
    }

    /// S09: A scenario with no anchor holds, and so does a pending one.
    #[test]
    fn s09_no_anchor_and_pending_both_hold() {
        assert_eq!(
            judge_binding("X", None, &[], Some("ci/a.sh")),
            BindingVerdict::Holds
        );
        assert_eq!(
            judge_binding("W", Some("a"), &[], None),
            BindingVerdict::Holds
        );
    }

    /// S10: A ref changed without a record is judged moved.
    #[test]
    fn s10_unrecorded_change_is_judged_moved() {
        let expected_old = BindingVerdict::Moved {
            expected: "ci/t.sh".to_string(),
        };
        assert_eq!(
            judge_binding("X", Some("ci/t.sh"), &[], Some("ci/t3.sh")),
            expected_old
        );

        let others = vec![mv("Y", "ci/t.sh", "ci/t3.sh")];
        assert_eq!(
            judge_binding("X", Some("ci/t.sh"), &others, Some("ci/t3.sh")),
            expected_old
        );

        let recorded = vec![mv("X", "ci/t.sh", "ci/t2.sh")];
        assert_eq!(
            judge_binding("X", Some("ci/t.sh"), &recorded, Some("ci/t.sh")),
            BindingVerdict::Moved {
                expected: "ci/t2.sh".to_string()
            }
        );
    }
}

mod part_2 {
    use hexa_core::ports::spec::{binding_anchor, judge_binding, BindingVerdict, Move, Spec};
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output, Stdio};

    const ADR: &str = "ADR-2601010000";
    const X: &str = "ZX01";
    const Y: &str = "ZY02";

    fn mv(scenario: &str, from: &str, to: &str) -> Move {
        Move {
            scenario: scenario.to_string(),
            from: from.to_string(),
            to: to.to_string(),
            by: "tester".to_string(),
            reason: "because".to_string(),
            at: "2026-01-01T00:00:00Z".to_string(),
            base_commit: "abc1234".to_string(),
        }
    }

    fn sha(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    fn text(o: &Output) -> String {
        format!(
            "{}\n{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        )
    }

    struct Repo {
        _tmp: tempfile::TempDir,
        root: PathBuf,
        home: PathBuf,
        bin: PathBuf,
    }

    impl Repo {
        fn new() -> Repo {
            let tmp = tempfile::tempdir().unwrap();
            let root = tmp.path().join("repo");
            let home = tmp.path().join("home");
            let bin = tmp.path().join("bin");
            for d in [&root, &home, &bin] {
                std::fs::create_dir_all(d).unwrap();
            }
            let r = Repo {
                _tmp: tmp,
                root,
                home,
                bin,
            };
            r.git(&["init", "-q"]);
            r.git(&["config", "user.email", "tester@example.com"]);
            r.git(&["config", "user.name", "Tester"]);
            r.git(&["config", "commit.gpgsign", "false"]);
            r.write(
            &format!("docs/adrs/{ADR}-demo.md"),
            &format!("# {ADR}: Demo\n\n**Status:** Accepted\n**Date:** 2026-01-01\n\n## Context\n\nDemo.\n\n## Decision\n\n1. **The answer.** The answer is 42.\n\n## Consequences\n\nNone.\n"),
        );
            r
        }

        fn env(&self, c: &mut Command) {
            let real_home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
            let cargo_home = std::env::var_os("CARGO_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| real_home.join(".cargo"));
            let rustup_home = std::env::var_os("RUSTUP_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| real_home.join(".rustup"));
            let mut paths = vec![self.bin.clone()];
            paths.extend(std::env::split_paths(
                &std::env::var_os("PATH").unwrap_or_default(),
            ));
            c.current_dir(&self.root)
                .env("PATH", std::env::join_paths(paths).unwrap())
                .env("HOME", &self.home)
                .env("CARGO_HOME", cargo_home)
                .env("RUSTUP_HOME", rustup_home)
                .env_remove("CARGO_TARGET_DIR")
                .env_remove("ANTHROPIC_API_KEY")
                .env_remove("CLAUDECODE")
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .stdin(Stdio::null());
        }

        fn git(&self, args: &[&str]) -> String {
            let mut c = Command::new("git");
            self.env(&mut c);
            let o = c.args(args).output().unwrap();
            assert!(o.status.success(), "git {args:?}: {}", text(&o));
            String::from_utf8_lossy(&o.stdout).trim().to_string()
        }

        fn hexa(&self, args: &[&str]) -> Output {
            let mut c = Command::new(env!("CARGO_BIN_EXE_hexa"));
            self.env(&mut c);
            c.args(args).output().unwrap()
        }

        fn write(&self, rel: &str, body: &str) {
            let p = self.root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, body).unwrap();
            if rel.ends_with(".sh") {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        }

        fn commit(&self, msg: &str) -> String {
            self.git(&["add", "-A"]);
            self.git(&["commit", "-q", "--allow-empty", "-m", msg]);
            self.git(&["rev-parse", "HEAD"])
        }

        fn commit_if_dirty(&self, msg: &str) {
            if !self.git(&["status", "--porcelain"]).is_empty() {
                self.commit(msg);
            }
        }

        fn spec_rel() -> String {
            format!("docs/specs/{ADR}.json")
        }

        fn spec(&self) -> Value {
            let s = std::fs::read_to_string(self.root.join(Self::spec_rel())).unwrap();
            serde_json::from_str(&s).unwrap()
        }

        fn put_spec(&self, v: &Value) {
            self.write(
                &Self::spec_rel(),
                &(serde_json::to_string_pretty(v).unwrap() + "\n"),
            );
        }

        fn spec_at(&self, rev: &str) -> Value {
            serde_json::from_str(&self.git(&["show", &format!("{rev}:{}", Self::spec_rel())]))
                .unwrap()
        }
    }

    fn unbound_spec() -> Value {
        let sc = |id: &str, kind: &str| {
            json!({"id": id, "decision": ["§1"], "kind": kind, "description": format!("{id} behaviour"),
               "given": "a project", "when": "it is asked", "then": "the answer holds"})
        };
        json!({"adr": ADR, "feature": "demo", "specs": [sc(X, "example"), sc(Y, "negative")]})
    }

    fn scenario<'a>(spec: &'a Value, id: &str) -> &'a Value {
        spec["specs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == id)
            .unwrap()
    }

    fn scenario_mut<'a>(spec: &'a mut Value, id: &str) -> &'a mut Value {
        spec["specs"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|s| s["id"] == id)
            .unwrap()
    }

    fn test_of(spec: &Value, id: &str) -> Option<String> {
        scenario(spec, id)["test"].as_str().map(str::to_string)
    }

    fn moves_for(spec: &Value, id: &str) -> Vec<Value> {
        spec["moves"]
            .as_array()
            .map(|a| a.iter().filter(|m| m["scenario"] == id).cloned().collect())
            .unwrap_or_default()
    }

    fn hash_candidate(spec: &Value, k: usize) -> String {
        let parsed: Spec = serde_json::from_value(spec.clone()).unwrap();
        let content = parsed.accepted_content();
        let bytes = match k {
            0 => serde_json::to_string(&content).unwrap(),
            1 => serde_json::to_string_pretty(&content).unwrap(),
            2 => serde_json::to_string_pretty(&content).unwrap() + "\n",
            _ => serde_json::to_string(&serde_json::to_value(&content).unwrap()).unwrap(),
        };
        sha(bytes.as_bytes())
    }

    /// Which way of hashing accepted content `verify` takes, found on a probe.
    fn hash_scheme() -> usize {
        for k in 0..4 {
            let r = Repo::new();
            shell_history(&r, Some(k));
            if r.hexa(&["spec", "verify", ADR]).status.success() {
                return k;
            }
        }
        panic!("no way found to build an accepted, sealed spec that verifies");
    }

    /// Accept the spec now on disk: by the CLI, else by writing the hash.
    fn accept(r: &Repo, scheme: Option<usize>) {
        let mut v = r.spec();
        v.as_object_mut().unwrap().remove("accepted");
        r.put_spec(&v);
        if scheme.is_none() {
            let o = r.hexa(&["spec", "accept", ADR]);
            if o.status.success() && r.spec()["accepted"].is_object() {
                return;
            }
        }
        let k = scheme.unwrap_or_else(hash_scheme);
        let mut v = r.spec();
        v["accepted"] = json!({"by": "Tester", "sha256": hash_candidate(&v, k)});
        r.put_spec(&v);
    }

    const T_X: &str = "#!/bin/sh\ntest \"$(cat answer 2>/dev/null)\" = 42\n";
    const T_Y: &str = "#!/bin/sh\ntest \"$(cat other 2>/dev/null)\" = 1\n";

    /// An accepted spec, shell tests red at one commit, then bound, sealed and green.
    fn shell_history(r: &Repo, scheme: Option<usize>) {
        r.put_spec(&unbound_spec());
        accept(r, scheme);
        r.write("ci/t.sh", T_X);
        r.write("ci/y.sh", T_Y);
        let red = r.commit("red tests");
        let mut v = r.spec();
        scenario_mut(&mut v, X)["test"] = json!("ci/t.sh");
        scenario_mut(&mut v, Y)["test"] = json!("ci/y.sh");
        v["red"] = json!(red);
        v["sealed"] = json!({"ci/t.sh": sha(T_X.as_bytes()), "ci/y.sh": sha(T_Y.as_bytes())});
        r.put_spec(&v);
        r.write("answer", "42\n");
        r.write("other", "1\n");
        r.commit("bind and seal");
    }

    fn shell_repo() -> Repo {
        let r = Repo::new();
        shell_history(&r, None);
        let o = r.hexa(&["spec", "verify", ADR]);
        assert!(
            o.status.success(),
            "the fixture verifies before any move: {}",
            text(&o)
        );
        r
    }

    /// Move X's test file and rewrite the ref and the seal by hand.
    fn hand_move(r: &Repo, from: &str, to: &str, body: &str, record: Option<Value>) {
        std::fs::remove_file(r.root.join(from)).unwrap();
        r.write(to, body);
        let mut v = r.spec();
        scenario_mut(&mut v, X)["test"] = json!(to);
        let sealed = v["sealed"].as_object_mut().unwrap();
        sealed.remove(from);
        sealed.insert(to.to_string(), json!(sha(body.as_bytes())));
        if let Some(m) = record {
            if !v["moves"].is_array() {
                v["moves"] = json!([]);
            }
            v["moves"].as_array_mut().unwrap().push(m);
        }
        r.put_spec(&v);
        r.commit("move a test by hand");
    }

    fn record(r: &Repo, from: &str, to: &str, reason: &str) -> Value {
        json!({"scenario": X, "from": from, "to": to, "by": "Tester", "reason": reason,
           "at": "2026-01-02T00:00:00Z", "base_commit": r.git(&["rev-parse", "HEAD"])})
    }

    const MODES: [&[&str]; 4] = [&[], &["--ci"], &["--prove"], &["--prove", "--ci"]];

    fn verify(r: &Repo, flags: &[&str]) -> Output {
        let mut args = vec!["spec", "verify", ADR];
        args.extend_from_slice(flags);
        r.hexa(&args)
    }

    /// Run `hexa spec red`, asking it to bind `id` to `to`.
    fn spec_red(r: &Repo, id: &str, to: &str) -> Output {
        let help = text(&r.hexa(&["spec", "red", "--help"]));
        let pair = format!("{id}={to}");
        // `spec red` takes the commit the test is red at: the one just made.
        let head = r.git(&["rev-parse", "HEAD"]);
        let flag = ["--bind", "--scenario", "--test", "--map", "--set"]
            .into_iter()
            .find(|f| {
                help.split(|c: char| c.is_whitespace() || c == ',')
                    .any(|w| w == *f)
            });
        match flag {
            Some(f) => r.hexa(&["spec", "red", ADR, &head, f, &pair]),
            None => r.hexa(&["spec", "red", ADR, &head, &pair]),
        }
    }

    /// A fake `claude` that answers with the given Rust tests.
    fn stub_writer(r: &Repo, tests: &[(&str, &str)]) {
        let mut code = String::new();
        for (id, name) in tests {
            code.push_str(&format!(
            "/// {id}: {id} behaviour\n#[test]\nfn {name}() {{\n    assert_eq!(demo::answer(), 42);\n}}\n\n"
        ));
        }
        let answer = format!("```rust\n{code}```\n");
        let reply = json!({"type": "result", "subtype": "success", "is_error": false, "result": answer,
                       "total_cost_usd": 0.0, "usage": {"input_tokens": 1, "output_tokens": 1}});
        let dir = r.bin.display();
        std::fs::write(r.bin.join("reply.txt"), &answer).unwrap();
        std::fs::write(r.bin.join("reply.json"), format!("{reply}\n")).unwrap();
        let script = format!(
        "#!/bin/sh\ncase \"$*\" in *--version*) echo '2.0.0 (Claude Code)'; exit 0;; esac\ncat >/dev/null 2>&1\ncase \"$*\" in *json*) cat '{dir}/reply.json';; *) cat '{dir}/reply.txt';; esac\n"
    );
        let p = r.bin.join("claude");
        std::fs::write(&p, script).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A cargo project whose X and Y tests were generated and bound by `spec tests`.
    fn generated_repo() -> Repo {
        let r = Repo::new();
        r.write(
            "Cargo.toml",
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n",
        );
        r.write("src/lib.rs", "pub fn answer() -> i32 {\n    0\n}\n");
        r.write(".gitignore", "target/\nCargo.lock\n");
        r.put_spec(&unbound_spec());
        accept(&r, None);
        r.commit("accepted spec");
        stub_writer(&r, &[(X, "old_zx01"), (Y, "old_zy02")]);
        let o = r.hexa(&["spec", "tests", ADR]);
        assert!(o.status.success(), "initial generation: {}", text(&o));
        r.commit_if_dirty("generated tests");
        let v = r.spec();
        assert!(
            test_of(&v, X).is_some_and(|t| t.ends_with("old_zx01")),
            "X is bound: {v}"
        );
        assert!(
            test_of(&v, Y).is_some_and(|t| t.ends_with("old_zy02")),
            "Y is bound: {v}"
        );
        r
    }

    /// The first commit after `before` in which X's ref differs from `old`.
    fn changing_commit(r: &Repo, before: &str, old: &str) -> String {
        r.git(&["rev-list", "--reverse", &format!("{before}..HEAD")])
            .lines()
            .find(|rev| test_of(&r.spec_at(rev), X).is_some_and(|t| t != old))
            .expect("a commit changes X's ref")
            .to_string()
    }

    fn assert_move(m: &Value, from: &str, to: &str, why: &str) {
        assert_eq!(m["from"], from, "{m}");
        assert_eq!(m["to"], to, "{m}");
        assert_eq!(m["reason"], why, "{m}");
        for k in ["by", "at", "base_commit"] {
            assert!(
                m[k].as_str().is_some_and(|s| !s.trim().is_empty()),
                "{k} is filled in: {m}"
            );
        }
    }

    /// S11: Refs are compared whole.
    #[test]
    fn s11_refs_are_compared_whole() {
        let v = judge_binding("X", Some("ci/t.sh::one"), &[], Some("ci/t.sh::two"));
        assert_eq!(
            v,
            BindingVerdict::Moved {
                expected: "ci/t.sh::one".to_string()
            }
        );
    }

    /// S12: A move that does not continue the chain breaks it, even if it ends at the current ref.
    #[test]
    fn s12_a_discontinuous_move_breaks_the_chain() {
        let moves = vec![mv("OTHER", "q", "r"), mv("X", "a", "b"), mv("X", "z", "c")];
        assert_eq!(
            judge_binding("X", Some("a"), &moves, Some("c")),
            BindingVerdict::BrokenChain(2)
        );
    }

    /// S13: Verify fails on a hand-moved binding in every mode.
    #[test]
    fn s13_verify_fails_on_a_hand_moved_binding() {
        let r = shell_repo();
        hand_move(
            &r,
            "ci/t.sh",
            "ci/t3.sh",
            "#!/bin/sh\ntest -f answer\n",
            None,
        );
        for flags in MODES {
            let o = verify(&r, flags);
            let out = text(&o);
            assert!(!o.status.success(), "{flags:?} must fail: {out}");
            for needle in [X, "ci/t.sh", "ci/t3.sh"] {
                assert!(out.contains(needle), "{flags:?} names {needle}: {out}");
            }
        }
    }

    /// S14: Verify fails on a broken move chain.
    #[test]
    fn s14_verify_fails_on_a_broken_chain() {
        let r = shell_repo();
        let m = record(&r, "ci/never.sh", "ci/t3.sh", "typed by hand");
        hand_move(&r, "ci/t.sh", "ci/t3.sh", T_X, Some(m));
        for flags in MODES {
            let o = verify(&r, flags);
            let out = text(&o);
            assert!(!o.status.success(), "{flags:?} must fail: {out}");
            assert!(
                out.to_lowercase().contains("broken"),
                "{flags:?} reports a broken chain: {out}"
            );
            for needle in [X, "ci/t.sh", "ci/t3.sh"] {
                assert!(out.contains(needle), "{flags:?} names {needle}: {out}");
            }
        }
    }

    /// S15: Verify reports the number of moves, and verbose output lists their reasons.
    #[test]
    fn s15_verify_counts_moves_and_verbose_lists_reasons() {
        let r = shell_repo();
        let first = "first-reason-quokka";
        let second = "second-reason-narwhal";
        let m = record(&r, "ci/t.sh", "ci/t2.sh", first);
        hand_move(&r, "ci/t.sh", "ci/t2.sh", T_X, Some(m));
        let m = record(&r, "ci/t2.sh", "ci/t3.sh", second);
        hand_move(&r, "ci/t2.sh", "ci/t3.sh", T_X, Some(m));
        let count =
            regex::Regex::new(r"(?i)(\b2\s+(recorded\s+)?moves?\b|\bmoves?\b[^\n0-9]{0,12}\b2\b)")
                .unwrap();
        let plain = verify(&r, &[]);
        assert!(plain.status.success(), "{}", text(&plain));
        assert!(
            count.is_match(&text(&plain)),
            "reports 2 moves: {}",
            text(&plain)
        );
        let verbose = verify(&r, &["--verbose"]);
        let out = text(&verbose);
        assert!(verbose.status.success(), "{out}");
        assert!(count.is_match(&out), "reports 2 moves: {out}");
        assert!(
            out.contains(first) && out.contains(second),
            "lists each reason: {out}"
        );
    }

    /// S16: A binding holds only when the scenario's own moves form a chain from the anchor to the current ref.
    #[test]
    fn s16_binding_property() {
        fn next(seed: &mut u64, n: u64) -> usize {
            *seed ^= *seed << 13;
            *seed ^= *seed >> 7;
            *seed ^= *seed << 17;
            (*seed % n) as usize
        }
        fn model(
            id: &str,
            anchor: Option<&str>,
            moves: &[Move],
            current: Option<&str>,
        ) -> BindingVerdict {
            let (Some(a), Some(c)) = (anchor, current) else {
                return BindingVerdict::Holds;
            };
            let mut at = a.to_string();
            for (i, m) in moves.iter().enumerate() {
                if m.scenario != id {
                    continue;
                }
                if m.from != at {
                    return BindingVerdict::BrokenChain(i);
                }
                at = m.to.clone();
            }
            if at == c {
                BindingVerdict::Holds
            } else {
                BindingVerdict::Moved { expected: at }
            }
        }
        let ids = ["X", "Y", "Z"];
        let refs = ["ci/t.sh", "ci/t.sh::one", "ci/t.sh::two", "tests/a.rs::f"];
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        for _ in 0..20_000 {
            let id = ids[next(&mut seed, 3)];
            let anchor = (next(&mut seed, 5) != 0).then(|| refs[next(&mut seed, 4)]);
            let current = (next(&mut seed, 5) != 0).then(|| refs[next(&mut seed, 4)]);
            let mut reached = anchor.unwrap_or(refs[0]).to_string();
            let mut moves = Vec::new();
            for _ in 0..next(&mut seed, 6) {
                let who = ids[next(&mut seed, 3)];
                let from = if who == id && next(&mut seed, 4) != 0 {
                    reached.clone()
                } else {
                    refs[next(&mut seed, 4)].to_string()
                };
                let to = refs[next(&mut seed, 4)];
                if who == id {
                    reached = to.to_string();
                }
                moves.push(mv(who, &from, to));
            }
            let got = judge_binding(id, anchor, &moves, current);
            assert_eq!(
                got,
                model(id, anchor, &moves, current),
                "{id} {anchor:?} {current:?}"
            );

            // Other scenarios' moves removed, then others added in front.
            let own: Vec<Move> = moves
                .iter()
                .filter(|m| m.scenario == id)
                .map(|m| mv(id, &m.from, &m.to))
                .collect();
            let mut padded = vec![mv("OTHER", "p", "q"), mv("OTHER", "r", "s")];
            padded.extend(own.iter().map(|m| mv(id, &m.from, &m.to)));
            for list in [own, padded] {
                let alt = judge_binding(id, anchor, &list, current);
                match (&got, &alt) {
                    (BindingVerdict::BrokenChain(i), BindingVerdict::BrokenChain(j)) => {
                        let nth = |l: &[Move], at: usize| {
                            l[..at].iter().filter(|m| m.scenario == id).count()
                        };
                        assert_eq!(nth(&moves, *i), nth(&list, *j));
                        assert_eq!(list[*j].scenario, id);
                    }
                    _ => assert_eq!(got, alt),
                }
            }
        }
    }

    /// S17: Re-proving named scenarios into a new file records their moves in the same commit.
    #[test]
    fn s17_only_records_a_move_in_the_same_commit() {
        let r = generated_repo();
        let before = r.git(&["rev-parse", "HEAD"]);
        let old = test_of(&r.spec(), X).unwrap();
        stub_writer(&r, &[(X, "new_zx01")]);
        let why = "reprove-reason-ibis";
        let o = r.hexa(&["spec", "tests", ADR, "--only", X, "--because", why]);
        assert!(o.status.success(), "{}", text(&o));
        r.commit_if_dirty("re-proved X");
        let rev = changing_commit(&r, &before, &old);
        let at = r.spec_at(&rev);
        let new = test_of(&at, X).unwrap();
        assert!(new.ends_with("new_zx01"), "{new}");
        let moves = moves_for(&at, X);
        assert_eq!(moves.len(), 1, "exactly one move: {at}");
        assert_move(&moves[0], &old, &new, why);
        assert!(moves_for(&at, Y).is_empty(), "no move for Y: {at}");
        assert!(moves_for(&r.spec_at(&before), X).is_empty());
        assert_eq!(moves_for(&r.spec(), X).len(), 1);
        r.write("src/lib.rs", "pub fn answer() -> i32 {\n    42\n}\n");
        r.commit("make green");
        let v = verify(&r, &[]);
        assert!(v.status.success(), "no binding failure: {}", text(&v));
    }

    /// S18: Replacing a generated test records a move for each scenario whose ref changes.
    #[test]
    fn s18_replace_records_a_move_per_changed_ref() {
        let r = generated_repo();
        let before = r.git(&["rev-parse", "HEAD"]);
        let old_x = test_of(&r.spec(), X).unwrap();
        let old_y = test_of(&r.spec(), Y).unwrap();
        stub_writer(&r, &[(X, "new_zx01"), (Y, "new_zy02")]);
        let why = "replace-reason-heron";
        let help = text(&r.hexa(&["spec", "tests", "--help"]));
        let file = old_x.split("::").next().unwrap().to_string();
        let mut args = vec!["spec", "tests", ADR, "--replace"];
        if help.contains("--replace <") {
            args.push(&file);
        }
        args.extend_from_slice(&["--because", why]);
        let o = r.hexa(&args);
        assert!(o.status.success(), "{}", text(&o));
        r.commit_if_dirty("replaced tests");
        let rev = changing_commit(&r, &before, &old_x);
        let at = r.spec_at(&rev);
        for (id, old, suffix) in [(X, &old_x, "new_zx01"), (Y, &old_y, "new_zy02")] {
            let new = test_of(&at, id).unwrap();
            assert!(new.ends_with(suffix), "{new}");
            let moves = moves_for(&at, id);
            assert_eq!(moves.len(), 1, "one move for {id}: {at}");
            assert_move(&moves[0], old, &new, why);
            assert_eq!(moves_for(&r.spec(), id).len(), 1);
        }
        assert_eq!(at["moves"].as_array().map(Vec::len), Some(2));
    }

    /// S19: Re-proving a scenario whose accepted text changed records no move.
    #[test]
    fn s19_changed_text_has_no_anchor_and_records_no_move() {
        let r = shell_repo();
        let old: Spec = serde_json::from_value(r.spec()).unwrap();

        let mut v = r.spec();
        scenario_mut(&mut v, X)["then"] = json!("the second answer holds");
        {
            let x = scenario_mut(&mut v, X).as_object_mut().unwrap();
            x.remove("test");
            x.remove("red");
        }
        r.put_spec(&v);
        accept(&r, None);
        r.commit("accept changed text");

        let new: Spec = serde_json::from_value(r.spec()).unwrap();
        let current = new.specs.iter().find(|s| s.id == X).unwrap().clone();
        assert_eq!(binding_anchor(&current, &[(true, old), (true, new)]), None);

        r.write(
            "ci/new.sh",
            "#!/bin/sh\ntest \"$(cat answer2 2>/dev/null)\" = 7\n",
        );
        r.commit("new red test");
        let o = spec_red(&r, X, "ci/new.sh");
        assert!(o.status.success(), "the binding is accepted: {}", text(&o));
        // A newly bound file is sealed by its own step, as `spec red` says.
        let sealed = r.hexa(&["spec", "seal", ADR]);
        assert!(sealed.status.success(), "{}", text(&sealed));
        assert_eq!(test_of(&r.spec(), X).as_deref(), Some("ci/new.sh"));
        assert!(moves_for(&r.spec(), X).is_empty());
        r.write("answer2", "7\n");
        r.commit("make green");
        assert!(moves_for(&r.spec(), X).is_empty());
        let out = verify(&r, &[]);
        assert!(out.status.success(), "no binding failure: {}", text(&out));
    }

    /// S20: Spec red cannot move an anchored binding.
    #[test]
    fn s20_spec_red_cannot_move_an_anchored_binding() {
        for cleared in [false, true] {
            let r = shell_repo();
            r.write("ci/b.sh", "#!/bin/sh\ntest -f never-there\n");
            if cleared {
                let mut v = r.spec();
                let x = scenario_mut(&mut v, X).as_object_mut().unwrap();
                x.remove("test");
                x.remove("red");
                r.put_spec(&v);
            }
            r.commit("another red test");
            let o = spec_red(&r, X, "ci/b.sh");
            let out = text(&o);
            assert!(!o.status.success(), "cleared={cleared} must refuse: {out}");
            assert!(
                out.contains("hexa spec move"),
                "cleared={cleared} names the verb: {out}"
            );
            let spec = r.spec();
            assert_ne!(
                test_of(&spec, X).as_deref(),
                Some("ci/b.sh"),
                "cleared={cleared}"
            );
            assert!(
                spec["moves"].as_array().map_or(true, |m| m.is_empty()),
                "no move is written: {spec}"
            );
        }
    }

    #[allow(dead_code)]
    fn unused_path_marker(_: &Path) {}
}

mod part_3 {
    #[allow(unused_imports, dead_code)]
    use std::collections::BTreeMap;
    #[allow(unused_imports)]
    use std::io::Write as _;
    #[allow(unused_imports)]
    use std::path::{Path, PathBuf};
    #[allow(unused_imports)]
    use std::process::{Command, Output, Stdio};

    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};

    const ADR: &str = "ADR-2601010000";

    fn sha(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    struct Repo {
        root: tempfile::TempDir,
        home: tempfile::TempDir,
        fake: tempfile::TempDir,
    }

    impl Repo {
        fn new() -> Repo {
            let r = Repo {
                root: tempfile::tempdir().unwrap(),
                home: tempfile::tempdir().unwrap(),
                fake: tempfile::tempdir().unwrap(),
            };
            r.git(&["init", "-q"]);
            r.git(&["config", "user.name", "Tester"]);
            r.git(&["config", "user.email", "tester@example.invalid"]);
            r.git(&["config", "commit.gpgsign", "false"]);
            r.write(
            &format!("docs/adrs/{ADR}-demo.md"),
            &format!("# {ADR}: demo\n\n**Status:** Accepted\n\n## Context\n\nDemo.\n\n## Decision\n\n1. **Answer.** The answer is 42.\n\n## Consequences\n\nNone.\n"),
        );
            r
        }

        fn path(&self) -> &Path {
            self.root.path()
        }

        fn write(&self, rel: &str, content: &str) {
            let p = self.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, content).unwrap();
        }

        fn read(&self, rel: &str) -> Vec<u8> {
            std::fs::read(self.path().join(rel)).unwrap()
        }

        fn file_sha(&self, rel: &str) -> String {
            sha(&self.read(rel))
        }

        fn git(&self, args: &[&str]) -> String {
            let o = Command::new("git")
                .args(args)
                .current_dir(self.path())
                .env("HOME", self.home.path())
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .output()
                .unwrap();
            assert!(
                o.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&o.stderr)
            );
            String::from_utf8_lossy(&o.stdout).trim().to_string()
        }

        fn commit(&self, msg: &str) -> String {
            self.git(&["add", "-A"]);
            self.git(&["commit", "-q", "--allow-empty", "-m", msg]);
            self.git(&["rev-parse", "HEAD"])
        }

        fn hexa(&self, args: &[&str]) -> (bool, Option<i32>, String) {
            self.hexa_in(args, "")
        }

        fn hexa_in(&self, args: &[&str], stdin: &str) -> (bool, Option<i32>, String) {
            let real_home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
            let rustup = std::env::var_os("RUSTUP_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| real_home.join(".rustup"));
            let cargo = std::env::var_os("CARGO_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| real_home.join(".cargo"));
            let path = format!(
                "{}:{}",
                self.fake.path().display(),
                std::env::var("PATH").unwrap_or_default()
            );
            let mut child = Command::new(env!("CARGO_BIN_EXE_hexa"))
                .args(args)
                .current_dir(self.path())
                .env("HOME", self.home.path())
                .env("RUSTUP_HOME", rustup)
                .env("CARGO_HOME", cargo)
                .env("PATH", path)
                .env("NO_COLOR", "1")
                .env("CARGO_NET_OFFLINE", "true")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env_remove("CARGO_TARGET_DIR")
                .env_remove("ANTHROPIC_API_KEY")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            if let Some(mut i) = child.stdin.take() {
                let _ = i.write_all(stdin.as_bytes());
            }
            let o: Output = child.wait_with_output().unwrap();
            let raw = format!(
                "{}\n{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            );
            let ansi = regex::Regex::new("\x1b\\[[0-9;]*m").unwrap();
            (
                o.status.success(),
                o.status.code(),
                ansi.replace_all(&raw, "").to_string(),
            )
        }

        fn spec_rel() -> String {
            format!("docs/specs/{ADR}.json")
        }

        fn spec(&self) -> Value {
            serde_json::from_slice(&self.read(&Self::spec_rel())).unwrap()
        }

        fn put_spec(&self, v: &Value) {
            self.write(
                &Self::spec_rel(),
                &(serde_json::to_string_pretty(v).unwrap() + "\n"),
            );
        }

        /// Every file outside `.git` and `target`.
        fn snapshot(&self) -> BTreeMap<PathBuf, Vec<u8>> {
            fn walk(dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
                for e in std::fs::read_dir(dir).unwrap() {
                    let e = e.unwrap();
                    let name = e.file_name();
                    if name == ".git" || name == "target" {
                        continue;
                    }
                    if e.path().is_dir() {
                        walk(&e.path(), out);
                    } else {
                        out.insert(e.path(), std::fs::read(e.path()).unwrap());
                    }
                }
            }
            let mut out = BTreeMap::new();
            walk(self.path(), &mut out);
            out
        }

        fn snapshot_without_spec(&self) -> BTreeMap<PathBuf, Vec<u8>> {
            let mut s = self.snapshot();
            s.remove(&self.path().join(Self::spec_rel()));
            s
        }

        /// The acceptance of `spec`'s behaviour. Leaves the spec file rewritten.
        fn acceptance(&self, spec: &Value) -> Value {
            let mut bare = spec.clone();
            {
                let o = bare.as_object_mut().unwrap();
                for k in ["accepted", "sealed", "red", "moves"] {
                    o.remove(k);
                }
                for s in o.get_mut("specs").unwrap().as_array_mut().unwrap() {
                    s.as_object_mut().unwrap().remove("test");
                    s.as_object_mut().unwrap().remove("red");
                }
            }

            self.put_spec(&bare);
            let (_, _, out) = self.hexa_in(&["spec", "accept", ADR], "y\ny\n");
            if let Some(a) = self.spec().get("accepted").filter(|a| !a.is_null()) {
                return a.clone();
            }
            let confirm = regex::Regex::new(r"--confirm[ =]([0-9a-f]{64})").unwrap();
            if let Some(c) = confirm.captures(&out) {
                self.hexa(&["spec", "accept", ADR, "--confirm", &c[1]]);
            } else {
                self.hexa(&["spec", "accept", ADR, "--yes"]);
            }
            if let Some(a) = self.spec().get("accepted").filter(|a| !a.is_null()) {
                return a.clone();
            }

            let typed: hexa_core::domain::spec::Spec =
                serde_json::from_value(bare.clone()).unwrap();
            let content = typed.accepted_content();
            let as_value = serde_json::to_value(&content).unwrap();
            let compact = serde_json::to_string(&content).unwrap();
            let pretty = serde_json::to_string_pretty(&content).unwrap();
            let candidates = [
                compact.clone(),
                pretty.clone(),
                format!("{pretty}\n"),
                format!("{compact}\n"),
                serde_json::to_string(&as_value).unwrap(),
                serde_json::to_string_pretty(&as_value).unwrap(),
            ];
            let hex = regex::Regex::new(r"[0-9a-f]{7,64}").unwrap();
            let observe = |h: &str| {
                let mut v = bare.clone();
                v["accepted"] = json!({"by": "Tester", "sha256": h});
                self.put_spec(&v);
                let a = self.hexa(&["spec", "check", ADR]).2;
                let b = self.hexa(&["spec", "verify", ADR, "--ci"]).2;
                hex.replace_all(&format!("{a}{b}"), "H").to_string()
            };
            let bogus = observe(&"0".repeat(64));
            for c in candidates {
                let h = sha(c.as_bytes());
                if observe(&h) != bogus {
                    return json!({"by": "Tester", "sha256": h});
                }
            }
            panic!("could not accept the fixture spec: {out}");
        }
    }

    fn scenario(id: &str, kind: &str, test: Option<&str>) -> Value {
        let mut v = json!({
            "id": id, "decision": ["§1"], "kind": kind, "description": format!("{id} works"),
            "given": "a", "when": "b", "then": "c"
        });
        if let Some(t) = test {
            v["test"] = json!(t);
        }
        v
    }

    fn rs_test(name: &str, marker: &str) -> String {
        format!("#[test]\nfn {name}() {{\n    let marker = \"{marker}\";\n    assert!(!marker.is_empty());\n}}\n")
    }

    const A: &str = "tests/a.rs::t_x";
    const G: &str = "tests/g.rs::t_x";
    const Y_OLD: &str = "tests/y_old.rs::t_y";
    const Y_NEW: &str = "tests/y_new.rs::t_y";

    fn file_of(r: &str) -> &str {
        r.split("::").next().unwrap()
    }

    fn set_ref(spec: &mut Value, id: &str, r: &str) {
        for s in spec["specs"].as_array_mut().unwrap() {
            if s["id"] == id {
                s["test"] = json!(r);
            }
        }
    }

    fn ref_of(spec: &Value, id: &str) -> Option<String> {
        spec["specs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == id)
            .unwrap()["test"]
            .as_str()
            .map(str::to_string)
    }

    fn moves_of(spec: &Value) -> Vec<Value> {
        spec.get("moves")
            .and_then(|m| m.as_array())
            .cloned()
            .unwrap_or_default()
    }

    fn without_moves(spec: &Value) -> Value {
        let mut v = spec.clone();
        v.as_object_mut().unwrap().remove("moves");
        v
    }

    /// X anchored to `x_ref`, Y to `Y_OLD`, committed and accepted.
    fn anchored(x_ref: &str, commit_old_file: bool) -> Repo {
        let r = Repo::new();
        if commit_old_file {
            r.write(file_of(x_ref), &rs_test("t_x", "FIRST_VERSION_MARK"));
        }
        r.write("tests/y_old.rs", &rs_test("t_y", "Y_OLD_MARK"));
        let red = r.commit("tests");
        let mut spec = json!({
            "adr": ADR, "feature": "demo",
            "specs": [scenario("X", "example", None), scenario("Y", "negative", None)]
        });
        let accepted = r.acceptance(&spec);
        spec["accepted"] = accepted;
        spec["red"] = json!(red);
        set_ref(&mut spec, "X", x_ref);
        set_ref(&mut spec, "Y", Y_OLD);
        let mut sealed = serde_json::Map::new();
        sealed.insert("tests/y_old.rs".into(), json!(r.file_sha("tests/y_old.rs")));
        if commit_old_file {
            sealed.insert(file_of(x_ref).into(), json!(r.file_sha(file_of(x_ref))));
        }
        spec["sealed"] = Value::Object(sealed);
        r.put_spec(&spec);
        r.commit("bind");
        r
    }

    /// X anchored to A; G exists; F (a.rs) was edited, then deleted.
    fn movable() -> Repo {
        let r = anchored(A, true);
        r.write("tests/a.rs", &rs_test("t_x", "LAST_VERSION_MARK"));
        r.commit("edit old");
        std::fs::remove_file(r.path().join("tests/a.rs")).unwrap();
        r.write("tests/g.rs", &rs_test("t_x", "NEW_FILE_MARK"));
        r.commit("new file");
        r
    }

    /// By hand: bind `id` to `to`, sealing the file, without a move record.
    fn hand_bind(r: &Repo, id: &str, to: &str) {
        let mut spec = r.spec();
        set_ref(&mut spec, id, to);
        spec["sealed"][file_of(to)] = json!(r.file_sha(file_of(to)));
        r.put_spec(&spec);
    }

    fn move_args(id: &str, to: &str, reason: &str, confirm: Option<&str>) -> Vec<String> {
        let mut a: Vec<String> = ["spec", "move", ADR, "--scenario"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        a.push(format!("{id}={to}"));
        a.push("--reason".into());
        a.push(reason.into());
        if let Some(c) = confirm {
            a.push(format!("--confirm={c}"));
        }
        a
    }

    fn run_move(
        r: &Repo,
        id: &str,
        to: &str,
        reason: &str,
        confirm: Option<&str>,
    ) -> (bool, Option<i32>, String) {
        let a = move_args(id, to, reason, confirm);
        let refs: Vec<&str> = a.iter().map(String::as_str).collect();
        r.hexa(&refs)
    }

    /// Preview, asserting it offers the confirm command for the new file's hash.
    fn preview(r: &Repo, id: &str, to: &str, reason: &str) -> (String, String) {
        let hash = r.file_sha(file_of(to));
        let (_, _, out) = run_move(r, id, to, reason, None);
        let tail = format!("--confirm {hash}");
        assert!(
            out.lines().any(|l| {
                let l = l.trim().trim_end_matches('`');
                l.contains("spec move") && l.ends_with(&tail)
            }),
            "no confirm command ending in `{tail}`: {out}"
        );
        (hash, out)
    }

    fn diff_has(out: &str, sign: char, mark: &str) -> bool {
        out.lines().any(|l| l.starts_with(sign) && l.contains(mark))
    }

    fn assert_one_move(
        before: &Value,
        after: &Value,
        id: &str,
        from: &str,
        to: &str,
        reason: &str,
    ) {
        let old = moves_of(before);
        let new = moves_of(after);
        assert_eq!(
            new.len(),
            old.len() + 1,
            "exactly one move appended: {after}"
        );
        assert_eq!(&new[..old.len()], &old[..], "earlier moves are kept");
        let m = &new[old.len()];
        assert_eq!(m["scenario"], id);
        assert_eq!(m["from"], from);
        assert_eq!(m["to"], to);
        assert_eq!(m["reason"], reason);
        for k in ["by", "at", "base_commit"] {
            assert!(
                m[k].as_str().is_some_and(|s| !s.is_empty()),
                "move lacks {k}: {m}"
            );
        }
        let mut expected = without_moves(before);
        set_ref(&mut expected, id, to);
        assert_eq!(
            without_moves(after),
            expected,
            "only the named scenario's ref may differ"
        );
    }

    // ---- S21 support ----

    fn writer_reply(r: &Repo, tests: &[(&str, &str)], note: &str) {
        let mut code = String::new();
        for (id, name) in tests {
            code.push_str(&format!(
            "/// {id}: {id} works\n#[test]\nfn {name}() {{\n    assert_eq!(demo::answer(), 42, \"{id} {note}\");\n}}\n\n"
        ));
        }
        let text = format!("```rust\n{code}```\n");
        let dir = r.fake.path();
        std::fs::write(dir.join("text.txt"), &text).unwrap();
        let result = json!({"type": "result", "subtype": "success", "is_error": false, "result": text,
        "session_id": "s", "total_cost_usd": 0.0, "duration_ms": 1, "num_turns": 1,
        "usage": {"input_tokens": 1, "output_tokens": 1}});
        std::fs::write(dir.join("json.txt"), format!("{result}\n")).unwrap();
        let assistant = json!({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "text", "text": text}]}});
        std::fs::write(dir.join("stream.txt"), format!("{assistant}\n{result}\n")).unwrap();
        let d = dir.display();
        let script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo '2.0.0 (Claude Code)'; exit 0; fi\ncat >/dev/null 2>&1\ncase \"$*\" in\n  *stream-json*) cat '{d}/stream.txt' ;;\n  *'--output-format json'*|*--output-format=json*) cat '{d}/json.txt' ;;\n  *) cat '{d}/text.txt' ;;\nesac\n"
    );
        let bin = dir.join("claude");
        std::fs::write(&bin, script).unwrap();
        let st = Command::new("chmod").arg("+x").arg(&bin).status().unwrap();
        assert!(st.success());
    }

    const ALL: [(&str, &str); 3] = [("X", "t_x"), ("Y", "t_y"), ("Z", "t_z")];

    /// A crate whose three scenarios were bound by `hexa spec tests` itself.
    fn generated() -> Repo {
        let r = Repo::new();
        r.write(
            "Cargo.toml",
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n",
        );
        r.write("src/lib.rs", "pub fn answer() -> u32 {\n    7\n}\n");
        r.write(".gitignore", "target\nCargo.lock\n");
        let mut spec = json!({
            "adr": ADR, "feature": "demo",
            "specs": [scenario("X", "example", None), scenario("Y", "negative", None), scenario("Z", "example", None)]
        });
        spec["accepted"] = r.acceptance(&spec);
        r.put_spec(&spec);
        r.commit("accepted spec");
        writer_reply(&r, &ALL, "first");
        let (ok, _, out) = r.hexa(&["spec", "tests", ADR]);
        assert!(ok, "initial generation failed: {out}");
        for (id, _) in ALL {
            assert!(ref_of(&r.spec(), id).is_some(), "{id} not bound: {out}");
        }
        if !r.git(&["status", "--porcelain"]).is_empty() {
            r.commit("generated");
        }
        r
    }

    /// Change Z's accepted text: it is pending and has no anchor.
    fn unanchor_z(r: &Repo) {
        let mut spec = r.spec();
        {
            let z = spec["specs"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|s| s["id"] == "Z")
                .unwrap();
            z["then"] = json!("something new");
            z.as_object_mut().unwrap().remove("test");
            z.as_object_mut().unwrap().remove("red");
        }
        let accepted = r.acceptance(&spec);
        spec["accepted"] = accepted;
        r.put_spec(&spec);
        r.commit("reword Z");
    }

    fn run_only(r: &Repo, ids: &[&str], reason: &str) {
        let tests: Vec<(&str, &str)> = ALL
            .iter()
            .copied()
            .filter(|(i, _)| ids.contains(i))
            .collect();
        writer_reply(r, &tests, reason);
        let joined = ids.join(",");
        let (mut ok, code, mut out) =
            r.hexa(&["spec", "tests", ADR, "--only", &joined, "--because", reason]);
        if !ok && code == Some(2) {
            let mut a = vec!["spec", "tests", ADR];
            for id in ids {
                a.extend(["--only", id]);
            }
            a.extend(["--because", reason]);
            let again = r.hexa(&a);
            ok = again.0;
            out = again.2;
        }
        assert!(ok, "spec tests --only {ids:?} failed: {out}");
    }

    fn check_moves(
        r: &Repo,
        before: &Value,
        anchored_ids: &[&str],
        must_change: &[&str],
        reason: &str,
    ) {
        let after: Value =
            serde_json::from_str(&r.git(&["show", &format!("HEAD:{}", Repo::spec_rel())])).unwrap();
        for id in must_change {
            assert_ne!(
                ref_of(before, id),
                ref_of(&after, id),
                "{id}'s ref should have changed"
            );
        }
        let old = moves_of(before);
        let new = moves_of(&after);
        assert!(new.len() >= old.len(), "moves were dropped: {after}");
        assert_eq!(
            &new[..old.len()],
            &old[..],
            "earlier moves keep order and content"
        );
        let added = &new[old.len()..];
        let mut expected = 0;
        for (id, _) in ALL {
            let (b, a) = (ref_of(before, id), ref_of(&after, id));
            let mine: Vec<&Value> = added.iter().filter(|m| m["scenario"] == id).collect();
            if anchored_ids.contains(&id) && b != a {
                expected += 1;
                assert_eq!(mine.len(), 1, "{id} needs exactly one move: {after}");
                assert_eq!(mine[0]["from"].as_str(), b.as_deref());
                assert_eq!(mine[0]["to"].as_str(), a.as_deref());
                assert_eq!(mine[0]["reason"], reason);
            } else {
                assert!(mine.is_empty(), "{id} must gain no move: {after}");
            }
        }
        assert_eq!(added.len(), expected, "no other moves: {after}");
    }

    /// S21: Moving verbs write exactly one move for each scenario whose ref changes.
    #[test]
    fn s21_moving_verbs_write_one_move_per_changed_ref() {
        // --only over several subsets, with Z unanchored.
        let r = generated();
        unanchor_z(&r);
        let before = r.spec();
        run_only(&r, &["X"], "reprove x");
        check_moves(&r, &before, &["X", "Y"], &["X"], "reprove x");
        let before = r.spec();
        run_only(&r, &["Y", "Z"], "reprove y and z");
        check_moves(&r, &before, &["X", "Y"], &["Y", "Z"], "reprove y and z");

        let r = generated();
        unanchor_z(&r);
        let before = r.spec();
        run_only(&r, &["Z"], "only the unanchored one");
        check_moves(&r, &before, &["X", "Y"], &["Z"], "only the unanchored one");

        // --replace: one renamed test, then none.
        let r = generated();
        let before = r.spec();
        writer_reply(
            &r,
            &[("X", "t_x"), ("Y", "t_y_renamed"), ("Z", "t_z")],
            "second",
        );
        let (ok, _, out) = r.hexa(&["spec", "tests", ADR, "--replace", "--because", "rename y"]);
        assert!(ok, "--replace failed: {out}");
        // `--replace` leaves the spec record for its caller to commit.
        r.commit("record the replacement");
        check_moves(&r, &before, &["X", "Y", "Z"], &["Y"], "rename y");

        let before = r.spec();
        writer_reply(
            &r,
            &[("X", "t_x"), ("Y", "t_y_renamed"), ("Z", "t_z")],
            "third",
        );
        let (ok, _, out) = r.hexa(&["spec", "tests", ADR, "--replace", "--because", "same names"]);
        assert!(ok, "second --replace failed: {out}");
        r.commit("record the second replacement");
        check_moves(&r, &before, &["X", "Y", "Z"], &[], "same names");
    }

    /// S22: Without confirmation, spec move shows the diff and the confirm command and writes nothing.
    #[test]
    fn s22_preview_shows_diff_and_writes_nothing() {
        let r = movable();
        let files = r.snapshot();
        let (_, out) = preview(&r, "X", G, "renamed the file");
        assert!(
            diff_has(&out, '-', "LAST_VERSION_MARK"),
            "diff from F's last version: {out}"
        );
        assert!(diff_has(&out, '+', "NEW_FILE_MARK"), "diff to G: {out}");
        assert!(
            !diff_has(&out, '-', "FIRST_VERSION_MARK"),
            "not F's first version: {out}"
        );
        assert!(
            out.lines().any(|l| l.starts_with("@@")),
            "a unified diff: {out}"
        );
        assert_eq!(r.snapshot(), files, "a preview writes nothing");
    }

    /// S23: Confirming with the current hash records one move and nothing else.
    #[test]
    fn s23_confirm_records_one_move_for_the_named_scenario() {
        let r = movable();
        r.write("tests/y_new.rs", &rs_test("t_y", "Y_NEW_MARK"));
        hand_bind(&r, "Y", Y_NEW);
        r.commit("move y by hand");
        let (hash, _) = preview(&r, "X", G, "renamed the file");
        let before = r.spec();
        let files = r.snapshot_without_spec();
        let (ok, _, out) = run_move(&r, "X", G, "renamed the file", Some(&hash));
        assert!(ok, "confirm failed: {out}");
        assert_one_move(&before, &r.spec(), "X", A, G, "renamed the file");
        assert!(moves_of(&r.spec()).iter().all(|m| m["scenario"] != "Y"));
        assert_eq!(r.snapshot_without_spec(), files);
    }

    /// S24: A move already present in history can be recorded afterwards.
    #[test]
    fn s24_a_move_already_in_history_can_be_recorded() {
        for chained in [false, true] {
            let a0 = "tests/a0.rs::t_x";
            let r = anchored(if chained { a0 } else { A }, true);
            if chained {
                let mut from = a0.to_string();
                for (to, mark) in [("tests/a1.rs::t_x", "MIDDLE_MARK"), (A, "REACHED_MARK")] {
                    r.write(file_of(to), &rs_test("t_x", mark));
                    r.commit("add next");
                    let base = r.git(&["rev-parse", "HEAD"]);
                    hand_bind(&r, "X", to);
                    let mut spec = r.spec();
                    let mut moves = moves_of(&spec);
                    moves.push(
                        json!({"scenario": "X", "from": from, "to": to, "by": "Tester",
                    "reason": "step", "at": "2026-01-01T00:00:00Z", "base_commit": base}),
                    );
                    spec["moves"] = Value::Array(moves);
                    spec["sealed"]
                        .as_object_mut()
                        .unwrap()
                        .remove(file_of(&from));
                    r.put_spec(&spec);
                    r.commit("recorded step");
                    from = to.to_string();
                }
            }
            let b = "tests/b.rs::t_x";
            r.write("tests/b.rs", &rs_test("t_x", "TARGET_MARK"));
            r.commit("add b");
            hand_bind(&r, "X", b);
            let mut spec = r.spec();
            spec["sealed"].as_object_mut().unwrap().remove("tests/a.rs");
            r.put_spec(&spec);
            r.commit("unrecorded move");

            let (ok, _, out) = r.hexa(&["spec", "verify", ADR, "--ci"]);
            assert!(!ok, "verify must fail on an unrecorded move: {out}");
            assert!(
                out.contains(A) && out.contains(b),
                "verify names both refs: {out}"
            );

            let (hash, out) = preview(&r, "X", b, "recording it late");
            let reached_mark = if chained {
                "REACHED_MARK"
            } else {
                "FIRST_VERSION_MARK"
            };
            assert!(
                diff_has(&out, '-', reached_mark),
                "diff starts where the chain reaches: {out}"
            );
            assert!(diff_has(&out, '+', "TARGET_MARK"), "{out}");
            if chained {
                assert!(
                    !diff_has(&out, '-', "FIRST_VERSION_MARK")
                        && !diff_has(&out, '-', "MIDDLE_MARK"),
                    "{out}"
                );
            }
            let before = r.spec();
            let files = r.snapshot_without_spec();
            let (ok, _, out) = run_move(&r, "X", b, "recording it late", Some(&hash));
            assert!(ok, "confirm failed: {out}");
            assert_one_move(&before, &r.spec(), "X", A, b, "recording it late");
            assert_eq!(ref_of(&r.spec(), "X").as_deref(), Some(b));
            assert_eq!(r.snapshot_without_spec(), files);

            r.commit("record the move");
            let (_, _, out) = r.hexa(&["spec", "verify", ADR, "--ci"]);
            assert!(!out.contains(A), "no binding failure for X remains: {out}");
        }
    }

    /// S25: A confirmation hash that does not match the new file now is refused.
    #[test]
    fn s25_a_stale_or_wrong_hash_is_refused() {
        let r = movable();
        let (stale, _) = preview(&r, "X", G, "why");
        r.write("tests/g.rs", &rs_test("t_x", "EDITED_AFTER_PREVIEW"));
        let files = r.snapshot();
        for wrong in [stale, "ab".repeat(32)] {
            let (ok, _, out) = run_move(&r, "X", G, "why", Some(&wrong));
            assert!(!ok, "{wrong} must be refused: {out}");
            assert_eq!(r.snapshot(), files, "nothing is written for {wrong}");
        }
        // The control: the file's hash now is accepted.
        let before = r.spec();
        let now = r.file_sha("tests/g.rs");
        let (ok, _, out) = run_move(&r, "X", G, "why", Some(&now));
        assert!(ok, "{out}");
        assert_one_move(&before, &r.spec(), "X", A, G, "why");
    }

    /// S26: Spec move refuses invalid requests, with or without confirmation.
    #[test]
    fn s26_invalid_requests_are_refused() {
        // The control: a valid request is offered and recorded.
        let control = movable();
        let (hash, _) = preview(&control, "X", G, "why");
        let before = control.spec();
        let (ok, _, out) = run_move(&control, "X", G, "why", Some(&hash));
        assert!(ok, "{out}");
        assert_one_move(&before, &control.spec(), "X", A, G, "why");

        let refused = |r: &Repo, to: &str, reason: &str, case: &str| {
            let hash = r.file_sha(file_of(to));
            let files = r.snapshot();
            for confirm in [None, Some(hash.as_str())] {
                let (ok, _, out) = run_move(r, "X", to, reason, confirm);
                assert!(!ok, "{case} (confirm {confirm:?}) must be refused: {out}");
                assert_eq!(r.snapshot(), files, "{case} must write nothing");
            }
        };

        let r = movable();
        refused(&r, G, "", "empty reason");
        refused(&r, G, "   ", "blank reason");

        let r = movable();
        let mut spec = r.spec();
        spec["accepted"]["sha256"] = json!("0".repeat(64));
        r.put_spec(&spec);
        r.commit("void acceptance");
        refused(&r, G, "why", "acceptance does not hold");

        let r = movable();
        r.write(
            "tests/h.rs",
            &format!(
                "// fn t_x() is only mentioned here\n{}",
                rs_test("other", "H")
            ),
        );
        r.commit("h");
        refused(&r, "tests/h.rs::t_x", "why", "test not in the file");

        let r = anchored("tests/never.rs::t_x", false);
        r.write("tests/g.rs", &rs_test("t_x", "NEW_FILE_MARK"));
        r.commit("g");
        refused(&r, G, "why", "old file with no version in history");

        let r = anchored(A, true);
        refused(&r, A, "why", "the anchor already reaches the ref");

        let r = movable();
        let (hash, _) = preview(&r, "X", G, "why");
        let (ok, _, out) = run_move(&r, "X", G, "why", Some(&hash));
        assert!(ok, "{out}");
        r.commit("moved");
        refused(&r, G, "again", "the recorded chain already reaches the ref");
    }

    /// S27: Spec move writes only a correctly confirmed record for one scenario.
    #[test]
    fn s27_only_the_current_hash_writes_and_only_one_record() {
        for already_held in [false, true] {
            let r = movable();
            if already_held {
                hand_bind(&r, "X", G);
                r.commit("held already");
            }
            let (hash, _) = preview(&r, "X", G, "why");
            let old_hash = sha(rs_test("t_x", "LAST_VERSION_MARK").as_bytes());
            let wrongs = [
                None,
                Some(String::new()),
                Some("yes".to_string()),
                Some("0".repeat(64)),
                Some(hash[..63].to_string()),
                Some(hash[..12].to_string()),
                Some(format!("{hash}0")),
                Some(old_hash),
            ];
            let files = r.snapshot();
            for w in &wrongs {
                run_move(&r, "X", G, "why", w.as_deref());
                assert_eq!(r.snapshot(), files, "confirmation {w:?} must write nothing");
            }
            let before = r.spec();
            let others = r.snapshot_without_spec();
            let (ok, _, out) = run_move(&r, "X", G, "why", Some(&hash));
            assert!(ok, "the current hash is accepted: {out}");
            assert_one_move(&before, &r.spec(), "X", A, G, "why");
            assert_eq!(
                r.snapshot_without_spec(),
                others,
                "no test or other file changes"
            );
        }
    }
}
