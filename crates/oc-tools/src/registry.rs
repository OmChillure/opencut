use crate::{ToolGroup, ToolId, ToolKind, ToolSpec};

const ALL: &[ToolSpec] = &[
    ToolSpec {
        id: ToolId::Select,
        label: "Select",
        tip: "Select and move clips (S)",
        shortcut: Some("S"),
        kind: ToolKind::Mode,
        group: ToolGroup::Modes,
    },
    ToolSpec {
        id: ToolId::Razor,
        label: "Razor",
        tip: "Click a clip to cut it (X)",
        shortcut: Some("X"),
        kind: ToolKind::Mode,
        group: ToolGroup::Modes,
    },
    ToolSpec {
        id: ToolId::Spacer,
        label: "Spacer",
        tip: "Drag to open or close space after a point (M)",
        shortcut: Some("M"),
        kind: ToolKind::Mode,
        group: ToolGroup::Modes,
    },
    ToolSpec {
        id: ToolId::Slip,
        label: "Slip",
        tip: "Slide the source inside a clip without moving it (Y)",
        shortcut: Some("Y"),
        kind: ToolKind::Mode,
        group: ToolGroup::Modes,
    },
    ToolSpec {
        id: ToolId::Ripple,
        label: "Ripple",
        tip: "Trim an edge and shift later clips on the track",
        shortcut: Some("B"),
        kind: ToolKind::Mode,
        group: ToolGroup::Modes,
    },
    ToolSpec {
        id: ToolId::Roll,
        label: "Roll",
        tip: "Move the cut between two touching clips (N)",
        shortcut: Some("N"),
        kind: ToolKind::Mode,
        group: ToolGroup::Modes,
    },
    ToolSpec {
        id: ToolId::Slide,
        label: "Slide",
        tip: "Move a clip; neighbors absorb the time (U)",
        shortcut: Some("U"),
        kind: ToolKind::Mode,
        group: ToolGroup::Modes,
    },
    ToolSpec {
        id: ToolId::RateStretch,
        label: "Stretch",
        tip: "Drag duration to change speed (R)",
        shortcut: Some("R"),
        kind: ToolKind::Mode,
        group: ToolGroup::Modes,
    },
    ToolSpec {
        id: ToolId::Multicam,
        label: "Multicam",
        tip: "While playing, click a track to cut to that camera",
        shortcut: None,
        kind: ToolKind::Mode,
        group: ToolGroup::Modes,
    },
    ToolSpec {
        id: ToolId::Split,
        label: "Split",
        tip: "Cut the clip at the playhead (Shift+R)",
        shortcut: Some("Shift+R"),
        kind: ToolKind::Action,
        group: ToolGroup::Cut,
    },
    ToolSpec {
        id: ToolId::SplitAll,
        label: "Split all",
        tip: "Cut every track at the playhead (Shift+S)",
        shortcut: Some("Shift+S"),
        kind: ToolKind::Action,
        group: ToolGroup::Cut,
    },
    ToolSpec {
        id: ToolId::Merge,
        label: "Merge",
        tip: "Join this clip with the next (undo a split)",
        shortcut: None,
        kind: ToolKind::Action,
        group: ToolGroup::Cut,
    },
    ToolSpec {
        id: ToolId::Extract,
        label: "Cut",
        tip: "Delete the clip and close the gap",
        shortcut: Some("Shift+Del"),
        kind: ToolKind::Action,
        group: ToolGroup::Cut,
    },
    ToolSpec {
        id: ToolId::Lift,
        label: "Delete",
        tip: "Delete the clip and leave a hole",
        shortcut: Some("Del"),
        kind: ToolKind::Action,
        group: ToolGroup::Cut,
    },
    ToolSpec {
        id: ToolId::TrimStart,
        label: "Trim",
        tip: "Trim the clip start to the playhead",
        shortcut: Some("("),
        kind: ToolKind::Action,
        group: ToolGroup::Cut,
    },
    ToolSpec {
        id: ToolId::TrimEnd,
        label: "Trim end",
        tip: "Trim the clip end to the playhead",
        shortcut: Some(")"),
        kind: ToolKind::Action,
        group: ToolGroup::Cut,
    },
    ToolSpec {
        id: ToolId::MarkIn,
        label: "Mark in",
        tip: "Set the In point at the playhead (I)",
        shortcut: Some("I"),
        kind: ToolKind::Action,
        group: ToolGroup::Cut,
    },
    ToolSpec {
        id: ToolId::MarkOut,
        label: "Mark out",
        tip: "Set the Out point at the playhead (O)",
        shortcut: Some("O"),
        kind: ToolKind::Action,
        group: ToolGroup::Cut,
    },
    ToolSpec {
        id: ToolId::InsertAt,
        label: "Insert",
        tip: "Insert the marked source at the playhead (V)",
        shortcut: Some("V"),
        kind: ToolKind::Action,
        group: ToolGroup::Cut,
    },
    ToolSpec {
        id: ToolId::OverwriteAt,
        label: "Overwrite",
        tip: "Overwrite the marked source at the playhead (B)",
        shortcut: None,
        kind: ToolKind::Action,
        group: ToolGroup::Cut,
    },
    ToolSpec {
        id: ToolId::InsertSpace,
        label: "Insert space",
        tip: "Open a 1s gap at the playhead",
        shortcut: None,
        kind: ToolKind::Action,
        group: ToolGroup::Cut,
    },
    ToolSpec {
        id: ToolId::DeleteSpace,
        label: "Close gap",
        tip: "Close the next gap on the target track",
        shortcut: None,
        kind: ToolKind::Action,
        group: ToolGroup::Cut,
    },
    ToolSpec {
        id: ToolId::DetachAudio,
        label: "Detach audio",
        tip: "Sound moves to the audio track",
        shortcut: Some("D"),
        kind: ToolKind::Action,
        group: ToolGroup::Cut,
    },
    ToolSpec {
        id: ToolId::Group,
        label: "Group",
        tip: "Group selected clips (G)",
        shortcut: Some("G"),
        kind: ToolKind::Action,
        group: ToolGroup::Cut,
    },
    ToolSpec {
        id: ToolId::Ungroup,
        label: "Ungroup",
        tip: "Ungroup clips (Shift+G)",
        shortcut: Some("Shift+G"),
        kind: ToolKind::Action,
        group: ToolGroup::Cut,
    },
    ToolSpec {
        id: ToolId::Link,
        label: "Link",
        tip: "Link video and audio clips",
        shortcut: None,
        kind: ToolKind::Action,
        group: ToolGroup::Cut,
    },
    ToolSpec {
        id: ToolId::Unlink,
        label: "Unlink",
        tip: "Unlink this clip from its pair",
        shortcut: None,
        kind: ToolKind::Action,
        group: ToolGroup::Cut,
    },
    ToolSpec {
        id: ToolId::AddMarker,
        label: "Marker",
        tip: "Add a marker at the playhead (M is spacer — use this button)",
        shortcut: None,
        kind: ToolKind::Action,
        group: ToolGroup::Cut,
    },
    ToolSpec {
        id: ToolId::AddVideo,
        label: "Video track",
        tip: "Add a video track",
        shortcut: None,
        kind: ToolKind::Action,
        group: ToolGroup::Tracks,
    },
    ToolSpec {
        id: ToolId::AddAudio,
        label: "Audio track",
        tip: "Add an audio track",
        shortcut: None,
        kind: ToolKind::Action,
        group: ToolGroup::Tracks,
    },
    ToolSpec {
        id: ToolId::AddCaption,
        label: "Caption track",
        tip: "Add a caption track",
        shortcut: None,
        kind: ToolKind::Action,
        group: ToolGroup::Tracks,
    },
];

#[must_use]
pub fn tools() -> &'static [ToolSpec] {
    ALL
}

#[must_use]
pub fn tool(id: ToolId) -> Option<&'static ToolSpec> {
    ALL.iter().find(|spec| spec.id == id)
}

#[must_use]
pub fn modes() -> impl Iterator<Item = &'static ToolSpec> {
    ALL.iter().filter(|spec| spec.group == ToolGroup::Modes)
}

#[must_use]
pub fn actions() -> impl Iterator<Item = &'static ToolSpec> {
    ALL.iter().filter(|spec| spec.group == ToolGroup::Cut)
}

#[must_use]
pub fn track_actions() -> impl Iterator<Item = &'static ToolSpec> {
    ALL.iter().filter(|spec| spec.group == ToolGroup::Tracks)
}
