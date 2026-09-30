//! Minimal ACP client — spawn the local vendor CLI over stdio.
//! Tokens stay in the CLI's own store.

use crate::llm::{ChatEvent, EventSink, LlmError, LlmReply};
use serde::Deserialize;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

#[derive(Deserialize, Debug)]
struct Rpc {
    id: Option<Value>,
    result: Option<Value>,
    error: Option<Value>,
    method: Option<String>,
    params: Option<Value>,
}

pub struct AcpClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl AcpClient {
    pub async fn connect(
        bin: &str,
        args: &[String],
        events: Option<EventSink>,
    ) -> Result<Self, LlmError> {
        tracing::info!(bin, args = %args.join(" "), "acp spawn");
        let mut cmd = Command::new(bin);
        cmd.args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|e| {
            let msg = format!("spawn {bin}: {e}. Is the CLI installed and logged in?");
            tracing::error!("{msg}");
            LlmError::Message(msg)
        })?;
        if let Some(stderr) = child.stderr.take() {
            let ev = events.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let line = line.trim().to_string();
                    if line.is_empty() {
                        continue;
                    }
                    tracing::warn!(acp_stderr = %line, "acp");
                    let lower = line.to_ascii_lowercase();
                    if lower.contains("error")
                        || lower.contains("fail")
                        || lower.contains("panic")
                        || lower.contains("denied")
                    {
                        emit(ev.as_ref(), ChatEvent::status(format!("acp: {line}"))).await;
                    }
                }
            });
        }
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| LlmError::Message("no stdin".into()))?;
        let stdout = BufReader::new(
            child
                .stdout
                .take()
                .ok_or_else(|| LlmError::Message("no stdout".into()))?,
        );
        let mut client = Self {
            child,
            stdin,
            stdout,
            next_id: 0,
        };
        tracing::info!("acp initialize");
        client
            .request(
                "initialize",
                serde_json::json!({
                    "protocolVersion": 1,
                    "clientInfo": { "name": "opencut", "version": "0.1.0" },
                    "clientCapabilities": {
                        "fs": { "readTextFile": false, "writeTextFile": false },
                    }
                }),
                events.as_ref(),
            )
            .await?;
        tracing::info!("acp initialized");
        Ok(client)
    }

    /// `session/new` once. Later turns reuse the id and send only the new text.
    pub async fn open_session(
        &mut self,
        cwd: &str,
        model: Option<&str>,
        mcp_servers: &[Value],
        events: Option<&EventSink>,
    ) -> Result<String, LlmError> {
        tracing::info!(cwd, mcp = mcp_servers.len(), "acp session/new");
        let new = self
            .request(
                "session/new",
                serde_json::json!({ "cwd": cwd, "mcpServers": mcp_servers }),
                events,
            )
            .await
            .inspect_err(|e| tracing::error!("acp session/new failed: {e}"))?;
        let session_id = new
            .get("sessionId")
            .or_else(|| new.get("session_id"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| LlmError::Message("ACP session/new missing sessionId".into()))?
            .to_string();
        if let Some(model) = model.filter(|m| !m.is_empty()) {
            let _ = self
                .request(
                    "session/set_config_option",
                    serde_json::json!({
                        "sessionId": session_id,
                        "configId": "model",
                        "value": model,
                    }),
                    events,
                )
                .await;
        }
        Ok(session_id)
    }

    pub async fn continue_prompt(
        &mut self,
        session_id: &str,
        message: &str,
        frames: &[crate::PromptImage],
        events: Option<&EventSink>,
    ) -> Result<String, LlmError> {
        tracing::info!(
            session_id = %session_id,
            chars = message.len(),
            frames = frames.len(),
            "acp session/prompt"
        );
        let prompt_id = self.next_id + 1;
        let mut blocks = vec![serde_json::json!({ "type": "text", "text": message })];
        for frame in frames {
            blocks.push(serde_json::json!({ "type": "text", "text": frame.caption }));
            blocks.push(serde_json::json!({
                "type": "image",
                "mimeType": "image/jpeg",
                "data": encode_b64(&frame.jpeg),
            }));
        }
        self.send(
            "session/prompt",
            serde_json::json!({
                "sessionId": session_id,
                "prompt": blocks
            }),
        )
        .await?;
        let mut text = String::new();
        loop {
            let msg = self.next_rpc(events).await?;
            if msg.method.as_deref() == Some("session/update") {
                emit_update(&msg.params, events).await;
                if let Some(chunk) = extract_text(&msg.params) {
                    text.push_str(&chunk);
                }
                continue;
            }
            if ids_match(&msg.id, prompt_id) {
                if let Some(err) = msg.error {
                    tracing::error!(error = %err, "acp session/prompt error");
                    return Err(LlmError::Message(format!("ACP prompt: {err}")));
                }
                tracing::info!(chars = text.len(), "acp session/prompt done");
                if let Some(chunk) = extract_text(&msg.result) {
                    text.push_str(&chunk);
                }
                break;
            }
        }
        Ok(text)
    }

    async fn request(
        &mut self,
        method: &str,
        params: Value,
        events: Option<&EventSink>,
    ) -> Result<Value, LlmError> {
        self.send(method, params).await?;
        let id = self.next_id;
        loop {
            let msg = self.next_rpc(events).await?;
            if msg.method.as_deref() == Some("session/update") {
                emit_update(&msg.params, events).await;
                continue;
            }
            if ids_match(&msg.id, id) {
                if let Some(err) = msg.error {
                    if method == "session/set_config_option" {
                        return Ok(Value::Null);
                    }
                    return Err(LlmError::Message(format!("ACP {method}: {err}")));
                }
                return Ok(msg.result.unwrap_or(Value::Null));
            }
        }
    }

    async fn next_rpc(&mut self, events: Option<&EventSink>) -> Result<Rpc, LlmError> {
        loop {
            let msg = self.read().await?;
            if is_incoming_request(&msg) {
                self.answer_request(&msg, events).await?;
                continue;
            }
            return Ok(msg);
        }
    }

    async fn answer_request(
        &mut self,
        msg: &Rpc,
        events: Option<&EventSink>,
    ) -> Result<(), LlmError> {
        let Some(id) = msg.id.clone() else {
            return Ok(());
        };
        let method = msg.method.as_deref().unwrap_or("");
        tracing::info!(method, "acp client request");
        let result = match method {
            "session/request_permission" | "session/requestPermission" => {
                if let Some(ev) = permission_tool_event(&msg.params) {
                    emit(events, ev).await;
                }
                permission_allow(&msg.params)
            }
            _ => {
                tracing::warn!(method, "acp client method not implemented — empty reply");
                serde_json::json!({})
            }
        };
        self.respond(id, result).await
    }

    async fn send(&mut self, method: &str, params: Value) -> Result<(), LlmError> {
        self.next_id += 1;
        let line = serde_json::json!({
            "jsonrpc": "2.0",
            "id": self.next_id,
            "method": method,
            "params": params,
        });
        self.write_line(&line).await
    }

    async fn respond(&mut self, id: Value, result: Value) -> Result<(), LlmError> {
        let line = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        });
        self.write_line(&line).await
    }

    async fn write_line(&mut self, line: &Value) -> Result<(), LlmError> {
        let raw = serde_json::to_string(line).unwrap();
        self.stdin
            .write_all(raw.as_bytes())
            .await
            .map_err(|e| LlmError::Message(e.to_string()))?;
        self.stdin
            .write_all(b"\n")
            .await
            .map_err(|e| LlmError::Message(e.to_string()))?;
        self.stdin
            .flush()
            .await
            .map_err(|e| LlmError::Message(e.to_string()))?;
        Ok(())
    }

    async fn read(&mut self) -> Result<Rpc, LlmError> {
        let mut line = String::new();
        loop {
            line.clear();
            let n = self
                .stdout
                .read_line(&mut line)
                .await
                .map_err(|e| LlmError::Message(e.to_string()))?;
            if n == 0 {
                tracing::error!("acp stdout closed (CLI exited)");
                return Err(LlmError::Message("ACP CLI closed".into()));
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(msg) = serde_json::from_str::<Rpc>(trimmed) {
                if let Some(method) = msg.method.as_deref() {
                    tracing::debug!(method, id = ?msg.id, "acp <<");
                }
                return Ok(msg);
            }
            tracing::warn!(line = %trimmed.chars().take(240).collect::<String>(), "acp non-json stdout");
        }
    }
}

