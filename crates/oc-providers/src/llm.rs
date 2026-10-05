//! AI providers. They receive MCP tools from `oc-tools` and return text or tool calls.

use oc_tools::{McpCall, McpTool};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio::sync::mpsc;

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("{0}")]
    Message(String),
}

#[derive(Clone, Debug)]
pub enum LlmReply {
    Text(String),
    Tools(Vec<McpCall>),
}

pub type EventSink = mpsc::Sender<ChatEvent>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatEvent {
    Status {
        text: String,
    },
    Text {
        text: String,
    },
    Thought {
        text: String,
    },
    Tool {
        id: String,
        name: String,
        #[serde(default)]
        args: Value,
        #[serde(default)]
        result: Option<String>,
        status: String,
    },
    Note {
        text: String,
    },
}

impl ChatEvent {
    #[must_use]
    pub fn status(text: impl Into<String>) -> Self {
        Self::Status { text: text.into() }
    }

    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    #[must_use]
    pub fn thought(text: impl Into<String>) -> Self {
        Self::Thought { text: text.into() }
    }

    #[must_use]
    pub fn note(text: impl Into<String>) -> Self {
        Self::Note { text: text.into() }
    }

    #[must_use]
    pub fn tool(
        id: impl Into<String>,
        name: impl Into<String>,
        args: Value,
        result: Option<String>,
        status: impl Into<String>,
    ) -> Self {
        Self::Tool {
            id: id.into(),
            name: name.into(),
            args,
            result,
            status: status.into(),
        }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Tool { name, .. } => name,
            _ => "",
        }
    }

    #[must_use]
    pub fn status_label(&self) -> &str {
        match self {
            Self::Tool { status, .. } => status,
            _ => "",
        }
    }
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
