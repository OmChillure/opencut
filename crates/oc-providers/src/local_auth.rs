//! Same local-subscription checks as cbot. We never read or forward tokens.
//! The vendor CLI reads `~/.grok`, `~/.claude`, `~/.codex` itself.

use std::path::PathBuf;

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

use crate::catalog::ModelInfo;

pub fn grok_models() -> Vec<ModelInfo> {
    let path = home().map(|h| h.join(".grok/models_cache.json"));
    let Some(path) = path else {
        return Vec::new();
    };
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(data) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Some(map) = data.get("models").and_then(|v| v.as_object()) {
        for (id, entry) in map {
            let info = entry.get("info");
            if info.and_then(|i| i.get("hidden")).and_then(|v| v.as_bool()) == Some(true) {
                continue;
            }
            let mid = info
                .and_then(|i| i.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or(id);
            let name = info
                .and_then(|i| i.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or(mid);
            out.push(ModelInfo::new(mid, name));
        }
    }
    out
}

pub fn codex_models() -> Vec<ModelInfo> {
    let path = home().map(|h| h.join(".codex/models_cache.json"));
    let Some(path) = path else {
        return Vec::new();
    };
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(data) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let list = data
        .get("models")
        .and_then(|v| v.as_array())
        .cloned()
        .or_else(|| data.as_array().cloned())
        .unwrap_or_default();
    for item in list {
        let id = item
            .get("id")
            .or_else(|| item.get("slug"))
            .and_then(|v| v.as_str());
        let Some(id) = id else {
            continue;
        };
        let name = item
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(id);
        out.push(ModelInfo::new(id, name));
    }
    out
}

pub fn grok_logged_in() -> bool {
    let path = home().map(|h| h.join(".grok/auth.json"));
    let Some(path) = path else {
        return false;
    };
    let Ok(raw) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(data) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return false;
    };
    let Some(map) = data.as_object() else {
        return false;
    };
    map.iter().any(|(key, entry)| {
        key.starts_with("https://auth.x.ai")
            && entry
                .get("refresh_token")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.is_empty())
    })
}

pub fn claude_logged_in() -> bool {
    if claude_has_tokens() {
        return true;
    }
    std::process::Command::new("claude")
        .args(["auth", "status"])
        .output()
        .ok()
        .and_then(|o| serde_json::from_slice::<serde_json::Value>(&o.stdout).ok())
        .and_then(|v| v.get("loggedIn")?.as_bool())
        .unwrap_or(false)
}

fn claude_has_tokens() -> bool {
    let path = home().map(|h| h.join(".claude/.credentials.json"));
    let Some(path) = path else {
        return false;
    };
    let Ok(raw) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(data) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return false;
    };
    let oauth = data.get("claudeAiOauth");
    oauth
        .and_then(|o| o.get("accessToken"))
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty())
        || oauth
            .and_then(|o| o.get("refreshToken"))
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty())
}

pub fn codex_logged_in() -> bool {
    let path = home().map(|h| h.join(".codex/auth.json"));
    let Some(path) = path else {
        return false;
    };
    let Ok(raw) = std::fs::read_to_string(path) else {
        return false;
    };
    serde_json::from_str::<serde_json::Value>(&raw)
        .ok()
        .and_then(|v| v.as_object().map(|o| !o.is_empty()))
        .unwrap_or(false)
}
