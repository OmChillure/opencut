//! AI integrations. Object storage lives in `oc-db`.

mod acp;
mod catalog;
mod compat;
mod llm;
mod local_auth;
mod orchestrate;
mod prompts;

pub use catalog::{ModelInfo, ProviderId, ProviderStatus, catalog};
pub use llm::{ChatEvent, ChatTurn, EventSink, Llm, LlmError, LlmReply, Xai};
pub use acp::encode_b64;
pub use orchestrate::{
    complete, complete_stream, followup_prompt, opening_prompt, DirectorSession, PromptImage,
};
pub use prompts::{shared_prompts, style_guide, with_shared_prompts};
