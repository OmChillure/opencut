//! AI providers. They receive MCP tools from `oc-tools` and return text or tool calls.

use oc_tools::{McpCall, McpTool};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("missing env var {0}")]
    MissingEnv(&'static str),
    #[error("{0}")]
    Message(String),
}

#[derive(Clone, Debug)]
pub enum LlmReply {
    Text(String),
    Tools(Vec<McpCall>),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatTurn {
    pub role: String,
    pub content: String,
}

/// Any chat model the editor can use. Implement this; do not call vendors from tools.
pub trait Llm {
    fn name(&self) -> &'static str;

    fn complete(
        &self,
        system: &str,
        turns: &[ChatTurn],
        tools: &[McpTool],
    ) -> impl std::future::Future<Output = Result<LlmReply, LlmError>> + Send;
}

/// SpaceXAI / xAI (OpenAI-compatible). Default AI integration.
#[derive(Clone)]
pub struct Xai {
    pub api_key: String,
    pub model: String,
    pub base_url: String,
}

impl Xai {
    pub fn from_env() -> Result<Self, LlmError> {
        Ok(Self {
            api_key: std::env::var("XAI_API_KEY")
                .ok()
                .filter(|s| !s.is_empty())
                .ok_or(LlmError::MissingEnv("XAI_API_KEY"))?,
            model: std::env::var("XAI_MODEL").unwrap_or_else(|_| "grok-4.5".into()),
            base_url: std::env::var("XAI_BASE_URL")
                .unwrap_or_else(|_| "https://api.x.ai/v1".into()),
        })
    }
}

impl Llm for Xai {
    fn name(&self) -> &'static str {
        "spacexai"
    }

    async fn complete(
        &self,
        system: &str,
        turns: &[ChatTurn],
        tools: &[McpTool],
    ) -> Result<LlmReply, LlmError> {
        crate::compat::chat_completions(
            &self.base_url,
            &self.api_key,
            &self.model,
            system,
            turns,
            tools,
        )
        .await
    }
}
