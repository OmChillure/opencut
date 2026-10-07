use crate::acp::{self, AcpClient};
use crate::catalog::ProviderId;
use crate::llm::{ChatEvent, ChatTurn, EventSink, LlmError, LlmReply};
use oc_tools::McpTool;
use serde_json::Value;

/// Run one turn on the local vendor CLI (same as cbot). No API keys.
/// One still the selected director model should see.
#[derive(Clone, Debug)]
pub struct PromptImage {
    pub caption: String,
    pub jpeg: Vec<u8>,
}

/// True when Claude, Grok, or Codex is signed in on this machine.
#[must_use]
pub fn subscription_ready() -> bool {
    ProviderId::Claude.connected() || ProviderId::Xai.connected() || ProviderId::Openai.connected()
}

/// One still-labeling turn on a local subscription. Claude, then Grok, then Codex.
pub async fn ask_with_stills(prompt: &str, frames: &[PromptImage]) -> Result<String, LlmError> {
    let ready = [ProviderId::Claude, ProviderId::Xai, ProviderId::Openai]
        .into_iter()
        .filter(|id| id.connected())
        .collect::<Vec<_>>();
    if ready.is_empty() {
        return Err(LlmError::Message(
            "Sign in with `claude auth login`, `grok`, or `codex`. Shot labels use that subscription."
                .into(),
        ));
    }
    let mut last = None;
    for id in ready {
        match ask_one(id, prompt, frames).await {
            Ok(text) => {
                tracing::info!(provider = id.as_str(), chars = text.len(), "shot labels");
                return Ok(text);
            }
            Err(err) => {
                tracing::warn!(provider = id.as_str(), "shot labels failed: {err}");
                last = Some(err);
            }
        }
    }
    Err(last.unwrap_or_else(|| LlmError::Message("no subscription answered".into())))
}

async fn ask_one(id: ProviderId, prompt: &str, frames: &[PromptImage]) -> Result<String, LlmError> {
    match id {
        ProviderId::Claude => claude_print(prompt, frames).await,
        ProviderId::Openai => codex_print(prompt, frames).await,
        ProviderId::Xai => grok_stills(prompt, frames).await,
    }
}

async fn grok_stills(prompt: &str, frames: &[PromptImage]) -> Result<String, LlmError> {
    let model = crate::local_auth::grok_models()
        .into_iter()
        .next()
        .map(|info| info.id)
        .unwrap_or_default();
    let bin = env_or("OPENCUT_GROK_ACP", "grok");
    let args = grok_agent_args(&model);
    spawn_acp(&bin, &args, &model, prompt, &[], frames, None).await
}

pub async fn complete(
    provider: &str,
    model: &str,
    system: &str,
    turns: &[ChatTurn],
    tools: &[McpTool],
) -> Result<LlmReply, LlmError> {
    complete_stream(provider, model, system, turns, tools, None, &[], &[]).await
}

pub async fn complete_stream(
    provider: &str,
    model: &str,
    system: &str,
    turns: &[ChatTurn],
    tools: &[McpTool],
    events: Option<EventSink>,
    mcp_servers: &[Value],
    frames: &[PromptImage],
) -> Result<LlmReply, LlmError> {
    let id = ProviderId::parse(provider)
        .ok_or_else(|| LlmError::Message(format!("unknown provider {provider}")))?;
    if !id.connected() {
        let hint = id.login_hint();
        tracing::error!(provider, "provider not logged in: {hint}");
        return Err(LlmError::Message(hint.into()));
    }
    let prompt = build_prompt(system, turns, tools, !mcp_servers.is_empty());
    tracing::info!(
        provider,
        model,
        turns = turns.len(),
        tools = tools.len(),
        mcp = mcp_servers.len(),
        prompt_chars = prompt.len(),
        "chat turn"
    );
    emit(
        events.as_ref(),
        ChatEvent::status(format!("ACP {} · {model}", id.name())),
    )
    .await;
    let text = run_provider(id, model, &prompt, mcp_servers, frames, events.as_ref())
        .await
        .inspect_err(|e| tracing::error!(provider, model, "provider failed: {e}"))?;
    let reply = parse_tool_reply(text);
    match &reply {
        LlmReply::Text(t) => tracing::info!(chars = t.len(), "provider text"),
        LlmReply::Tools(calls) => {
            tracing::info!(n = calls.len(), "provider TOOL lines (MCP fallback)");
            for c in calls {
                tracing::info!(tool = %c.name, "fallback tool");
            }
        }
    }
    Ok(reply)
}

