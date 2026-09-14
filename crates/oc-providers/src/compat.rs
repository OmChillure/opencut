use crate::llm::{ChatTurn, LlmError, LlmReply};
use oc_tools::{McpCall, McpTool};

pub async fn chat_completions(
    base_url: &str,
    api_key: &str,
    model: &str,
    system: &str,
    turns: &[ChatTurn],
    tools: &[McpTool],
) -> Result<LlmReply, LlmError> {
    let mut messages = vec![serde_json::json!({
        "role": "system",
        "content": crate::prompts::with_shared_prompts(system),
    })];
    for turn in turns {
        messages.push(serde_json::json!({
            "role": turn.role,
            "content": turn.content,
        }));
    }
    let mut body = serde_json::json!({
        "model": model,
        "messages": messages,
    });
    if !tools.is_empty() {
        body["tools"] = tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema,
                    }
                })
            })
            .collect();
    }
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let resp: serde_json::Value = reqwest::Client::new()
        .post(url)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| LlmError::Message(e.to_string()))?
        .error_for_status()
        .map_err(|e| LlmError::Message(e.to_string()))?
        .json()
        .await
        .map_err(|e| LlmError::Message(e.to_string()))?;
    parse_openai_reply(&resp)
}

pub fn parse_openai_reply(resp: &serde_json::Value) -> Result<LlmReply, LlmError> {
    let msg = resp
        .pointer("/choices/0/message")
        .ok_or_else(|| LlmError::Message("no message in LLM response".into()))?;
    if let Some(calls) = msg.get("tool_calls").and_then(|v| v.as_array()) {
        let mut out = Vec::new();
        for call in calls {
            let name = call
                .pointer("/function/name")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let raw = call
                .pointer("/function/arguments")
                .and_then(|v| v.as_str())
                .unwrap_or("{}");
            let arguments = serde_json::from_str(raw).unwrap_or(serde_json::json!({}));
            if !name.is_empty() {
                out.push(McpCall { name, arguments });
            }
        }
        if !out.is_empty() {
            return Ok(LlmReply::Tools(out));
        }
    }
    let text = msg
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Ok(LlmReply::Text(text))
}
