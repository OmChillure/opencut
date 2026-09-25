use oc_time::{Duration, Time};
use oc_timeline::{
    AspectRatio, AudioFx, CaptionCue, CaptionStyle, Clip, ClipId, ClipKind, ClipLook, Crop, Fx,
    Grade, Graphic,
    MarkerId, MediaId, PlaceMode, Timeline, TimelineError, Track, TrackId, TrackKind,
    Transform, TransitionKind, UndoStack,
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

impl ExportPreset {
    #[must_use]
    pub fn size(self) -> (u32, u32) {
        match self {
            Self::Youtube1080 => (1920, 1080),
            Self::Vertical1080 => (1080, 1920),
            Self::Square1080 => (1080, 1080),
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Youtube1080 => "youtube-1080",
            Self::Vertical1080 => "vertical-1080",
            Self::Square1080 => "square-1080",
        }
    }
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
        #[serde(default)]
        source_in: Time,
        kind: TrackKind,
        mode: TimelineEditMode,
    },
    Assemble {
        items: Vec<AssembleItem>,
        style: AssembleStyle,
        #[serde(default)]
        target_seconds: Option<f64>,
    },
    ClearTimeline,
    SetTransition {
        clip_id: ClipId,
        kind: TransitionKind,
    },
    SetGrade {
        clip_id: ClipId,
        grade: Grade,
    },
    SetFx {
        clip_id: ClipId,
        fx: Fx,
    },
    SetFade {
        clip_id: ClipId,
        fade_in: Duration,
        fade_out: Duration,
    },
    SetVolume {
        clip_id: ClipId,
        volume: f32,
    },
    /// Kdenlive Transform / Shotcut Size-Position-Rotate: pan, zoom, rotation.
    SetTransform {
        clip_id: ClipId,
        x: f32,
        y: f32,
        scale: f32,
        rotation: f32,
    },
    /// Picture on a higher track over `at`, with its own audio muted.
    /// Same idea as a Kdenlive clip on V2 covering a jump.
    Cover {
        media_id: MediaId,
        at: Time,
        source_in: Time,
        duration: Duration,
    },
    /// Move scale and pan from the clip's transform to this pose.
    SetMove {
        clip_id: ClipId,
        end_x: f32,
        end_y: f32,
        end_scale: f32,
    },
    /// Speed at the start is `speed`; `end_speed` is the speed at the tail.
    SetSpeedRamp {
        clip_id: ClipId,
        speed: f32,
        end_speed: f32,
    },
    SetStabilize {
        clip_id: ClipId,
        on: bool,
    },
    /// Keep a fraction of the frame. x,y,w,h are 0–1.
    SetCrop {
        clip_id: ClipId,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
    SetAudio {
        clip_id: ClipId,
        audio: AudioFx,
    },
    AddGraphic {
        graphic: Graphic,
        start: Time,
        duration: Duration,
        track_id: Option<TrackId>,
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
    /// Takes to cut from this source. Empty = one take from hook_in.
    #[serde(default)]
    pub excerpts: Vec<Excerpt>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Excerpt {
    pub source_in: Time,
    pub duration: Duration,
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
            excerpts: Vec::new(),
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
            source_in,
            kind,
            mode,
        } => {
            let track_id = resolve_track(timeline, *track_id, *kind);
            let clip = clip_for_media(*media_id, *start, *duration, *kind, *source_in);
            let id = timeline.place_clip(track_id, clip, mode.as_place())?;
            format!(
                "placed {id} from {media_id} at {:.2}s (src {:.2}s, {:.2}s)",
                start.as_seconds(),
                source_in.as_seconds(),
                duration.as_seconds()
            )
        }
        Op::Assemble {
            items,
            style,
            target_seconds,
        } => {
            let n = assemble(timeline, items, *style, *target_seconds)?;
            format!("assembled {n} clips ({style:?})")
        }
        Op::ClearTimeline => {
            let n = clear_timeline(timeline);
            format!("cleared {n} clips")
        }
        Op::SetTransition { clip_id, kind } => {
            let clip = timeline
                .clip_mut(*clip_id)
                .ok_or(TimelineError::ClipNotFound(*clip_id))?;
            clip.look.transition = *kind;
            format!("{} on {clip_id}", kind.label())
        }
        Op::SetGrade { clip_id, grade } => {
            let clip = timeline
                .clip_mut(*clip_id)
                .ok_or(TimelineError::ClipNotFound(*clip_id))?;
            clip.look.grade = *grade;
            format!("grade on {clip_id}")
        }
        Op::SetFx { clip_id, fx } => {
            let clip = timeline
                .clip_mut(*clip_id)
                .ok_or(TimelineError::ClipNotFound(*clip_id))?;
            clip.look.fx = *fx;
            format!("fx on {clip_id}")
        }
        Op::SetFade {
            clip_id,
            fade_in,
            fade_out,
        } => {
            let clip = timeline
                .clip_mut(*clip_id)
                .ok_or(TimelineError::ClipNotFound(*clip_id))?;
            clip.look.fade_in = *fade_in;
            clip.look.fade_out = *fade_out;
            format!(
                "fade in {:.1}s out {:.1}s on {clip_id}",
                fade_in.as_seconds(),
                fade_out.as_seconds()
            )
        }
        Op::SetTransform {
            clip_id,
            x,
            y,
            scale,
            rotation,
        } => {
            let clip = timeline
                .clip_mut(*clip_id)
                .ok_or(TimelineError::ClipNotFound(*clip_id))?;
            match &mut clip.kind {
                ClipKind::Video { transform } => {
                    *transform = Transform {
                        x: *x,
                        y: *y,
                        scale: scale.clamp(0.25, 4.0),
                        rotation: *rotation,
                    };
                }
                _ => {
                    return Err(OpError::Message(
                        "transform is for video clips".into(),
                    ));
                }
            }
            format!(
                "transform {clip_id} scale {:.2} pan {:.0},{:.0}",
                scale, x, y
            )
        }
        Op::Cover {
            media_id,
            at,
            source_in,
            duration,
        } => {
            let track_id = overlay_track(timeline);
            let mut picture = clip_for_media(*media_id, *at, *duration, TrackKind::Video, *source_in);
            if let ClipKind::Video { transform } = &mut picture.kind {
                transform.scale = 1.28;
            }
            let id = timeline.place_clip(track_id, picture, PlaceMode::Normal)?;
            let audio_track = resolve_track(timeline, None, TrackKind::Audio);
            let mut bed = clip_for_media(*media_id, *at, *duration, TrackKind::Audio, *source_in);
            if let ClipKind::Audio { volume, .. } = &mut bed.kind {
                *volume = 0.0;
            }
            let _ = timeline.place_clip(audio_track, bed, PlaceMode::Normal);
            format!(
                "cover {id} from {media_id} at {:.2}s (src {:.2}s)",
                at.as_seconds(),
                source_in.as_seconds()
            )
        }
        Op::SetMove {
            clip_id,
            end_x,
            end_y,
            end_scale,
        } => {
            let clip = timeline
                .clip_mut(*clip_id)
                .ok_or(TimelineError::ClipNotFound(*clip_id))?;
            let start = match &clip.kind {
                ClipKind::Video { transform } => *transform,
                _ => {
                    return Err(OpError::Message("move is for video clips".into()));
                }
            };
            clip.look.move_to = Some(Transform {
                x: *end_x,
                y: *end_y,
                scale: end_scale.clamp(0.25, 4.0),
                rotation: start.rotation,
            });
            format!("move {clip_id} to scale {end_scale:.2}")
        }
        Op::SetSpeedRamp {
            clip_id,
            speed,
            end_speed,
        } => {
            let clip = timeline
                .clip_mut(*clip_id)
                .ok_or(TimelineError::ClipNotFound(*clip_id))?;
            let speed = speed.clamp(0.25, 4.0);
            let end_speed = end_speed.clamp(0.25, 4.0);
            clip.speed = speed;
            clip.look.speed_to = Some(end_speed);
            format!("speed {speed:.2} → {end_speed:.2} on {clip_id}")
        }
        Op::SetStabilize { clip_id, on } => {
            let clip = timeline
                .clip_mut(*clip_id)
                .ok_or(TimelineError::ClipNotFound(*clip_id))?;
            clip.look.stabilize = *on;
            format!("stabilize {on} on {clip_id}")
        }
        Op::SetCrop {
            clip_id,
            x,
            y,
            w,
            h,
        } => {
            let clip = timeline
                .clip_mut(*clip_id)
                .ok_or(TimelineError::ClipNotFound(*clip_id))?;
            let w = w.clamp(0.05, 1.0);
            let h = h.clamp(0.05, 1.0);
            clip.look.crop = Some(Crop {
                x: x.clamp(0.0, 1.0 - w),
                y: y.clamp(0.0, 1.0 - h),
                w,
                h,
            });
            format!("crop {clip_id}")
        }
        Op::SetAudio { clip_id, audio } => {
            let clip = timeline
                .clip_mut(*clip_id)
                .ok_or(TimelineError::ClipNotFound(*clip_id))?;
            clip.look.audio = *audio;
            format!("audio fx on {clip_id}")
        }
        Op::SetVolume { clip_id, volume } => {
            let clip = timeline
                .clip_mut(*clip_id)
                .ok_or(TimelineError::ClipNotFound(*clip_id))?;
            match &mut clip.kind {
                ClipKind::Audio { volume: v, .. } => {
                    *v = (*volume).clamp(0.0, 2.0);
                }
                _ => {
                    return Err(OpError::Message(
                        "volume is for audio clips — select an audio clip".into(),
                    ));
                }
            }
            format!("volume {volume:.2} on {clip_id}")
        }
        Op::AddGraphic {
            graphic,
            start,
            duration,
            track_id,
        } => {
            let track_id = match track_id {
                Some(id) if timeline.track(*id).is_some() => *id,
                _ => overlay_track(timeline),
            };
            let mut clip = clip_for_media(
                MediaId::new(),
                *start,
                *duration,
                TrackKind::Video,
                Time::ZERO,
            );
            clip.media_id = None;
            clip.kind = ClipKind::Graphic {
                graphic: graphic.clone(),
            };
            clip.look.graphic = Some(graphic.clone());
            let id = timeline.place_clip(track_id, clip, PlaceMode::Normal)?;
            format!("graphic {id}")
        }
    };
    Ok(AppliedOp { op, note })
}

fn clip_for_media(
    media_id: MediaId,
    start: Time,
    duration: Duration,
    kind: TrackKind,
    source_in: Time,
) -> Clip {
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
        source_in,
        speed: 1.0,
        group_id: None,
        link_id: None,
        disabled: false,
        look: ClipLook::default(),
    }
}