async fn run_provider(
    id: ProviderId,
    model: &str,
    prompt: &str,
    mcp_servers: &[Value],
    frames: &[PromptImage],
    events: Option<&EventSink>,
) -> Result<String, LlmError> {
    let (bin, args) = acp_launch(id, model);
    let acp_missing = matches!(id, ProviderId::Claude | ProviderId::Openai)
        && bin != "npx"
        && !std::path::Path::new(&bin).is_file()
        && !which(&bin);
    if acp_missing {
        let err = refuse_untoolable(id, None);
        tracing::warn!(bin = %bin, "{err}");
        emit(events, ChatEvent::status(err.to_string())).await;
        return Err(err);
    }
    tracing::info!(bin = %bin, args = %args.join(" "), "acp launch");
    match spawn_acp(&bin, &args, model, prompt, mcp_servers, frames, events).await {
        Ok(text) => Ok(text),
        Err(err) => {
            let err = edit_failure(id, err);
            tracing::warn!("{err}");
            emit(events, ChatEvent::status(err.to_string())).await;
            Err(err)
        }
    }
}

fn edit_failure(id: ProviderId, err: LlmError) -> LlmError {
    match id {
        ProviderId::Xai => err,
        ProviderId::Claude | ProviderId::Openai => refuse_untoolable(id, Some(err.to_string())),
    }
}

/// `claude -p` and `codex exec` return text only. They cannot call OpenCut tools.
fn refuse_untoolable(id: ProviderId, detail: Option<String>) -> LlmError {
    let hint = acp_missing_message(id);
    match detail {
        Some(detail) if !detail.is_empty() => LlmError::Message(format!("{detail}\n{hint}")),
        _ => LlmError::Message(hint),
    }
}

fn acp_missing_message(id: ProviderId) -> String {
    match id {
        ProviderId::Openai => "Codex cannot edit without codex-acp. Install codex-acp, set \
             OPENCUT_CODEX_ACP to that binary, or set OPENCUT_ACP_NPX=1."
            .into(),
        ProviderId::Claude => "Claude cannot edit without claude-agent-acp. Install it, set \
             OPENCUT_CLAUDE_ACP to that binary, or set OPENCUT_ACP_NPX=1."
            .into(),
        ProviderId::Xai => "ACP adapter missing.".into(),
    }
}

fn acp_launch(id: ProviderId, model: &str) -> (String, Vec<String>) {
    match id {
        ProviderId::Xai => (env_or("OPENCUT_GROK_ACP", "grok"), grok_agent_args(model)),
        ProviderId::Claude => claude_launch(model),
        ProviderId::Openai => codex_launch(model),
    }
}

/// A private grok process. `--no-leader` keeps it off the user's running session.
fn grok_agent_args(model: &str) -> Vec<String> {
    let mut args = vec![
        "agent".into(),
        "--always-approve".into(),
        "--no-leader".into(),
    ];
    if let Some(sock) = private_grok_socket() {
        args.push("--leader-socket".into());
        args.push(sock);
    }
    if !model.is_empty() {
        args.push("-m".into());
        args.push(model.into());
    }
    args.push("stdio".into());
    args
}

fn private_grok_socket() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    Some(format!(
        "{home}/.grok/leader-opencut-{}.sock",
        std::process::id()
    ))
}

fn claude_launch(model: &str) -> (String, Vec<String>) {
    if let Ok(bin) = std::env::var("OPENCUT_CLAUDE_ACP") {
        return (bin, acp_model_args(model));
    }
    if let Some(bin) = resolve_bin("claude-agent-acp") {
        return (bin, acp_model_args(model));
    }
    if let Some(bin) = resolve_bin("claude-code-acp") {
        return (bin, acp_model_args(model));
    }
    if npx_ok() {
        return (
            "npx".into(),
            vec!["-y".into(), "@agentclientprotocol/claude-agent-acp".into()],
        );
    }
    // No adapter on PATH. Editing refuses this path instead of `claude -p`.
    ("claude-agent-acp".into(), acp_model_args(model))
}

