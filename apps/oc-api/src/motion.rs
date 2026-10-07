//! Catalog of motion designs. One call reads the skill page, renders the design, and places it.

use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct Design {
    pub id: &'static str,
    pub page: &'static str,
    /// A lower third or a quote sits behind the person unless the director picks a layout.
    pub behind: bool,
}

const DESIGNS: &[Design] = &[
    Design { id: "kinetic-slam", page: "hyperframes-animation/rules/kinetic-beat-slam.md", behind: false },
    Design { id: "kinetic-typewriter", page: "hyperframes-animation/blueprints/typewriter-reveal.md", behind: false },
    Design { id: "kinetic-words", page: "hyperframes-animation/rules/discrete-text-sequence.md", behind: false },
    Design { id: "kinetic-wave", page: "hyperframes-animation/rules/waterfall-entry.md", behind: false },
    Design { id: "kinetic-bounce", page: "hyperframes-animation/rules/spring-pop-entrance.md", behind: false },
    Design { id: "kinetic-punch", page: "hyperframes-animation/rules/kinetic-beat-slam.md", behind: false },
    Design { id: "kinetic-blur", page: "hyperframes-animation/techniques.md", behind: false },
    Design { id: "kinetic-glitch", page: "hyperframes-animation/rules/chromatic-glitch.md", behind: false },
    Design { id: "kinetic-editorial", page: "talking-head-recut/references/styles/editorial.html", behind: true },
    Design { id: "kinetic-burst", page: "hyperframes-animation/rules/particle-burst.md", behind: false },
    Design { id: "stat-count", page: "hyperframes-animation/rules/counting-dynamic-scale.md", behind: false },
    Design { id: "stat-ring", page: "hyperframes-animation/rules/stat-bars-and-fills.md", behind: false },
    Design { id: "stat-bars", page: "hyperframes-animation/rules/stat-bars-and-fills.md", behind: false },
    Design { id: "chart-bars", page: "motion-graphics/categories/charts/module.md", behind: false },
    Design { id: "chart-line", page: "hyperframes-animation/rules/chart-scrub-readout.md", behind: false },
    Design { id: "chart-pie", page: "motion-graphics/categories/charts/module.md", behind: false },
    Design { id: "chart-race", page: "motion-graphics/categories/charts/module.md", behind: false },
    Design { id: "lower-bar", page: "motion-graphics/categories/lower-thirds/module.md", behind: true },
    Design { id: "lower-callout", page: "talking-head-recut/references/layouts/overlay.html", behind: true },
    Design { id: "lower-quote", page: "talking-head-recut/references/styles/editorial.html", behind: true },
    Design { id: "lower-split", page: "talking-head-recut/references/layouts/split.html", behind: true },
    Design { id: "logo-draw", page: "hyperframes-animation/rules/svg-path-draw.md", behind: false },
    Design { id: "logo-lockup", page: "hyperframes-animation/blueprints/logo-assemble-lockup.md", behind: false },
    Design { id: "map-highlight", page: "motion-graphics/categories/maps/module.md", behind: false },
    Design { id: "map-route", page: "motion-graphics/categories/maps/module.md", behind: false },
];

pub fn design_ids() -> String {
    DESIGNS
        .iter()
        .map(|design| design.id)
        .collect::<Vec<_>>()
        .join(", ")
}

/// `design` wins. A coarse `kind` picks the usual design for that family.
pub fn resolve(design: &str, kind: &str) -> Result<&'static Design, String> {
    let design = design.trim().to_ascii_lowercase().replace('_', "-");
    let kind = kind.trim().to_ascii_lowercase().replace('_', "-");
    let id = if !design.is_empty() {
        alias(&design)
    } else if !kind.is_empty() {
        alias(&kind)
    } else {
        return Err(format!(
            "add_motion needs a design. Choose one of: {}",
            design_ids()
        ));
    };
    DESIGNS
        .iter()
        .find(|item| item.id == id)
        .ok_or_else(|| format!("design is one of: {}", design_ids()))
}

fn alias(name: &str) -> &str {
    match name {
        "kinetic" | "kinetic-type" | "type" | "title" => "kinetic-slam",
        "stat" | "count" | "count-up" | "number" => "stat-count",
        "chart" | "charts" | "graph" => "chart-bars",
        "lower" | "lower-third" | "lower-thirds" | "callout" => "lower-bar",
        "logo" | "logo-reveal" | "sting" | "logo-sting" => "logo-draw",
        "map" | "maps" => "map-route",
        other => other,
    }
}

