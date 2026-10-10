//! Timeline tools: UI catalog, ops the engine applies, MCP schemas for providers.

mod finish;
mod intent;
mod mcp;
mod ops;
mod plan;
mod registry;
mod review;
mod watch;

pub use finish::{
    CoverShot, SpokenLine, caption_recipe_for, has_burnable_captions, mapped_cues, program_clips,
    redress_unset_captions,
};
pub use intent::{Intent, asks_for_whole_piece, is_director_request, parse_intent};
pub use mcp::{
    Inspect, McpCall, McpTool, call_changes_timeline, inspect_from_mcp, mcp_tools, op_from_mcp,
    tools_for_request,
};
pub use oc_timeline::PlaceMode;
pub use ops::{
    AppliedOp, AssembleItem, AssembleStyle, DesignLayout, Excerpt, ExportPreset, Op, OpError,
    PiecePick, SourceBeat, TimeRange, TimelineEditMode, apply, asks_for_judgment, choose_piece,
    excerpts_for_request, pick_reel_excerpts, revises_existing_cut,
};
pub use plan::{SourceWindow, build_plan, cue_faces, plan_from_value, revise_plan};
pub use registry::{actions, modes, tool, tools, track_actions};
pub use review::{
    CutReview, ReviewFacts, ShotNote, SourceSpan, Spoken, follow_after_text, follow_after_tools,
    review_cut, review_with,
};
pub use watch::{GapKind, SILENCE_GAP_SECS, SourceGap, gaps_overlapping, source_gaps, watch_times};

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

impl Default for ToolId {
    fn default() -> Self {
        Self::Select
    }
}
