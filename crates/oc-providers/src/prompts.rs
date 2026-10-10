//! Load every `*.md` in the repo `prompts/` folder.
//! `complete()` prepends this for Grok, Claude, Codex, and any future provider.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const FALLBACK_DIRECTOR: &str = include_str!("../../../prompts/00-director.md");
const FALLBACK_PLAN: &str = include_str!("../../../prompts/01-edit-plan.md");
const FALLBACK_MOTION: &str = include_str!("../../../prompts/02-motion.md");

/// All shared director briefs, concatenated. Cached after first read.
#[must_use]
pub fn shared_prompts() -> &'static str {
    static CACHE: OnceLock<String> = OnceLock::new();
    CACHE.get_or_init(load)
}

/// Prepend director briefs to a per-request system string.
#[must_use]
pub fn with_shared_prompts(system: &str) -> String {
    let shared = shared_prompts().trim();
    if shared.is_empty() {
        return system.to_string();
    }
    if system.trim().is_empty() {
        return shared.to_string();
    }
    format!("{shared}\n\n{system}")
}

fn load() -> String {
    if let Some(from_disk) = read_dir_markdown(&prompts_dir()) {
        if !from_disk.trim().is_empty() {
            return from_disk;
        }
    }
    format!("{FALLBACK_DIRECTOR}\n\n{FALLBACK_PLAN}\n\n{FALLBACK_MOTION}\n")
}

fn prompts_dir() -> PathBuf {
    if let Ok(raw) = std::env::var("OPENCUT_PROMPTS_DIR") {
        let path = PathBuf::from(raw);
        if path.is_dir() {
            return path;
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(found) = find_prompts(&cwd) {
            return found;
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../prompts")
}

fn find_prompts(start: &Path) -> Option<PathBuf> {
    let mut dir = start.to_path_buf();
    for _ in 0..8 {
        let candidate = dir.join("prompts");
        if candidate.is_dir() && has_markdown(&candidate) {
            return Some(candidate);
        }
        if !dir.pop() {
            break;
        }
    }
    None
}

fn has_markdown(dir: &Path) -> bool {
    std::fs::read_dir(dir).ok().is_some_and(|entries| {
        entries.flatten().any(|e| {
            e.path()
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
        })
    })
}

fn read_dir_markdown(dir: &Path) -> Option<String> {
    let mut names: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| !n.eq_ignore_ascii_case("README.md"))
        })
        .collect();
    names.sort();
    if names.is_empty() {
        return None;
    }
    let mut out = String::new();
    for path in names {
        if let Ok(text) = std::fs::read_to_string(&path) {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                continue;
            }
            out.push_str(trimmed);
            out.push_str("\n\n");
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ships_director_brief() {
        let text = shared_prompts();
        assert!(text.contains("picture editor"), "{text}");
        assert!(text.contains("place_clip"), "{text}");
        assert!(text.contains("source_in"), "{text}");
        assert!(text.contains("one caption theme"), "{text}");
        assert!(text.contains("caption_mood"), "{text}");
        assert!(
            text.contains("vertical reel is not automatically hype"),
            "{text}"
        );
        assert!(!text.contains("1.06 to 1.08"), "{text}");
        assert!(!text.contains("add_design"), "{text}");
        assert!(text.contains("shot list already has `motion`"), "{text}");
        assert!(text.contains("add_motion"), "{text}");
        assert!(text.contains("edit_skill"), "{text}");
        assert!(!text.contains("about 1.12"), "{text}");
        assert!(!text.contains("about 1.06"), "{text}");
        assert!(!text.contains("about 1.04"), "{text}");
        assert!(!text.contains("`caption_mood` kinetic"), "{text}");
        assert!(text.contains("kinetic-slam"), "{text}");
        assert!(text.contains("chart-bars"), "{text}");
        assert!(text.contains("On the plan"), "{text}");
        assert!(text.contains("Do not call `edit_skill` again"), "{text}");
        assert!(text.contains("Do not write HTML"), "{text}");
        assert!(!text.contains("design page"), "{text}");
        assert!(!text.contains("load_motion_skill"), "{text}");
        assert!(!text.contains("import_render"), "{text}");
        assert!(!text.to_ascii_lowercase().contains("only claude"));
        assert!(!text.contains("npx hyperframes init"), "{text}");
        assert!(text.contains("already has the bin"), "{text}");
        assert!(text.contains("A `note:` line is taste"), "{text}");
        assert!(!text.contains("Repeat for each keep"), "{text}");
        assert!(
            !text.contains("Call `list_bin` and `list_timeline` first"),
            "{text}"
        );
        assert!(!text.contains("call the tool each join needs"), "{text}");
        assert!(!text.contains("See the start, middle, and end"), "{text}");
        assert!(!text.contains("Call `watch` before you cut"), "{text}");
    }
}
