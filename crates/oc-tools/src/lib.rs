//! Timeline tools: UI catalog, ops the engine applies, MCP schemas for providers.

mod finish;
mod intent;
mod mcp;
mod ops;
mod plan;
mod registry;
mod review;

pub use intent::{asks_for_whole_piece, Intent, is_director_request, parse_intent};
pub use mcp::{inspect_from_mcp, Inspect, McpCall, McpTool, mcp_tools, op_from_mcp};
pub use plan::{build_plan, plan_from_value, revise_plan, SourceWindow};
pub use ops::{
    AppliedOp, AssembleItem, AssembleStyle, DesignLayout, Excerpt, ExportPreset, Op, OpError,
    TimeRange,
    TimelineEditMode, apply, asks_for_judgment, choose_piece, excerpts_for_request,
    pick_reel_excerpts, revises_existing_cut, PiecePick, SourceBeat,
};
pub use oc_timeline::PlaceMode;
pub use registry::{actions, modes, tool, tools, track_actions};
pub use finish::{
    CoverShot, SpokenLine, already_finished, finish_reel, has_burnable_captions, mapped_cues,
    program_clips, wants_picture_finish,
};
pub use review::{review_cut, review_with, CutReview, ReviewFacts, ShotNote, SourceSpan, Spoken};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolId {
    Select,
    Razor,
    Spacer,
    Slip,
    Ripple,
    Roll,
    Slide,
    RateStretch,
    Multicam,
    Split,
    SplitAll,
    Merge,
    Extract,
    Lift,
    TrimStart,
    TrimEnd,
    MarkIn,
    MarkOut,
    InsertAt,
    OverwriteAt,
    InsertSpace,
    DeleteSpace,
    DetachAudio,
    Group,
    Ungroup,
    Link,
    Unlink,
    AddMarker,
    AddVideo,
    AddAudio,
    AddCaption,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind {
    /// Stays selected (Select, Razor, Spacer, Slip, Ripple, Roll, Slide, …).
    Mode,
    /// Runs once at the playhead (Split, Merge, …).
    Action,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolGroup {
    Modes,
    Cut,
    Tracks,
}

#[derive(Clone, Copy, Debug)]
pub struct ToolSpec {
    pub id: ToolId,
    pub label: &'static str,
    pub tip: &'static str,
    pub shortcut: Option<&'static str>,
    pub kind: ToolKind,
    pub group: ToolGroup,
}

impl ToolId {
    #[must_use]
    pub fn is_mode(self) -> bool {
        tool(self).is_some_and(|t| t.kind == ToolKind::Mode)
    }
}

impl Default for ToolId {
    fn default() -> Self {
        Self::Select
    }
}
