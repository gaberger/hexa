//! What every `claude -p` call hexa makes has in common: the budget is
//! asked first, the call runs with `--output-format json`, and the answer's
//! own usage and `total_cost_usd` are recorded before the text is returned.
//! Before this, the frontier path ran for its text alone and its cost was
//! never written anywhere.
//!
//! The model recorded is the one the answer itself names (ADR-2609160300 §4),
//! read out of the JSON envelope by `spend::model_from_frontier_json`. The
//! earlier value, `"claude-code"`, was the name of a code path, not of a
//! model: it could not tell Opus from Haiku, so a spend log full of it could
//! not say what the money was spent on, and no per-model total or price
//! comparison could be recovered from it afterwards. An answer that names no
//! model is recorded as unknown rather than as the path that carried it.

use serde_json::Value;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::ports::{Frontier, FrontierError, SpendLog};

/// The arguments that make `claude -p` report its usage.
const OUTPUT_JSON: [&str; 2] = ["--output-format", "json"];

pub use crate::ports::{HEXA_GIT_EMAIL, HEXA_GIT_NAME};

fn claude_binary() -> String {
    std::env::var("HEXA_CLAUDE_BINARY").unwrap_or_else(|_| "claude".to_string())
}

/// The arguments of one `claude -p` call. With a model, `--model` pins the
/// run to the model the operator named instead of the CLI's default.
fn claude_args(model: Option<&str>, prompt: &str) -> Vec<String> {
    let mut args = vec!["-p".to_string()];
    args.extend(OUTPUT_JSON.iter().map(|a| a.to_string()));
    if let Some(m) = model {
        args.push("--model".to_string());
        args.push(m.to_string());
    }
    args.push("--dangerously-skip-permissions".to_string());
    args.push(prompt.to_string());
    args
}

/// One `claude -p` call: the budget asked first, the answer's usage recorded
/// under `source`. `model` is passed as `--model` when given.
async fn run_claude(
    model: Option<&str>,
    prompt: &str,
    cwd: &Path,
    timeout: Duration,
    source: &str,
    spend: &dyn SpendLog,
) -> Result<String, FrontierError> {
    run_claude_with(&claude_binary(), model, prompt, cwd, timeout, source, spend).await
}

/// [`run_claude`] on a named binary.
async fn run_claude_with(
    binary: &str,
    model: Option<&str>,
    prompt: &str,
    cwd: &Path,
    timeout: Duration,
    source: &str,
    spend: &dyn SpendLog,
) -> Result<String, FrontierError> {
    // Refuse to spend when the project's daily budget is reached.
    spend.check_budget().map_err(FrontierError::OverBudget)?;
    let fut = tokio::process::Command::new(binary)
        .args(claude_args(model, prompt))
        // hexa's own prompt. The project's hooks run inside this claude and must
        // not treat it as a person's work: `route` once drafted workplans from the
        // harden reviewer prompts. `hexa hook` returns early when this is set.
        .env("HEXA_INTERNAL", "1")
        // Any commit this agent makes is hexa's, by identity: harden folds back
        // only commits made as hexa and stops on anyone else's (ADR-2611160730 §1).
        .env("GIT_AUTHOR_NAME", HEXA_GIT_NAME)
        .env("GIT_AUTHOR_EMAIL", HEXA_GIT_EMAIL)
        .env("GIT_COMMITTER_NAME", HEXA_GIT_NAME)
        .env("GIT_COMMITTER_EMAIL", HEXA_GIT_EMAIL)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // A call past its deadline is dropped by the timeout below; without
        // this the agent kept running and spending, unrecorded, beside the
        // retry that replaced it. Not a process group of its own: Ctrl-C at
        // the terminal must still reach it.
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(timeout, fut).await {
        // Its usage and cost are recorded either way: they were spent.
        Ok(Ok(o)) => Ok(take_answer(&String::from_utf8_lossy(&o.stdout), source, spend)),
        Ok(Err(e)) => Err(FrontierError::Spawn(format!("{binary}: {e}"))),
        Err(_) => Err(FrontierError::Timeout(timeout)),
    }
}

/// [`Frontier`] on the operator's logged-in `claude` CLI. Both loops that
/// used it — the do-loop's frontier fallback and the adversarial harness —
/// spawned it themselves, identically; this is that spawn, once. What it
/// spends goes to `spend`.
pub struct ClaudeCli {
    spend: Arc<dyn SpendLog>,
}

impl ClaudeCli {
    pub fn new(spend: Arc<dyn SpendLog>) -> Self {
        Self { spend }
    }
}

#[async_trait]
impl Frontier for ClaudeCli {
    async fn run(&self, prompt: &str, cwd: &Path, timeout: Duration, source: &str) -> Result<String, FrontierError> {
        run_claude(None, prompt, cwd, timeout, source, self.spend.as_ref()).await
    }

    /// A [`PinnedClaudeCli`] on `model`.
    fn on_model(&self, model: &str) -> Option<Arc<dyn Frontier>> {
        Some(Arc::new(PinnedClaudeCli { model: model.to_string(), spend: self.spend.clone() }))
    }
}

/// [`ClaudeCli`] pinned to one model: it passes `--model` so the run uses the
/// model the operator named. The spend record still takes the model from the
/// answer's JSON, so it names what actually ran.
pub struct PinnedClaudeCli {
    model: String,
    spend: Arc<dyn SpendLog>,
}

#[async_trait]
impl Frontier for PinnedClaudeCli {
    async fn run(&self, prompt: &str, cwd: &Path, timeout: Duration, source: &str) -> Result<String, FrontierError> {
        run_claude(Some(&self.model), prompt, cwd, timeout, source, self.spend.as_ref()).await
    }