fn codex_launch(model: &str) -> (String, Vec<String>) {
    let explicit = std::env::var("OPENCUT_CODEX_ACP")
        .ok()
        .filter(|path| !path.is_empty());
    let resolved = explicit
        .or_else(|| resolve_bin("codex-acp"))
        .or_else(|| npm_global_bin("codex-acp"));
    codex_command(model, resolved.as_deref(), resolve_bin("npx").is_some())
}

/// `codex-acp` takes the model later, through `session/set_config_option`.
/// A `--model` argument is ignored unless the process is the bundled CLI.
fn codex_command(model: &str, resolved: Option<&str>, npx: bool) -> (String, Vec<String>) {
    let _ = model;
    if let Some(bin) = resolved.map(str::trim).filter(|path| !path.is_empty()) {
        return (bin.to_string(), Vec::new());
    }
    if npx {
        return (
            "npx".into(),
            vec!["-y".into(), "@agentclientprotocol/codex-acp".into()],
        );
    }
    ("codex-acp".into(), Vec::new())
}

fn npm_global_bin(name: &str) -> Option<String> {
    let npm = resolve_bin("npm")?;
    let output = std::process::Command::new(npm)
        .args(["prefix", "-g"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let prefix = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if prefix.is_empty() {
        return None;
    }
    let path = format!("{prefix}/bin/{name}");
    std::path::Path::new(&path).is_file().then_some(path)
}

fn acp_model_args(model: &str) -> Vec<String> {
    if model.is_empty() {
        Vec::new()
    } else {
        vec!["--model".into(), model.into()]
    }
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| default.into())
}

fn which(bin: &str) -> bool {
    resolve_bin(bin).is_some()
}

fn resolve_bin(bin: &str) -> Option<String> {
    if let Ok(out) = std::process::Command::new("which").arg(bin).output() {
        if out.status.success() {
            let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !p.is_empty() && std::path::Path::new(&p).is_file() {
                return Some(p);
            }
        }
    }
    let home = std::env::var("HOME").ok()?;
    let extras = [
        format!("{home}/.local/bin/{bin}"),
        format!("{home}/.nvm/versions/node/v24.10.0/bin/{bin}"),
    ];
    extras
        .into_iter()
        .find(|p| std::path::Path::new(p).is_file())
}

fn npx_ok() -> bool {
    std::env::var("OPENCUT_ACP_NPX")
        .ok()
        .is_some_and(|v| matches!(v.as_str(), "1" | "true" | "yes"))
        && which("npx")
}

async fn spawn_acp(
    bin: &str,
    args: &[String],
    model: &str,
    prompt: &str,
    mcp_servers: &[Value],
    frames: &[PromptImage],
    events: Option<&EventSink>,
) -> Result<String, LlmError> {
    tracing::info!(
        bin,
        mcp = mcp_servers.len(),
        frames = frames.len(),
        "acp connect"
    );
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| ".".into());
    let mut client = AcpClient::connect(bin, args, events.cloned()).await?;
    let session_id = client
        .open_session(&cwd, Some(model), mcp_servers, events)
        .await?;
    client
        .continue_prompt(&session_id, prompt, frames, events)
        .await
}

async fn emit(events: Option<&EventSink>, ev: ChatEvent) {
    if let Some(tx) = events {
        let _ = tx.send(ev).await;
    }
}

/// One ACP process for a whole chat. Later turns send only the new text.
pub struct DirectorSession {
    client: AcpClient,
    session_id: String,
    chars: usize,
}

