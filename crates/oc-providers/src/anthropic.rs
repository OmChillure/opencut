use crate::llm::{ChatTurn, LlmError, LlmReply};
use oc_tools::{McpCall, McpTool};

pub async fn messages(
    api_key: &str,
    model: &str,
    system: &str,
    turns: &[ChatTurn],
    tools: &[McpTool],
) -> Result<LlmReply, LlmError> {
    let messages: Vec<_> = turns
        .iter()
        .map(|t| {
            serde_json::json!({
                "role": if t.role == "assistant" { "assistant" } else { "user" },
                "content": t.content,
            })
        })
        .collect();
    let mut body = serde_json::json!({
        "model": model,
        "max_tokens": 2048,
        "system": system,
        "messages": messages,
    });
    if !tools.is_empty() {
        body["tools"] = tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema,
                })
            })
            .collect();
    }
    let resp: serde_json::Value = reqwest::Client::new()
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await
        .map_err(|e| LlmError::Message(e.to_string()))?
        .error_for_status()
        .map_err(|e| LlmError::Message(e.to_string()))?
        .json()
        .await
        .map_err(|e| LlmError::Message(e.to_string()))?;
    parse_anthropic_reply(&resp)
}

fn parse_anthropic_reply(resp: &serde_json::Value) -> Result<LlmReply, LlmError> {
    let blocks = resp
        .get("content")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut calls = Vec::new();
    let mut text = String::new();
    for block in blocks {
        match block.get("type").and_then(|v| v.as_str()) {
            Some("tool_use") => {
                let name = block
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let arguments = block.get("input").cloned().unwrap_or(serde_json::json!({}));
                if !name.is_empty() {
                    calls.push(McpCall { name, arguments });
                }
            }
            Some("text") => {
                if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(t);
                }
            }
            _ => {}
        }
    }
    if !calls.is_empty() {
        return Ok(LlmReply::Tools(calls));
    }
    Ok(LlmReply::Text(text))
}
