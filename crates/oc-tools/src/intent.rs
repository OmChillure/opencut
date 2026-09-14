/// User or AI phrasing mapped onto a timeline tool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    Split,
    Merge,
    Delete,
    TrimIn,
    TrimOut,
    Slip,
    Roll,
    Slide,
    Stretch,
    SplitAll,
    DetachAudio,
    Marker,
    Direct,
    Unknown,
}

/// True when the user wants a finished short, not a single razor edit.
#[must_use]
pub fn is_director_request(text: &str) -> bool {
    let t = text.trim().to_ascii_lowercase();
    if t.is_empty() {
        return false;
    }
    const PHRASES: &[&str] = &[
        "make a vlog",
        "make a video",
        "make a short",
        "make a reel",
        "make a tiktok",
        "make the video",
        "make me a",
        "create a vlog",
        "create a video",
        "create a short",
        "cut a vlog",
        "cut a video",
        "cut a short",
        "cut me a",
        "turn these into",
        "turn this into",
        "edit these",
        "edit this",
        "from these clips",
        "these are",
        "make something",
        "director",
    ];
    if PHRASES.iter().any(|p| t.contains(p)) {
        return true;
    }
    let wants = t.contains("vlog")
        || t.contains("reel")
        || t.contains("tiktok")
        || ((t.contains("video") || t.contains("short") || t.contains("clip"))
            && (t.contains("make") || t.contains("create") || t.contains("edit")));
    wants && !t.contains("split") && !t.contains("trim") && !t.contains("delete")
}

pub fn parse_intent(text: &str) -> Intent {
    let t = text.trim().to_ascii_lowercase();
    if t.is_empty() {
        return Intent::Unknown;
    }
    if is_director_request(&t) {
        return Intent::Direct;
    }
    if looks_like(&t, &["split all", "cut all", "razor all"]) {
        return Intent::SplitAll;
    }
    if looks_like(&t, &["split", "cut", "razor", "slice"]) {
        return Intent::Split;
    }
    if looks_like(&t, &["merge", "join", "combine", "unsplit"]) {
        return Intent::Merge;
    }
    if looks_like(&t, &["delete", "remove clip", "ripple delete", "extract"]) {
        return Intent::Delete;
    }
    if looks_like(&t, &["trim in", "trim start", "cut start"]) {
        return Intent::TrimIn;
    }
    if looks_like(&t, &["trim out", "trim end", "cut end"]) {
        return Intent::TrimOut;
    }
    if looks_like(&t, &["slip"]) {
        return Intent::Slip;
    }
    if looks_like(&t, &["roll"]) {
        return Intent::Roll;
    }
    if looks_like(&t, &["slide"]) {
        return Intent::Slide;
    }
    if looks_like(&t, &["stretch", "slow mo", "slowmo", "speed"]) {
        return Intent::Stretch;
    }
    if looks_like(&t, &["detach audio", "unlink audio", "split audio"]) {
        return Intent::DetachAudio;
    }
    if looks_like(&t, &["marker", "add mark"]) {
        return Intent::Marker;
    }
    Intent::Unknown
}

fn looks_like(text: &str, keys: &[&str]) -> bool {
    keys.iter().any(|key| text.contains(key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn director_phrases() {
        assert_eq!(
            parse_intent("these are 10 clips make a vlog"),
            Intent::Direct
        );
        assert_eq!(parse_intent("make a video from these"), Intent::Direct);
        assert_eq!(parse_intent("split at the playhead"), Intent::Split);
        assert!(!is_director_request("trim the start"));
    }
}
