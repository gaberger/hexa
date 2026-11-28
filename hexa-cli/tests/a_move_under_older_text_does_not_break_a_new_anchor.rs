//! ADR-2611251652 §2: a move recorded under a scenario's older text belongs to
//! the anchor that text had. After the text is re-accepted and re-proved, it
//! must not read as a broken chain.

use std::path::Path;
use std::process::Command;

const ADR: &str = "ADR-2701010000";

fn run(dir: &Path, program: &str, args: &[&str]) -> (bool, String) {
    let o = Command::new(program).args(args).current_dir(dir).output().unwrap();
    (o.status.success(), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
}

fn hexa(dir: &Path, args: &[&str]) -> String {
    let (ok, out) = run(dir, env!("CARGO_BIN_EXE_hexa"), args);
    assert!(ok, "hexa {args:?}: {out}");
    out
}

fn commit(dir: &Path, msg: &str) -> String {
    for args in [vec!["add", "-A"], vec!["commit", "-q", "--allow-empty", "-m", msg]] {
        let (ok, out) = run(dir, "git", &args);
        assert!(ok, "git {args:?}: {out}");
    }
    run(dir, "git", &["rev-parse", "HEAD"]).1.trim().to_string()
}

fn write(dir: &Path, rel: &str, body: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body).unwrap();
}

fn spec_json(then: &str) -> String {
    let sc = |id: &str, kind: &str, then: &str| {
        format!(r#"{{"id":"{id}","decision":["§1"],"kind":"{kind}","description":"d","given":"g","when":"w","then":"{then}"}}"#)
    };
    format!(r#"{{"adr":"{ADR}","feature":"x","specs":[{},{}]}}"#, sc("S01", "example", then), sc("S02", "negative", "t"))
}

/// The check is that `answer` holds `want`.
fn check(want: u32) -> String {
    format!("#!/bin/sh\ntest \"$(cat answer 2>/dev/null)\" = {want}\n")
}

#[test]
fn a_move_under_older_text_does_not_break_a_new_anchor() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    for args in [vec!["init", "-q"], vec!["config", "user.name", "t"], vec!["config", "user.email", "t@example.invalid"]] {
        assert!(run(d, "git", &args).0);
    }
    write(d, "Cargo.toml", "[package]\nname = \"r\"\nversion = \"0.1.0\"\nedition = \"2021\"\n");
    write(d, "src/lib.rs", "");
    write(
        d,
        &format!("docs/adrs/{ADR}-x.md"),
        &format!("# {ADR}: x\n\n**Status:** Accepted\n\n## Decision\n\n1. The answer holds.\n"),
    );
    let spec = format!("docs/specs/{ADR}.json");
    write(d, &spec, &spec_json("42"));
    commit(d, "draft");
    hexa(d, &["spec", "accept", ADR, "--by", "t"]);
    commit(d, "accepted");

    // Bound red at a.sh, then moved to b.sh by a recorded move.
    write(d, "ci/a.sh", &check(42));
    let red = commit(d, "red");
    hexa(d, &["spec", "red", ADR, &red, "--test", "S01=ci/a.sh", "--test", "S02=ci/a.sh"]);
    hexa(d, &["spec", "seal", ADR]);
    write(d, "answer", "42\n");
    commit(d, "bound");
    write(d, "ci/b.sh", &check(42));
    commit(d, "copy");
    let preview = hexa(d, &["spec", "move", ADR, "--scenario", "S01=ci/b.sh", "--reason", "split"]);
    let hash = preview.split("--confirm ").nth(1).unwrap().split_whitespace().next().unwrap().to_string();
    hexa(d, &["spec", "move", ADR, "--scenario", "S01=ci/b.sh", "--reason", "split", "--confirm", &hash]);
    hexa(d, &["spec", "seal", ADR]);
    commit(d, "moved");
    hexa(d, &["spec", "verify", ADR]);

    // S01's text changes and is accepted again; it is re-proved into c.sh.
    let text = std::fs::read_to_string(d.join(&spec)).unwrap().replacen("\"then\": \"42\"", "\"then\": \"43\"", 1);
    write(d, &spec, &text);
    commit(d, "reword");
    hexa(d, &["spec", "accept", ADR, "--by", "t"]);
    commit(d, "re-accepted");
    write(d, "ci/c.sh", &check(43));
    let red = commit(d, "red again");
    hexa(d, &["spec", "red", ADR, &red, "--test", "S01=ci/c.sh"]);
    hexa(d, &["spec", "seal", ADR]);
    write(d, "ci/a.sh", &check(42));
    commit(d, "rebound");

    // S02 still checks 42 and S01 now checks 43: judge the bindings alone.
    let (_, out) = run(d, env!("CARGO_BIN_EXE_hexa"), &["spec", "verify", ADR]);
    assert!(!out.contains("binding"), "the older move must not apply to the new anchor: {out}");
    assert!(out.contains("1 move"), "the older move stays recorded: {out}");
}

/// A repository whose S01 and S02 are accepted, red at `ci/a.sh`, bound and green.
fn bound_repo(d: &Path) {
    for args in [vec!["init", "-q", "-b", "main"], vec!["config", "user.name", "t"], vec!["config", "user.email", "t@example.invalid"]] {
        assert!(run(d, "git", &args).0);
    }
    write(d, "Cargo.toml", "[package]\nname = \"r\"\nversion = \"0.1.0\"\nedition = \"2021\"\n");
    write(d, "src/lib.rs", "");
    write(d, &format!("docs/adrs/{ADR}-x.md"), &format!("# {ADR}: x\n\n**Status:** Accepted\n\n## Decision\n\n1. The answer holds.\n"));
    write(d, &format!("docs/specs/{ADR}.json"), &spec_json("42"));
    commit(d, "draft");
    hexa(d, &["spec", "accept", ADR, "--by", "t"]);
    commit(d, "accepted");
    write(d, "ci/a.sh", &check(42));
    let red = commit(d, "red");
    hexa(d, &["spec", "red", ADR, &red, "--test", "S01=ci/a.sh", "--test", "S02=ci/a.sh"]);
    hexa(d, &["spec", "seal", ADR]);
    write(d, "answer", "42\n");
    commit(d, "bound");
}

fn git(d: &Path, args: &[&str]) {
    let (ok, out) = run(d, "git", args);
    assert!(ok, "git {args:?}: {out}");
}

/// Rebind S01 by hand to `to`, with a weakened test, as the attack does.
fn hand_rebind(d: &Path, to: &str) {
    write(d, to, "#!/bin/sh\ntest -f answer\n");
    let spec = format!("docs/specs/{ADR}.json");
    let mut v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(d.join(&spec)).unwrap()).unwrap();
    v["specs"][0]["test"] = serde_json::json!(to);
    let (_, sum) = run(d, "sha256sum", &[to]);
    v["sealed"][to] = serde_json::json!(sum.split_whitespace().next().unwrap());
    write(d, &spec, &(serde_json::to_string_pretty(&v).unwrap() + "\n"));
}

