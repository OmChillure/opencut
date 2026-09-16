use crate::acp::{self, AcpClient};
use crate::catalog::ProviderId;
use crate::llm::{ChatEvent, ChatTurn, EventSink, LlmError, LlmReply};
use oc_tools::McpTool;
use serde_json::Value;

/// Run one turn on the local vendor CLI (same as cbot). No API keys.
pub async fn complete(
    provider: &str,
    model: &str,
    system: &str,
    turns: &[ChatTurn],
    tools: &[McpTool],
) -> Result<LlmReply, LlmError> {
    complete_stream(provider, model, system, turns, tools, None, &[]).await
}

pub async fn complete_stream(
    provider: &str,
    model: &str,
    system: &str,
    turns: &[ChatTurn],
    tools: &[McpTool],
    events: Option<EventSink>,
    mcp_servers: &[Value],
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
    let text = run_provider(id, model, &prompt, mcp_servers, events.as_ref())
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
    events: Option<&EventSink>,
) -> Result<String, LlmError> {
    let (bin, args) = acp_launch(id, model);
    let acp_missing = matches!(id, ProviderId::Claude | ProviderId::Openai)
        && bin != "npx"
        && !std::path::Path::new(&bin).is_file()
        && !which(&bin);
    if acp_missing {
        tracing::warn!(bin = %bin, "acp adapter not on PATH — print fallback");
        return match id {
            ProviderId::Claude => {
                emit(events, ChatEvent::status("ACP adapter missing — `claude -p`")).await;
                claude_print(prompt).await
            }
            ProviderId::Openai => {
                emit(events, ChatEvent::status("ACP adapter missing — `codex exec`")).await;
                codex_print(prompt).await
            }
            ProviderId::Xai => Err(LlmError::Message(format!("ACP adapter missing: {bin}"))),
        };
    }
    tracing::info!(bin = %bin, args = %args.join(" "), "acp launch");
    match spawn_acp(&bin, &args, model, prompt, mcp_servers, events).await {
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
    if let Some(bin) = resolve_bin("claude-agent-acp") {
        return (bin, acp_model_args(model));
    }
    if let Some(bin) = resolve_bin("claude-code-acp") {
        return (bin, acp_model_args(model));
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
    // No adapter on PATH — run_provider falls through to `claude -p`.
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
    extras.into_iter().find(|p| std::path::Path::new(p).is_file())
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
    events: Option<&EventSink>,
) -> Result<String, LlmError> {
    tracing::info!(bin, mcp = mcp_servers.len(), "acp connect");
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| ".".into());
    let mut client = AcpClient::connect(bin, args, events.cloned()).await?;
    client
        .prompt(&cwd, prompt, Some(model), mcp_servers, events)
        .await
}

async fn emit(events: Option<&EventSink>, ev: ChatEvent) {
    if let Some(tx) = events {
        let _ = tx.send(ev).await;
    }
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
