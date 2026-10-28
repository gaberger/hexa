//! A completion served by an executable the operator names (ADR-2610271736).
//!
//! `HEXA_COMPLETION_COMMAND=<path>` routes every tool-less completion to that
//! executable: the prompt on stdin, the requested model id in
//! `HEXA_COMPLETION_MODEL`, the reply on stdout. It is a route for an offline
//! or self-hosted model, and the fake route the design and drift tests drive.
//! It runs no tools, so it is never an agent.

use std::process::Stdio;
use std::time::Instant;

use async_trait::async_trait;
use hexa_core::ports::inference::{
    futures_stream, ContentBlock, HealthStatus, IInferencePort, InferenceCapabilities, InferenceError,
    InferenceRequest, InferenceResponse, ModelInfo, ModelTier, Role, StopReason, StreamChunk,
};
use tokio::io::AsyncWriteExt;

use super::vec_stream::VecStream;

/// The environment variable naming the completion executable.
pub const COMMAND_ENV: &str = "HEXA_COMPLETION_COMMAND";

/// The same route under the name the spec pipeline's tests give it.
pub const AGENT_ENV: &str = "HEXA_AGENT";

/// The executable `HEXA_COMPLETION_COMMAND` (or `HEXA_AGENT`) names, if set.
pub fn configured() -> Option<String> {
    [COMMAND_ENV, AGENT_ENV].iter().find_map(|k| std::env::var(k).ok().filter(|c| !c.trim().is_empty()))
}

pub struct CommandCompletionAdapter {
    command: String,
}

impl CommandCompletionAdapter {
    pub fn new(command: String) -> Self {
        Self { command }
    }

    async fn run(&self, request: &InferenceRequest) -> Result<String, InferenceError> {
        if !request.tools.is_empty() {
            return Err(InferenceError::ProviderUnavailable(format!(
                "{COMMAND_ENV} serves tool-less completions and cannot run the {} tool(s) this request carries",
                request.tools.len()
            )));
        }
        let mut child = tokio::process::Command::new(&self.command)
            .env("HEXA_COMPLETION_MODEL", &request.model)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| InferenceError::ProviderUnavailable(format!("{COMMAND_ENV} {}: {e}", self.command)))?;
        let prompt = prompt_of(request);
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(prompt.as_bytes())
                .await
                .map_err(|e| InferenceError::ProviderUnavailable(format!("writing the prompt: {e}")))?;
        }
        let out = child
            .wait_with_output()
            .await
            .map_err(|e| InferenceError::ProviderUnavailable(format!("{COMMAND_ENV} {}: {e}", self.command)))?;
        if !out.status.success() {
            return Err(InferenceError::ApiError {
                status: 500,
                body: format!(
                    "{} exited with {}: {}",
                    self.command,
                    out.status,
                    String::from_utf8_lossy(&out.stderr)
                ),
            });
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

/// The system prompt, then each turn's text.
fn prompt_of(request: &InferenceRequest) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !request.system_prompt.is_empty() {
        parts.push(request.system_prompt.clone());
    }
    for m in &request.messages {
        let text: String = m
            .content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        match m.role {
            Role::User => parts.push(text),
            _ => parts.push(format!("Assistant: {text}")),
        }
    }
    parts.join("\n\n")
}

#[async_trait]
impl IInferencePort for CommandCompletionAdapter {
    async fn complete(&self, request: InferenceRequest) -> Result<InferenceResponse, InferenceError> {
        let start = Instant::now();
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
        })
    }

    async fn stream(
        &self,
        request: InferenceRequest,
    ) -> Result<Box<dyn futures_stream::Stream<Item = StreamChunk> + Send + Unpin>, InferenceError> {
        let text = self.run(&request).await?;
        Ok(Box::new(VecStream::new(vec![StreamChunk::TextDelta(text)])))
    }

    async fn health(&self) -> Result<HealthStatus, InferenceError> {
        if std::path::Path::new(&self.command).exists() {
            Ok(HealthStatus::Ok { models: vec![] })
        } else {
            Ok(HealthStatus::Unreachable { reason: format!("{} not found", self.command) })
        }
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            models: vec![ModelInfo {
                id: "command".to_string(),
                provider: "command".to_string(),
                tier: ModelTier::Local,
                context_window: 0,
            }],
            supports_tool_use: false,
            supports_thinking: false,
            supports_caching: false,
            supports_streaming: false,
            max_context_tokens: 0,
            cost_per_mtok_input: 0.0,
            cost_per_mtok_output: 0.0,
        }
    }
}
