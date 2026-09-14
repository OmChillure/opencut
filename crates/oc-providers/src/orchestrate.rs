use crate::acp::{self, AcpClient};
use crate::catalog::ProviderId;
use crate::llm::{ChatTurn, LlmError, LlmReply};
use oc_tools::McpTool;

/// Run one turn on the local vendor CLI (same as cbot). No API keys.
pub async fn complete(
    provider: &str,
    model: &str,
    system: &str,
    turns: &[ChatTurn],
    tools: &[McpTool],
) -> Result<LlmReply, LlmError> {
    let id = ProviderId::parse(provider)
        .ok_or_else(|| LlmError::Message(format!("unknown provider {provider}")))?;
    if !id.connected() {
        return Err(LlmError::Message(id.login_hint().into()));
    }
    let prompt = build_prompt(system, turns, tools);
    let text = match id {
        ProviderId::Xai => grok_acp(model, &prompt).await?,
        ProviderId::Claude => claude_print(&prompt).await?,
        ProviderId::Openai => codex_print(&prompt).await?,
    };
    Ok(parse_tool_reply(text))
}

fn build_prompt(system: &str, turns: &[ChatTurn], tools: &[McpTool]) -> String {
    let mut out = String::new();
    out.push_str(system);
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

async fn grok_acp(model: &str, prompt: &str) -> Result<String, LlmError> {
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| ".".into());
    let mut client = AcpClient::connect(
        "grok",
        &["agent", "--always-approve", "-m", model, "stdio"],
    )
    .await?;
    client.prompt(&cwd, prompt).await
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
