//! A semantic compressor that reduces conversation history using recursive summarization.
//!
//! This adapter implements the [`PromptCompressor`] port. It preserves recent turns
//! and summarizes older turns using a cheap model. If the summaries themselves
//! exceed the budget, it recursively summarizes them.

use async_trait::async_trait;
use hexa_core::ports::inference::{ContentBlock, Message, Role};
use hexa_core::ports::inference::InferenceRequest;
use crate::ports::{Backends, PromptCompressor};
use std::sync::Arc;

/// Configuration for the [`SemanticCompressor`].
pub struct CompressorConfig {
    /// The model used for summarization (e.g., a cheap T1 model).
    pub cheap_model: String,
    /// The number of most recent messages to keep as-is.
    pub recent_window: usize,
    /// The maximum token budget for the compressed history.
    pub budget: u64,
}

impl Default for CompressorConfig {
    fn default() -> Self {
        Self {
            cheap_model: "t1".to_string(),
            recent_window: 5,
            budget: 2000,
        }
    }
}

pub struct SemanticCompressor {
    backends: Arc<dyn Backends>,
    config: CompressorConfig,
}

impl SemanticCompressor {
    pub fn new(backends: Arc<dyn Backends>, config: CompressorConfig) -> Self {
        Self { backends, config }
    }

    /// Recursively summarizes messages until they fit within the budget.
    async fn recursive_summarize(&self, messages: &[Message]) -> Result<String, String> {
        let mut current_messages = messages.to_vec();

        loop {
            let content = self.messages_to_text(&current_messages);
            let token_count = self.estimate_tokens(&content);

            if token_count <= self.config.budget {
                return Ok(content);
            }

            let chunk_size = (current_messages.len() + 3) / 4;
            let mut summaries = Vec::new();
            for chunk in current_messages.chunks(chunk_size) {
                summaries.push(self.summarize_chunk(chunk).await?);
            }

            let combined = summaries.join("\n\n");
            current_messages = vec![Message::user(&combined)];
        }
    }

    async fn summarize_chunk(&self, messages: &[Message]) -> Result<String, String> {
        let content = self.messages_to_text(messages);
        let prompt = format!(
            "Summarize the following conversation history concisely, preserving all key semantic signals, facts, and decisions. \
            The summary will be used to provide context for a future conversation.\n\nHistory:\n{}",
            content
        );

        let request = InferenceRequest {
            model: self.config.cheap_model.clone(),
            system_prompt: "You are a helpful assistant that creates concise, high-signal summaries of conversation history.".to_string(),
            messages: vec![Message::user(&prompt)],
            tools: vec![],
            max_tokens: 1024,
            temperature: 0.0,
            thinking_budget: None,
            cache_control: false,
            priority: hexa_core::ports::inference::Priority::Normal,
            grammar: None,
        };

        let response = self.backends.complete(request).await
            .map_err(|e| format!("Inference error during summarization: {:?}", e))?;

        // Extract text from ContentBlocks
        let text = response.content.iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");

        Ok(text)
    }

    fn messages_to_text(&self, messages: &[Message]) -> String {
        messages.iter()
            .map(|m| {
                let role = match m.role {
                    Role::User => "User",
                    Role::Assistant => "Assistant",
                };
                format!("{}: {}", role, m.text_content())
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn estimate_tokens(&self, text: &str) -> u64 {
        text.len() as u64 / 4
    }
}

#[async_trait]
impl PromptCompressor for SemanticCompressor {
    async fn compress(&self, _model: &str, messages: Vec<Message>) -> Result<(Vec<Message>, std::collections::HashMap<String, String>), String> {
        if messages.len() <= self.config.recent_window {
            return Ok((messages, std::collections::HashMap::new()));
        }

        let original_tokens = messages.iter().map(|m| m.text_content().len() as u64 / 4).sum::<u64>();
        let (older, recent) = messages.split_at(messages.len() - self.config.recent_window);

        let summary_text = self.recursive_summarize(older).await?;

        let mut result = Vec::new();
        if !summary_text.is_empty() {
            result.push(Message::assistant(&format!("Summary of previous conversation:\n{}", summary_text)));
        }
        result.extend(recent.to_vec());

        let compressed_tokens = result.iter().map(|m| m.text_content().len() as u64 / 4).sum::<u64>();
        let ratio = if original_tokens > 0 {
            original_tokens as f64 / compressed_tokens.max(1) as f64
        } else {
            1.0
        };

        let mut telemetry = std::collections::HashMap::new();
        telemetry.insert("compression_ratio".to_string(), format!("{:.1}x", ratio));
        telemetry.insert("tokens_saved".to_string(), format!("{}k", (original_tokens.saturating_sub(compressed_tokens) / 1000)));
        telemetry.insert("original_tokens".to_string(), original_tokens.to_string());
        telemetry.insert("compressed_tokens".to_string(), compressed_tokens.to_string());

        Ok((result, telemetry))
    }

}