impl DirectorSession {
    pub async fn open(
        provider: &str,
        model: &str,
        mcp_servers: &[Value],
        events: Option<EventSink>,
    ) -> Result<Self, LlmError> {
        let id = ProviderId::parse(provider)
            .ok_or_else(|| LlmError::Message(format!("unknown provider {provider}")))?;
        if !id.connected() {
            return Err(LlmError::Message(id.login_hint().into()));
        }
        let (bin, args) = acp_launch(id, model);
        let acp_missing = matches!(id, ProviderId::Claude | ProviderId::Openai)
            && bin != "npx"
            && !std::path::Path::new(&bin).is_file()
            && !which(&bin);
        if acp_missing {
            let err = refuse_untoolable(id, None);
            tracing::warn!(bin = %bin, "{err}");
            emit(events.as_ref(), ChatEvent::status(err.to_string())).await;
            return Err(err);
        }
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| ".".into());
        let mut client = match AcpClient::connect(&bin, &args, events.clone()).await {
            Ok(client) => client,
            Err(err) => {
                let err = edit_failure(id, err);
                tracing::warn!("{err}");
                emit(events.as_ref(), ChatEvent::status(err.to_string())).await;
                return Err(err);
            }
        };
        match client
            .open_session(&cwd, Some(model), mcp_servers, events.as_ref())
            .await
        {
            Ok(session_id) => Ok(Self {
                client,
                session_id,
                chars: 0,
            }),
            Err(err) => {
                let err = edit_failure(id, err);
                tracing::warn!("{err}");
                emit(events.as_ref(), ChatEvent::status(err.to_string())).await;
                Err(err)
            }
        }
    }

    /// An open session can call tools. A missing adapter is an error from [`open`].
    #[must_use]
    pub fn remembers(&self) -> bool {
        true
    }

    #[must_use]
    pub fn prompt_chars(&self) -> usize {
        self.chars
    }

    pub async fn turn(
        &mut self,
        message: &str,
        frames: &[PromptImage],
        events: Option<&EventSink>,
    ) -> Result<LlmReply, LlmError> {
        self.chars += message.len();
        let text = self
            .client
            .continue_prompt(&self.session_id, message, frames, events)
            .await?;
        Ok(parse_tool_reply(text))
    }
}

/// Rules, tool list, and the opening turns. Sent once.
pub fn opening_prompt(
    system: &str,
    turns: &[ChatTurn],
    tools: &[McpTool],
    mcp_attached: bool,
) -> String {
    build_prompt(system, turns, tools, mcp_attached)
}

/// Later rounds. The session already has the rules and the earlier turns.
#[must_use]
pub fn followup_prompt(delta: &str) -> String {
    delta.to_string()
}

fn build_prompt(system: &str, turns: &[ChatTurn], tools: &[McpTool], mcp_attached: bool) -> String {
    let mut out = String::new();
    // Every provider goes through here — Grok, Claude, Codex, and any added later.
    out.push_str(&crate::prompts::with_shared_prompts(system));
    if mcp_attached && !tools.is_empty() {
        out.push_str(
            "\n\nThe OpenCut MCP server `opencut` is attached. Call these tools \
             through MCP (do not invent media ids):\n",
        );
        for tool in tools {
            out.push_str(&format!("- {}: {}\n", tool.name, tool.description));
        }
        out.push_str(
            "Call the tools. Do not print TOOL lines when MCP works. \
             After edits, reply in 2–4 short sentences.\n",
        );
    } else if !tools.is_empty() {
        out.push_str(
            "\n\nYou can edit the timeline by emitting one or more lines of the form:\n\
             TOOL <name> <json-args>\n\
             Use only these tools:\n",
        );
        for tool in tools {
            let schema = serde_json::to_string(&tool.input_schema).unwrap_or_else(|_| "{}".into());
            out.push_str(&format!(
                "- {}: {} {}\n",
                tool.name, tool.description, schema
            ));
        }
        out.push_str("If you edit, output TOOL lines first. Then a short sentence for the user.\n");
    }
    out.push('\n');
    for turn in turns {
        out.push_str(&format!("{}: {}\n", turn.role, turn.content));
    }
    out
}

fn parse_tool_reply(text: String) -> LlmReply {
    let mut calls = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("TOOL ") else {
            continue;
        };
        let Some((name, args)) = rest.split_once(' ') else {
            continue;
        };
        let arguments = serde_json::from_str(args.trim()).unwrap_or(serde_json::json!({}));
        if !name.is_empty() {
            calls.push(oc_tools::McpCall {
                name: name.to_string(),
                arguments,
            });
        }
    }
    if calls.is_empty() {
        acp::to_reply(acp::user_visible_text(text))
    } else {
        LlmReply::Tools(calls)
    }
}

