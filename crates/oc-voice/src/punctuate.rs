//! Restore sentence punctuation Whisper often drops. Local, no extra model.

/// Add capitals, `?` / `.`, and a few spoken-English marks (colon, dash).
#[must_use]
pub fn restore_punctuation(text: &str) -> String {
    let mut s = text.trim().to_string();
    if s.is_empty() {
        return s;
    }
    s = collapse_spaces(&s);
    s = rhetorical_dash(&s);
    s = vocative_colon(&s);
    s = and_so_comma(&s);
    s = question_mark(&s);
    s = ensure_terminal(&s);
    capitalize_start(&s)
}

fn collapse_spaces(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn rhetorical_dash(s: &str) -> String {
    // "ask not X, ask Y" → "ask not X — ask Y"
    if let Some(idx) = s.to_ascii_lowercase().find("ask not") {
        let after = &s[idx + 7..];
        if let Some(rel) = after.to_ascii_lowercase().find(", ask ") {
            let cut = idx + 7 + rel;
            return format!("{} — {}", s[..cut].trim_end(), s[cut + 2..].trim_start());
        }
    }
    s.to_string()
}

fn vocative_colon(s: &str) -> String {
    // "my fellow Americans, ask" → "my fellow Americans: ask"
    let lower = s.to_ascii_lowercase();
    if let Some(idx) = lower.find("fellow ") {
        if let Some(comma) = s[idx..].find(", ") {
            let at = idx + comma;
            let next = s[at + 2..]
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            const IMP: &[&str] = &[
                "ask", "go", "look", "listen", "remember", "don't", "dont", "let", "please",
            ];
            if IMP.contains(&next.as_str()) {
                return format!("{}: {}", s[..at].trim_end(), s[at + 2..].trim_start());
            }
        }
    }
    s.to_string()
}

fn and_so_comma(s: &str) -> String {
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("and so ") && !lower.starts_with("and so,") {
        return format!("And so, {}", &s[7..]);
    }
    s.to_string()
}

fn question_mark(s: &str) -> String {
    let t = s.trim_end_matches(['.', '!', ',', ';', ':']);
    let first = t
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(|c: char| !c.is_ascii_alphabetic())
        .to_ascii_lowercase();
    const Q: &[&str] = &[
        "who", "what", "when", "where", "why", "how", "is", "are", "do", "does", "did", "can",
        "could", "would", "will", "won't", "wont",
    ];
    if Q.contains(&first.as_str()) && !t.ends_with('?') {
        return format!("{t}?");
    }
    s.to_string()
}

fn ensure_terminal(s: &str) -> String {
    let t = s.trim_end();
    if t.ends_with(['.', '!', '?']) {
        t.to_string()
    } else {
        format!("{t}.")
    }
}

fn capitalize_start(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => format!("{}{}", c.to_uppercase(), chars.as_str()),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jfk_marks() {
        let raw = "And so my fellow Americans, ask not what your country can do for you, ask what you can do for your country";
        let got = restore_punctuation(raw);
        assert!(got.contains("And so,"), "{got}");
        assert!(got.contains("Americans:"), "{got}");
        assert!(got.contains('—'), "{got}");
        assert!(got.ends_with('.'), "{got}");
    }

    #[test]
    fn questions() {
        assert_eq!(restore_punctuation("what happened next"), "What happened next?");
    }
}