impl Drop for AcpClient {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

fn ids_match(id: &Option<Value>, want: u64) -> bool {
    match id {
        Some(Value::Number(n)) => n.as_u64() == Some(want),
        Some(Value::String(s)) => s.parse::<u64>().ok() == Some(want),
        _ => false,
    }
}

fn is_incoming_request(msg: &Rpc) -> bool {
    msg.method.is_some() && msg.id.is_some() && msg.result.is_none() && msg.error.is_none()
}

fn extract_text(params: &Option<Value>) -> Option<String> {
    let v = params.as_ref()?;
    let kind = session_update_kind(v);
    if matches!(kind.as_deref(), Some("agent_message_chunk" | "agent_message")) {
        if let Some(text) = content_text(v) {
            return Some(text);
        }
    }
    if kind.is_some() {
        return None;
    }
    content_text(v)
}

fn session_update_kind(v: &Value) -> Option<&str> {
    v.pointer("/update/sessionUpdate")
        .or_else(|| v.pointer("/update/session_update"))
        .or_else(|| v.get("sessionUpdate"))
        .or_else(|| v.get("session_update"))
        .and_then(|x| x.as_str())
}

fn update_node(v: &Value) -> &Value {
    v.get("update").unwrap_or(v)
}

fn looks_like_tool_trace(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return false;
    }
    if t.contains("function_call")
        || t.contains("\"type\":\"function\"")
        || t.contains("\"type\": \"function\"")
        || t.contains("tool_call")
        || t.contains("sessionUpdate")
        || t.contains("session/prompt")
        || t.contains("$schema")
        || t.contains("json-schema.org")
        || t.contains("\"parameters\"")
        || t.contains("\"input_schema\"")
        || t.contains("\"properties\"")
        || t.contains("scheduler_")
        || t.contains("Usage notes:")
        || (t.starts_with('{') && t.contains("\"name\"") && t.contains("\"arguments\""))
    {
        return true;
    }
    let punct = t
        .chars()
        .filter(|c| matches!(c, '{' | '}' | '"' | '[' | ']' | ':'))
        .count();
    t.len() > 80 && punct * 4 > t.len()
}

