//! Which runner a scenario's test names, and where tests run (ADR-2611191120).
//!
//! Pure rules over values: parsing a test reference, choosing its runner by
//! extension, filling command templates, the remote-run protocol (markers and
//! the commit it must name), and judging one test's output. Running anything
//! is the adapter's (`hexa-exec`).

use std::collections::BTreeMap;

/// A scenario's test: `<file>::<name>`, or a bare `<file>.sh` (§1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestRef {
    /// The path from the repository root, as written.
    pub file: String,
    /// What follows the first `::`; `None` only for a bare `.sh`.
    pub name: Option<String>,
}

impl TestRef {
    /// The file's extension, without its dot.
    pub fn ext(&self) -> &str {
        extension(&self.file).unwrap_or("")
    }
}

/// The extension of the last component of `file`, when it has a stem.
fn extension(file: &str) -> Option<&str> {
    let base = file.rsplit('/').next()?;
    let (stem, ext) = base.rsplit_once('.')?;
    (!stem.is_empty() && !ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphanumeric())).then_some(ext)
}

/// Split `r` at its first `::` (§1). The file must have an extension; the
/// name must be non-empty, except that a `.sh` file may have none.
pub fn parse_test_ref(r: &str) -> Option<TestRef> {
    let (file, name) = match r.split_once("::") {
        Some((f, n)) => (f, Some(n)),
        None => (r, None),
    };
    let ext = extension(file)?;
    match name {
        Some("") => None,
        None if ext != "sh" => None,
        _ => Some(TestRef { file: file.to_string(), name: name.map(str::to_string) }),
    }
}

/// The runner a test reference uses (§1, §2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Runner {
    Cargo,
    Vitest,
    Bash,
    /// pytest, for a `.py` test (ADR-2611211324 §8).
    Pytest,
    /// A `test.runners` template from `.hexa/project.json`.
    Configured(String),
}

/// The extensions vitest runs (§1).
const VITEST_EXTENSIONS: [&str; 8] = ["ts", "tsx", "js", "mjs", "cjs", "mts", "cts", "jsx"];

/// The runner for files with extension `ext`: a configured template first,
/// then the built-in ones; `None` when there is neither (§1, §2).
pub fn runner_for(ext: &str, configured: &BTreeMap<String, String>) -> Option<Runner> {
    if let Some(t) = configured.get(ext).filter(|t| !t.trim().is_empty()) {
        return Some(Runner::Configured(t.clone()));
    }
    match ext {
        "rs" => Some(Runner::Cargo),
        "sh" => Some(Runner::Bash),
        "py" => Some(Runner::Pytest),
        e if VITEST_EXTENSIONS.contains(&e) => Some(Runner::Vitest),
        _ => None,
    }
}

/// Whether a file bound to a scenario is a test file that may be sealed (§8).
pub fn is_test_file(path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let Some((base, dirs)) = parts.split_last() else { return false };
    dirs.iter().any(|d| matches!(*d, "tests" | "test" | "__tests__"))
        || base.contains(".test.")
        || base.contains(".spec.")
        || extension(path) == Some("sh")
        || (base.ends_with(".py") && (base.starts_with("test_") || base.ends_with("_test.py")))
}