async fn claude_print(prompt: &str, frames: &[PromptImage]) -> Result<String, LlmError> {
    if frames.is_empty() {
        let out = tokio::process::Command::new("claude")
            .args(["-p", "--output-format", "text", prompt])
            .output()
            .await
            .map_err(|e| {
                LlmError::Message(format!("spawn claude: {e}. Run `claude auth login` first."))
            })?;
        if !out.status.success() {
            return Err(LlmError::Message(
                String::from_utf8_lossy(&out.stderr).into(),
            ));
        }
        return Ok(String::from_utf8_lossy(&out.stdout).into());
    }
    let paths = write_stills(frames).await?;
    let prompt = prompt_with_stills(prompt, &paths, frames);
    let message = claude_stream_message(&prompt, frames);
    let mut child = tokio::process::Command::new("claude")
        .args([
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--allowedTools",
            "Read",
            "--dangerously-skip-permissions",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            LlmError::Message(format!(
                "spawn claude: {e}. The stills are on disk ({}) but were not sent.",
                still_list(&paths)
            ))
        })?;
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin.write_all(message.as_bytes()).await.map_err(|e| {
            LlmError::Message(format!(
                "claude stdin: {e}. Stills were not dropped on purpose; see {}",
                still_list(&paths)
            ))
        })?;
    }
    let out = child.wait_with_output().await.map_err(|e| {
        LlmError::Message(format!("claude: {e}. Stills are at {}", still_list(&paths)))
    })?;
    if !out.status.success() {
        return Err(LlmError::Message(format!(
            "{}\nStills were attached at {}.",
            String::from_utf8_lossy(&out.stderr).trim(),
            still_list(&paths)
        )));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    claude_stream_text(&stdout).ok_or_else(|| {
        LlmError::Message(format!(
            "claude returned no text for the stills at {}. Raw: {}",
            still_list(&paths),
            stdout.chars().take(400).collect::<String>()
        ))
    })
}

async fn codex_print(prompt: &str, frames: &[PromptImage]) -> Result<String, LlmError> {
    let paths = write_stills(frames).await?;
    let mut cmd = tokio::process::Command::new("codex");
    cmd.arg("exec").arg("--skip-git-repo-check");
    for path in &paths {
        cmd.arg("-i").arg(path);
    }
    cmd.arg(prompt);
    let out = cmd.output().await.map_err(|e| {
        LlmError::Message(format!(
            "spawn codex: {e}. Run `codex` to login first.{}",
            if paths.is_empty() {
                String::new()
            } else {
                format!(" Stills are at {}.", still_list(&paths))
            }
        ))
    })?;
    if !out.status.success() {
        return Err(LlmError::Message(format!(
            "{}{}",
            String::from_utf8_lossy(&out.stderr).trim(),
            if paths.is_empty() {
                String::new()
            } else {
                format!("\nStills were attached at {}.", still_list(&paths))
            }
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into())
}

async fn write_stills(frames: &[PromptImage]) -> Result<Vec<std::path::PathBuf>, LlmError> {
    if frames.is_empty() {
        return Ok(Vec::new());
    }
    let dir = std::env::temp_dir().join(format!("oc-see-{}", std::process::id()));
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| LlmError::Message(format!("see temp dir: {e}")))?;
    let mut paths = Vec::with_capacity(frames.len());
    for (i, frame) in frames.iter().enumerate() {
        let path = dir.join(format!("see-{i}.jpg"));
        tokio::fs::write(&path, &frame.jpeg)
            .await
            .map_err(|e| LlmError::Message(format!("see still: {e}")))?;
        paths.push(path);
    }
    Ok(paths)
}

fn prompt_with_stills(
    prompt: &str,
    paths: &[std::path::PathBuf],
    frames: &[PromptImage],
) -> String {
    let mut out = prompt.to_string();
    out.push_str(
        "\n\nStills for this turn (also attached). Read the file if the image block is missing:\n",
    );
    for (path, frame) in paths.iter().zip(frames) {
        out.push_str(&format!("- {} — {}\n", path.display(), frame.caption));
    }
    out
}

fn still_list(paths: &[std::path::PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn claude_stream_message(prompt: &str, frames: &[PromptImage]) -> String {
    let mut content = vec![serde_json::json!({ "type": "text", "text": prompt })];
    for frame in frames {
        content.push(serde_json::json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": "image/jpeg",
                "data": crate::acp::encode_b64(&frame.jpeg),
            }
        }));
    }
    let message = serde_json::json!({
        "type": "user",
        "message": { "role": "user", "content": content }
    });
    format!("{message}\n")
}

fn claude_stream_text(stdout: &str) -> Option<String> {
    let mut assistant = String::new();
    let mut result = None;
    for line in stdout.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match value.get("type").and_then(Value::as_str) {
            Some("result") => {
                if let Some(text) = value.get("result").and_then(Value::as_str) {
                    if !text.trim().is_empty() {
                        result = Some(text.to_string());
                    }
                }
                if result.is_none() {
                    if let Some(text) = message_text(value.get("message")) {
                        result = Some(text);
                    }
                }
            }
            Some("assistant") => {
                if let Some(text) = message_text(value.get("message")) {
                    assistant.push_str(&text);
                }
            }
            _ => {}
        }
    }
    result.filter(|text| !text.trim().is_empty()).or_else(|| {
        if assistant.trim().is_empty() {
            None
        } else {
            Some(assistant)
        }
    })
}

