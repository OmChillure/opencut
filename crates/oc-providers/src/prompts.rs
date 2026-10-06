//! Load every `*.md` in the repo `prompts/` folder.
//! `complete()` prepends this for Grok, Claude, Codex, and any future provider.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const FALLBACK_DIRECTOR: &str = include_str!("../../../prompts/00-director.md");
const FALLBACK_PLAN: &str = include_str!("../../../prompts/01-edit-plan.md");
const FALLBACK_MOTION: &str = include_str!("../../../prompts/02-motion.md");
const FALLBACK_MOTION_SKILL: &str =
    include_str!("../../../prompts/skills/motion-graphics/SKILL.md");

/// All shared director briefs, concatenated. Cached after first read.
#[must_use]
pub fn shared_prompts() -> &'static str {
    static CACHE: OnceLock<String> = OnceLock::new();
    CACHE.get_or_init(load)
}

/// The built-in motion-graphics skill. The model calls this when it decides a line needs one.
/// `file` is empty for the skill entry, or a path relative to the skill folder.
pub fn load_motion_skill(file: Option<&str>) -> Result<String, String> {
    let dir = prompts_dir().join("skills").join("motion-graphics");
    let rel = file.map(str::trim).filter(|text| !text.is_empty());
    match rel {
        None | Some("SKILL.md") => {
            let body = read_skill_file(&dir, "SKILL.md")
                .unwrap_or_else(|_| FALLBACK_MOTION_SKILL.to_string());
            Ok(format!("{}\n\n{}", motion_skill_bridge(&dir), body.trim()))
        }
        Some(rel) => read_skill_file(&dir, rel),
    }
}

fn motion_skill_bridge(dir: &Path) -> String {
    format!(
        "\
# Built-in motion-graphics skill

You decided this line needs a motion graphic. The skill is already part of OpenCut. \
Do not install it, do not run `npx skills add`, and do not update it.

Call `load_motion_skill` again with `file` set to the category or agent page this skill names. \
Example: `categories/charts/module.md`. The folder is {dir}.

Make a short graphic for the footage: kinetic type, a count-up, a chart from the spoken numbers, \
a lower third or callout, a logo sting, or a map when the line is about a place. \
Skip webpage, tweet, and news-card graphics.

The camera rules in the Motion brief still apply to the footage. \
Do not call `add_design` for this line.

Render the clip (MP4, or a transparent WebM or MOV). \
Then call `import_render` with that file path and its duration. \
`place_clip` the media id it returns, on the spoken line. \
Do not stop to ask whether to render.",
        dir = dir.display()
    )
}

fn read_skill_file(dir: &Path, rel: &str) -> Result<String, String> {
    let rel_path = Path::new(rel);
    let escapes = rel_path.is_absolute()
        || rel_path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir));
    if rel.contains('\0') || escapes {
        return Err("file must stay inside the motion-graphics skill".into());
    }
    let path = dir.join(rel_path);
    let root = dir
        .canonicalize()
        .map_err(|_| "motion-graphics skill is not on disk".to_string())?;
    let canon = path
        .canonicalize()
        .map_err(|_| format!("no skill file {rel}"))?;
    if !canon.starts_with(&root) || !canon.is_file() {
        return Err("file must stay inside the motion-graphics skill".into());
    }
    let text = std::fs::read_to_string(&canon).map_err(|err| err.to_string())?;
    if text.len() > 80_000 {
        return Err("skill file is too large".into());
    }
    Ok(text)
}

/// One style guide, only when the request names it. The others stay on disk.
#[must_use]
pub fn style_guide(request: &str) -> Option<String> {
    let lower = request.to_ascii_lowercase();
    let name = if lower.contains("cinematic") {
        "cinematic"
    } else if lower.contains("hype") || lower.contains("tiktok") {
        "hype"
    } else if lower.contains("documentary") || lower.contains("doc ") {
        "documentary"
    } else if lower.contains("vlog") {
        "vlog"
    } else {
        return None;
    };
    let path = prompts_dir().join("styles").join(format!("{name}.md"));
    std::fs::read_to_string(path).ok()
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
        assert!(text.contains("one path"), "{text}");
        assert!(text.contains("shot list already has `motion`"), "{text}");
        assert!(text.contains("load_motion_skill"), "{text}");
        assert!(!text.to_ascii_lowercase().contains("only claude"));
        assert!(!text.contains("npx hyperframes init"), "{text}");
    }

    #[test]
    fn the_model_loads_the_skill_by_calling_for_it() {
        let skill = load_motion_skill(None).unwrap();
        assert!(skill.contains("You decided"), "{skill}");
        assert!(skill.contains("import_render"), "{skill}");
        assert!(skill.contains("kinetic-type"), "{skill}");
        let charts = load_motion_skill(Some("categories/charts/module.md")).unwrap();
        assert!(charts.contains("data-chart"), "{charts}");
        assert!(load_motion_skill(Some("../00-director.md")).is_err());
        assert!(load_motion_skill(Some("/etc/passwd")).is_err());
    }
}