fn overlay_track(timeline: &mut Timeline) -> TrackId {
    let videos: Vec<TrackId> = timeline
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Video)
        .map(|t| t.id)
        .collect();
    if let Some(id) = videos.get(1).copied() {
        return id;
    }
    timeline.add_track(TrackKind::Video, "GFX")
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

fn clear_timeline(timeline: &mut Timeline) -> usize {
    let mut n = 0;
    for track in &mut timeline.tracks {
        n += track.clips.len();
        track.clips.clear();
    }
    n
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
    target_seconds: Option<f64>,
) -> Result<usize> {
    if items.is_empty() {
        return Err(OpError::Message("no media to assemble".into()));
    }
    match style {
        AssembleStyle::Sequential => assemble_linear(timeline, items),
        AssembleStyle::Vlog => assemble_short(timeline, items, target_seconds),
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
            let clip = clip_for_media(
                item.media_id,
                t_audio,
                item.duration,
                TrackKind::Audio,
                Time::ZERO,
            );
            let dur = clip.duration;
            timeline.place_clip(audio_track, clip, PlaceMode::Normal)?;
            t_audio += dur;
            n += 1;
            continue;
        }
        let clip = clip_for_media(
            item.media_id,
            t_video,
            item.duration,
            TrackKind::Video,
            Time::ZERO,
        );
        let dur = clip.duration;
        timeline.place_clip(video_track, clip, PlaceMode::Normal)?;
        t_video += dur;
        n += 1;
    }
    Ok(n)
}