/// `s` as one single-quoted shell word.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// `template` with each `{key}` of `vars` replaced by its value, single-quoted
/// for the shell (§2, §3). One pass: a value is never itself substituted.
pub fn fill(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    'scan: while !rest.is_empty() {
        if rest.starts_with('{') {
            for (k, v) in vars {
                let key = format!("{{{k}}}");
                if let Some(after) = rest.strip_prefix(key.as_str()) {
                    out.push_str(&shell_quote(v));
                    rest = after;
                    continue 'scan;
                }
            }
        }
        let c = rest.chars().next().unwrap_or_default();
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// `Err` naming what is missing when a `test.remote` template lacks `{commit}`
/// or `{cmd}` (§3).
pub fn check_remote_template(template: &str) -> Result<(), String> {
    let missing: Vec<&str> = ["{commit}", "{cmd}"].into_iter().filter(|p| !template.contains(p)).collect();
    if missing.is_empty() {
        return Ok(());
    }
    Err(format!(
        "test.remote in .hexa/project.json lacks {}: the template gets the commit as {{commit}} and the command to run in \
         a checkout of it as {{cmd}}, e.g. `bash ci/remote-gate.sh --commit {{commit}} --no-gate --extra {{cmd}}`",
        missing.join(" and ")
    ))
}

/// Whether `commit` is a full 40-hex commit id.
fn is_full_commit(commit: &str) -> bool {
    commit.len() == 40 && commit.chars().all(|c| c.is_ascii_hexdigit())
}

/// The command that runs `cmd` at `commit` through `template` (§3). `commit`
/// must be a full 40-hex commit id.
pub fn remote_command(template: &str, commit: &str, cmd: &str) -> Result<String, String> {
    check_remote_template(template)?;
    if !is_full_commit(commit) {
        return Err(format!("`{commit}` is not a full commit id (40 hex characters): a remote run names exactly one commit"));
    }
    Ok(fill(template, &[("commit", commit), ("cmd", cmd)]))
}

/// Whether a remote run's output names the full commit it was sent (§5).
pub fn runner_ran(output: &str, commit: &str) -> bool {
    is_full_commit(commit) && output.contains(commit)
}

const MARK: &str = "hexa-batch";

/// One script that runs every command in turn, each between a begin and an
/// end marker holding `tag`, a per-run tag (not a secret: it only tells this run's markers from text a command prints), the end marker carrying its exit status (§4).
/// The markers are printed by `printf` from their parts, so a runner that
/// echoes the script does not print them; each command reads no stdin.
pub fn batch_script(tag: &str, cmds: &[String]) -> String {
    let mut s = String::from("set +e\n");
    for (i, c) in cmds.iter().enumerate() {
        s.push_str(&format!(
            "printf '\\n%s:%s:begin:%d\\n' {MARK} {n} {i}\nbash -o pipefail -c {c} </dev/null 2>&1\nrc=$?\nprintf '\\n%s:%s:end:%d:%d\\n' {MARK} {n} {i} \"$rc\"\n",
            n = shell_quote(tag),
            c = shell_quote(c)
        ));
    }
    s
}

/// The environment variable that tells a hexa verb it is running on the
/// remote runner, in a checkout of the commit it names (ADR-2611240900 §3).
pub const ON_RUNNER_ENV: &str = "HEXA_ON_RUNNER";

/// [`batch_script`] for `commit`: each command also sees [`ON_RUNNER_ENV`] set
/// to `commit` (ADR-2611240900 §3).
pub fn batch_script_for(tag: &str, commit: &str, cmds: &[String]) -> String {
    format!("export {ON_RUNNER_ENV}={}\n{}", shell_quote(commit), batch_script(tag, cmds))
}

/// Whether this process runs on the remote runner (ADR-2611240900 §3):
/// `marker` is [`ON_RUNNER_ENV`]'s value, `head` the checkout's HEAD.
pub fn on_runner(marker: Option<&str>, head: Option<&str>) -> Result<bool, String> {
    let Some(marker) = marker.map(str::trim).filter(|m| !m.is_empty()) else { return Ok(false) };
    if !is_full_commit(marker) {
        return Err(format!(
            "{ON_RUNNER_ENV}={marker} is not a full commit id (40 hex characters): it names the commit a remote runner \
             checked out, so a hexa verb there runs its tests in place. Unset it, or set it to `$(git rev-parse HEAD)` \
             in the runner's checkout"
        ));
    }
    let Some(head) = head.filter(|h| !h.is_empty()) else {
        return Err(format!(
            "{ON_RUNNER_ENV} is set to {marker}, but this checkout has no HEAD commit to compare it with: \
             run the verb in the runner's checkout of {marker}, or unset {ON_RUNNER_ENV}"
        ));
    };
    if head != marker {
        return Err(format!(
            "{ON_RUNNER_ENV} names {marker}, but this checkout's HEAD is {head}: the runner checked out another commit \
             than the one it was sent, so nothing runs. Check that the runner checks out the commit it is given, \
             or unset {ON_RUNNER_ENV} if this is not the runner's checkout"
        ));
    }
    Ok(true)
}

/// Each command's (exit 0, output) from a batch run's output, in order. `Err`
/// when any end marker is missing: the run is a runner failure (§4, §5).
pub fn split_batch(tag: &str, n: usize, output: &str) -> Result<Vec<(bool, String)>, String> {
    let lines: Vec<&str> = output.lines().map(|l| l.trim_end_matches('\r')).collect();
    let mut out = Vec::with_capacity(n);
    let mut at = 0;
    for i in 0..n {
        let begin = format!("{MARK}:{tag}:begin:{i}");
        let end = format!("{MARK}:{tag}:end:{i}:");
        let Some(b) = lines[at..].iter().position(|l| *l == begin).map(|p| p + at) else {
            return Err(format!("the runner's output has no begin marker for test {}: it did not run every test", i + 1));
        };
        let Some(e) = lines[b + 1..].iter().position(|l| l.starts_with(&end)).map(|p| p + b + 1) else {
            return Err(format!(
                "the runner's output has no end marker for test {}: the run was cut short, or the runner lost its output",
                i + 1
            ));
        };
        let Ok(rc) = lines[e][end.len()..].trim().parse::<i64>() else {
            return Err(format!("the end marker of test {} carries no exit status", i + 1));
        };
        let body = lines[b + 1..e].join("\n");
        out.push((rc == 0, body.trim_end_matches('\n').to_string() + "\n"));
        at = e + 1;
    }
    Ok(out)
}

/// What one test's run established.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Passed,
    Failed(String),
    /// Nothing ran, or what matched was skipped.
    Vacuous,
    /// More than one test ran.
    Ambiguous(u64),
    /// The output is not one this judge can read: never a pass.
    Unreadable(String),
}

