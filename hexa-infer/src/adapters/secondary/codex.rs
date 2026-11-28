//! Codex inference adapter: `codex exec` behind the same `ProcessSpawner` as
//! `ClaudeCodeInferenceAdapter` (ADR-2611231735).
//!
//! The reply is the agent's last message, which `codex exec -o <file>` writes to
//! a file; a non-zero exit or an empty file is an error, never an empty success
//! (§3). The permission-bypass flag is hardcoded, as Claude's is (§2): a
//! `codex exec` that stops to ask for approval exits without an answer. There is
//! no constructor knob and no environment override for it.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use hexa_core::ports::inference::{
    futures_stream, ContentBlock, HealthStatus, IInferencePort, InferenceCapabilities, InferenceError,
    InferenceRequest, InferenceResponse, Role, StopReason, StreamChunk,
};

use super::claude_code::{ProcessSpawner, TokioProcessSpawner};
use super::vec_stream::VecStream;

/// The executable, resolved through `PATH` at spawn time.
const BINARY: &str = "codex";
/// The bare model id: the CLI's own default model, so no `-m` (§4).
const BARE_MODEL: &str = "codex";
/// Default subprocess timeout when `CODEX_TIMEOUT_SECS` is unset.
const DEFAULT_TIMEOUT_SECS: u64 = 300;

/// Serves a registered model through the `codex` CLI (ADR-2611231735 §1).
pub struct CodexInferenceAdapter {
    model: Option<String>,
    spawner: Arc<dyn ProcessSpawner>,
    timeout: Duration,
}

impl CodexInferenceAdapter {
    /// Production constructor: the real process spawner.
    pub fn new(model: Option<String>) -> Self {
        let secs = std::env::var("CODEX_TIMEOUT_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(DEFAULT_TIMEOUT_SECS);
        Self { model, spawner: Arc::new(TokioProcessSpawner::new()), timeout: Duration::from_secs(secs) }
    }

    /// Test constructor: a caller-supplied spawner stands in for `codex`.
    pub fn with_spawner(model: Option<String>, spawner: Arc<dyn ProcessSpawner>) -> Self {
        Self { model, spawner, timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS) }
    }

    /// The `-m` argument: the registered model when it is not the bare id,
    /// else the request's, else none (§4).
    fn model_arg(&self, request: &InferenceRequest) -> Option<String> {
        [self.model.as_deref(), Some(request.model.as_str())]
            .into_iter()
            .flatten()
            .find(|m| !m.is_empty() && !m.eq_ignore_ascii_case(BARE_MODEL))
            .map(str::to_string)
    }

    async fn run(&self, request: &InferenceRequest) -> Result<String, InferenceError> {
        let out = std::env::temp_dir().join(format!(
            "hexa-codex-{}-{}.txt",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
        ));
        let args = args_for_exec(&out.to_string_lossy(), self.model_arg(request).as_deref(), &prompt_of(request));
        let spawned = tokio::time::timeout(self.timeout, self.spawner.spawn(BINARY, &args))
            .await
            .map_err(|_| InferenceError::ProviderUnavailable(format!("codex exec timed out after {}s", self.timeout.as_secs())))?;
        let reply = std::fs::read_to_string(&out);
        let _ = std::fs::remove_file(&out);
        let spawned = spawned?;
        if spawned.exit_code != 0 {
            return Err(InferenceError::ApiError {
                status: u16::try_from(spawned.exit_code).unwrap_or(0),
                body: format!("codex exited with {}: {}", spawned.exit_code, String::from_utf8_lossy(&spawned.stderr)),
            });
        }
        match reply {
            Ok(text) if !text.trim().is_empty() => Ok(text),
            _ => Err(InferenceError::ApiError { status: 0, body: "codex wrote no last message".to_string() }),
        }
    }
}

