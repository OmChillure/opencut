//! Stdio MCP: every OpenCut edit/inspect tool. Spawned as `oc-api mcp`
//! with OPENCUT_PROJECT_ID set. ACP `session/new` gets this as `mcpServers`.

use oc_core::mcp_tools;
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use std::sync::OnceLock;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

static PROJECT_ID: OnceLock<Uuid> = OnceLock::new();
static DB: OnceLock<oc_db::Db> = OnceLock::new();

pub async fn serve() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    // stderr only — stdout is JSON-RPC for the ACP agent.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::new("info,oc_api=debug")
        }))
        .init();
    let raw = std::env::var("OPENCUT_PROJECT_ID")
        .map_err(|_| anyhow::anyhow!("OPENCUT_PROJECT_ID required for `oc-api mcp`"))?;
    let project_id = Uuid::parse_str(raw.trim())
        .map_err(|_| anyhow::anyhow!("bad OPENCUT_PROJECT_ID"))?;
    tracing::info!(project = %project_id, "opencut mcp starting");
    let db = oc_db::connect().await?;
    let _ = PROJECT_ID.set(project_id);
    let _ = DB.set(db);
    tracing::info!(project = %project_id, "opencut mcp ready");

    let stdin = io::stdin();
    let mut lock = stdin.lock();
    let mut stdout = io::stdout();
    loop {
        let Some(msg) = read_message(&mut lock) else {
            break;
        };
        if let Some(resp) = handle_rpc(&msg).await {
            write_rpc(&mut stdout, resp);
        }
    }
    Ok(())
}

/// ACP `mcpServers` entry so Grok / Claude / Codex get the OpenCut tools.
pub fn builtin_mcp_acp(project_id: &str) -> Option<Value> {
    let exe = std::env::current_exe().ok()?;
    if !exe.is_file() {
        return None;
    }
    let mut env = vec![
        json!({ "name": "OPENCUT_PROJECT_ID", "value": project_id }),
        json!({ "name": "DB_MAX_CONNECTIONS", "value": "1" }),
        json!({ "name": "RUST_LOG", "value": "info,oc_api=debug" }),
    ];
    if let Ok(url) = std::env::var("DATABASE_URL") {
        if !url.is_empty() {
            env.push(json!({ "name": "DATABASE_URL", "value": url }));
        }
    }
    for key in [
        "R2_ACCOUNT_ID",
        "R2_ACCESS_KEY_ID",
        "R2_SECRET_ACCESS_KEY",
        "R2_BUCKET",
        "R2_ENDPOINT",
        "OPENCUT_MEDIA_DIR",
    ] {
        if let Ok(value) = std::env::var(key) {
            if !value.is_empty() {
                env.push(json!({ "name": key, "value": value }));
            }
        }
    }
    Some(json!({
        "name": "opencut",
        "command": exe.to_string_lossy(),
        "args": ["mcp"],
        "env": env,
    }))
}

async fn handle_rpc(msg: &Value) -> Option<Value> {
    let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let id = msg.get("id").cloned();
    if id.is_none() {
        tracing::debug!(method, "mcp notification");
        return None;
    }
    tracing::info!(method, "mcp <<");
    let result = match method {
        "initialize" => {
            let ver = msg
                .pointer("/params/protocolVersion")
                .and_then(|v| v.as_str())
                .unwrap_or("2024-11-05");
            json!({
                "protocolVersion": ver,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "opencut", "version": "0.1.0" }
            })
        }
        "tools/list" | "list_tools" => tools_list(),
        "tools/call" | "call_tool" => {
            let name = msg
                .pointer("/params/name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let args = msg
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            if name == "see" {
                return Some(see_result(id, args).await);
            }
            match dispatch(name, args).await {
                Ok(text) => {
                    tracing::info!(tool = name, chars = text.len(), "mcp tool ok");
                    json!({
                        "content": [{ "type": "text", "text": text }]
                    })
                }
                Err(err) => {
                    tracing::error!(tool = name, "mcp tool failed: {err}");
                    json!({
                        "content": [{ "type": "text", "text": err }],
                        "isError": true
                    })
                }
            }
        }
        "ping" => json!({}),
        _ => {
            return Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": format!("unknown method {method}") }
            }));
        }
    };
    Some(json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    }))
}

