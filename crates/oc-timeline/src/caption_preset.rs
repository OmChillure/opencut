//! One caption preset for the whole video. The names come from the HyperFrames
//! embedded-captions catalog. OpenCut paints each name in front of the picture.

use crate::{CaptionEffect, CaptionFont, CaptionPlace};
use serde::{Deserialize, Serialize};

/// Fill of the type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptionInk {
    #[default]
    White,
    Cream,
    Bone,
    Black,
    Amber,
    Cyan,
    Mint,
    Rose,
    Violet,
}

impl CaptionInk {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::White => "white",
            Self::Cream => "cream",
            Self::Bone => "bone",
            Self::Black => "black",
            Self::Amber => "amber",
            Self::Cyan => "cyan",
            Self::Mint => "mint",
            Self::Rose => "rose",
            Self::Violet => "violet",
        }
    }

    /// ASS primary colour, `&H00BBGGRR`.
    #[must_use]
    pub fn ass(self) -> &'static str {
        match self {
            Self::White => "&H00FFFFFF",
            Self::Cream => "&H00C8E4F4",
            Self::Bone => "&H00D4E0E6",
            Self::Black => "&H00141414",
            Self::Amber => "&H004AC1FF",
            Self::Cyan => "&H00FFF07E",
            Self::Mint => "&H005CF2B6",
            Self::Rose => "&H00D48AFF",
            Self::Violet => "&H00FFA6C9",
        }
    }
}

/// Edge that keeps the line readable on the picture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptionEdge {
    #[default]
    Dark,
    Light,
}

impl CaptionEdge {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }

    /// ASS outline colour.
    #[must_use]
    pub fn ass(self) -> &'static str {
        match self {
            Self::Dark => "&H00000000",
            Self::Light => "&H00FFFFFF",
        }
    }
}

/// What OpenCut can burn for one preset. Type stays in front of the picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptionPaint {
    pub place: CaptionPlace,
    pub font: CaptionFont,
    pub effect: CaptionEffect,
    pub ink: CaptionInk,
    pub edge: CaptionEdge,
}

/// A named look. Thirty-five ids. One of them is the video.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptionPreset {
    Cream,
    Ink,
    Editorial,
    Keynote,
    Documentary,
    Loud,
    Neon,
    Glitch,
    Chrome,
    Velocity,
    Anchor,
    Arcade,
    Aurora,
    Biolume,
    Brush,
    Chalkboard,
    Dossier,
    Graffiti,
    Hologram,
    Inkwater,
    Laser,
    Lastpage,
    Neonsign,
    Ordnance,
    Papercut,
    Popup,
    Ransom,
    Scoreboard,
    Spectrum,
    Stardust,
    Stomp,
    Terminal,
    Thunder,
    Transit,
    Vhs,
}

const ALL: [CaptionPreset; 35] = [
    CaptionPreset::Cream,
    CaptionPreset::Ink,
    CaptionPreset::Editorial,
    CaptionPreset::Keynote,
    CaptionPreset::Documentary,
    CaptionPreset::Loud,
    CaptionPreset::Neon,
    CaptionPreset::Glitch,
    CaptionPreset::Chrome,
    CaptionPreset::Velocity,
    CaptionPreset::Anchor,
    CaptionPreset::Arcade,
    CaptionPreset::Aurora,
    CaptionPreset::Biolume,
    CaptionPreset::Brush,
    CaptionPreset::Chalkboard,
    CaptionPreset::Dossier,
    CaptionPreset::Graffiti,
    CaptionPreset::Hologram,
    CaptionPreset::Inkwater,
    CaptionPreset::Laser,
    CaptionPreset::Lastpage,
    CaptionPreset::Neonsign,
    CaptionPreset::Ordnance,
    CaptionPreset::Papercut,
    CaptionPreset::Popup,
    CaptionPreset::Ransom,
    CaptionPreset::Scoreboard,
    CaptionPreset::Spectrum,
    CaptionPreset::Stardust,
    CaptionPreset::Stomp,
    CaptionPreset::Terminal,
    CaptionPreset::Thunder,
    CaptionPreset::Transit,
    CaptionPreset::Vhs,
];

