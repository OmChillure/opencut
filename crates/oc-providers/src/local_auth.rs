//! Login checks for the local Claude, Grok, and Codex subscriptions.
//!
//! Chat and shot labels spawn those CLIs and do not forward tokens.
//! B-roll and motion design read the grok login access token. No API key.

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
    grok_auth_entry()
        .and_then(|entry| {
            entry
                .get("refresh_token")
                .and_then(|v| v.as_str())
                .map(|s| !s.is_empty())
        })
        .unwrap_or(false)
}

/// Access token from `grok login`, when it is still inside its expiry.
/// Callers must not log or persist this value.
#[must_use]
pub fn grok_access_token() -> Option<String> {
    let entry = grok_auth_entry()?;
    let key = entry
        .get("key")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())?;
    if access_token_expired(key) {
        return None;
    }
    Some(key.to_string())
}

fn grok_auth_entry() -> Option<serde_json::Value> {
    let path = home().map(|h| h.join(".grok/auth.json"))?;
    let raw = std::fs::read_to_string(path).ok()?;
    let data: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let map = data.as_object()?;
    map.iter().find_map(|(key, entry)| {
        key.starts_with("https://auth.x.ai")
            .then(|| entry.clone())
    })
}

pub(crate) fn access_token_expired(key: &str) -> bool {
    let Some(exp) = jwt_exp(key) else {
        return false;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    exp <= now + 60
}

fn jwt_exp(token: &str) -> Option<i64> {
    let mut parts = token.split('.');
    let _header = parts.next()?;
    let payload = parts.next()?;
    if parts.next().is_none() {
        return None;
    }
    let bytes = b64url_decode(payload)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    value.get("exp").and_then(|v| v.as_i64())
}

fn b64url_decode(text: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'-' | b'+' => Some(62),
            b'_' | b'/' => Some(63),
            _ => None,
        }
    }
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut buf = 0u32;
    let mut n = 0u32;
    for &c in text.as_bytes() {
        if c == b'=' {
            break;
        }
        let v = val(c)?;
        buf = (buf << 6) | u32::from(v);
        n += 6;
        if n >= 8 {
            n -= 8;
            out.push((buf >> n) as u8);
            buf &= (1u32 << n) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::access_token_expired;

    #[test]
    fn expired_jwt_is_rejected() {
        let token = "eyJhbGciOiJub25lIn0.eyJleHAiOjF9.x";
        assert!(access_token_expired(token));
    }

    #[test]
    fn a_far_expiry_stays_usable() {
        let token = "eyJhbGciOiJub25lIn0.eyJleHAiOjk5OTk5OTk5OTl9.x";
        assert!(!access_token_expired(token));
    }

    #[test]
    fn a_non_jwt_is_left_to_the_server() {
        assert!(!access_token_expired("not-a-jwt"));
    }
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