/// A side branch whose spec wins the merge must not take main's binding out of
/// history: main's own line is the history that anchors.
#[test]
fn a_merge_that_takes_a_side_branchs_spec_does_not_move_the_anchor() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    bound_repo(d);
    git(d, &["checkout", "-q", "-b", "side", "HEAD~1"]);
    // The side branch never saw main's binding: it binds S01 to its own test.
    write(d, "ci/side.sh", "#!/bin/sh\ntest -f answer\n");
    let red = commit(d, "side red");
    hexa(d, &["spec", "red", ADR, &red, "--test", "S01=ci/side.sh", "--test", "S02=ci/side.sh"]);
    hexa(d, &["spec", "seal", ADR]);
    write(d, "answer", "42\n");
    commit(d, "side bound");
    git(d, &["checkout", "-q", "main"]);
    let _ = run(d, "git", &["merge", "-q", "--no-ff", "-X", "theirs", "-m", "merge side", "side"]);
    git(d, &["checkout", "side", "--", &format!("docs/specs/{ADR}.json")]);
    commit(d, "resolved with the side's spec");
    let (ok, out) = run(d, env!("CARGO_BIN_EXE_hexa"), &["spec", "verify", ADR]);
    assert!(!ok && out.contains("binding"), "main bound S01 to ci/a.sh and nothing records a move: {out}");
}

/// A recorded move made on a side branch still verifies after the merge.
#[test]
fn a_recorded_move_on_a_merged_branch_still_holds() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    bound_repo(d);
    git(d, &["checkout", "-q", "-b", "side"]);
    write(d, "ci/b.sh", &check(42));
    commit(d, "copy");
    let preview = hexa(d, &["spec", "move", ADR, "--scenario", "S01=ci/b.sh", "--reason", "split"]);
    let hash = preview.split("--confirm ").nth(1).unwrap().split_whitespace().next().unwrap().to_string();
    hexa(d, &["spec", "move", ADR, "--scenario", "S01=ci/b.sh", "--reason", "split", "--confirm", &hash]);
    hexa(d, &["spec", "seal", ADR]);
    commit(d, "moved");
    git(d, &["checkout", "-q", "main"]);
    git(d, &["merge", "-q", "--no-ff", "-m", "merge side", "side"]);
    hexa(d, &["spec", "verify", ADR]);
}