    fn on_model(&self, model: &str) -> Option<Arc<dyn Frontier>> {
        Some(Arc::new(PinnedClaudeCli { model: model.to_string(), spend: self.spend.clone() }))
    }
}

/// The text of a `claude -p --output-format json` answer, with its usage
/// recorded under `source`. Plain text is returned as is: an older CLI, or
/// a run that printed something else, still yields its output.
fn take_answer(stdout: &str, source: &str, spend: &dyn SpendLog) -> String {
    let Ok(v) = serde_json::from_str::<Value>(stdout.trim()) else {
        return stdout.to_string();
    };
    let input = v.get("usage").and_then(|u| u.get("input_tokens")).and_then(Value::as_u64).unwrap_or(0);
    let output = v.get("usage").and_then(|u| u.get("output_tokens")).and_then(Value::as_u64).unwrap_or(0);
    let cost = v.get("total_cost_usd").and_then(Value::as_f64);
    spend.record_frontier(&v, input, output, cost, source);
    v.get("result").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| stdout.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spends nothing and records nothing.
    struct NoSpend;
    impl SpendLog for NoSpend {
        fn check_budget(&self) -> Result<(), String> {
            Ok(())
        }
        fn record_frontier(&self, _: &Value, _: u64, _: u64, _: Option<f64>, _: &str) {}
    }

    /// ADR-2611160730 §1: a commit made by a claude hexa started carries
    /// hexa's identity, so harden can tell it from a person's commit and fold
    /// it back instead of stopping (a real agent ignores "do not commit").
    #[tokio::test]
    async fn a_claude_hexa_starts_commits_as_hexa() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("claude");
        std::fs::write(
            &fake,
            "#!/bin/sh\necho \"$GIT_AUTHOR_NAME|$GIT_AUTHOR_EMAIL|$GIT_COMMITTER_NAME|$GIT_COMMITTER_EMAIL\" > ident.txt\necho '{\"result\":\"ok\"}'\n",
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::env::set_var("HEXA_CLAUDE_BINARY", &fake);
        let r = run_claude(None, "p", dir.path(), Duration::from_secs(20), "test", &NoSpend).await;
        std::env::remove_var("HEXA_CLAUDE_BINARY");
        assert!(r.is_ok(), "{r:?}");
        let ident = std::fs::read_to_string(dir.path().join("ident.txt")).unwrap();
        assert_eq!(ident.trim(), format!("{HEXA_GIT_NAME}|{HEXA_GIT_EMAIL}|{HEXA_GIT_NAME}|{HEXA_GIT_EMAIL}"));
        assert_eq!((HEXA_GIT_NAME, HEXA_GIT_EMAIL), ("hexa-factory", "factory@hexa.local"), "the identity harden folds back");
    }

    /// A `claude` that runs past its timeout is stopped, not left running.
    /// harden retries a timed-out call (DEFAULT_RETRIES), and every lens and
    /// verifier has a 600 s deadline: an abandoned agent kept spending, with
    /// nothing recorded, beside the retry that replaced it.
    #[tokio::test]
    async fn a_claude_past_its_timeout_is_killed_not_abandoned() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("claude-slow");
        std::fs::write(&fake, "#!/bin/sh\necho $$ > pid.txt\nexec sleep 30\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        // A fork in a parallel test can briefly hold the script open for
        // writing (ETXTBSY, "Text file busy"): that is the test's race, not
        // the spawn under test, so try again.
        let mut r = Err(FrontierError::Spawn(String::new()));
        for _ in 0..20 {
            r = run_claude_with(&fake.display().to_string(), None, "p", dir.path(), Duration::from_millis(1500), "test", &NoSpend).await;
            if !matches!(&r, Err(FrontierError::Spawn(e)) if e.contains("os error 26")) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(matches!(r, Err(FrontierError::Timeout(_))), "{r:?}");
        let pid = std::fs::read_to_string(dir.path().join("pid.txt")).unwrap().trim().to_string();
        assert!(!pid.is_empty());
        // gone, or a zombie waiting to be reaped: either way it runs no more
        let running = || {
            let o = std::process::Command::new("ps").args(["-o", "stat=", "-p", &pid]).output().unwrap();
            let st = String::from_utf8_lossy(&o.stdout).trim().to_string();
            !st.is_empty() && !st.starts_with('Z')
        };
        for _ in 0..40 {
            if !running() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = std::process::Command::new("kill").args(["-9", &pid]).status();
        panic!("the timed-out claude (pid {pid}) is still running");
    }

    #[test]
    fn the_answer_text_comes_out_of_the_json_envelope() {
        let out = r#"{"type":"result","is_error":false,"result":"the review","total_cost_usd":0.0123,"usage":{"input_tokens":1200,"output_tokens":300}}"#;
        assert_eq!(take_answer(out, "test", &NoSpend), "the review");
    }

    #[test]
    fn plain_text_passes_through() {
        assert_eq!(take_answer("not json\n", "test", &NoSpend), "not json\n");
    }

    #[test]
    fn no_model_passes_no_model_flag_and_ends_with_the_prompt() {
        let args = claude_args(None, "p");
        assert!(!args.iter().any(|a| a == "--model"));
        assert_eq!(args.last().map(String::as_str), Some("p"));
    }

    #[test]
    fn a_model_is_passed_after_the_model_flag() {
        let args = claude_args(Some("claude-opus-5-5"), "p");
        let i = args.iter().position(|a| a == "--model").expect("--model");
        assert_eq!(args[i + 1], "claude-opus-5-5");
    }

    #[test]
    fn the_cli_can_be_pinned_to_a_model() {
        assert!(ClaudeCli::new(Arc::new(NoSpend)).on_model("m").is_some());
    }
}
