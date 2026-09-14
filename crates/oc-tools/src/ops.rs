use oc_time::{Duration, Time};
use oc_timeline::{
    AspectRatio, CaptionCue, CaptionStyle, Clip, ClipId, ClipKind, MarkerId, MediaId, PlaceMode,
    Timeline, TimelineError, Track, TrackId, TrackKind, Transform, UndoStack,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TimelineEditMode {
    #[default]
    Normal,
    Insert,
    Overwrite,
}

impl TimelineEditMode {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Insert => "Insert",
            Self::Overwrite => "Overwrite",
        }
    }

    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Insert => "insert",
            Self::Overwrite => "overwrite",
        }
    }

    #[must_use]
    pub fn from_key(key: &str) -> Self {
        match key {
            "insert" => Self::Insert,
            "overwrite" => Self::Overwrite,
            _ => Self::Normal,
        }
    }

    pub const ALL: [Self; 3] = [Self::Normal, Self::Insert, Self::Overwrite];

    #[must_use]
    pub fn as_place(self) -> PlaceMode {
        match self {
            Self::Normal => PlaceMode::Normal,
            Self::Insert => PlaceMode::Insert,
            Self::Overwrite => PlaceMode::Overwrite,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OpError {
    #[error(transparent)]
    Timeline(#[from] TimelineError),
    #[error("no clip on the selected track at that time")]
    NoClipAtTime,
    #[error("{0}")]
    Message(String),
}

pub type Result<T> = std::result::Result<T, OpError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportPreset {
    Youtube1080,
    Vertical1080,
    Square1080,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    Split {
        clip_id: ClipId,
        at: Time,
    },
    Trim {
        clip_id: ClipId,
        start: Time,
        duration: Duration,
    },
    Move {
        clip_id: ClipId,
        track_id: TrackId,
        start: Time,
    },
    RippleDelete {
        clip_id: ClipId,
    },
    AddCaptions {
        style: CaptionStyle,
        cues: Vec<CaptionCue>,
    },
    RemoveSilence {
        ranges: Vec<TimeRange>,
    },
    Reframe {
        aspect: AspectRatio,
    },
    Duck {
        amount: f32,
    },
    Export {
        preset: ExportPreset,
    },
    AddTrack {
        id: TrackId,
        kind: TrackKind,
        name: String,
    },
    RemoveTrack {
        track_id: TrackId,
    },
    AddClip {
        track_id: TrackId,
        clip: Clip,
    },
    RemoveClip {
        clip_id: ClipId,
    },
    SetTrackFlags {
        track_id: TrackId,
        muted: bool,
        hidden: bool,
    },
    SetTimeline {
        timeline: Timeline,
    },
    Merge {
        clip_id: ClipId,
    },
    Slip {
        clip_id: ClipId,
        source_in: Time,
    },
    Roll {
        clip_id: ClipId,
        at: Time,
    },
    Slide {
        clip_id: ClipId,
        start: Time,
    },
    RippleTrim {
        clip_id: ClipId,
        start: Time,
        duration: Duration,
    },
    RateStretch {
        clip_id: ClipId,
        duration: Duration,
    },
    SetSpeed {
        clip_id: ClipId,
        speed: f32,
    },
    SplitAll {
        at: Time,
    },
    MulticamCut {
        track_id: TrackId,
        at: Time,
    },
    InsertSpace {
        track_id: Option<TrackId>,
        at: Time,
        amount: Duration,
    },
    DeleteSpace {
        track_id: Option<TrackId>,
        at: Time,
    },
    PlaceClip {
        track_id: TrackId,
        clip: Clip,
        mode: TimelineEditMode,
    },
    Group {
        clip_ids: Vec<ClipId>,
    },
    Ungroup {
        clip_id: ClipId,
    },
    Link {
        clip_ids: Vec<ClipId>,
    },
    Unlink {
        clip_id: ClipId,
    },
    DetachAudio {
        clip_id: ClipId,
    },
    SetDisabled {
        clip_id: ClipId,
        disabled: bool,
    },
    AddMarker {
        time: Time,
        name: String,
        color: u8,
    },
    RemoveMarker {
        marker_id: MarkerId,
    },
    SetMarkIn {
        time: Option<Time>,
    },
    SetMarkOut {
        time: Option<Time>,
    },
    PlaceMedia {
        media_id: MediaId,
        track_id: Option<TrackId>,
        start: Time,
        duration: Duration,
        kind: TrackKind,
        mode: TimelineEditMode,
    },
    Assemble {
        items: Vec<AssembleItem>,
        style: AssembleStyle,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AssembleStyle {
    Sequential,
    #[default]
    Vlog,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AssembleItem {
    pub media_id: MediaId,
    pub duration: Duration,
    pub kind: TrackKind,
    #[serde(default)]
    pub name: String,
    /// Spoken word count from STT. 0 = not heard / no speech.
    #[serde(default)]
    pub words: u32,
    /// Seconds covered by transcript cues.
    #[serde(default)]
    pub speech_seconds: f64,
    /// Best in-point from the first real sentence (seconds into source).
    #[serde(default)]
    pub hook_in: Time,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub still: bool,
    /// Local look label: wide, close, dark, action, graphic, interior…
    #[serde(default)]
    pub look: String,
    #[serde(default)]
    pub motion: f32,
    #[serde(default)]
    pub scenes: u32,
}

impl Default for AssembleItem {
    fn default() -> Self {
        Self {
            media_id: MediaId::new(),
            duration: Duration::ZERO,
            kind: TrackKind::Video,
            name: String::new(),
            words: 0,
            speech_seconds: 0.0,
            hook_in: Time::ZERO,
            text: String::new(),
            still: false,
            look: String::new(),
            motion: 0.0,
            scenes: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeRange {
    pub start: Time,
    pub end: Time,
}

impl TimeRange {
    #[must_use]
    pub fn duration(self) -> Duration {
        self.end - self.start
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppliedOp {
    pub op: Op,
    pub note: String,
}

pub fn apply(timeline: &mut Timeline, undo: &mut UndoStack, op: Op) -> Result<AppliedOp> {
    undo.checkpoint(timeline.clone());
    let note = match &op {
        Op::Split { clip_id, at } => {
            let right = timeline.split(*clip_id, *at)?;
            format!("split clip {clip_id} at {}; new clip {right}", at.as_seconds())
        }
        Op::Trim {
            clip_id,
            start,
            duration,
        } => {
            timeline.trim(*clip_id, *start, *duration)?;
            format!("trim {clip_id}")
        }
        Op::Move {
            clip_id,
            track_id,
            start,
        } => {
            timeline.move_clip(*clip_id, *track_id, *start)?;
            format!("move {clip_id}")
        }
        Op::RippleDelete { clip_id } => {
            timeline.ripple_delete(*clip_id)?;
            format!("ripple delete {clip_id}")
        }
        Op::AddCaptions { style, cues } => {
            let id = timeline.replace_caption_cues(*style, cues.clone())?;
            format!("added {} caption cues as {id}", cues.len())
        }
        Op::RemoveSilence { ranges } => {
            let n = cut_ranges(timeline, ranges)?;
            format!("removed {n} silence ranges")
        }
        Op::Reframe { aspect } => {
            timeline.set_aspect(*aspect);
            format!("reframe to {aspect:?}")
        }
        Op::Duck { amount } => {
            duck_audio(timeline, *amount);
            format!("duck music by {amount:.0}%")
        }
        Op::Export { preset } => format!("queued export {preset:?}"),
        Op::AddTrack { id, kind, name } => {
            if timeline.track(*id).is_some() {
                return Err(OpError::Message(format!("track {id} already exists")));
            }
            timeline.push_track(Track::with_id(*id, *kind, name.clone()));
            format!("add track {name}")
        }
        Op::RemoveTrack { track_id } => {
            let removed = timeline.remove_track(*track_id)?;
            format!("remove track {}", removed.name)
        }
        Op::AddClip { track_id, clip } => {
            let id = timeline.add_clip(*track_id, clip.clone())?;
            format!("add clip {id}")
        }
        Op::RemoveClip { clip_id } => {
            timeline.remove_clip(*clip_id)?;
            format!("remove clip {clip_id}")
        }
        Op::SetTrackFlags {
            track_id,
            muted,
            hidden,
        } => {
            let track = timeline
                .track_mut(*track_id)
                .ok_or(TimelineError::TrackNotFound(*track_id))?;
            track.muted = *muted;
            track.hidden = *hidden;
            format!("flags on {track_id}")
        }
        Op::SetTimeline { timeline: next } => {
            *timeline = next.clone();
            "updated timeline".into()
        }
        Op::Merge { clip_id } => {
            timeline.merge_with_next(*clip_id)?;
            format!("merge {clip_id} with next")
        }
        Op::Slip { clip_id, source_in } => {
            timeline.slip(*clip_id, *source_in)?;
            format!("slip {clip_id}")
        }
        Op::Roll { clip_id, at } => {
            timeline.roll(*clip_id, *at)?;
            format!("roll {clip_id} to {}", at.as_seconds())
        }
        Op::Slide { clip_id, start } => {
            timeline.slide(*clip_id, *start)?;
            format!("slide {clip_id}")
        }
        Op::RippleTrim {
            clip_id,
            start,
            duration,
        } => {
            timeline.ripple_trim(*clip_id, *start, *duration)?;
            format!("ripple trim {clip_id}")
        }
        Op::RateStretch { clip_id, duration } => {
            timeline.rate_stretch(*clip_id, *duration)?;
            format!("rate stretch {clip_id}")
        }
        Op::SetSpeed { clip_id, speed } => {
            timeline.set_speed(*clip_id, *speed)?;
            format!("speed {clip_id} x{speed:.2}")
        }
        Op::SplitAll { at } => {
            let created = timeline.split_all(*at)?;
            format!("split {} clips at {}", created.len(), at.as_seconds())
        }
        Op::MulticamCut { track_id, at } => {
            timeline.multicam_cut(*track_id, *at)?;
            format!("multicam cut to {track_id} at {}", at.as_seconds())
        }
        Op::InsertSpace {
            track_id,
            at,
            amount,
        } => {
            timeline.insert_space(*track_id, *at, *amount)?;
            format!("insert {:.2}s space", amount.as_seconds())
        }
        Op::DeleteSpace { track_id, at } => {
            let closed = timeline.delete_space(*track_id, *at)?;
            format!("closed {:.2}s gap", closed.as_seconds())
        }
        Op::PlaceClip {
            track_id,
            clip,
            mode,
        } => {
            let id = timeline.place_clip(*track_id, clip.clone(), mode.as_place())?;
            format!("place {id} ({})", mode.label())
        }
        Op::Group { clip_ids } => {
            let id = timeline.group_clips(clip_ids)?;
            format!("grouped {} clips as {id}", clip_ids.len())
        }
        Op::Ungroup { clip_id } => {
            timeline.ungroup(*clip_id)?;
            format!("ungroup {clip_id}")
        }
        Op::Link { clip_ids } => {
            let id = timeline.link_clips(clip_ids)?;
            format!("linked {} clips as {id}", clip_ids.len())
        }
        Op::Unlink { clip_id } => {
            timeline.unlink(*clip_id)?;
            format!("unlink {clip_id}")
        }
        Op::DetachAudio { clip_id } => {
            let id = timeline.detach_audio(*clip_id)?;
            format!("detach audio {id} from {clip_id}")
        }
        Op::SetDisabled { clip_id, disabled } => {
            timeline.set_disabled(*clip_id, *disabled)?;
            format!("{} {clip_id}", if *disabled { "disable" } else { "enable" })
        }
        Op::AddMarker { time, name, color } => {
            let id = timeline.add_marker(*time, name.clone(), *color);
            format!("marker {id} at {}", time.as_seconds())
        }
        Op::RemoveMarker { marker_id } => {
            if !timeline.remove_marker(*marker_id) {
                return Err(OpError::Message(format!("marker {marker_id} not found")));
            }
            format!("removed marker {marker_id}")
        }
        Op::SetMarkIn { time } => {
            timeline.set_mark_in(*time);
            "set mark in".into()
        }
        Op::SetMarkOut { time } => {
            timeline.set_mark_out(*time);
            "set mark out".into()
        }
        Op::PlaceMedia {
            media_id,
            track_id,
            start,
            duration,
            kind,
            mode,
        } => {
            let track_id = resolve_track(timeline, *track_id, *kind);
            let clip = clip_for_media(*media_id, *start, *duration, *kind);
            let id = timeline.place_clip(track_id, clip, mode.as_place())?;
            format!("placed {id} from {media_id} at {:.2}s", start.as_seconds())
        }
        Op::Assemble { items, style } => {
            let n = assemble(timeline, items, *style)?;
            format!("assembled {n} clips ({style:?})")
        }
    };
    Ok(AppliedOp { op, note })
}

fn clip_for_media(media_id: MediaId, start: Time, duration: Duration, kind: TrackKind) -> Clip {
    let duration = if duration.as_ticks() <= 0 {
        Duration::from_seconds(match kind {
            TrackKind::Caption => 3.0,
            TrackKind::Audio => 8.0,
            TrackKind::Video => 5.0,
        })
    } else {
        duration
    };
    Clip {
        id: ClipId::new(),
        media_id: Some(media_id),
        kind: match kind {
            TrackKind::Video => ClipKind::Video {
                transform: Transform::default(),
            },
            TrackKind::Audio => ClipKind::Audio {
                volume: 1.0,
                ducked: false,
            },
            TrackKind::Caption => ClipKind::Caption {
                style: CaptionStyle::default(),
                cues: Vec::new(),
            },
        },
        start,
        duration,
        source_in: Time::ZERO,
        speed: 1.0,
        group_id: None,
        link_id: None,
        disabled: false,
    }
}

fn resolve_track(timeline: &mut Timeline, track_id: Option<TrackId>, kind: TrackKind) -> TrackId {
    if let Some(id) = track_id {
        if timeline.track(id).is_some() {
            return id;
        }
    }
    if let Some(track) = timeline.first_track(kind) {
        return track.id;
    }
    let name = match kind {
        TrackKind::Video => "V1",
        TrackKind::Audio => "A1",
        TrackKind::Caption => "Captions",
    };
    timeline.add_track(kind, name)
}

fn clear_kind(timeline: &mut Timeline, kind: TrackKind) {
    let Some(id) = timeline.first_track(kind).map(|t| t.id) else {
        return;
    };
    let ids: Vec<ClipId> = timeline
        .track(id)
        .map(|t| t.clips.iter().map(|c| c.id).collect())
        .unwrap_or_default();
    for id in ids {
        let _ = timeline.remove_clip(id);
    }
}

fn assemble(
    timeline: &mut Timeline,
    items: &[AssembleItem],
    style: AssembleStyle,
) -> Result<usize> {
    if items.is_empty() {
        return Err(OpError::Message("no media to assemble".into()));
    }
    match style {
        AssembleStyle::Sequential => assemble_linear(timeline, items),
        AssembleStyle::Vlog => assemble_short(timeline, items),
    }
}

fn assemble_linear(timeline: &mut Timeline, items: &[AssembleItem]) -> Result<usize> {
    clear_kind(timeline, TrackKind::Video);
    clear_kind(timeline, TrackKind::Audio);
    let video_track = resolve_track(timeline, None, TrackKind::Video);
    let audio_track = resolve_track(timeline, None, TrackKind::Audio);
    let mut t_video = Time::ZERO;
    let mut t_audio = Time::ZERO;
    let mut n = 0;
    for item in items {
        if role_of(item) == Role::Music {
            let clip = clip_for_media(item.media_id, t_audio, item.duration, TrackKind::Audio);
            let dur = clip.duration;
            timeline.place_clip(audio_track, clip, PlaceMode::Normal)?;
            t_audio += dur;
            n += 1;
            continue;
        }
        let clip = clip_for_media(item.media_id, t_video, item.duration, TrackKind::Video);
        let dur = clip.duration;
        timeline.place_clip(video_track, clip, PlaceMode::Normal)?;
        t_video += dur;
        n += 1;
    }
    Ok(n)
}

/// Cut a 30s–60s short: hook + A-roll spine, B-roll on V2, music ducked under.
fn assemble_short(timeline: &mut Timeline, items: &[AssembleItem]) -> Result<usize> {
    let mut a_roll = Vec::new();
    let mut b_roll = Vec::new();
    let mut music = Vec::new();
    for item in items {
        match role_of(item) {
            Role::Music => music.push(item),
            Role::BRoll | Role::Still => b_roll.push(item),
            Role::ARoll => a_roll.push(item),
        }
    }
    if a_roll.is_empty() {
        a_roll = b_roll.clone();
        b_roll.clear();
    }
    if a_roll.is_empty() && music.is_empty() {
        return Err(OpError::Message("no media to assemble".into()));
    }
    sort_aroll(&mut a_roll);
    sort_broll(&mut b_roll);

    clear_kind(timeline, TrackKind::Video);
    clear_kind(timeline, TrackKind::Audio);
    let v1 = resolve_track(timeline, None, TrackKind::Video);
    let v2 = {
        if let Some(t) = timeline
            .tracks
            .iter()
            .find(|t| t.kind == TrackKind::Video && t.id != v1)
        {
            t.id
        } else {
            timeline.add_track(TrackKind::Video, "V2")
        }
    };
    let a1 = resolve_track(timeline, None, TrackKind::Audio);

    let n_a = a_roll.len().max(1) as f64;
    let target = (n_a * 5.0).clamp(30.0, 60.0);
    let per = (target / n_a).clamp(2.8, 10.0);
    let mut cursor = Time::ZERO;
    let mut n = 0;
    for (i, item) in a_roll.iter().enumerate() {
        let src = item.duration.as_seconds().max(0.4);
        let take = if i == 0 {
            per.min(4.0).min(src)
        } else if i + 1 == a_roll.len() && a_roll.len() > 1 {
            per.min(3.0).min(src)
        } else {
            per.min(src)
        };
        let mut clip = clip_for_media(
            item.media_id,
            cursor,
            Duration::from_seconds(take),
            TrackKind::Video,
        );
        clip.source_in = item.hook_in;
        let dur = clip.duration;
        timeline.place_clip(v1, clip, PlaceMode::Normal)?;
        cursor += dur;
        n += 1;
    }
    let span = cursor;
    if span.as_ticks() <= 0 {
        return Ok(n);
    }

    if !b_roll.is_empty() {
        let slot = span.as_seconds() / (b_roll.len() as f64 + 1.0);
        for (i, item) in b_roll.iter().enumerate() {
            let at = Time::from_seconds(slot * (i as f64 + 1.0)).min(span);
            let src = item.duration.as_seconds().max(0.8);
            let take = 2.6_f64.min(src).min((span - at).as_seconds().max(0.8));
            if take < 0.6 {
                continue;
            }
            let clip = clip_for_media(
                item.media_id,
                at,
                Duration::from_seconds(take),
                TrackKind::Video,
            );
            timeline.place_clip(v2, clip, PlaceMode::Normal)?;
            n += 1;
        }
    }

    if let Some(bed) = music.first() {
        let take = span.as_seconds().min(bed.duration.as_seconds().max(span.as_seconds()));
        let clip = clip_for_media(
            bed.media_id,
            Time::ZERO,
            Duration::from_seconds(take.max(span.as_seconds())),
            TrackKind::Audio,
        );
        timeline.place_clip(a1, clip, PlaceMode::Normal)?;
        duck_audio(timeline, 0.7);
        n += 1;
    }
    Ok(n)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    ARoll,
    BRoll,
    Still,
    Music,
}

fn role_of(item: &AssembleItem) -> Role {
    if item.kind == TrackKind::Audio {
        return Role::Music;
    }
    if item.still {
        return Role::Still;
    }
    // Speech from STT — not the filename.
    if item.words >= 8 || item.speech_seconds >= 2.5 {
        Role::ARoll
    } else {
        Role::BRoll
    }
}

fn sort_aroll(items: &mut Vec<&AssembleItem>) {
    items.sort_by(|a, b| {
        aroll_rank(a)
            .cmp(&aroll_rank(b))
            .then(b.words.cmp(&a.words))
            .then(
                b.speech_seconds
                    .partial_cmp(&a.speech_seconds)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then(
                b.motion
                    .partial_cmp(&a.motion)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
    });
}

fn aroll_rank(item: &AssembleItem) -> u8 {
    let t = item.text.to_ascii_lowercase();
    if t.contains('?') || t.contains('!') {
        return 0;
    }
    if item.words >= 8 {
        return 1;
    }
    match item.look.as_str() {
        "wide" | "bright-wide" | "action" => 2,
        "close" => 3,
        "dark" => 5,
        _ => 4,
    }
}

fn sort_broll(items: &mut Vec<&AssembleItem>) {
    items.sort_by(|a, b| {
        b.motion
            .partial_cmp(&a.motion)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

fn cut_ranges(timeline: &mut Timeline, ranges: &[TimeRange]) -> Result<usize> {
    let mut count = 0;
    let mut ranges: Vec<TimeRange> = ranges.to_vec();
    ranges.sort_by_key(|r| r.start);
    ranges.reverse();
    for range in ranges {
        if range.end <= range.start {
            continue;
        }
        let clip_ids: Vec<ClipId> = timeline
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Video || t.kind == TrackKind::Audio)
            .flat_map(|t| t.clips.iter())
            .filter(|c| c.start < range.end && c.end() > range.start)
            .map(|c| c.id)
            .collect();
        for clip_id in clip_ids {
            let Some((_, clip)) = timeline.find_clip(clip_id) else {
                continue;
            };
            let clip_start = clip.start;
            let clip_end = clip.end();
            if range.start > clip_start && range.start < clip_end {
                let _ = timeline.split(clip_id, range.start);
            }
            let Some((_, clip)) = timeline.find_clip(clip_id) else {
                continue;
            };
            let target = if clip.contains(range.start) {
                clip_id
            } else {
                timeline
                    .tracks
                    .iter()
                    .flat_map(|t| t.clips.iter())
                    .find(|c| c.start == range.start)
                    .map(|c| c.id)
                    .unwrap_or(clip_id)
            };
            if let Some((_, clip)) = timeline.find_clip(target)
                && range.end > clip.start
                && range.end < clip.end()
            {
                let _ = timeline.split(target, range.end);
            }
            let to_delete: Vec<ClipId> = timeline
                .tracks
                .iter()
                .flat_map(|t| t.clips.iter())
                .filter(|c| c.start >= range.start && c.end() <= range.end)
                .map(|c| c.id)
                .collect();
            for id in to_delete {
                let _ = timeline.ripple_delete(id);
                count += 1;
            }
        }
    }
    Ok(count)
}

fn duck_audio(timeline: &mut Timeline, amount: f32) {
    let amount = amount.clamp(0.0, 1.0);
    for track in &mut timeline.tracks {
        if track.kind != TrackKind::Audio {
            continue;
        }
        for clip in &mut track.clips {
            if let oc_timeline::ClipKind::Audio { volume, ducked } = &mut clip.kind {
                *volume = (1.0 - amount).clamp(0.05, 1.0);
                *ducked = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oc_timeline::{Clip, ClipKind, MediaId, Transform};

    #[test]
    fn apply_split() {
        let mut tl = Timeline::default();
        let mut undo = UndoStack::new();
        let track = tl.first_track(TrackKind::Video).unwrap().id;
        let clip = Clip {
            id: ClipId::new(),
            media_id: Some(MediaId::new()),
            kind: ClipKind::Video {
                transform: Transform::default(),
            },
            start: Time::ZERO,
            duration: Duration::from_seconds(4.0),
            source_in: Time::ZERO,
            speed: 1.0,
            group_id: None,
            link_id: None,
            disabled: false,
        };
        let id = tl.add_clip(track, clip).unwrap();
        apply(
            &mut tl,
            &mut undo,
            Op::Split {
                clip_id: id,
                at: Time::from_seconds(1.0),
            },
        )
        .unwrap();
        assert_eq!(tl.first_track(TrackKind::Video).unwrap().clips.len(), 2);
        assert!(undo.undo(&mut tl));
        assert_eq!(tl.first_track(TrackKind::Video).unwrap().clips.len(), 1);
    }

    #[test]
    fn set_timeline_replaces() {
        let mut tl = Timeline::default();
        let mut undo = UndoStack::new();
        let mut next = Timeline::default();
        next.width = 1080;
        next.height = 1920;
        apply(
            &mut tl,
            &mut undo,
            Op::SetTimeline { timeline: next.clone() },
        )
        .unwrap();
        assert_eq!(tl.width, 1080);
        assert_eq!(tl.height, 1920);
        assert!(undo.undo(&mut tl));
        assert_eq!(tl.width, 1920);
    }

    #[test]
    fn add_and_remove_clip() {
        let mut tl = Timeline::default();
        let mut undo = UndoStack::new();
        let track = tl.first_track(TrackKind::Video).unwrap().id;
        let clip = Clip {
            id: ClipId::new(),
            media_id: Some(MediaId::new()),
            kind: ClipKind::Video {
                transform: Transform::default(),
            },
            start: Time::ZERO,
            duration: Duration::from_seconds(2.0),
            source_in: Time::ZERO,
            speed: 1.0,
            group_id: None,
            link_id: None,
            disabled: false,
        };
        let id = clip.id;
        apply(
            &mut tl,
            &mut undo,
            Op::AddClip {
                track_id: track,
                clip,
            },
        )
        .unwrap();
        assert!(tl.find_clip(id).is_some());
        apply(&mut tl, &mut undo, Op::RemoveClip { clip_id: id }).unwrap();
        assert!(tl.find_clip(id).is_none());
    }

    #[test]
    fn assemble_lays_video_then_audio() {
        let mut tl = Timeline::default();
        let mut undo = UndoStack::new();
        let v1 = MediaId::new();
        let v2 = MediaId::new();
        let a1 = MediaId::new();
        apply(
            &mut tl,
            &mut undo,
            Op::Assemble {
                style: AssembleStyle::Vlog,
                items: vec![
                    AssembleItem {
                        media_id: v1,
                        duration: Duration::from_seconds(20.0),
                        kind: TrackKind::Video,
                        words: 40,
                        speech_seconds: 18.0,
                        text: "so today we went out and it was wild".into(),
                        ..AssembleItem::default()
                    },
                    AssembleItem {
                        media_id: v2,
                        duration: Duration::from_seconds(8.0),
                        kind: TrackKind::Video,
                        words: 0,
                        speech_seconds: 0.0,
                        ..AssembleItem::default()
                    },
                    AssembleItem {
                        media_id: a1,
                        duration: Duration::from_seconds(10.0),
                        kind: TrackKind::Audio,
                        ..AssembleItem::default()
                    },
                ],
            },
        )
        .unwrap();
        let video = tl.first_track(TrackKind::Video).unwrap();
        assert_eq!(video.clips.len(), 1);
        assert_eq!(video.clips[0].media_id, Some(v1));
        let broll = tl
            .tracks
            .iter()
            .find(|t| t.kind == TrackKind::Video && t.id != video.id)
            .expect("b-roll track");
        assert_eq!(broll.clips.len(), 1);
        assert_eq!(broll.clips[0].media_id, Some(v2));
        let audio = tl.first_track(TrackKind::Audio).unwrap();
        assert_eq!(audio.clips.len(), 1);
        assert!(matches!(
            audio.clips[0].kind,
            ClipKind::Audio { ducked: true, .. }
        ));
    }

    #[test]
    fn director_caps_long_talking_clips() {
        let mut tl = Timeline::default();
        let mut undo = UndoStack::new();
        apply(
            &mut tl,
            &mut undo,
            Op::Assemble {
                style: AssembleStyle::Vlog,
                items: vec![AssembleItem {
                    media_id: MediaId::new(),
                    duration: Duration::from_seconds(90.0),
                    kind: TrackKind::Video,
                    words: 200,
                    speech_seconds: 80.0,
                    text: "okay so let me tell you what happened".into(),
                    ..AssembleItem::default()
                }],
            },
        )
        .unwrap();
        let video = tl.first_track(TrackKind::Video).unwrap();
        assert_eq!(video.clips.len(), 1);
        assert!(video.clips[0].duration.as_seconds() <= 8.1);
    }
}