fn message_text(message: Option<&Value>) -> Option<String> {
    let content = message?.get("content")?;
    if let Some(text) = content.as_str() {
        return Some(text.to_string());
    }
    let mut out = String::new();
    for block in content.as_array()? {
        if block.get("type").and_then(Value::as_str) == Some("text") {
            if let Some(text) = block.get("text").and_then(Value::as_str) {
                out.push_str(text);
            }
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn followup_is_only_the_new_turn() {
        let opening = opening_prompt(
            "Project demo.",
            &[ChatTurn {
                role: "user".into(),
                content: "Make a 1 minute cinematic short.".into(),
            }],
            &[],
            true,
        );
        let follow = followup_prompt("fix: length 80s is outside 55–68s");
        assert!(follow.len() < opening.len());
        assert_eq!(follow, "fix: length 80s is outside 55–68s");
        assert!(!follow.contains("Project demo."));
    }

    #[test]
    fn parses_tool_lines() {
        let reply =
            parse_tool_reply("TOOL list_bin {}\nTOOL split {\"at\": 1.2}\nCut at 1.2s.".into());
        match reply {
            LlmReply::Tools(calls) => {
                assert_eq!(calls.len(), 2);
                assert_eq!(calls[0].name, "list_bin");
                assert_eq!(calls[1].name, "split");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn codex_without_acp_cannot_edit() {
        let msg = acp_missing_message(ProviderId::Openai);
        assert!(msg.contains("codex-acp"));
        assert!(msg.contains("OPENCUT_CODEX_ACP"));
        assert!(msg.contains("OPENCUT_ACP_NPX"));
        assert!(!msg.contains("codex exec"));
        let wrapped = refuse_untoolable(ProviderId::Openai, Some("connect failed".into()));
        let text = wrapped.to_string();
        assert!(text.contains("connect failed"));
        assert!(text.contains("codex-acp"));
        let claude = acp_missing_message(ProviderId::Claude);
        assert!(claude.contains("OPENCUT_CLAUDE_ACP"));
        assert!(!claude.contains("claude -p"));
    }

    #[test]
    fn codex_launches_the_resolved_binary_without_a_model_flag() {
        let (bin, args) = codex_command("gpt-5", Some("/opt/codex-acp"), false);
        assert_eq!(bin, "/opt/codex-acp");
        assert!(args.is_empty());
        let (bin, args) = codex_command("gpt-5", None, true);
        assert_eq!(bin, "npx");
        assert_eq!(
            args,
            vec![
                "-y".to_string(),
                "@agentclientprotocol/codex-acp".to_string()
            ]
        );
        let (bin, args) = codex_command("", Some("  "), false);
        assert_eq!(bin, "codex-acp");
        assert!(args.is_empty());
    }

    #[test]
    fn claude_stream_keeps_the_result_and_the_still() {
        let text = claude_stream_text(
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"looking\"}]}}\n{\"type\":\"result\",\"result\":\"cut on the smile\"}\n",
        );
        assert_eq!(text.as_deref(), Some("cut on the smile"));
        let message = claude_stream_message(
            "look",
            &[PromptImage {
                caption: "wide".into(),
                jpeg: vec![1, 2, 3],
            }],
        );
        assert!(message.contains("\"media_type\":\"image/jpeg\""));
        assert!(message.contains("look"));
    }
}