fn tools_list() -> Value {
    json!({ "tools": mcp_tools() })
}

async fn see_result(id: Option<Value>, args: Value) -> Value {
    let fail = |err: String| {
        json!({
            "jsonrpc": "2.0",
            "id": id.clone(),
            "result": {
                "content": [{ "type": "text", "text": err }],
                "isError": true
            }
        })
    };
    let Some(project_id) = PROJECT_ID.get().copied() else {
        return fail("mcp not initialized".into());
    };
    let Some(db) = DB.get() else {
        return fail("mcp db missing".into());
    };
    let media = args.get("media_id").and_then(|v| v.as_str()).unwrap_or("");
    let at = args.get("at").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let Ok(media_id) = Uuid::parse_str(media) else {
        return fail(format!("bad media id {media}"));
    };
    let r2 = oc_db::R2::from_env().await.ok();
    match crate::edit::see_frame(db, r2.as_ref(), project_id, media_id, at).await {
        Ok(frame) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "content": [
                    { "type": "text", "text": frame.caption },
                    {
                        "type": "image",
                        "data": oc_providers::encode_b64(&frame.jpeg),
                        "mimeType": "image/jpeg"
                    }
                ]
            }
        }),
        Err(err) => fail(err),
    }
}

async fn dispatch(name: &str, args: Value) -> Result<String, String> {
    let project_id = *PROJECT_ID
        .get()
        .ok_or_else(|| "mcp not initialized".to_string())?;
    let db = DB.get().ok_or_else(|| "mcp db missing".to_string())?;
    crate::edit::call_tool(db, project_id, name, args).await
}

fn read_message(stdin: &mut impl BufRead) -> Option<Value> {
    let mut first = String::new();
    if stdin.read_line(&mut first).ok()? == 0 {
        return None;
    }
    let trimmed = first.trim();
    if trimmed.is_empty() {
        return read_message(stdin);
    }
    if trimmed.to_ascii_lowercase().starts_with("content-length:") {
        let len: usize = trimmed.split(':').nth(1)?.trim().parse().ok()?;
        let mut rest = String::new();
        loop {
            rest.clear();
            if stdin.read_line(&mut rest).ok()? == 0 {
                return None;
            }
            if rest.trim().is_empty() {
                break;
            }
        }
        let mut buf = vec![0u8; len];
        stdin.read_exact(&mut buf).ok()?;
        return serde_json::from_slice(&buf).ok();
    }
    serde_json::from_str(trimmed).ok()
}

fn write_rpc(out: &mut impl Write, msg: Value) {
    if let Ok(s) = serde_json::to_string(&msg) {
        let _ = writeln!(out, "{s}");
        let _ = out.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn lists_opencut_tools() {
        let listed = handle_rpc(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list"
        }))
        .await
        .unwrap();
        let names: Vec<&str> = listed["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect();
        for need in [
            "list_bin",
            "list_timeline",
            "get_media",
            "list_cues",
            "split",
            "assemble",
            "place_clip",
            "clear_timeline",
            "move",
        ] {
            assert!(names.contains(&need), "missing {need} in {names:?}");
        }
    }

    #[tokio::test]
    async fn initialize_names_server() {
        let init = handle_rpc(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "protocolVersion": "2024-11-05" }
        }))
        .await
        .unwrap();
        assert_eq!(init["result"]["serverInfo"]["name"], "opencut");
    }

    #[test]
    fn builtin_acp_entry() {
        let Some(mcp) = builtin_mcp_acp("11111111-1111-1111-1111-111111111111") else {
            return;
        };
        assert_eq!(mcp["name"], "opencut");
        assert_eq!(mcp["args"][0], "mcp");
        let env = mcp["env"].as_array().unwrap();
        assert!(
            env.iter()
                .any(|e| e["name"] == "OPENCUT_PROJECT_ID")
        );
    }
}
