use crate::acp::{self, AcpClient};
use crate::catalog::ProviderId;
use crate::llm::{ChatEvent, ChatTurn, EventSink, LlmError, LlmReply};
use oc_tools::McpTool;

/// Run one turn on the local vendor CLI (same as cbot). No API keys.
pub async fn complete(
    provider: &str,
    model: &str,
    system: &str,
    turns: &[ChatTurn],
    tools: &[McpTool],
) -> Result<LlmReply, LlmError> {
    complete_stream(provider, model, system, turns, tools, None).await
}

pub async fn complete_stream(
    provider: &str,
    model: &str,
    system: &str,
    turns: &[ChatTurn],
    tools: &[McpTool],
    events: Option<EventSink>,
) -> Result<LlmReply, LlmError> {
    let id = ProviderId::parse(provider)
        .ok_or_else(|| LlmError::Message(format!("unknown provider {provider}")))?;
    if !id.connected() {
        return Err(LlmError::Message(id.login_hint().into()));
    }
    let prompt = build_prompt(system, turns, tools);
    emit(
        events.as_ref(),
        ChatEvent::status(format!("ACP {} · {model}", id.name())),
    )
    .await;
    let text = run_provider(id, model, &prompt, events.as_ref()).await?;
    Ok(parse_tool_reply(text))
}

async fn run_provider(
    id: ProviderId,
    model: &str,
    prompt: &str,
    events: Option<&EventSink>,
) -> Result<String, LlmError> {
    let (bin, args) = acp_launch(id, model);
    match spawn_acp(&bin, &args, model, prompt, events).await {
        Ok(text) => Ok(text),
        Err(err) => match id {
            ProviderId::Xai => Err(err),
            ProviderId::Claude => {
                tracing::warn!("{err}; falling back to `claude -p`");
                emit(
                    events,
                    ChatEvent::status("ACP adapter missing — `claude -p`"),
                )
                .await;
                claude_print(prompt).await
            }
            ProviderId::Openai => {
                tracing::warn!("{err}; falling back to `codex exec`");
                emit(
                    events,
                    ChatEvent::status("ACP adapter missing — `codex exec`"),
                )
                .await;
                codex_print(prompt).await
            }
        },
    }
}

fn acp_launch(id: ProviderId, model: &str) -> (String, Vec<String>) {
    match id {
        ProviderId::Xai => (
            env_or("OPENCUT_GROK_ACP", "grok"),
            vec![
                "agent".into(),
                "--always-approve".into(),
                "-m".into(),
                model.into(),
                "stdio".into(),
            ],
        ),
        ProviderId::Claude => claude_launch(model),
        ProviderId::Openai => codex_launch(model),
    }
}

fn claude_launch(model: &str) -> (String, Vec<String>) {
    if let Ok(bin) = std::env::var("OPENCUT_CLAUDE_ACP") {
        return (bin, acp_model_args(model));
    }
    if which("claude-agent-acp") {
        return ("claude-agent-acp".into(), acp_model_args(model));
    }
    if which("claude-code-acp") {
        return ("claude-code-acp".into(), acp_model_args(model));
    }
    if npx_ok() {
        return (
            "npx".into(),
            vec![
                "-y".into(),
                "@agentclientprotocol/claude-agent-acp".into(),
            ],
        );
    }
    ("claude-agent-acp".into(), acp_model_args(model))
}

fn codex_launch(model: &str) -> (String, Vec<String>) {
    if let Ok(bin) = std::env::var("OPENCUT_CODEX_ACP") {
        return (bin, acp_model_args(model));
    }
    if which("codex-acp") {
        return ("codex-acp".into(), acp_model_args(model));
    }
    if npx_ok() {
        return (
            "npx".into(),
            vec!["-y".into(), "@agentclientprotocol/codex-acp".into()],
        );
    }
    ("codex-acp".into(), acp_model_args(model))
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
    std::process::Command::new("which")
        .arg(bin)
        .output()
        .ok()
        .is_some_and(|o| o.status.success())
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
    events: Option<&EventSink>,
) -> Result<String, LlmError> {
    tracing::info!(bin, "acp connect");
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| ".".into());
    let mut client = AcpClient::connect(bin, args).await?;
    client.prompt(&cwd, prompt, Some(model), events).await
}

async fn emit(events: Option<&EventSink>, ev: ChatEvent) {
    if let Some(tx) = events {
        let _ = tx.send(ev).await;
    }
}

fn build_prompt(system: &str, turns: &[ChatTurn], tools: &[McpTool]) -> String {
    let mut out = String::new();
    // Every provider goes through here — Grok, Claude, Codex, and any added later.
    out.push_str(&crate::prompts::with_shared_prompts(system));
    if !tools.is_empty() {
        out.push_str(
            "\n\nYou can edit the timeline by emitting one or more lines of the form:\n\
             TOOL <name> <json-args>\n\
             Use only these tools:\n",
        );
        for tool in tools {
            let schema = serde_json::to_string(&tool.input_schema).unwrap_or_else(|_| "{}".into());
            out.push_str(&format!("- {}: {} {}\n", tool.name, tool.description, schema));
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
        acp::to_reply(text)
    } else {
        LlmReply::Tools(calls)
    }
}

async fn claude_print(prompt: &str) -> Result<String, LlmError> {
    let out = tokio::process::Command::new("claude")
        .args(["-p", "--output-format", "text", prompt])
        .output()
        .await
        .map_err(|e| {
            LlmError::Message(format!("spawn claude: {e}. Run `claude auth login` first."))
        })?;
    if !out.status.success() {
        return Err(LlmError::Message(String::from_utf8_lossy(&out.stderr).into()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into())
}

async fn codex_print(prompt: &str) -> Result<String, LlmError> {
    let out = tokio::process::Command::new("codex")
        .args(["exec", "--skip-git-repo-check", prompt])
        .output()
        .await
        .map_err(|e| LlmError::Message(format!("spawn codex: {e}. Run `codex` to login first.")))?;
    if !out.status.success() {
        return Err(LlmError::Message(String::from_utf8_lossy(&out.stderr).into()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tool_lines() {
        let reply = parse_tool_reply(
            "TOOL list_bin {}\nTOOL split {\"at\": 1.2}\nCut at 1.2s.".into(),
        );
        match reply {
            LlmReply::Tools(calls) => {
                assert_eq!(calls.len(), 2);
                assert_eq!(calls[0].name, "list_bin");
                assert_eq!(calls[1].name, "split");
            }
            other => panic!("{other:?}"),
        }
    }
}