/// Cut a 30s–60s short: hook + A-roll spine, B-roll on V2, music ducked under.
fn assemble_short(
    timeline: &mut Timeline,
    items: &[AssembleItem],
    target_seconds: Option<f64>,
) -> Result<usize> {
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

    let excerpt_n = a_roll.iter().map(|i| i.excerpts.len().max(1)).sum::<usize>() as f64;
    let n_a = excerpt_n.max(1.0);
    let target = target_seconds.unwrap_or((n_a * 5.0).clamp(30.0, 60.0));
    let per = (target / n_a).clamp(2.8, 10.0);
    let mut cursor = Time::ZERO;
    let mut n = 0;
    for (i, item) in a_roll.iter().enumerate() {
        if !item.excerpts.is_empty() {
            for ex in &item.excerpts {
                let clip = clip_for_media(
                    item.media_id,
                    cursor,
                    ex.duration,
                    TrackKind::Video,
                    ex.source_in,
                );
                let dur = clip.duration;
                timeline.place_clip(v1, clip, PlaceMode::Normal)?;
                cursor += dur;
                n += 1;
            }
            continue;
        }
        let src = item.duration.as_seconds().max(0.4);
        let take = if i == 0 {
            per.min(4.0).min(src)
        } else if i + 1 == a_roll.len() && a_roll.len() > 1 {
            per.min(3.0).min(src)
        } else {
            per.min(src)
        };
        let clip = clip_for_media(
            item.media_id,
            cursor,
            Duration::from_seconds(take),
            TrackKind::Video,
            item.hook_in,
        );
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
                Time::ZERO,
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
            Time::ZERO,
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

/// Pick keep-takes from timestamped speech so one long source can become a short.
#[must_use]
/// Excerpts for a timed short. A quoted line in the request becomes the open.
pub fn excerpts_for_request(cues: &[(Time, Time, &str)], request: &str, target_s: f64) -> Vec<Excerpt> {
    let quote = quoted_line(request);
    if let Some(q) = quote {
        let needle = q.to_ascii_lowercase();
        if let Some(i) = cues.iter().position(|(_, _, text)| {
            text.to_ascii_lowercase().contains(needle.trim())
        }) {
            return excerpts_from(cues, i, target_s);
        }
    }
    pick_reel_excerpts(cues, target_s)
}

fn quoted_line(request: &str) -> Option<&str> {
    let bytes = request.as_bytes();
    for (open, close) in [(b'"', b'"'), (b'\'', b'\'')] {
        if let Some(a) = bytes.iter().position(|c| *c == open) {
            if let Some(rel) = bytes[a + 1..].iter().position(|c| *c == close) {
                let s = request.get(a + 1..a + 1 + rel)?.trim();
                if s.len() >= 8 {
                    return Some(s);
                }
            }
        }
    }
    None
}

fn excerpts_from(cues: &[(Time, Time, &str)], start_idx: usize, target_s: f64) -> Vec<Excerpt> {
    let target = target_s.clamp(20.0, 75.0);
    let mut used = 0.0;
    let mut out = Vec::new();
    for (start, end, text) in cues.iter().skip(start_idx) {
        if end <= start || is_filler(text) {
            continue;
        }
        let dur = (*end - *start).as_seconds();
        if dur < 0.4 {
            continue;
        }
        out.push(Excerpt {
            source_in: *start,
            duration: *end - *start,
        });
        used += dur;
        if used >= target {
            break;
        }
    }
    out
}

pub fn pick_reel_excerpts(cues: &[(Time, Time, &str)], target_s: f64) -> Vec<Excerpt> {
    let target = target_s.clamp(20.0, 75.0);
    let takes = merge_speech_takes(cues);
    if takes.is_empty() {
        return Vec::new();
    }
    let hook = takes
        .iter()
        .position(|t| t.hook)
        .or_else(|| takes.iter().position(|t| t.words >= 4))
        .unwrap_or(0);
    let mut picked = vec![takes[hook]];
    let mut used = takes[hook].duration_s();
    for take in takes.iter().skip(hook + 1) {
        if used >= target {
            break;
        }
        let last_end = picked.last().map(|t| t.end).unwrap_or(Time::ZERO);
        if take.start < last_end {
            continue;
        }
        let gap = (take.start - last_end).as_seconds();
        if gap < 0.25 && take.start != last_end {
            continue;
        }
        used += take.duration_s();
        picked.push(*take);
    }
    if used < target * 0.7 {
        for take in takes.iter().take(hook) {
            if used >= target {
                break;
            }
            if picked.iter().any(|p| p.start == take.start) {
                continue;
            }
            used += take.duration_s();
            picked.push(*take);
        }
        picked.sort_by_key(|t| t.start);
    }
    picked
        .into_iter()
        .map(|t| Excerpt {
            source_in: t.start,
            duration: t.end - t.start,
        })
        .filter(|e| e.duration.as_seconds() >= 0.6)
        .collect()
}

#[derive(Clone, Copy)]
struct SpeechTake {
    start: Time,
    end: Time,
    words: u32,
    hook: bool,
}

impl SpeechTake {
    fn duration_s(self) -> f64 {
        (self.end - self.start).as_seconds()
    }
}

fn merge_speech_takes(cues: &[(Time, Time, &str)]) -> Vec<SpeechTake> {
    let mut out = Vec::new();
    let mut cur: Option<SpeechTake> = None;
    for (start, end, text) in cues {
        if end <= start || is_filler(text) {
            if let Some(take) = cur.take() {
                if take.duration_s() >= 0.8 {
                    out.push(take);
                }
            }
            continue;
        }
        let words = word_count(text);
        let hook = text.contains('?') || text.contains('!');
        match cur.as_mut() {
            None => {
                cur = Some(SpeechTake {
                    start: *start,
                    end: *end,
                    words,
                    hook,
                });
            }
            Some(take) => {
                let gap = (*start - take.end).as_seconds();
                let merged = (take.end.max(*end) - take.start).as_seconds();
                if gap <= 0.7 && merged <= 6.5 {
                    take.end = take.end.max(*end);
                    take.words += words;
                    take.hook |= hook;
                } else {
                    if take.duration_s() >= 0.8 {
                        out.push(*take);
                    }
                    *take = SpeechTake {
                        start: *start,
                        end: *end,
                        words,
                        hook,
                    };
                }
            }
        }
    }
    if let Some(take) = cur {
        if take.duration_s() >= 0.8 {
            out.push(take);
        }
    }
    out
}

fn word_count(text: &str) -> u32 {
    text.split_whitespace()
        .filter(|w| w.chars().any(|c| c.is_alphanumeric()))
        .count() as u32
}

fn is_filler(text: &str) -> bool {
    let t = text.trim().to_ascii_lowercase();
    if t.is_empty() {
        return true;
    }
    t.split_whitespace().all(|w| {
        matches!(
            w.trim_matches(|c: char| !c.is_alphanumeric()),
            "um" | "uh" | "uhm" | "hmm" | "mm" | "yeah" | "yep" | "ok" | "okay" | "so" | "like"
        )
    })
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
            look: ClipLook::default(),
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
            look: ClipLook::default(),
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
                target_seconds: None,
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
                target_seconds: None,
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

    #[test]
    fn assemble_long_source_uses_excerpts() {
        let mut tl = Timeline::default();
        let mut undo = UndoStack::new();
        let media = MediaId::new();
        apply(
            &mut tl,
            &mut undo,
            Op::Assemble {
                style: AssembleStyle::Vlog,
                target_seconds: Some(30.0),
                items: vec![AssembleItem {
                    media_id: media,
                    duration: Duration::from_seconds(300.0),
                    kind: TrackKind::Video,
                    words: 400,
                    speech_seconds: 240.0,
                    excerpts: vec![
                        Excerpt {
                            source_in: Time::from_seconds(4.0),
                            duration: Duration::from_seconds(5.0),
                        },
                        Excerpt {
                            source_in: Time::from_seconds(40.0),
                            duration: Duration::from_seconds(8.0),
                        },
                        Excerpt {
                            source_in: Time::from_seconds(120.0),
                            duration: Duration::from_seconds(6.0),
                        },
                        Excerpt {
                            source_in: Time::from_seconds(200.0),
                            duration: Duration::from_seconds(5.0),
                        },
                    ],
                    ..AssembleItem::default()
                }],
            },
        )
        .unwrap();
        let video = tl.first_track(TrackKind::Video).unwrap();
        assert_eq!(video.clips.len(), 4);
        assert!((video.clips[0].source_in.as_seconds() - 4.0).abs() < 1e-6);
        assert!((video.clips[1].source_in.as_seconds() - 40.0).abs() < 1e-6);
        let span: f64 = video.clips.iter().map(|c| c.duration.as_seconds()).sum();
        assert!((span - 24.0).abs() < 0.1);
    }

    #[test]
    fn place_media_keeps_source_in() {
        let mut tl = Timeline::default();
        let mut undo = UndoStack::new();
        let media = MediaId::new();
        apply(
            &mut tl,
            &mut undo,
            Op::PlaceMedia {
                media_id: media,
                track_id: None,
                start: Time::from_seconds(2.0),
                duration: Duration::from_seconds(3.5),
                source_in: Time::from_seconds(81.0),
                kind: TrackKind::Video,
                mode: TimelineEditMode::Normal,
            },
        )
        .unwrap();
        let clip = &tl.first_track(TrackKind::Video).unwrap().clips[0];
        assert!((clip.source_in.as_seconds() - 81.0).abs() < 1e-6);
        assert!((clip.start.as_seconds() - 2.0).abs() < 1e-6);
        assert!((clip.duration.as_seconds() - 3.5).abs() < 1e-6);
    }

    #[test]
    fn pick_reel_skips_filler_and_hits_target() {
        let cues = [
            (Time::from_seconds(0.0), Time::from_seconds(0.6), "um"),
            (
                Time::from_seconds(1.0),
                Time::from_seconds(4.0),
                "what if we just left tonight?",
            ),
            (Time::from_seconds(4.2), Time::from_seconds(7.0), "pack the car"),
            (Time::from_seconds(20.0), Time::from_seconds(24.0), "the road was empty"),
            (Time::from_seconds(40.0), Time::from_seconds(44.0), "we made it by dawn"),
            (Time::from_seconds(80.0), Time::from_seconds(84.0), "that was the whole trip"),
        ];
        let takes = pick_reel_excerpts(&cues, 30.0);
        assert!(takes.len() >= 3, "{takes:?}");
        assert!(takes[0].source_in.as_seconds() >= 0.9);
        let span: f64 = takes.iter().map(|t| t.duration.as_seconds()).sum();
        assert!(span >= 12.0, "{span}");
        assert!(span <= 40.0, "{span}");
    }

    #[test]
    fn clear_timeline_drops_clips() {
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
            look: ClipLook::default(),
        };
        apply(
            &mut tl,
            &mut undo,
            Op::AddClip {
                track_id: track,
                clip,
            },
        )
        .unwrap();
        apply(&mut tl, &mut undo, Op::ClearTimeline).unwrap();
        assert!(tl.first_track(TrackKind::Video).unwrap().clips.is_empty());
    }

    #[test]
    fn set_transition_and_graphic() {
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
            look: ClipLook::default(),
        };
        let id = tl.add_clip(track, clip).unwrap();
        apply(
            &mut tl,
            &mut undo,
            Op::SetTransition {
                clip_id: id,
                kind: TransitionKind::Dissolve,
            },
        )
        .unwrap();
        assert_eq!(
            tl.find_clip(id).unwrap().1.look.transition,
            TransitionKind::Dissolve
        );
        apply(
            &mut tl,
            &mut undo,
            Op::AddGraphic {
                graphic: Graphic::title("Hello"),
                start: Time::from_seconds(1.0),
                duration: Duration::from_seconds(2.0),
                track_id: None,
            },
        )
        .unwrap();
        let has_title = tl.tracks.iter().flat_map(|t| &t.clips).any(|c| {
            matches!(
                &c.kind,
                ClipKind::Graphic {
                    graphic
                } if graphic.text == "Hello"
            )
        });
        assert!(has_title);
    }
}
