//! AI integrations. Object storage lives in `oc-db`.

mod acp;
mod catalog;
mod imagine;
mod llm;
mod local_auth;
mod orchestrate;
mod prompts;

pub use catalog::{ModelInfo, ProviderId, ProviderStatus, catalog};
pub use llm::{ChatEvent, ChatTurn, EventSink, Llm, LlmError, LlmReply};
pub use acp::encode_b64;
pub use imagine::imagine_clip;
pub use orchestrate::{
    ask_with_stills, complete, complete_stream, followup_prompt, opening_prompt, subscription_ready,
    DirectorSession, PromptImage,
};
pub use prompts::{shared_prompts, style_guide, with_shared_prompts};