/// An unrecorded hand move made after a merge is still refused.
#[test]
fn a_hand_move_after_a_merge_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    bound_repo(d);
    git(d, &["checkout", "-q", "-b", "side"]);
    write(d, "note", "x\n");
    commit(d, "side work");
    git(d, &["checkout", "-q", "main"]);
    git(d, &["merge", "-q", "--no-ff", "-m", "merge side", "side"]);
    hand_rebind(d, "ci/weak.sh");
    commit(d, "hand move");
    let (ok, out) = run(d, env!("CARGO_BIN_EXE_hexa"), &["spec", "verify", ADR]);
    assert!(!ok && out.contains("binding moved"), "{out}");
}

/// What was reviewed is what must be committed: a strong test confirmed and a
/// weak one committed in its place is refused.
#[test]
fn a_move_confirmed_on_content_history_never_held_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    bound_repo(d);
    write(d, "ci/b.sh", &check(42));
    let preview = hexa(d, &["spec", "move", ADR, "--scenario", "S01=ci/b.sh", "--reason", "split"]);
    let hash = preview.split("--confirm ").nth(1).unwrap().split_whitespace().next().unwrap().to_string();
    hexa(d, &["spec", "move", ADR, "--scenario", "S01=ci/b.sh", "--reason", "split", "--confirm", &hash]);
    write(d, "ci/b.sh", "#!/bin/sh\ntest -f answer\n");
    hexa(d, &["spec", "seal", ADR]);
    commit(d, "the weak one");
    let (ok, out) = run(d, env!("CARGO_BIN_EXE_hexa"), &["spec", "verify", ADR]);
    assert!(!ok && out.contains("content reviewed"), "{out}");
}

/// Moving the spec file to another of its homes keeps its anchors.
#[test]
fn moving_the_spec_file_keeps_the_anchors() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    bound_repo(d);
    std::fs::create_dir_all(d.join(".hexa/specs")).unwrap();
    git(d, &["mv", &format!("docs/specs/{ADR}.json"), &format!(".hexa/specs/{ADR}.json")]);
    let spec = format!(".hexa/specs/{ADR}.json");
    write(d, "ci/weak.sh", "#!/bin/sh\ntest -f answer\n");
    let mut v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(d.join(&spec)).unwrap()).unwrap();
    v["specs"][0]["test"] = serde_json::json!("ci/weak.sh");
    let (_, sum) = run(d, "sha256sum", &["ci/weak.sh"]);
    v["sealed"]["ci/weak.sh"] = serde_json::json!(sum.split_whitespace().next().unwrap());
    write(d, &spec, &(serde_json::to_string_pretty(&v).unwrap() + "\n"));
    commit(d, "spec moves home and S01 is rebound by hand, in one commit");
    let (ok, out) = run(d, env!("CARGO_BIN_EXE_hexa"), &["spec", "verify", ADR]);
    assert!(!ok && out.contains("binding moved"), "{out}");
}

/// A project that sits below the top of its git repository is judged the same.
#[test]
fn a_project_nested_in_a_larger_repository_is_judged_too() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path();
    let d = &top.join("services/app");
    std::fs::create_dir_all(d).unwrap();
    git(top, &["init", "-q", "-b", "main"]);
    git(top, &["config", "user.name", "t"]);
    git(top, &["config", "user.email", "t@example.invalid"]);
    write(d, "Cargo.toml", "[package]\nname = \"r\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n");
    write(d, "src/lib.rs", "");
    write(d, &format!("docs/adrs/{ADR}-x.md"), &format!("# {ADR}: x\n\n**Status:** Accepted\n\n## Decision\n\n1. The answer holds.\n"));
    write(d, &format!("docs/specs/{ADR}.json"), &spec_json("42"));
    commit(d, "draft");
    hexa(d, &["spec", "accept", ADR, "--by", "t"]);
    commit(d, "accepted");
    // Bound by writing the record: `spec red` has its own limit in a nested
    // project, and this is about how history is read.
    write(d, "ci/a.sh", &check(42));
    let spec = format!("docs/specs/{ADR}.json");
    let mut v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(d.join(&spec)).unwrap()).unwrap();
    let (_, sum) = run(d, "sha256sum", &["ci/a.sh"]);
    for i in 0..2 {
        v["specs"][i]["test"] = serde_json::json!("ci/a.sh");
    }
    v["sealed"] = serde_json::json!({"ci/a.sh": sum.split_whitespace().next().unwrap()});
    write(d, &spec, &(serde_json::to_string_pretty(&v).unwrap() + "\n"));
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(d.join("ci/a.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    write(d, "answer", "42\n");
    commit(d, "bound");
    hexa(d, &["spec", "verify", ADR]);
    hand_rebind(d, "ci/weak.sh");
    commit(d, "hand move");
    let (ok, out) = run(d, env!("CARGO_BIN_EXE_hexa"), &["spec", "verify", ADR]);
    assert!(!ok && out.contains("binding moved"), "{out}");
}
