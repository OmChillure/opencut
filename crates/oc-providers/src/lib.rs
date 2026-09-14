//! AI integrations. Object storage lives in `oc-db`.

mod acp;
mod anthropic;
mod catalog;
mod compat;
mod llm;
mod local_auth;
mod orchestrate;
mod prompts;

pub use catalog::{ModelInfo, ProviderId, ProviderStatus, catalog};
pub use llm::{ChatTurn, Llm, LlmError, LlmReply, Xai};
pub use orchestrate::complete;
pub use prompts::{shared_prompts, with_shared_prompts};
