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
}

fn default_volume() -> f32 {
    1.0
}

impl ClipKind {
    #[must_use]
    pub fn track_kind(&self) -> TrackKind {
        match self {
            Self::Video { .. } => TrackKind::Video,
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
            (track.id, offset, right)
        };
        let right_id = new_clip.id;
        if let Some(clip) = self.clip_mut(clip_id) {
            clip.duration = left_end;
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
        };
        self.add_clip(track_id, clip)
    }
}
