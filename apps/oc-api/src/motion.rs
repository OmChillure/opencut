//! Catalog of motion designs. One call renders the design and places it.
//! `render.mjs` draws the picture. No skill page is loaded.

use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct Design {
    pub id: &'static str,
    /// A lower third or a quote sits behind the person unless the director picks a layout.
    pub behind: bool,
}

const DESIGNS: &[Design] = &[
    Design {
        id: "kinetic-slam",
        behind: false,
    },
    Design {
        id: "kinetic-typewriter",
        behind: false,
    },
    Design {
        id: "kinetic-words",
        behind: false,
    },
    Design {
        id: "kinetic-wave",
        behind: false,
    },
    Design {
        id: "kinetic-bounce",
        behind: false,
    },
    Design {
        id: "kinetic-punch",
        behind: false,
    },
    Design {
        id: "kinetic-blur",
        behind: false,
    },
    Design {
        id: "kinetic-glitch",
        behind: false,
    },
    Design {
        id: "kinetic-editorial",
        behind: true,
    },
    Design {
        id: "kinetic-burst",
        behind: false,
    },
    Design {
        id: "stat-count",
        behind: false,
    },
    Design {
        id: "stat-ring",
        behind: false,
    },
    Design {
        id: "stat-bars",
        behind: false,
    },
    Design {
        id: "chart-bars",
        behind: false,
    },
    Design {
        id: "chart-line",
        behind: false,
    },
    Design {
        id: "chart-pie",
        behind: false,
    },
    Design {
        id: "chart-race",
        behind: false,
    },
    Design {
        id: "lower-bar",
        behind: true,
    },
    Design {
        id: "lower-callout",
        behind: true,
    },
    Design {
        id: "lower-quote",
        behind: true,
    },
    Design {
        id: "lower-split",
        behind: true,
    },
    Design {
        id: "logo-draw",
        behind: false,
    },
    Design {
        id: "logo-lockup",
        behind: false,
    },
    Design {
        id: "map-highlight",
        behind: false,
    },
    Design {
        id: "map-route",
        behind: false,
    },
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
        .arg(if job.style.is_empty() {
            "bold"
        } else {
            job.style
        })
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
        Ok(output) if output.status.success() => {
            tokio::fs::read(&out).await.map_err(|e| e.to_string())
        }
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
    fn a_kind_picks_a_design_and_an_unknown_design_is_refused() {
        assert_eq!(resolve("", "stat").unwrap().id, "stat-count");
        assert_eq!(resolve("kinetic-glitch", "").unwrap().id, "kinetic-glitch");
        assert_eq!(resolve("logo-reveal", "").unwrap().id, "logo-draw");
        let err = resolve("webpage", "").unwrap_err();
        assert!(err.contains("kinetic-slam"), "{err}");
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
