use crate::ids::{ClipId, GroupId, LinkId, MarkerId, MediaId, TrackId};
use crate::{Result, TimelineError};
use oc_time::{Duration, FrameRate, Time};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    Video,
    Audio,
    Caption,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub x: f32,
    pub y: f32,
    pub scale: f32,
    pub rotation: f32,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            scale: 1.0,
            rotation: 0.0,
        }
    }
}

/// Same-track mix at a clip's outgoing cut. Kdenlive Mix / Shotcut overlap / ffmpeg xfade.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionKind {
    #[default]
    Cut,
    Dissolve,
    FadeBlack,
    FadeWhite,
    /// Kept as the default slide (right). Prefer `SlideRight`.
    Slide,
    SlideLeft,
    SlideRight,
    SlideUp,
    SlideDown,
    /// Kept as the default wipe (left). Prefer `WipeLeft`.
    Wipe,
    WipeLeft,
    WipeRight,
    WipeUp,
    WipeDown,
    WipeTl,
    WipeTr,
    WipeBl,
    WipeBr,
    SmoothLeft,
    SmoothRight,
    SmoothUp,
    SmoothDown,
    CoverLeft,
    CoverRight,
    CoverUp,
    CoverDown,
    RevealLeft,
    RevealRight,
    RevealUp,
    RevealDown,
    CircleOpen,
    CircleClose,
    Radial,
    Pixelize,
    HorzOpen,
    VertOpen,
}

