//! Chat replies from a signed-in CLI. The editor does not call a vendor SDK.

use oc_tools::McpCall;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tool_event_keeps_its_name_and_status_through_json() {
        let event = ChatEvent::tool(
            "1",
            "see",
            serde_json::json!({"at": 1}),
            Some("ok".into()),
            "done",
        );
        assert_eq!(event.name(), "see");
        assert_eq!(event.status_label(), "done");
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "tool");
        let back: ChatEvent = serde_json::from_value(json).unwrap();
        assert_eq!(back.name(), "see");
        assert_eq!(back.status_label(), "done");
        assert_eq!(ChatEvent::text("hi").name(), "");
    }
}