fn content_text(v: &Value) -> Option<String> {
    let u = update_node(v);
    if let Some(text) = u.pointer("/content/text").and_then(|x| x.as_str()) {
        return Some(text.to_string());
    }
    if let Some(text) = v.pointer("/content/text").and_then(|x| x.as_str()) {
        return Some(text.to_string());
    }
    None
}

async fn emit_update(params: &Option<Value>, events: Option<&EventSink>) {
    let Some(v) = params.as_ref() else {
        return;
    };
    let Some(kind) = session_update_kind(v) else {
        return;
    };
    let u = update_node(v);
    match kind {
        "agent_message_chunk" | "agent_message" => {
            if let Some(text) = content_text(v) {
                if looks_like_tool_trace(&text) {
                    tracing::debug!(chars = text.len(), "acp tool trace");
                    emit(events, ChatEvent::thought(text)).await;
                } else {
                    tracing::debug!(chars = text.len(), "acp text");
                    emit(events, ChatEvent::text(text)).await;
                }
            }
        }
        "agent_thought_chunk" | "agent_thought" => {
            if let Some(text) = content_text(v) {
                tracing::debug!(chars = text.len(), "acp thought");
                emit(events, ChatEvent::thought(text)).await;
            }
        }
        "tool_call" | "tool_call_update" => {
            if let Some(ev) = tool_event_from_update(u) {
                tracing::info!(
                    name = %ev.name(),
                    status = %ev.status_label(),
                    "acp tool"
                );
                emit(events, ev).await;
            }
        }
        _ => {}
    }
}

pub fn short_tool_name(raw: &str) -> String {
    let s = raw.rsplit([':', '/', '@']).next().unwrap_or(raw);
    s.rsplit("__").next().unwrap_or(s).trim().to_string()
}