impl CaptionPreset {
    #[must_use]
    pub fn all() -> &'static [Self] {
        &ALL
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cream => "cream",
            Self::Ink => "ink",
            Self::Editorial => "editorial",
            Self::Keynote => "keynote",
            Self::Documentary => "documentary",
            Self::Loud => "loud",
            Self::Neon => "neon",
            Self::Glitch => "glitch",
            Self::Chrome => "chrome",
            Self::Velocity => "velocity",
            Self::Anchor => "anchor",
            Self::Arcade => "arcade",
            Self::Aurora => "aurora",
            Self::Biolume => "biolume",
            Self::Brush => "brush",
            Self::Chalkboard => "chalkboard",
            Self::Dossier => "dossier",
            Self::Graffiti => "graffiti",
            Self::Hologram => "hologram",
            Self::Inkwater => "inkwater",
            Self::Laser => "laser",
            Self::Lastpage => "lastpage",
            Self::Neonsign => "neonsign",
            Self::Ordnance => "ordnance",
            Self::Papercut => "papercut",
            Self::Popup => "popup",
            Self::Ransom => "ransom",
            Self::Scoreboard => "scoreboard",
            Self::Spectrum => "spectrum",
            Self::Stardust => "stardust",
            Self::Stomp => "stomp",
            Self::Terminal => "terminal",
            Self::Thunder => "thunder",
            Self::Transit => "transit",
            Self::Vhs => "vhs",
        }
    }

    #[must_use]
    pub fn list() -> String {
        Self::all()
            .iter()
            .map(|preset| preset.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Unknown names are refused. A style word is not a preset.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        let name = raw.trim().to_ascii_lowercase().replace(['_', ' '], "-");
        Self::all()
            .iter()
            .copied()
            .find(|preset| preset.as_str() == name)
    }

    /// The burn for this id. The line stays in front of the picture.
    #[must_use]
    pub fn paint(self) -> CaptionPaint {
        use CaptionEdge::{Dark, Light};
        use CaptionEffect::{Fade, None, Pop, Typewriter};
        use CaptionFont::{Display, Mono, Sans, Serif};
        use CaptionInk::{Amber, Black, Bone, Cream, Cyan, Mint, Rose, Violet, White};
        use CaptionPlace::{Bottom, Lower, Middle, Top};
        let (place, font, effect, ink, edge) = match self {
            Self::Anchor => (Bottom, Sans, Fade, White, Dark),
            Self::Documentary => (Bottom, Sans, None, Bone, Dark),
            Self::Cream => (Lower, Serif, Fade, Cream, Dark),
            Self::Ink => (Bottom, Serif, None, Black, Light),
            Self::Editorial => (Lower, Serif, Fade, Bone, Dark),
            Self::Keynote => (Lower, Sans, Fade, White, Dark),
            Self::Loud => (Lower, Display, Pop, White, Dark),
            Self::Neon => (Lower, Display, Fade, Cyan, Dark),
            Self::Glitch => (Lower, Mono, Fade, Mint, Dark),
            Self::Chrome => (Lower, Display, Fade, Bone, Dark),
            Self::Velocity => (Lower, Display, Pop, Bone, Dark),
            Self::Ordnance => (Lower, Display, Pop, Amber, Dark),
            Self::Neonsign => (Lower, Display, None, Cyan, Dark),
            Self::Terminal => (Bottom, Mono, Typewriter, Mint, Dark),
            Self::Stomp => (Middle, Display, Pop, White, Dark),
            Self::Stardust => (Lower, Serif, Fade, Violet, Dark),
            Self::Scoreboard => (Top, Display, None, Amber, Dark),
            Self::Transit => (Bottom, Sans, None, Bone, Dark),
            Self::Chalkboard => (Bottom, Serif, None, Cream, Dark),
            Self::Dossier => (Bottom, Serif, Fade, Bone, Dark),
            Self::Popup => (Lower, Display, Pop, Rose, Dark),
            Self::Vhs => (Bottom, Mono, None, Cream, Dark),
            Self::Arcade => (Lower, Display, Pop, Mint, Dark),
            Self::Hologram => (Lower, Sans, Fade, Cyan, Dark),
            Self::Graffiti => (Lower, Display, Pop, Rose, Dark),
            Self::Papercut => (Bottom, Serif, None, White, Dark),
            Self::Thunder => (Lower, Display, Pop, White, Dark),
            Self::Laser => (Lower, Mono, None, Mint, Dark),
            Self::Spectrum => (Lower, Sans, Fade, Violet, Dark),
            Self::Aurora => (Lower, Serif, Fade, Mint, Dark),
            Self::Biolume => (Lower, Sans, Fade, Mint, Dark),
            Self::Brush => (Lower, Serif, Fade, Cream, Dark),
            Self::Inkwater => (Bottom, Serif, Fade, Black, Light),
            Self::Lastpage => (Bottom, Serif, None, Bone, Dark),
            Self::Ransom => (Lower, Display, None, White, Dark),
        };
        CaptionPaint {
            place,
            font,
            effect,
            ink,
            edge,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thirty_five_presets_paint_and_a_style_word_is_refused() {
        assert_eq!(CaptionPreset::all().len(), 35);
        let mut names = CaptionPreset::all()
            .iter()
            .map(|preset| preset.as_str())
            .collect::<Vec<_>>();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 35);
        assert_eq!(
            CaptionPreset::parse("ink").unwrap().paint().ink,
            CaptionInk::Black
        );
        assert_eq!(
            CaptionPreset::parse("INK").unwrap().paint().edge,
            CaptionEdge::Light
        );
        assert_eq!(
            CaptionPreset::parse("anchor").unwrap().paint().ink,
            CaptionInk::White
        );
        assert!(CaptionPreset::parse("hormozi").is_none());
        assert!(CaptionPreset::parse("cinematic").is_none());
        assert!(CaptionPreset::parse("clean").is_none());
    }
}