pub fn design_page(design: &Design) -> Result<String, String> {
    let path = skills_dir().join(design.page);
    let text = std::fs::read_to_string(&path)
        .map_err(|_| format!("design page is not on disk: {}", design.page))?;
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

pub fn frame_size(aspect: &str) -> (u32, u32) {
    match aspect {
        "9:16" => (720, 1280),
        "1:1" => (720, 720),
        "4:3" => (960, 720),
        _ => (1280, 720),
    }
}

pub struct RenderJob<'a> {
    pub design: &'a str,
    pub text: &'a str,
    pub prompt: &'a str,
    pub style: &'a str,
    pub duration: f64,
    pub width: u32,
    pub height: u32,
}

pub async fn render_mp4(job: &RenderJob<'_>) -> Result<Vec<u8>, String> {
    let dir = std::env::temp_dir().join(format!(
        "oc-motion-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| e.to_string())?;
    let out = dir.join("design.mp4");
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("motion/render.mjs");
    let result = tokio::process::Command::new(node_bin())
        .arg(&script)
        .arg("--out")
        .arg(&out)
        .arg("--design")
        .arg(job.design)
        .arg("--text")
        .arg(job.text)
        .arg("--prompt")
        .arg(job.prompt)
        .arg("--style")
        .arg(if job.style.is_empty() { "bold" } else { job.style })
        .arg("--dur")
        .arg(format!("{:.2}", job.duration))
        .arg("--w")
        .arg(job.width.to_string())
        .arg("--h")
        .arg(job.height.to_string())
        .kill_on_drop(true)
        .output()
        .await;
    let bytes = match result {
        Ok(output) if output.status.success() => tokio::fs::read(&out).await.map_err(|e| e.to_string()),
        Ok(output) => {
            let err = String::from_utf8_lossy(&output.stderr);
            let err = err.trim();
            Err(if err.is_empty() {
                "motion render failed".into()
            } else {
                format!("motion render failed: {err}")
            })
        }
        Err(err) => Err(format!("motion render failed: {err}")),
    };
    let _ = tokio::fs::remove_dir_all(&dir).await;
    let bytes = bytes?;
    if bytes.len() < 32 {
        return Err("motion render was empty".into());
    }
    Ok(bytes)
}

fn skills_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../prompts/skills")
}

fn node_bin() -> String {
    if let Ok(bin) = std::env::var("OPENCUT_NODE") {
        if !bin.is_empty() {
            return bin;
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        let root = Path::new(&home).join(".nvm/versions/node");
        if let Ok(entries) = std::fs::read_dir(root) {
            let mut bins: Vec<PathBuf> = entries
                .flatten()
                .map(|entry| entry.path().join("bin/node"))
                .filter(|path| path.is_file())
                .collect();
            bins.sort();
            if let Some(bin) = bins.last() {
                return bin.display().to_string();
            }
        }
    }
    "node".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_design_page_is_on_disk() {
        for design in DESIGNS {
            let page = design_page(design).unwrap_or_else(|err| panic!("{}: {err}", design.id));
            assert!(page.len() > 40, "{}", design.id);
        }
    }

    #[test]
    fn a_kind_picks_a_design_and_an_unknown_design_is_refused() {
        assert_eq!(resolve("", "stat").unwrap().id, "stat-count");
        assert_eq!(resolve("kinetic-glitch", "").unwrap().id, "kinetic-glitch");
        assert_eq!(resolve("logo-reveal", "").unwrap().id, "logo-draw");
        let err = resolve("webpage", "").unwrap_err();
        assert!(err.contains("kinetic-slam"), "{err}");
    }

    #[test]
    fn a_design_page_does_not_ask_for_another_render() {
        for design in DESIGNS {
            let page = design_page(design).unwrap_or_else(|err| panic!("{}: {err}", design.id));
            assert!(
                !page.to_ascii_lowercase().contains("npx hyperframes"),
                "{}",
                design.id
            );
        }
    }

    #[test]
    fn the_tool_and_the_brief_name_every_design() {
        let brief = std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../prompts/02-motion.md"),
        )
        .unwrap();
        let tool = oc_core::mcp_tools()
            .into_iter()
            .find(|tool| tool.name == "add_motion")
            .expect("add_motion");
        for design in DESIGNS {
            assert!(brief.contains(design.id), "brief missing {}", design.id);
            assert!(
                tool.description.contains(design.id),
                "tool missing {}",
                design.id
            );
        }
    }
}
