//! Minimal ACP client — same usage as cbot: spawn the local vendor CLI over stdio.
//! Tokens stay in the CLI's own store.

use crate::llm::{LlmError, LlmReply};
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
    pub async fn connect(bin: &str, args: &[&str]) -> Result<Self, LlmError> {
        let mut cmd = Command::new(bin);
        cmd.args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        let mut child = cmd
            .spawn()
            .map_err(|e| LlmError::Message(format!("spawn {bin}: {e}. Is the CLI installed and logged in?")))?;
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
        client
            .request(
                "initialize",
                serde_json::json!({
                    "protocolVersion": 1,
                    "clientInfo": { "name": "opencut", "version": "0.1.0" },
                    "clientCapabilities": {}
                }),
            )
            .await?;
        Ok(client)
    }

    pub async fn prompt(&mut self, cwd: &str, message: &str) -> Result<String, LlmError> {
        let new = self
            .request(
                "session/new",
                serde_json::json!({ "cwd": cwd, "mcpServers": [] }),
            )
            .await?;
        let session_id = new
            .get("sessionId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| LlmError::Message("ACP session/new missing sessionId".into()))?
            .to_string();
        let prompt_id = self.next_id + 1;
        self.send(
            "session/prompt",
            serde_json::json!({
                "sessionId": session_id,
                "prompt": [{ "type": "text", "text": message }]
            }),
        )
        .await?;
        let mut text = String::new();
        loop {
            let msg = self.read().await?;
            if msg.method.as_deref() == Some("session/update") {
                if let Some(chunk) = extract_text(&msg.params) {
                    text.push_str(&chunk);
                }
                continue;
            }
            if msg.id.as_ref().and_then(Value::as_u64) == Some(prompt_id)
                || msg.result.is_some()
                || msg.error.is_some()
            {
                if let Some(err) = msg.error {
                    return Err(LlmError::Message(format!("ACP prompt: {err}")));
                }
                if let Some(chunk) = extract_text(&msg.result) {
                    text.push_str(&chunk);
                }
                break;
            }
        }
        Ok(text)
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value, LlmError> {
        self.send(method, params).await?;
        let id = self.next_id;
        loop {
            let msg = self.read().await?;
            if msg.id.as_ref().and_then(Value::as_u64) == Some(id) {
                if let Some(err) = msg.error {
                    return Err(LlmError::Message(format!("ACP {method}: {err}")));
                }
                return Ok(msg.result.unwrap_or(Value::Null));
            }
        }
    }

    async fn send(&mut self, method: &str, params: Value) -> Result<(), LlmError> {
        self.next_id += 1;
        let line = serde_json::json!({
            "jsonrpc": "2.0",
            "id": self.next_id,
            "method": method,
            "params": params,
        });
        let raw = serde_json::to_string(&line).unwrap();
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
                return Err(LlmError::Message("ACP CLI closed".into()));
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(msg) = serde_json::from_str::<Rpc>(trimmed) {
                return Ok(msg);
            }
        }
    }
}

impl Drop for AcpClient {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

fn extract_text(params: &Option<Value>) -> Option<String> {
    let v = params.as_ref()?;
    if let Some(t) = v.pointer("/update/sessionUpdate").and_then(|x| x.as_str()) {
        if t == "agent_message_chunk" || t == "agent_thought_chunk" {
            if let Some(text) = v.pointer("/update/content/text").and_then(|x| x.as_str()) {
                return Some(text.to_string());
            }
        }
    }
    if let Some(text) = v.pointer("/content/text").and_then(|x| x.as_str()) {
        return Some(text.to_string());
    }
    None
}

pub fn to_reply(text: String) -> LlmReply {
    LlmReply::Text(text)
}