/// The arguments of one `codex exec`. The bypass flag is always present (§2).
fn args_for_exec(out_file: &str, model: Option<&str>, prompt: &str) -> Vec<String> {
    let mut args: Vec<String> =
        ["exec", "--skip-git-repo-check", "--ephemeral", "--dangerously-bypass-approvals-and-sandbox", "-o", out_file]
            .iter()
            .map(|s| s.to_string())
            .collect();
    if let Some(m) = model {
        args.extend(["-m".to_string(), m.to_string()]);
    }
    args.extend(["--".to_string(), prompt.to_string()]);
    args
}

/// The whole conversation as one prompt: the system prompt, then each turn.
fn prompt_of(request: &InferenceRequest) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !request.system_prompt.is_empty() {
        parts.push(request.system_prompt.clone());
    }
    for m in &request.messages {
        let text = m
            .content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                ContentBlock::ToolResult { content, .. } => Some(content.as_str()),
                ContentBlock::ToolUse { .. } => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        parts.push(if request.messages.len() > 1 {
            format!("{}: {}", if m.role == Role::User { "User" } else { "Assistant" }, text)
        } else {
            text
        });
    }
    parts.join("\n\n")
}

#[async_trait]
impl IInferencePort for CodexInferenceAdapter {
    async fn complete(&self, request: InferenceRequest) -> Result<InferenceResponse, InferenceError> {
        let start = std::time::Instant::now();
        let text = self.run(&request).await?;
        Ok(InferenceResponse {
            content: vec![ContentBlock::Text { text }],
            model_used: request.model.clone(),
            stop_reason: StopReason::EndTurn,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            latency_ms: start.elapsed().as_millis() as u64,
            telemetry: std::collections::HashMap::new(),
        })
    }

    async fn stream(
        &self,
        request: InferenceRequest,
    ) -> Result<Box<dyn futures_stream::Stream<Item = StreamChunk> + Send + Unpin>, InferenceError> {
        let mut chunks: Vec<StreamChunk> = self
            .run(&request)
            .await?
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| StreamChunk::TextDelta(l.to_string()))
            .collect();
        chunks.push(StreamChunk::MessageStop(StopReason::EndTurn));
        Ok(Box::new(VecStream::new(chunks)))
    }

    async fn health(&self) -> Result<HealthStatus, InferenceError> {
        match self.spawner.spawn(BINARY, &["--version".to_string()]).await {
            Ok(p) if p.exit_code == 0 => Ok(HealthStatus::Ok { models: vec![] }),
            Ok(p) => Ok(HealthStatus::Unreachable { reason: format!("codex --version exited {}", p.exit_code) }),
            Err(e) => Ok(HealthStatus::Unreachable { reason: e.to_string() }),
        }
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            models: vec![],
            supports_tool_use: true,
            supports_thinking: false,
            supports_caching: false,
            supports_streaming: false,
            max_context_tokens: 200_000,
            cost_per_mtok_input: 0.0,
            cost_per_mtok_output: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bypass_flag_is_always_in_the_arguments() {
        assert!(args_for_exec("/tmp/x", None, "hi").iter().any(|a| a == "--dangerously-bypass-approvals-and-sandbox"));
        assert!(args_for_exec("/tmp/x", Some("gpt-x"), "hi").iter().any(|a| a == "--dangerously-bypass-approvals-and-sandbox"));
    }

    #[test]
    fn a_prompt_that_starts_with_a_dash_is_not_read_as_a_flag() {
        let args = args_for_exec("/tmp/x", None, "--help");
        assert_eq!(args[args.len() - 2], "--");
        assert_eq!(args[args.len() - 1], "--help");
    }

    #[test]
    fn the_bare_id_passes_no_model_flag() {
        let a = CodexInferenceAdapter::new(Some("codex".to_string()));
        let req = InferenceRequest { model: "codex".to_string(), ..Default::default() };
        assert_eq!(a.model_arg(&req), None);
        let b = CodexInferenceAdapter::new(Some("gpt-x".to_string()));
        assert_eq!(b.model_arg(&req).as_deref(), Some("gpt-x"));
    }
}