impl TransitionKind {
    pub const ALL: &'static [Self] = &[
        Self::Cut,
        Self::Dissolve,
        Self::FadeBlack,
        Self::FadeWhite,
        Self::SlideLeft,
        Self::SlideRight,
        Self::SlideUp,
        Self::SlideDown,
        Self::WipeLeft,
        Self::WipeRight,
        Self::WipeUp,
        Self::WipeDown,
        Self::WipeTl,
        Self::WipeTr,
        Self::WipeBl,
        Self::WipeBr,
        Self::SmoothLeft,
        Self::SmoothRight,
        Self::SmoothUp,
        Self::SmoothDown,
        Self::CoverLeft,
        Self::CoverRight,
        Self::CoverUp,
        Self::CoverDown,
        Self::RevealLeft,
        Self::RevealRight,
        Self::RevealUp,
        Self::RevealDown,
        Self::CircleOpen,
        Self::CircleClose,
        Self::Radial,
        Self::Pixelize,
        Self::HorzOpen,
        Self::VertOpen,
    ];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Cut => "Cut",
            Self::Dissolve => "Dissolve",
            Self::FadeBlack => "Fade black",
            Self::FadeWhite => "Fade white",
            Self::Slide | Self::SlideRight => "Slide right",
            Self::SlideLeft => "Slide left",
            Self::SlideUp => "Slide up",
            Self::SlideDown => "Slide down",
            Self::Wipe | Self::WipeLeft => "Wipe left",
            Self::WipeRight => "Wipe right",
            Self::WipeUp => "Wipe up",
            Self::WipeDown => "Wipe down",
            Self::WipeTl => "Wipe top-left",
            Self::WipeTr => "Wipe top-right",
            Self::WipeBl => "Wipe bottom-left",
            Self::WipeBr => "Wipe bottom-right",
            Self::SmoothLeft => "Smooth left",
            Self::SmoothRight => "Smooth right",
            Self::SmoothUp => "Smooth up",
            Self::SmoothDown => "Smooth down",
            Self::CoverLeft => "Cover left",
            Self::CoverRight => "Cover right",
            Self::CoverUp => "Cover up",
            Self::CoverDown => "Cover down",
            Self::RevealLeft => "Reveal left",
            Self::RevealRight => "Reveal right",
            Self::RevealUp => "Reveal up",
            Self::RevealDown => "Reveal down",
            Self::CircleOpen => "Circle open",
            Self::CircleClose => "Circle close",
            Self::Radial => "Radial",
            Self::Pixelize => "Pixelize",
            Self::HorzOpen => "Open horizontal",
            Self::VertOpen => "Open vertical",
        }
    }

    #[must_use]
    pub fn hint(self) -> &'static str {
        match self {
            Self::Cut => "Hard cut, no blend",
            Self::Dissolve => "Crossfade between shots",
            Self::FadeBlack => "Dip to black, then the next shot",
            Self::FadeWhite => "Dip to white, then the next shot",
            Self::Slide | Self::SlideRight | Self::SlideLeft | Self::SlideUp | Self::SlideDown => {
                "Push the next shot on"
            }
            Self::Wipe
            | Self::WipeLeft
            | Self::WipeRight
            | Self::WipeUp
            | Self::WipeDown
            | Self::WipeTl
            | Self::WipeTr
            | Self::WipeBl
            | Self::WipeBr
            | Self::SmoothLeft
            | Self::SmoothRight
            | Self::SmoothUp
            | Self::SmoothDown => "Edge wipe across the frame",
            Self::CoverLeft
            | Self::CoverRight
            | Self::CoverUp
            | Self::CoverDown => "Next shot covers this one",
            Self::RevealLeft
            | Self::RevealRight
            | Self::RevealUp
            | Self::RevealDown => "This shot slides off, revealing the next",
            Self::CircleOpen | Self::CircleClose | Self::Radial => "Iris / clock wipe",
            Self::Pixelize => "Pixelate into the next shot",
            Self::HorzOpen | Self::VertOpen => "Split open to the next shot",
        }
    }

    #[must_use]
    pub fn group(self) -> &'static str {
        match self {
            Self::Cut => "Cut",
            Self::Dissolve | Self::FadeBlack | Self::FadeWhite => "Dissolve",
            Self::Slide
            | Self::SlideLeft
            | Self::SlideRight
            | Self::SlideUp
            | Self::SlideDown
            | Self::CoverLeft
            | Self::CoverRight
            | Self::CoverUp
            | Self::CoverDown
            | Self::RevealLeft
            | Self::RevealRight
            | Self::RevealUp
            | Self::RevealDown => "Slide",
            Self::Wipe
            | Self::WipeLeft
            | Self::WipeRight
            | Self::WipeUp
            | Self::WipeDown
            | Self::WipeTl
            | Self::WipeTr
            | Self::WipeBl
            | Self::WipeBr
            | Self::SmoothLeft
            | Self::SmoothRight
            | Self::SmoothUp
            | Self::SmoothDown => "Wipe",
            Self::CircleOpen
            | Self::CircleClose
            | Self::Radial
            | Self::Pixelize
            | Self::HorzOpen
            | Self::VertOpen => "Shape",
        }
    }

    #[must_use]
    pub fn mix_seconds(self) -> f64 {
        match self {
            Self::Cut => 0.0,
            _ => 0.8,
        }
    }

    /// ffmpeg `xfade=transition=` name.
    #[must_use]
    pub fn xfade(self) -> &'static str {
        match self {
            Self::Cut | Self::Dissolve => "fade",
            Self::FadeBlack => "fadeblack",
            Self::FadeWhite => "fadewhite",
            Self::Slide | Self::SlideRight => "slideright",
            Self::SlideLeft => "slideleft",
            Self::SlideUp => "slideup",
            Self::SlideDown => "slidedown",
            Self::Wipe | Self::WipeLeft => "wipeleft",
            Self::WipeRight => "wiperight",
            Self::WipeUp => "wipeup",
            Self::WipeDown => "wipedown",
            Self::WipeTl => "wipetl",
            Self::WipeTr => "wipetr",
            Self::WipeBl => "wipebl",
            Self::WipeBr => "wipebr",
            Self::SmoothLeft => "smoothleft",
            Self::SmoothRight => "smoothright",
            Self::SmoothUp => "smoothup",
            Self::SmoothDown => "smoothdown",
            Self::CoverLeft => "coverleft",
            Self::CoverRight => "coverright",
            Self::CoverUp => "coverup",
            Self::CoverDown => "coverdown",
            Self::RevealLeft => "revealleft",
            Self::RevealRight => "revealright",
            Self::RevealUp => "revealup",
            Self::RevealDown => "revealdown",
            Self::CircleOpen => "circleopen",
            Self::CircleClose => "circleclose",
            Self::Radial => "radial",
            Self::Pixelize => "pixelize",
            Self::HorzOpen => "horzopen",
            Self::VertOpen => "vertopen",
        }
    }

    #[must_use]
    pub fn from_key(raw: &str) -> Self {
        let k = raw.trim().to_ascii_lowercase().replace('-', "_");
        match k.as_str() {
            "cut" => Self::Cut,
            "dissolve" | "fade" | "crossfade" => Self::Dissolve,
            "fade_black" | "fadeblack" | "dip_to_black" => Self::FadeBlack,
            "fade_white" | "fadewhite" => Self::FadeWhite,
            "slide" | "slide_right" | "slideright" => Self::SlideRight,
            "slide_left" | "slideleft" => Self::SlideLeft,
            "slide_up" | "slideup" => Self::SlideUp,
            "slide_down" | "slidedown" => Self::SlideDown,
            "wipe" | "wipe_left" | "wipeleft" => Self::WipeLeft,
            "wipe_right" | "wiperight" => Self::WipeRight,
            "wipe_up" | "wipeup" => Self::WipeUp,
            "wipe_down" | "wipedown" => Self::WipeDown,
            "wipe_tl" | "wipetl" => Self::WipeTl,
            "wipe_tr" | "wipetr" => Self::WipeTr,
            "wipe_bl" | "wipebl" => Self::WipeBl,
            "wipe_br" | "wipebr" => Self::WipeBr,
            "smooth_left" | "smoothleft" => Self::SmoothLeft,
            "smooth_right" | "smoothright" => Self::SmoothRight,
            "smooth_up" | "smoothup" => Self::SmoothUp,
            "smooth_down" | "smoothdown" => Self::SmoothDown,
            "cover_left" | "coverleft" => Self::CoverLeft,
            "cover_right" | "coverright" => Self::CoverRight,
            "cover_up" | "coverup" => Self::CoverUp,
            "cover_down" | "coverdown" => Self::CoverDown,
            "reveal_left" | "revealleft" => Self::RevealLeft,
            "reveal_right" | "revealright" => Self::RevealRight,
            "reveal_up" | "revealup" => Self::RevealUp,
            "reveal_down" | "revealdown" => Self::RevealDown,
            "circle_open" | "circleopen" => Self::CircleOpen,
            "circle_close" | "circleclose" => Self::CircleClose,
            "radial" => Self::Radial,
            "pixelize" | "pixel" => Self::Pixelize,
            "horz_open" | "horzopen" => Self::HorzOpen,
            "vert_open" | "vertopen" => Self::VertOpen,
            _ => Self::Dissolve,
        }
    }

    #[must_use]
    pub fn slide_delta(self) -> Option<(f64, f64)> {
        match self {
            Self::Slide | Self::SlideRight | Self::CoverRight | Self::RevealLeft => Some((-1.0, 0.0)),
            Self::SlideLeft | Self::CoverLeft | Self::RevealRight => Some((1.0, 0.0)),
            Self::SlideUp | Self::CoverUp | Self::RevealDown => Some((0.0, 1.0)),
            Self::SlideDown | Self::CoverDown | Self::RevealUp => Some((0.0, -1.0)),
            _ => None,
        }
    }

    #[must_use]
    pub fn wipe_inset(self, p: f64) -> Option<String> {
        let p = (p * 100.0).clamp(0.0, 100.0);
        match self {
            Self::Wipe | Self::WipeLeft | Self::SmoothLeft => {
                Some(format!("inset(0 {p:.1}% 0 0)"))
            }
            Self::WipeRight | Self::SmoothRight => Some(format!("inset(0 0 0 {p:.1}%)")),
            Self::WipeUp | Self::SmoothUp => Some(format!("inset(0 0 {p:.1}% 0)")),
            Self::WipeDown | Self::SmoothDown => Some(format!("inset({p:.1}% 0 0 0)")),
            Self::WipeTl => Some(format!("inset(0 {p:.1}% {p:.1}% 0)")),
            Self::WipeTr => Some(format!("inset(0 0 {p:.1}% {p:.1}%)")),
            Self::WipeBl => Some(format!("inset({p:.1}% {p:.1}% 0 0)")),
            Self::WipeBr => Some(format!("inset({p:.1}% 0 0 {p:.1}%)")),
            Self::HorzOpen => {
                let h = p / 2.0;
                Some(format!("inset(0 {h:.1}% 0 {h:.1}%)"))
            }
            Self::VertOpen => {
                let h = p / 2.0;
                Some(format!("inset({h:.1}% 0 {h:.1}% 0)"))
            }
            _ => None,
        }
    }
}