fn tool_event_from_update(u: &Value) -> Option<ChatEvent> {
    let id = u
        .get("toolCallId")
        .or_else(|| u.get("tool_call_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("tool")
        .to_string();
    let name = u
        .get("name")
        .or_else(|| u.get("title"))
        .and_then(|v| v.as_str())
        .or_else(|| {
            u.pointer("/rawInput/name")
                .or_else(|| u.pointer("/raw_input/name"))
                .and_then(|v| v.as_str())
        })
        .map(short_tool_name)
        .filter(|s| !s.is_empty() && s != "other" && s != "mcp")
        .or_else(|| {
            u.get("kind")
                .and_then(|v| v.as_str())
                .map(short_tool_name)
                .filter(|s| !s.is_empty() && s != "other" && s != "mcp")
        })
        .unwrap_or_else(|| "tool".into());
    let args = u
        .get("rawInput")
        .or_else(|| u.get("raw_input"))
        .cloned()
        .unwrap_or(Value::Null);
    let result = tool_result_text(u);
    let status = match u.get("status").and_then(|v| v.as_str()).unwrap_or("") {
        "completed" | "done" | "success" => "done",
        "failed" | "error" | "cancelled" => "error",
        _ if result.is_some() => "done",
        _ => "pending",
    }
    .to_string();
    Some(ChatEvent::tool(id, name, args, result, status))
}

fn tool_result_text(u: &Value) -> Option<String> {
    if let Some(out) = u.get("rawOutput").or_else(|| u.get("raw_output")) {
        return Some(match out {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        });
    }
    if let Some(content) = u.get("content") {
        if let Some(text) = content.get("text").and_then(|v| v.as_str()) {
            return Some(text.to_string());
        }
        if let Some(arr) = content.as_array() {
            let joined: Vec<String> = arr
                .iter()
                .filter_map(|item| {
                    item.get("text")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                })
                .collect();
            if !joined.is_empty() {
                return Some(joined.join("\n"));
            }
        }
        return Some(content.to_string());
    }
    None
}

fn permission_tool_event(params: &Option<Value>) -> Option<ChatEvent> {
    let v = params.as_ref()?;
    let call = v
        .get("toolCall")
        .or_else(|| v.get("tool_call"))
        .unwrap_or(v);
    tool_event_from_update(call)
}

fn permission_allow(params: &Option<Value>) -> Value {
    let option_id = params
        .as_ref()
        .and_then(first_allow_option)
        .unwrap_or_else(|| "allow-once".into());
    serde_json::json!({
        "outcome": {
            "outcome": "selected",
            "optionId": option_id,
        }
    })
}

fn first_allow_option(params: &Value) -> Option<String> {
    let options = params.get("options")?.as_array()?;
    options
        .iter()
        .find(|opt| {
            opt.get("kind")
                .and_then(|v| v.as_str())
                .is_some_and(|k| k.starts_with("allow"))
        })
        .or_else(|| options.first())
        .and_then(|opt| {
            opt.get("optionId")
                .or_else(|| opt.get("option_id"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
}

async fn emit(events: Option<&EventSink>, ev: ChatEvent) {
    if let Some(tx) = events {
        let _ = tx.send(ev).await;
    }
}

pub fn to_reply(text: String) -> LlmReply {
    LlmReply::Text(text)
}

pub fn encode_b64(bytes: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let n = ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8) | bytes[i + 2] as u32;
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(T[((n >> 6) & 63) as usize] as char);
        out.push(T[(n & 63) as usize] as char);
        i += 3;
    }
    if i < bytes.len() {
        let b0 = bytes[i] as u32;
        let b1 = bytes.get(i + 1).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8);
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        if i + 1 < bytes.len() {
            out.push(T[((n >> 6) & 63) as usize] as char);
            out.push('=');
        } else {
            out.push('=');
            out.push('=');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_agent_text_chunks() {
        let params = Some(serde_json::json!({
            "update": {
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": "hello" }
            }
        }));
        assert_eq!(extract_text(&params).as_deref(), Some("hello"));
    }

    #[test]
    fn reads_tool_call_update() {
        let u = serde_json::json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "tc1",
            "title": "list_bin",
            "status": "pending",
            "rawInput": { "x": 1 }
        });
        let ev = tool_event_from_update(&u).unwrap();
        assert_eq!(ev.name(), "list_bin");
        assert_eq!(ev.status_label(), "pending");
    }

    #[test]
    fn tool_traces_are_not_chat_text() {
        assert!(looks_like_tool_trace(
            r#"{"type":"function_call","name":"read_file","arguments":{}}"#
        ));
        assert!(looks_like_tool_trace(
            r#"Usage notes: {"$schema":"http://json-schema.org/draft-07/schema#","name":"scheduler_create","parameters":{"properties":{}}}"#
        ));
        assert!(!looks_like_tool_trace("Cut a 40s reel from the interview."));
        assert!(!looks_like_tool_trace(
            "through the drive, the newsroom, and home. I'm building that into one short."
        ));
    }

    #[test]
    fn shortens_mcp_tool_names() {
        assert_eq!(short_tool_name("mcp__opencut__list_bin"), "list_bin");
        assert_eq!(short_tool_name("opencut/split"), "split");
        assert_eq!(short_tool_name("assemble"), "assemble");
    }

    #[test]
    fn picks_allow_once() {
        let params = serde_json::json!({
            "options": [
                { "optionId": "reject-once", "kind": "reject_once" },
                { "optionId": "allow-once", "kind": "allow_once" }
            ]
        });
        assert_eq!(first_allow_option(&params).as_deref(), Some("allow-once"));
    }
}