/// The vitest `-t` pattern that selects a test named `name` (§1): the name,
/// regex-escaped, at the end of the test's full name.
pub fn vitest_pattern(name: &str) -> String {
    let mut p = String::with_capacity(name.len() + 2);
    for c in name.chars() {
        if "\\^$.|?*+()[]{}/".contains(c) {
            p.push('\\');
        }
        p.push(c);
    }
    p.push('$');
    p
}

/// The first JSON object in `output`, at the start of a line, that is a
/// vitest (Jest-shaped) report.
fn vitest_report(output: &str) -> Option<serde_json::Value> {
    let mut at = 0;
    for line in output.split_inclusive('\n') {
        if line.trim_start().starts_with('{') {
            let start = at + (line.len() - line.trim_start().len());
            let mut it = serde_json::Deserializer::from_str(&output[start..]).into_iter::<serde_json::Value>();
            if let Some(Ok(v)) = it.next() {
                if v.get("testResults").is_some_and(|t| t.is_array()) {
                    return Some(v);
                }
            }
        }
        at += line.len();
    }
    None
}

/// Judge a `vitest run --reporter=json` output for the test named `name` (§1).
pub fn judge_vitest(output: &str, name: &str) -> Verdict {
    let Some(report) = vitest_report(output) else {
        return Verdict::Unreadable("no vitest JSON report in the output: vitest did not run, or not with --reporter=json".into());
    };
    let files = report["testResults"].as_array().cloned().unwrap_or_default();
    let mut ran: Vec<&serde_json::Value> = Vec::new();
    for f in &files {
        let results = f["assertionResults"].as_array();
        // A file that failed to load runs no test and says why.
        if results.is_none_or(|r| r.is_empty()) && f["status"] == "failed" {
            let msg = f["message"].as_str().unwrap_or("the test file failed");
            return Verdict::Failed(format!("{}: {msg}", f["name"].as_str().unwrap_or("a test file")));
        }
        ran.extend(results.into_iter().flatten().filter(|a| matches!(a["status"].as_str(), Some("passed" | "failed"))));
    }
    match ran.as_slice() {
        [] => Verdict::Vacuous,
        [one] => {
            let full = one["fullName"].as_str().unwrap_or("");
            let title = one["title"].as_str().unwrap_or("");
            if full != name && title != name {
                return Verdict::Failed(format!("the test that ran is `{full}`, not `{name}`"));
            }
            if one["status"] == "passed" {
                Verdict::Passed
            } else {
                let msgs: Vec<&str> = one["failureMessages"].as_array().into_iter().flatten().filter_map(|m| m.as_str()).collect();
                Verdict::Failed(format!("`{full}` failed: {}", msgs.join("\n")))
            }
        }
        many => Verdict::Ambiguous(many.len() as u64),
    }
}