/// Lift / gamma / gain plus a preset LUT. 0 = unchanged. Range roughly -1..1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Grade {
    #[serde(default)]
    pub exposure: f32,
    #[serde(default)]
    pub contrast: f32,
    #[serde(default)]
    pub saturation: f32,
    #[serde(default)]
    pub temperature: f32,
    /// Shadows. Kdenlive lift.
    #[serde(default)]
    pub lift: f32,
    /// Midtones. Kdenlive gamma.
    #[serde(default)]
    pub gamma: f32,
    /// Highlights. Kdenlive gain.
    #[serde(default)]
    pub gain: f32,
    #[serde(default)]
    pub lut: Lut,
}

/// Named looks. Applied as filter chains, not a loaded .cube file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lut {
    #[default]
    None,
    Film,
    Cool,
    Warm,
    TealOrange,
    Mono,
}

impl Grade {
    #[must_use]
    pub fn punchy() -> Self {
        Self {
            exposure: 0.08,
            contrast: 0.14,
            saturation: 0.12,
            temperature: 0.06,
            lut: Lut::Film,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn interview() -> Self {
        Self {
            exposure: 0.03,
            contrast: 0.06,
            saturation: 0.02,
            temperature: 0.0,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn ad() -> Self {
        Self {
            exposure: 0.06,
            contrast: 0.2,
            saturation: 0.08,
            temperature: -0.02,
            gain: 0.06,
            lut: Lut::TealOrange,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn vlog() -> Self {
        Self {
            exposure: 0.05,
            contrast: 0.1,
            saturation: 0.1,
            temperature: 0.04,
            lut: Lut::Warm,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn documentary() -> Self {
        Self {
            contrast: 0.05,
            saturation: -0.04,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn is_identity(self) -> bool {
        self.exposure.abs() < 1e-4
            && self.contrast.abs() < 1e-4
            && self.saturation.abs() < 1e-4
            && self.temperature.abs() < 1e-4
            && self.lift.abs() < 1e-4
            && self.gamma.abs() < 1e-4
            && self.gain.abs() < 1e-4
            && self.lut == Lut::None
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Fx {
    #[serde(default)]
    pub blur: f32,
    #[serde(default)]
    pub grain: f32,
    #[serde(default)]
    pub vignette: f32,
}

impl Fx {
    #[must_use]
    pub fn film() -> Self {
        Self {
            blur: 0.0,
            grain: 0.18,
            vignette: 0.35,
        }
    }

    #[must_use]
    pub fn is_identity(self) -> bool {
        self.blur < 1e-4 && self.grain < 1e-4 && self.vignette < 1e-4
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphicKind {
    #[default]
    Title,
    LowerThird,
    Card,
    Shape,
    Sticker,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Graphic {
    pub kind: GraphicKind,
    #[serde(default)]
    pub text: String,
}

impl Graphic {
    #[must_use]
    pub fn title(text: impl Into<String>) -> Self {
        Self {
            kind: GraphicKind::Title,
            text: text.into(),
        }
    }

    #[must_use]
    pub fn lower_third(text: impl Into<String>) -> Self {
        Self {
            kind: GraphicKind::LowerThird,
            text: text.into(),
        }
    }

    #[must_use]
    pub fn card(text: impl Into<String>) -> Self {
        Self {
            kind: GraphicKind::Card,
            text: text.into(),
        }
    }

    #[must_use]
    pub fn shape() -> Self {
        Self {
            kind: GraphicKind::Shape,
            text: String::new(),
        }
    }

    #[must_use]
    pub fn sticker(text: impl Into<String>) -> Self {
        Self {
            kind: GraphicKind::Sticker,
            text: text.into(),
        }
    }
}

/// Per-clip mix / look. Defaults keep old project JSON valid.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct ClipLook {
    #[serde(default)]
    pub fade_in: Duration,
    #[serde(default)]
    pub fade_out: Duration,
    #[serde(default)]
    pub transition: TransitionKind,
    #[serde(default)]
    pub grade: Grade,
    #[serde(default)]
    pub fx: Fx,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graphic: Option<Graphic>,
    /// End pose. When set, scale and pan move across the clip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub move_to: Option<Transform>,
    /// End speed. With `Clip::speed`, this is a ramp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_to: Option<f32>,
    /// Fraction of the frame kept. None = full frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop: Option<Crop>,
    #[serde(default)]
    pub stabilize: bool,
    #[serde(default)]
    pub audio: AudioFx,
    /// Curves (avfilter). Empty channels stay a straight line.
    #[serde(default)]
    pub curves: Curves,
    /// Alpha Shapes mask. Outside the shape is transparent so the track below shows through.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<AlphaShape>,
    /// Time Remap keys. Empty keeps `Clip::speed` and `speed_to`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub speed_keys: Vec<SpeedKey>,
    /// Color, bars, noise, or a counter. No media file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator: Option<Generator>,
    /// Overrides `TransitionKind::mix_seconds` when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition_seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub move_ease: Option<Ease>,
}

/// Rectangle inside the frame, each edge 0–1.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Crop {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Track fader. Kdenlive's audio mixer: 0 dB is unity, pan is balance (−1 left, +1 right).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mix {
    #[serde(default)]
    pub gain_db: f32,
    #[serde(default)]
    pub pan: f32,
    /// Exclusive unless the caller adds to the existing solos (Shift+click in the mixer).
    #[serde(default)]
    pub solo: bool,
}

impl Default for Mix {
    fn default() -> Self {
        Self {
            gain_db: 0.0,
            pan: 0.0,
            solo: false,
        }
    }
}

impl Mix {
    #[must_use]
    pub fn linear(self) -> f32 {
        10f32.powf(self.gain_db.clamp(-60.0, 12.0) / 20.0)
    }

    /// Constant-power balance. `(left, right)`.
    #[must_use]
    pub fn balance(self) -> (f32, f32) {
        let p = self.pan.clamp(-1.0, 1.0);
        let angle = (p + 1.0) * 0.5 * std::f32::consts::FRAC_PI_2;
        (angle.cos(), angle.sin())
    }
}

/// One point on a Curves (avfilter) graph. `x` is input luma, `y` is output, both 0–1.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CurvePoint {
    pub x: f32,
    pub y: f32,
}

/// Kdenlive Curves (avfilter): All, R, G, B. Empty channel = straight line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Curves {
    #[serde(default)]
    pub all: Vec<CurvePoint>,
    #[serde(default)]
    pub red: Vec<CurvePoint>,
    #[serde(default)]
    pub green: Vec<CurvePoint>,
    #[serde(default)]
    pub blue: Vec<CurvePoint>,
}

impl Curves {
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.all.is_empty() && self.red.is_empty() && self.green.is_empty() && self.blue.is_empty()
    }
}

/// Alpha Shapes. Position and size are fractions of the frame; the shape is centered on x, y.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaskShape {
    Rectangle,
    Ellipse,
    Triangle,
    Diamond,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AlphaShape {
    pub shape: MaskShape,
    /// Center, 0–1.
    pub x: f32,
    pub y: f32,
    /// Size, 0–1 of the frame.
    pub w: f32,
    pub h: f32,
    /// Feather, 0–1.
    pub feather: f32,
    pub invert: bool,
}

/// Time Remap key. `at` is 0–1 along the clip on the timeline; `speed` is the playback rate there.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpeedKey {
    pub at: f32,
    pub speed: f32,
}

/// Project-bin generators. Kdenlive: Color Clip, Color Bars, White Noise, Counter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Generator {
    Color {
        #[serde(default = "default_color")]
        color: String,
    },
    ColorBars,
    WhiteNoise,
    Counter,
}

fn default_color() -> String {
    "#000000".into()
}

/// Loudness, noise, EQ, and compression.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AudioFx {
    #[serde(default)]
    pub normalize: bool,
    #[serde(default)]
    pub denoise: bool,
    #[serde(default)]
    pub compressor: bool,
    /// dB at ~120 Hz.
    #[serde(default)]
    pub low: f32,
    /// dB at ~1 kHz.
    #[serde(default)]
    pub mid: f32,
    /// dB at ~8 kHz.
    #[serde(default)]
    pub high: f32,
}

impl ClipLook {
    /// Linear fade gain like Shotcut fade-in / fade-out filters. 1 = full.
    #[must_use]
    pub fn fade_gain(&self, local: f64, duration: f64) -> f64 {
        let mut g = 1.0;
        let fi = self.fade_in.as_seconds();
        let fo = self.fade_out.as_seconds();
        if fi > 1e-4 && local < fi {
            g *= (local / fi).clamp(0.0, 1.0);
        }
        if fo > 1e-4 && duration - local < fo {
            g *= ((duration - local) / fo).clamp(0.0, 1.0);
        }
        g.clamp(0.0, 1.0)
    }

    /// Mix length at a join, clamped to half of each side (Kdenlive Mix / xfade).
    #[must_use]
    pub fn mix_window(&self, clip_dur: f64, next_dur: f64) -> f64 {
        let raw = self
            .transition_seconds
            .unwrap_or_else(|| self.transition.mix_seconds());
        if raw <= 1e-4 {
            return 0.0;
        }
        let half = clip_dur.min(next_dur).max(0.0) * 0.5;
        raw.min(half).min(clip_dur).max(0.0)
    }
}

/// How a move eases from the start transform to the end.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ease {
    #[default]
    Linear,
    In,
    Out,
    InOut,
}

impl Ease {
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "in" | "ease_in" => Self::In,
            "out" | "ease_out" => Self::Out,
            "in_out" | "inout" | "ease" => Self::InOut,
            _ => Self::Linear,
        }
    }
}

/// One piece the model chose. Rust places it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EditSlot {
    pub media_id: MediaId,
    pub source_in: f64,
    pub duration: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition_duration: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_scale: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ease: Option<Ease>,
    /// Set only when this shot needs a grade. Absent leaves the picture untouched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grade: Option<Grade>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fx: Option<Fx>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_in: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_out: Option<f64>,
    /// Cover the join into this slot with a silent range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<bool>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub why: String,
}

/// The saved plan. Revisions patch slots and rebuild from this.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EditPlan {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub style: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub aspect: String,
    #[serde(default)]
    pub letterbox: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub music_id: Option<MediaId>,
    /// Music fader, 1 is unity. Set it only when the bed should sit under speech.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub music_volume: Option<f32>,
    /// Write transcript captions onto the new timeline.
    #[serde(default)]
    pub captions: bool,
    /// Applied to every slot that does not set its own grade.
    #[serde(default)]
    pub grade: Grade,
    pub slots: Vec<EditSlot>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AspectRatio {
    Landscape,
    Vertical,
    Square,
    Tall,
}

impl AspectRatio {
    #[must_use]
    pub fn size(self, long_edge: u32) -> (u32, u32) {
        match self {
            Self::Landscape => (long_edge, long_edge * 9 / 16),
            Self::Vertical => (long_edge * 9 / 16, long_edge),
            Self::Square => (long_edge, long_edge),
            Self::Tall => (long_edge * 4 / 5, long_edge),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptionStyle {
    Plain,
    #[default]
    Stacked,
    SpeakerColor,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CaptionCue {
    pub start: Time,
    pub end: Time,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClipKind {
    Video {
        #[serde(default)]
        transform: Transform,
    },
    Audio {
        #[serde(default = "default_volume")]
        volume: f32,
        #[serde(default)]
        ducked: bool,
    },
    Caption {
        #[serde(default)]
        style: CaptionStyle,
        #[serde(default)]
        cues: Vec<CaptionCue>,
    },
    Graphic {
        graphic: Graphic,
    },
}

fn default_volume() -> f32 {
    1.0
}

impl ClipKind {
    #[must_use]
    pub fn track_kind(&self) -> TrackKind {
        match self {
            Self::Video { .. } | Self::Graphic { .. } => TrackKind::Video,
            Self::Audio { .. } => TrackKind::Audio,
            Self::Caption { .. } => TrackKind::Caption,
        }
    }
}

fn default_speed() -> f32 {
    1.0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    pub id: MarkerId,
    pub time: Time,
    pub name: String,
    #[serde(default)]
    pub color: u8,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: ClipId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_id: Option<MediaId>,
    pub kind: ClipKind,
    pub start: Time,
    pub duration: Duration,
    pub source_in: Time,
    #[serde(default = "default_speed")]
    pub speed: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<GroupId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_id: Option<LinkId>,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub look: ClipLook,
}

impl Clip {
    #[must_use]
    pub fn end(&self) -> Time {
        self.start + self.duration
    }

    #[must_use]
    pub fn source_out(&self) -> Time {
        self.source_in + self.duration
    }

    #[must_use]
    pub fn contains(&self, time: Time) -> bool {
        time >= self.start && time < self.end()
    }

    #[must_use]
    pub fn source_time_at(&self, timeline_time: Time) -> Option<Time> {
        if !self.contains(timeline_time) {
            return None;
        }
        let elapsed = timeline_time - self.start;
        let speed = if self.speed.is_finite() && self.speed > 0.0 {
            self.speed
        } else {
            1.0
        };
        let scaled = Duration::from_ticks((elapsed.as_ticks() as f64 * f64::from(speed)).round() as i64);
        Some(self.source_in + scaled)
    }

    #[must_use]
    pub fn source_duration(&self) -> Duration {
        let speed = if self.speed.is_finite() && self.speed > 0.0 {
            self.speed
        } else {
            1.0
        };
        Duration::from_ticks((self.duration.as_ticks() as f64 * f64::from(speed)).round() as i64)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub kind: TrackKind,
    pub name: String,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub locked: bool,
    /// Mixer strip for this track. Mute stays on `muted`.
    #[serde(default)]
    pub mix: Mix,
    #[serde(default)]
    pub clips: Vec<Clip>,
}

impl Track {
    #[must_use]
    pub fn new(kind: TrackKind, name: impl Into<String>) -> Self {
        Self {
            id: TrackId::new(),
            kind,
            name: name.into(),
            muted: false,
            hidden: false,
            locked: false,
            mix: Mix::default(),
            clips: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_id(id: TrackId, kind: TrackKind, name: impl Into<String>) -> Self {
        let mut track = Self::new(kind, name);
        track.id = id;
        track
    }

    pub fn clip(&self, id: ClipId) -> Option<&Clip> {
        self.clips.iter().find(|c| c.id == id)
    }

    pub fn clip_mut(&mut self, id: ClipId) -> Option<&mut Clip> {
        self.clips.iter_mut().find(|c| c.id == id)
    }

    pub fn clip_at(&self, time: Time) -> Option<&Clip> {
        self.clips.iter().find(|c| c.contains(time))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Timeline {
    pub frame_rate: FrameRate,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub tracks: Vec<Track>,
    #[serde(default)]
    pub markers: Vec<Marker>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mark_in: Option<Time>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mark_out: Option<Time>,
    /// Master fader. Solo and pan are unused; gain is the master volume.
    #[serde(default)]
    pub master: Mix,
    /// Black bars top and bottom. The renderer draws them on export.
    #[serde(default)]
    pub letterbox: bool,
    /// Last submit_edit plan. Revisions rebuild from this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit_plan: Option<EditPlan>,
}

impl Default for Timeline {
    fn default() -> Self {
        Self::new(FrameRate::FPS_30, 1920, 1080)
    }
}

impl Timeline {
    #[must_use]
    pub fn new(frame_rate: FrameRate, width: u32, height: u32) -> Self {
        Self {
            frame_rate,
            width,
            height,
            tracks: vec![
                Track::new(TrackKind::Video, "V1"),
                Track::new(TrackKind::Audio, "A1"),
                Track::new(TrackKind::Caption, "Captions"),
            ],
            markers: Vec::new(),
            mark_in: None,
            mark_out: None,
            master: Mix::default(),
            letterbox: false,
            edit_plan: None,
        }
    }

    #[must_use]
    pub fn duration(&self) -> Duration {
        self.tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .map(Clip::end)
            .max()
            .map(|end| end - Time::ZERO)
            .unwrap_or(Duration::ZERO)
    }

    pub fn track(&self, id: TrackId) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id)
    }

    pub fn track_mut(&mut self, id: TrackId) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| t.id == id)
    }

    pub fn locate(&self, id: ClipId) -> Option<(usize, usize)> {
        for (ti, track) in self.tracks.iter().enumerate() {
            if let Some(ci) = track.clips.iter().position(|c| c.id == id) {
                return Some((ti, ci));
            }
        }
        None
    }

    pub fn find_clip(&self, id: ClipId) -> Option<(&Track, &Clip)> {
        let (ti, ci) = self.locate(id)?;
        let track = &self.tracks[ti];
        Some((track, &track.clips[ci]))
    }

    pub fn clip_mut(&mut self, id: ClipId) -> Option<&mut Clip> {
        let (ti, ci) = self.locate(id)?;
        Some(&mut self.tracks[ti].clips[ci])
    }

    pub fn add_track(&mut self, kind: TrackKind, name: impl Into<String>) -> TrackId {
        self.push_track(Track::new(kind, name))
    }

    pub fn push_track(&mut self, track: Track) -> TrackId {
        let id = track.id;
        self.tracks.push(track);
        id
    }

    pub fn remove_track(&mut self, track_id: TrackId) -> Result<Track> {
        let pos = self
            .tracks
            .iter()
            .position(|t| t.id == track_id)
            .ok_or(TimelineError::TrackNotFound(track_id))?;
        if self.tracks[pos].locked {
            return Err(TimelineError::TrackLocked);
        }
        Ok(self.tracks.remove(pos))
    }

    pub fn add_clip(&mut self, track_id: TrackId, clip: Clip) -> Result<ClipId> {
        let track = self
            .track_mut(track_id)
            .ok_or(TimelineError::TrackNotFound(track_id))?;
        if track.locked {
            return Err(TimelineError::TrackLocked);
        }
        if track.kind != clip.kind.track_kind() {
            return Err(TimelineError::TrackKindMismatch);
        }
        let id = clip.id;
        track.clips.push(clip);
        track.clips.sort_by_key(|c| c.start);
        Ok(id)
    }

    pub fn remove_clip(&mut self, clip_id: ClipId) -> Result<Clip> {
        for track in &mut self.tracks {
            if track.locked && track.clips.iter().any(|c| c.id == clip_id) {
                return Err(TimelineError::TrackLocked);
            }
            if let Some(pos) = track.clips.iter().position(|c| c.id == clip_id) {
                return Ok(track.clips.remove(pos));
            }
        }
        Err(TimelineError::ClipNotFound(clip_id))
    }

    pub fn clip_at(&self, track_id: TrackId, at: Time) -> Option<ClipId> {
        let track = self.track(track_id)?;
        track
            .clips
            .iter()
            .find(|clip| at >= clip.start && at < clip.end())
            .map(|clip| clip.id)
    }

    pub fn clip_at_any(&self, at: Time) -> Option<ClipId> {
        self.tracks.iter().find_map(|track| self.clip_at(track.id, at))
    }

    /// Join `clip_id` with the next clip on the same track if they touch and
    /// share media (the inverse of a razor).
    pub fn merge_with_next(&mut self, clip_id: ClipId) -> Result<ClipId> {
        let frame = self.frame_rate;
        let (ti, ci) = self
            .locate(clip_id)
            .ok_or(TimelineError::ClipNotFound(clip_id))?;
        if self.tracks[ti].locked {
            return Err(TimelineError::TrackLocked);
        }
        if ci + 1 >= self.tracks[ti].clips.len() {
            return Err(TimelineError::CannotMerge);
        }
        let left = &self.tracks[ti].clips[ci];
        let right = &self.tracks[ti].clips[ci + 1];
        let slack = Duration::from_seconds(1.0 / frame.as_f64().max(1.0));
        if right.start > left.end() + slack {
            return Err(TimelineError::CannotMerge);
        }
        if left.media_id != right.media_id {
            return Err(TimelineError::CannotMerge);
        }
        let expected_src = left.source_in + left.duration;
        if (right.source_in - expected_src).as_ticks().abs() > slack.as_ticks() {
            return Err(TimelineError::CannotMerge);
        }
        let new_duration = right.end() - left.start;
        self.tracks[ti].clips[ci].duration = new_duration;
        self.tracks[ti].clips.remove(ci + 1);
        Ok(clip_id)
    }

    /// Split `clip_id` at a timeline time. Returns the new right-hand clip.
    pub fn split(&mut self, clip_id: ClipId, at: Time) -> Result<ClipId> {
        let at = at.snap_to_frame(self.frame_rate);
        let (track_id, left_end, new_clip) = {
            let (track, clip) = self
                .find_clip(clip_id)
                .ok_or(TimelineError::ClipNotFound(clip_id))?;
            if track.locked {
                return Err(TimelineError::TrackLocked);
            }
            if at <= clip.start || at >= clip.end() {
                return Err(TimelineError::SplitOutOfRange);
            }
            let offset = at - clip.start;
            let mut right = clip.clone();
            right.id = ClipId::new();
            right.start = at;
            right.duration = clip.duration - offset;
            right.source_in = clip.source_in + offset;
            // Inner cut is a hard cut. Outgoing mix / fade-out stay on the right.
            right.look.fade_in = Duration::ZERO;
            (track.id, offset, right)
        };
        let right_id = new_clip.id;
        if let Some(clip) = self.clip_mut(clip_id) {
            clip.duration = left_end;
            clip.look.fade_out = Duration::ZERO;
            clip.look.transition = TransitionKind::Cut;
        }
        self.add_clip(track_id, new_clip)?;
        Ok(right_id)
    }

    pub fn trim(&mut self, clip_id: ClipId, new_start: Time, new_duration: Duration) -> Result<()> {
        let rate = self.frame_rate;
        let new_start = new_start.snap_to_frame(rate);
        let new_duration = Duration::from_ticks(
            Time::from_ticks(new_duration.as_ticks())
                .snap_to_frame(rate)
                .as_ticks(),
        );
        if new_duration.as_ticks() <= 0 {
            return Err(TimelineError::EmptyTrim);
        }
        let (ti, _) = self
            .locate(clip_id)
            .ok_or(TimelineError::ClipNotFound(clip_id))?;
        if self.tracks[ti].locked {
            return Err(TimelineError::TrackLocked);
        }
        let clip = self
            .clip_mut(clip_id)
            .ok_or(TimelineError::ClipNotFound(clip_id))?;
        let delta = new_start - clip.start;
        clip.source_in += delta;
        clip.start = new_start;
        clip.duration = new_duration;
        Ok(())
    }

    pub fn move_clip(&mut self, clip_id: ClipId, track_id: TrackId, new_start: Time) -> Result<()> {
        let new_start = new_start.snap_to_frame(self.frame_rate);
        let dest_ok = {
            let dest = self
                .track(track_id)
                .ok_or(TimelineError::TrackNotFound(track_id))?;
            if dest.locked {
                return Err(TimelineError::TrackLocked);
            }
            dest.kind
        };
        let (origin_id, origin_kind) = {
            let (track, _) = self
                .find_clip(clip_id)
                .ok_or(TimelineError::ClipNotFound(clip_id))?;
            (track.id, track.kind)
        };
        if dest_ok != origin_kind {
            return Err(TimelineError::TrackKindMismatch);
        }
        let mut clip = self.remove_clip(clip_id)?;
        let backup = clip.clone();
        clip.start = new_start;
        if let Err(err) = self.add_clip(track_id, clip) {
            let _ = self.add_clip(origin_id, backup);
            return Err(err);
        }
        Ok(())
    }

    /// Delete the clip and pull later clips on the same track left by its duration.
    pub fn ripple_delete(&mut self, clip_id: ClipId) -> Result<Clip> {
        let (track_id, start, duration) = {
            let (track, clip) = self
                .find_clip(clip_id)
                .ok_or(TimelineError::ClipNotFound(clip_id))?;
            if track.locked {
                return Err(TimelineError::TrackLocked);
            }
            (track.id, clip.start, clip.duration)
        };
        let removed = self.remove_clip(clip_id)?;
        let threshold = start + duration;
        if let Some(track) = self.track_mut(track_id) {
            for clip in &mut track.clips {
                if clip.start >= threshold {
                    clip.start -= duration;
                }
            }
        }
        Ok(removed)
    }

    pub fn set_aspect(&mut self, aspect: AspectRatio) {
        let long = self.width.max(self.height);
        let (w, h) = aspect.size(long);
        self.width = w.max(2);
        self.height = h.max(2);
    }

    pub fn first_track(&self, kind: TrackKind) -> Option<&Track> {
        self.tracks.iter().find(|t| t.kind == kind)
    }

    pub fn first_track_mut(&mut self, kind: TrackKind) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| t.kind == kind)
    }

    pub fn replace_caption_cues(
        &mut self,
        style: CaptionStyle,
        cues: Vec<CaptionCue>,
    ) -> Result<ClipId> {
        let duration = cues
            .iter()
            .map(|c| c.end)
            .max()
            .map(|end| end - Time::ZERO)
            .unwrap_or(Duration::ZERO);
        let track_id = match self.first_track(TrackKind::Caption) {
            Some(t) => t.id,
            None => self.add_track(TrackKind::Caption, "Captions"),
        };
        if let Some(track) = self.track_mut(track_id) {
            track.clips.clear();
        }
        let clip = Clip {
            id: ClipId::new(),
            media_id: None,
            kind: ClipKind::Caption { style, cues },
            start: Time::ZERO,
            duration,
            source_in: Time::ZERO,
            speed: 1.0,
            group_id: None,
            link_id: None,
            disabled: false,
            look: ClipLook::default(),
        };
        self.add_clip(track_id, clip)
    }
}
