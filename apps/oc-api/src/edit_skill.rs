//! One decision page per call. The director does not receive the set.

use std::path::PathBuf;

struct Page {
    id: &'static str,
    file: &'static str,
}

const PAGES: &[Page] = &[
    Page {
        id: "promise",
        file: "promise.md",
    },
    Page {
        id: "explainer",
        file: "explainer.md",
    },
    Page {
        id: "product",
        file: "product.md",
    },
    Page {
        id: "ad",
        file: "ad.md",
    },
    Page {
        id: "trailer",
        file: "trailer.md",
    },
    Page {
        id: "documentary",
        file: "documentary.md",
    },
    Page {
        id: "music",
        file: "music.md",
    },
    Page {
        id: "shot",
        file: "shot.md",
    },
    Page {
        id: "rhythm",
        file: "rhythm.md",
    },
    Page {
        id: "revise",
        file: "revise.md",
    },
    Page {
        id: "color",
        file: "color.md",
    },
];

pub fn page_ids() -> String {
    PAGES
        .iter()
        .map(|page| page.id)
        .collect::<Vec<_>>()
        .join(", ")
}

/// One page. An unknown name, including a mood word, is refused.
pub fn page(name: &str) -> Result<String, String> {
    let id = canonical(name);
    if id.is_empty() {
        return Err(format!(
            "edit_skill needs a page. Choose one of: {}",
            page_ids()
        ));
    }
    let Some(page) = PAGES.iter().find(|page| page.id == id) else {
        return Err(refuse(&id));
    };
    let path = skills_dir().join(page.file);
    let text = std::fs::read_to_string(&path)
        .map_err(|_| format!("page is not on disk: {}", page.file))?;
    let cleaned = text
        .lines()
        .filter(|line| !line.to_ascii_lowercase().contains("npx hyperframes"))
        .collect::<Vec<_>>()
        .join("\n");
    let trimmed = cleaned.trim();
    const MAX: usize = 8_000;
    if trimmed.chars().count() <= MAX {
        return Ok(trimmed.to_string());
    }
    let kept = trimmed.chars().take(MAX).collect::<String>();
    Ok(format!("{kept}\n\n[page trimmed]"))
}

fn canonical(name: &str) -> String {
    let name = name.trim().to_ascii_lowercase().replace('_', "-");
    match name.as_str() {
        "doc" | "docs" => "documentary".to_string(),
        "ads" | "commercial" => "ad".to_string(),
        "teaser" | "teasers" => "trailer".to_string(),
        "beat" | "beats" | "beat-sync" => "music".to_string(),
        "grade" | "grading" | "color-grade" | "color-grading" | "finishing" => "color".to_string(),
        "cut" | "cutting" | "continuity" => "shot".to_string(),
        "pacing" | "camera" => "rhythm".to_string(),
        "revision" | "feedback" => "revise".to_string(),
        "ladder" => "promise".to_string(),
        "explain" => "explainer".to_string(),
        other => other.to_string(),
    }
}

fn refuse(name: &str) -> String {
    let list = page_ids();
    match name {
        "hype" | "tiktok" | "cinematic" | "vlog" | "mood" => format!(
            "there is no {name} page. page is one of: {list}. Pick the spine the footage is, then color only if a frame you saw needs a correction."
        ),
        _ => format!("page is one of: {list}"),
    }
}

fn skills_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../prompts/skills/edit")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_edit_page_is_on_disk_and_names_its_source() {
        for page in PAGES {
            let text = super::page(page.id).unwrap_or_else(|err| panic!("{}: {err}", page.id));
            assert!(text.len() > 200, "{}", page.id);
            assert!(
                text.contains("Copyright (c) 2026 generative-media-skills contributors"),
                "{}",
                page.id
            );
            assert!(!text.contains("1.12"), "{}", page.id);
            assert!(!text.contains("1.06"), "{}", page.id);
            assert!(!text.contains("1.04"), "{}", page.id);
            assert!(
                !text.to_ascii_lowercase().contains("npx hyperframes"),
                "{}",
                page.id
            );
            assert!(!text.contains("jl_cut"), "{}", page.id);
            assert!(!text.to_ascii_lowercase().contains("whip"), "{}", page.id);
            assert!(!text.contains("once more"), "{}", page.id);
        }
    }

    #[test]
    fn an_alias_returns_one_page_and_a_mood_is_refused() {
        let color = page("grade").unwrap();
        assert!(color.contains("# Color"), "{color}");
        assert!(page("doc").unwrap().contains("# Documentary"));
        let err = page("cinematic").unwrap_err();
        assert!(err.contains("there is no cinematic page"), "{err}");
        assert!(err.contains("color"), "{err}");
        let missing = page("webpage").unwrap_err();
        assert!(missing.contains("promise"), "{missing}");
        assert!(page("").is_err());
    }

    #[test]
    fn the_tool_lists_every_page() {
        let tool = oc_core::mcp_tools()
            .into_iter()
            .find(|tool| tool.name == "edit_skill")
            .expect("edit_skill");
        for page in PAGES {
            assert!(
                tool.description.contains(page.id),
                "tool missing {}",
                page.id
            );
        }
        assert!(
            tool.description.contains("does not change the timeline"),
            "{}",
            tool.description
        );
    }
}