fn tail(out: &str, n: usize) -> String {
    let lines: Vec<&str> = out.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// Judge a bash test script's run (§1): exit 0 passes, unless a runner the
/// evidence reader knows reports a failure or zero tests.
pub fn judge_script(exit_ok: bool, output: &str) -> Verdict {
    use crate::domain::evidence::{read_evidence, Evidence};
    if !exit_ok {
        return Verdict::Failed(format!("the script exited non-zero:\n{}", tail(output, 10)));
    }
    match read_evidence(true, output, false) {
        Evidence::Passed { .. } | Evidence::Unrecognised | Evidence::Uncounted => Verdict::Passed,
        Evidence::Vacuous => Verdict::Vacuous,
        Evidence::Failed { reason } => Verdict::Failed(reason),
    }
}

/// Judge a configured runner's run, by the evidence reader (§2): exactly one
/// counted test passes.
pub fn judge_counted(exit_ok: bool, output: &str) -> Verdict {
    use crate::domain::evidence::{read_evidence, Evidence, KNOWN_RUNNERS};
    match read_evidence(exit_ok, output, false) {
        Evidence::Passed { tests: 1 } => Verdict::Passed,
        Evidence::Passed { tests } => Verdict::Ambiguous(tests),
        Evidence::Vacuous => Verdict::Vacuous,
        Evidence::Failed { reason } => Verdict::Failed(format!("{reason}:\n{}", tail(output, 10))),
        Evidence::Unrecognised | Evidence::Uncounted => Verdict::Unreadable(format!(
            "the runner exited 0 but no test runner hexa reads wrote its output ({KNOWN_RUNNERS})"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "0123456789abcdef0123456789abcdef01234567";
    const B: &str = "89abcdef0123456789abcdef0123456789abcdef";

    /// S10, S11 (the rule): the marker runs here only for the commit checked out.
    #[test]
    fn the_runner_marker_holds_only_for_the_commit_checked_out() {
        assert_eq!(on_runner(None, Some(A)), Ok(false), "no marker: not on the runner");
        assert_eq!(on_runner(Some(""), Some(A)), Ok(false), "an empty marker is no marker");
        assert_eq!(on_runner(Some(A), Some(A)), Ok(true));
        assert_eq!(on_runner(Some(&format!(" {A}\n")), Some(A)), Ok(true), "surrounding space is not part of the id");
        let e = on_runner(Some(B), Some(A)).unwrap_err();
        assert!(e.contains(ON_RUNNER_ENV) && e.contains(A) && e.contains(B), "{e}");
        let e = on_runner(Some("1"), Some(A)).unwrap_err();
        assert!(e.contains(ON_RUNNER_ENV) && e.contains("40"), "a marker that is not a commit id: {e}");
        let e = on_runner(Some(A), None).unwrap_err();
        assert!(e.contains(ON_RUNNER_ENV) && e.contains("HEAD"), "{e}");
    }

    /// S12 (the script): the batch exports the marker before the commands run.
    #[test]
    fn the_batch_script_exports_the_runner_marker() {
        let s = batch_script_for("t", A, &["true".to_string()]);
        let at = s.find(&format!("export {ON_RUNNER_ENV}=")).expect("the marker is exported");
        assert!(at < s.find("bash -o pipefail").unwrap(), "before the first command: {s}");
        assert!(s.contains(A), "{s}");
        assert!(batch_script_for("t", "'; touch pwned; '", &["true".to_string()]).contains("''\\'';"), "the commit is quoted as one word");
    }
}
