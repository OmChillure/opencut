use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderId {
    Xai,
    Openai,
    Claude,
}

impl ProviderId {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "xai" | "spacexai" => Some(Self::Xai),
            "openai" => Some(Self::Openai),
            "claude" | "anthropic" => Some(Self::Claude),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Xai => "xai",
            Self::Openai => "openai",
            Self::Claude => "claude",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Xai => "xAI",
            Self::Openai => "OpenAI",
            Self::Claude => "Claude",
        }
    }

    pub fn login_hint(self) -> &'static str {
        match self {
            Self::Xai => "Run `grok` in a terminal to sign in with your xAI subscription.",
            Self::Openai => "Run `codex` in a terminal to sign in with ChatGPT.",
            Self::Claude => "Run `claude auth login` so we can use the local Claude store.",
        }
    }

    pub fn connected(self) -> bool {
        match self {
            Self::Xai => crate::local_auth::grok_logged_in(),
            Self::Openai => crate::local_auth::codex_logged_in(),
            Self::Claude => crate::local_auth::claude_logged_in(),
        }
    }

    pub fn fallback_models(self) -> Vec<ModelInfo> {
        match self {
            Self::Xai => vec![
                ModelInfo::new("grok-4.6", "Grok 4.6"),
                ModelInfo::new("grok-4.5", "Grok 4.5"),
            ],
            Self::Openai => vec![
                ModelInfo::new("gpt-5.6-sol", "GPT-5.6 Sol"),
                ModelInfo::new("gpt-5.6-terra", "GPT-5.6 Terra"),
                ModelInfo::new("gpt-5.6-luna", "GPT-5.6 Luna"),
                ModelInfo::new("gpt-5.5", "GPT-5.5"),
            ],
            Self::Claude => vec![
                ModelInfo::new("claude-opus-5", "Opus 5"),
                ModelInfo::new("claude-sonnet-5", "Sonnet 5"),
                ModelInfo::new("claude-fable-5[1m]", "Fable"),
                ModelInfo::new("claude-haiku-4-5", "Haiku 4.5"),
            ],
        }
    }

    pub fn models(self) -> Vec<ModelInfo> {
        let live = match self {
            Self::Xai => crate::local_auth::grok_models(),
            Self::Openai => crate::local_auth::codex_models(),
            Self::Claude => Vec::new(),
        };
        if live.is_empty() {
            self.fallback_models()
        } else {
            live
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
}

impl ModelInfo {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ProviderStatus {
    pub id: &'static str,
    pub name: &'static str,
    pub connected: bool,
    pub login_hint: &'static str,
    pub models: Vec<ModelInfo>,
}

pub fn all_providers() -> [ProviderId; 3] {
    [ProviderId::Xai, ProviderId::Openai, ProviderId::Claude]
}

pub fn catalog() -> Vec<ProviderStatus> {
    all_providers()
        .into_iter()
        .map(|id| ProviderStatus {
            id: id.as_str(),
            name: id.name(),
            connected: id.connected(),
            login_hint: id.login_hint(),
            models: id.models(),
        })
        .collect()
}
