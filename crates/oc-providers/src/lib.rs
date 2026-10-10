//! AI integrations. Object storage lives in `oc-db`.

mod acp;
mod catalog;
mod imagine;
mod llm;
mod local_auth;
mod orchestrate;
mod prompts;

pub use acp::encode_b64;
pub use catalog::{ModelInfo, ProviderId, ProviderStatus, catalog};
pub use imagine::imagine_clip;
pub use llm::{ChatEvent, ChatTurn, EventSink, LlmError, LlmReply};
pub use orchestrate::{
    DirectorSession, PromptImage, ask_with_stills, complete, complete_stream, followup_prompt,
    opening_prompt, subscription_ready,
};
pub use prompts::{shared_prompts, with_shared_prompts};
