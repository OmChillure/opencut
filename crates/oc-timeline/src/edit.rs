use crate::ids::{ClipId, GroupId, LinkId, MarkerId, TrackId};
use crate::model::{Clip, ClipKind, ClipLook, Lut, Marker, Timeline, TrackKind};
use crate::{Result, TimelineError};
use oc_time::{Duration, Time};

impl Timeline {
    fn frame_slack(&self) -> Duration {
        Duration::from_seconds(1.0 / self.frame_rate.as_f64().max(1.0))
    }

    /// Keep duration and position; change which source frames play.
    pub fn slip(&mut self, clip_id: ClipId, new_source_in: Time) -> Result<()> {
        let rate = self.frame_rate;
        let new_source_in = new_source_in.snap_to_frame(rate);
        if new_source_in.as_ticks() < 0 {
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
        clip.source_in = new_source_in;
        Ok(())
    }

    /// Move the cut between two touching clips. Sequence length stays the same.
    pub fn roll(&mut self, left_id: ClipId, at: Time) -> Result<()> {
        let at = at.snap_to_frame(self.frame_rate);
        let slack = self.frame_slack();
        let (ti, ci) = self
            .locate(left_id)
            .ok_or(TimelineError::ClipNotFound(left_id))?;
        if self.tracks[ti].locked {
            return Err(TimelineError::TrackLocked);
        }
        if ci + 1 >= self.tracks[ti].clips.len() {
            return Err(TimelineError::CannotRoll);
        }
        let left_start = self.tracks[ti].clips[ci].start;
        let right_end = self.tracks[ti].clips[ci + 1].end();
        let right_start = self.tracks[ti].clips[ci + 1].start;
        let left_end = self.tracks[ti].clips[ci].end();
        if right_start > left_end + slack {
            return Err(TimelineError::CannotRoll);
        }
        if at <= left_start || at >= right_end {
            return Err(TimelineError::CannotRoll);
        }
        let right_delta = (at - right_start).as_seconds();
        self.tracks[ti].clips[ci].duration = at - left_start;
        let right = &mut self.tracks[ti].clips[ci + 1];
        right.take_head(right_delta);
        right.start = at;
        right.duration = right_end - at;
        Ok(())
    }

    /// Move a clip; neighbors absorb the time. Sequence length stays the same.
    pub fn slide(&mut self, clip_id: ClipId, new_start: Time) -> Result<()> {
        let new_start = new_start.snap_to_frame(self.frame_rate);
        let slack = self.frame_slack();
        let (ti, ci) = self
            .locate(clip_id)
            .ok_or(TimelineError::ClipNotFound(clip_id))?;
        if self.tracks[ti].locked {
            return Err(TimelineError::TrackLocked);
        }
        if ci == 0 || ci + 1 >= self.tracks[ti].clips.len() {
            return Err(TimelineError::CannotSlide);
        }
        let left_start = self.tracks[ti].clips[ci - 1].start;
        let mid = self.tracks[ti].clips[ci].clone();
        let right_end = self.tracks[ti].clips[ci + 1].end();
        let left_end = self.tracks[ti].clips[ci - 1].end();
        let right_start = self.tracks[ti].clips[ci + 1].start;
        if mid.start > left_end + slack || right_start > mid.end() + slack {
            return Err(TimelineError::CannotSlide);
        }
        let min_start = left_start + slack;
        let max_start = right_end - mid.duration - slack;
        if max_start <= min_start {
            return Err(TimelineError::CannotSlide);
        }
        let new_start = clamp_time(new_start, min_start, max_start);
        let delta = new_start - mid.start;
        if delta.as_ticks() == 0 {
            return Ok(());
        }
        self.tracks[ti].clips[ci - 1].duration = new_start - left_start;
        self.tracks[ti].clips[ci].start = new_start;
        let right = &mut self.tracks[ti].clips[ci + 1];
        let new_right_start = new_start + mid.duration;
        let delta = (new_right_start - right.start).as_seconds();
        right.take_head(delta);
        right.start = new_right_start;
        right.duration = right_end - new_right_start;
        Ok(())
    }

    /// Trim and shift later clips on the same track by the duration change.
    pub fn ripple_trim(
        &mut self,
        clip_id: ClipId,
        new_start: Time,
        new_duration: Duration,
    ) -> Result<()> {
        let old = {
            let (_, clip) = self
                .find_clip(clip_id)
                .ok_or(TimelineError::ClipNotFound(clip_id))?;
            (clip.start, clip.end(), clip.duration)
        };
        self.trim(clip_id, new_start, new_duration)?;
        let new_end = new_start + new_duration;
        let delta = new_end - old.1;
        let (ti, _) = self
            .locate(clip_id)
            .ok_or(TimelineError::ClipNotFound(clip_id))?;
        let threshold = old.0 + old.2;
        if let Some(track) = self.track_mut(self.tracks[ti].id) {
            for clip in &mut track.clips {
                if clip.id != clip_id && clip.start >= threshold {
                    clip.start += delta;
                }
            }
        }
        Ok(())
    }

    /// Change playback speed by stretching duration. Source in-point stays.
    pub fn rate_stretch(&mut self, clip_id: ClipId, new_duration: Duration) -> Result<()> {
        let new_duration = Duration::from_ticks(
            Time::from_ticks(new_duration.as_ticks())
                .snap_to_frame(self.frame_rate)
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
        let source = clip.source_duration();
        let speed = source.as_ticks() as f64 / new_duration.as_ticks() as f64;
        if !speed.is_finite() || speed <= 0.0 {
            return Err(TimelineError::InvalidSpeed);
        }
        clip.speed = speed as f32;
        clip.duration = new_duration;
        Ok(())
    }

    pub fn set_speed(&mut self, clip_id: ClipId, speed: f32) -> Result<()> {
        if !speed.is_finite() || speed <= 0.0 {
            return Err(TimelineError::InvalidSpeed);
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
        let source = clip.source_duration();
        clip.speed = speed;
        clip.duration =
            Duration::from_ticks((source.as_ticks() as f64 / f64::from(speed)).round() as i64);
        if clip.duration.as_ticks() <= 0 {
            return Err(TimelineError::EmptyTrim);
        }
        Ok(())
    }

    pub fn split_all(&mut self, at: Time) -> Result<Vec<ClipId>> {
        let at = at.snap_to_frame(self.frame_rate);
        let ids: Vec<ClipId> = self
            .tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .filter(|c| c.contains(at))
            .map(|c| c.id)
            .collect();
        let mut created = Vec::new();
        for id in ids {
            if let Ok(right) = self.split(id, at) {
                created.push(right);
            }
        }
        Ok(created)
    }

    /// Cut every video track at `at` and enable only `track_id` from there on.
    pub fn multicam_cut(&mut self, track_id: TrackId, at: Time) -> Result<()> {
        let at = at.snap_to_frame(self.frame_rate);
        let dest = self
            .track(track_id)
            .ok_or(TimelineError::TrackNotFound(track_id))?;
        if dest.kind != TrackKind::Video {
            return Err(TimelineError::TrackKindMismatch);
        }
        let reference = self
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Video)
            .flat_map(|t| t.clips.iter())
            .find(|c| !c.disabled && c.contains(at) && matches!(c.kind, ClipKind::Video { .. }))
            .map(|c| {
                let into = (at - c.start).as_seconds().max(0.0);
                let speed = f64::from(c.speed.max(0.01));
                c.source_in.as_seconds() + into * speed
            });
        let _ = self.split_all(at);
        for track in &mut self.tracks {
            if track.kind != TrackKind::Video {
                continue;
            }
            let enable = track.id == track_id;
            for clip in &mut track.clips {
                if clip.start >= at {
                    clip.disabled = !enable;
                }
            }
        }
        if let Some(reference) = reference {
            if let Some(track) = self.track_mut(track_id) {
                if let Some(clip) = track
                    .clips
                    .iter_mut()
                    .find(|c| !c.disabled && c.contains(at))
                {
                    let into = (at - clip.start).as_seconds().max(0.0);
                    let speed = f64::from(clip.speed.max(0.01));
                    let current = clip.source_in.as_seconds() + into * speed;
                    let next = (clip.source_in.as_seconds() + (reference - current)).max(0.0);
                    clip.source_in = Time::from_seconds(next);
                }
            }
        }
        Ok(())
    }

    pub fn insert_space(
        &mut self,
        track_id: Option<TrackId>,
        at: Time,
        amount: Duration,
    ) -> Result<()> {
        let at = at.snap_to_frame(self.frame_rate);
        if amount.as_ticks() <= 0 {
            return Err(TimelineError::EmptyTrim);
        }
        let ids: Vec<TrackId> = match track_id {
            Some(id) => vec![id],
            None => self.tracks.iter().map(|t| t.id).collect(),
        };
        for id in ids {
            if self.track(id).is_none() {
                return Err(TimelineError::TrackNotFound(id));
            }
            if self.track(id).is_some_and(|t| t.locked) {
                return Err(TimelineError::TrackLocked);
            }
            if let Some(clip) = self.clip_at(id, at) {
                let _ = self.split(clip, at);
            }
            if let Some(track) = self.track_mut(id) {
                for clip in &mut track.clips {
                    if clip.start >= at {
                        clip.start += amount;
                    }
                }
            }
        }
        Ok(())
    }

    pub fn delete_space(&mut self, track_id: Option<TrackId>, at: Time) -> Result<Duration> {
        let at = at.snap_to_frame(self.frame_rate);
        let ids: Vec<TrackId> = match track_id {
            Some(id) => vec![id],
            None => self.tracks.iter().map(|t| t.id).collect(),
        };
        let mut closed = Duration::ZERO;
        for id in ids {
            let gap = {
                let track = self.track(id).ok_or(TimelineError::TrackNotFound(id))?;
                if track.locked {
                    return Err(TimelineError::TrackLocked);
                }
                next_gap(track, at)
            };
            let Some((start, dur)) = gap else {
                continue;
            };
            if let Some(track) = self.track_mut(id) {
                for clip in &mut track.clips {
                    if clip.start >= start + dur {
                        clip.start -= dur;
                    }
                }
            }
            if dur > closed {
                closed = dur;
            }
        }
        if closed.as_ticks() <= 0 {
            return Err(TimelineError::NoGap);
        }
        Ok(closed)
    }

    pub fn place_clip(
        &mut self,
        track_id: TrackId,
        mut clip: Clip,
        mode: PlaceMode,
    ) -> Result<ClipId> {
        if matches!(mode, PlaceMode::Insert | PlaceMode::Overwrite) {
            let (start, duration) = marked_place(clip.start, clip.duration, self.mark_range());
            clip.start = start;
            clip.duration = duration;
        }
        match mode {
            PlaceMode::Normal => {
                if let Some(hit) = overlapping(self, track_id, clip.start, clip.end(), None) {
                    clip.start = hit;
                }
                self.add_clip(track_id, clip)
            }
            PlaceMode::Insert => {
                if let Some(existing) = self.clip_at(track_id, clip.start) {
                    let _ = self.split(existing, clip.start);
                }
                let amount = clip.duration;
                let start = clip.start;
                if let Some(track) = self.track_mut(track_id) {
                    if track.locked {
                        return Err(TimelineError::TrackLocked);
                    }
                    for other in &mut track.clips {
                        if other.start >= start {
                            other.start += amount;
                        }
                    }
                }
                self.add_clip(track_id, clip)
            }
            PlaceMode::Overwrite => {
                let start = clip.start;
                let end = clip.end();
                if let Some(existing) = self.clip_at(track_id, start) {
                    let _ = self.split(existing, start);
                }
                if let Some(existing) = self.clip_at(track_id, end) {
                    let _ = self.split(existing, end);
                }
                let doomed: Vec<ClipId> = self
                    .track(track_id)
                    .map(|t| {
                        t.clips
                            .iter()
                            .filter(|c| c.start >= start && c.end() <= end)
                            .map(|c| c.id)
                            .collect()
                    })
                    .unwrap_or_default();
                for id in doomed {
                    let _ = self.remove_clip(id);
                }
                self.add_clip(track_id, clip)
            }
        }
    }

    pub fn set_disabled(&mut self, clip_id: ClipId, disabled: bool) -> Result<()> {
        let clip = self
            .clip_mut(clip_id)
            .ok_or(TimelineError::ClipNotFound(clip_id))?;
        clip.disabled = disabled;
        Ok(())
    }

    pub fn group_clips(&mut self, ids: &[ClipId]) -> Result<GroupId> {
        if ids.len() < 2 {
            return Err(TimelineError::CannotMerge);
        }
        let group = GroupId::new();
        for id in ids {
            let clip = self.clip_mut(*id).ok_or(TimelineError::ClipNotFound(*id))?;
            clip.group_id = Some(group);
        }
        Ok(group)
    }

    pub fn ungroup(&mut self, clip_id: ClipId) -> Result<()> {
        let group = self
            .find_clip(clip_id)
            .and_then(|(_, c)| c.group_id)
            .ok_or(TimelineError::ClipNotFound(clip_id))?;
        for track in &mut self.tracks {
            for clip in &mut track.clips {
                if clip.group_id == Some(group) {
                    clip.group_id = None;
                }
            }
        }
        Ok(())
    }

    pub fn link_clips(&mut self, ids: &[ClipId]) -> Result<LinkId> {
        if ids.len() < 2 {
            return Err(TimelineError::CannotMerge);
        }
        let link = LinkId::new();
        for id in ids {
            let clip = self.clip_mut(*id).ok_or(TimelineError::ClipNotFound(*id))?;
            clip.link_id = Some(link);
        }
        Ok(link)
    }

    pub fn unlink(&mut self, clip_id: ClipId) -> Result<()> {
        let clip = self
            .clip_mut(clip_id)
            .ok_or(TimelineError::ClipNotFound(clip_id))?;
        clip.link_id = None;
        Ok(())
    }

    /// Copy a video clip onto the first audio track, linked.
    pub fn detach_audio(&mut self, clip_id: ClipId) -> Result<ClipId> {
        let (src_kind, clone) = {
            let (_, clip) = self
                .find_clip(clip_id)
                .ok_or(TimelineError::ClipNotFound(clip_id))?;
            if !matches!(clip.kind, ClipKind::Video { .. }) {
                return Err(TimelineError::TrackKindMismatch);
            }
            (clip.kind.track_kind(), clip.clone())
        };
        let _ = src_kind;
        let audio_id = match self.first_track(TrackKind::Audio) {
            Some(t) => t.id,
            None => self.add_track(TrackKind::Audio, "A1"),
        };
        let link = clone.link_id.unwrap_or_else(LinkId::new);
        if let Some(clip) = self.clip_mut(clip_id) {
            clip.link_id = Some(link);
        }
        let audio = Clip {
            id: ClipId::new(),
            media_id: clone.media_id,
            kind: ClipKind::Audio {
                volume: 1.0,
                ducked: false,
            },
            start: clone.start,
            duration: clone.duration,
            source_in: clone.source_in,
            speed: clone.speed,
            group_id: clone.group_id,
            link_id: Some(link),
            disabled: false,
            look: ClipLook::default(),
        };
        self.add_clip(audio_id, audio)
    }

    /// J-cut (`lead`) and L-cut (`tail`), in seconds of media time.
    ///
    /// Audio starts `lead` before the picture and runs `tail` past it.
    /// The linked audio clip is written absolutely so a later move keeps the offset.
    pub fn jl_cut(&mut self, clip_id: ClipId, lead: Duration, tail: Duration) -> Result<ClipId> {
        let (picture_start, picture_dur, picture_in, link) = {
            let (_, clip) = self
                .find_clip(clip_id)
                .ok_or(TimelineError::ClipNotFound(clip_id))?;
            if !matches!(clip.kind, ClipKind::Video { .. }) {
                return Err(TimelineError::TrackKindMismatch);
            }
            (clip.start, clip.duration, clip.source_in, clip.link_id)
        };
        let audio_id = match self.linked_audio_id(clip_id, link) {
            Some(id) => id,
            None => self.detach_audio(clip_id)?,
        };
        let (audio_track, _) = self
            .locate(audio_id)
            .ok_or(TimelineError::ClipNotFound(audio_id))?;
        if self.tracks[audio_track].locked {
            return Err(TimelineError::TrackLocked);
        }
        let lead = lead
            .max(Duration::ZERO)
            .min(Duration::from_ticks(picture_in.as_ticks().max(0)))
            .min(picture_start - Time::ZERO);
        let tail = tail.max(Duration::ZERO);
        let audio = self
            .clip_mut(audio_id)
            .ok_or(TimelineError::ClipNotFound(audio_id))?;
        audio.start = picture_start - lead;
        audio.duration = picture_dur + lead + tail;
        audio.source_in = picture_in - lead;
        Ok(audio_id)
    }

    /// Store a `.cube` and point this video clip at it. Clears the named LUT preset.
    pub fn import_cube(&mut self, clip_id: ClipId, text: &str) -> Result<u32> {
        let mut cube = crate::parse_cube(text).map_err(TimelineError::Message)?;
        let is_video = self
            .find_clip(clip_id)
            .map(|(_, clip)| matches!(clip.kind, ClipKind::Video { .. }))
            .ok_or(TimelineError::ClipNotFound(clip_id))?;
        if !is_video {
            return Err(TimelineError::TrackKindMismatch);
        }
        let next = self
            .cubes
            .iter()
            .map(|cube| cube.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        cube.id = next;
        self.cubes.push(cube);
        let clip = self
            .clip_mut(clip_id)
            .ok_or(TimelineError::ClipNotFound(clip_id))?;
        clip.look.grade.cube = Some(next);
        clip.look.grade.lut = Lut::None;
        Ok(next)
    }

    fn linked_audio_id(&self, video_id: ClipId, link: Option<LinkId>) -> Option<ClipId> {
        let link = link?;
        self.tracks
            .iter()
            .flat_map(|track| track.clips.iter())
            .find_map(|clip| {
                if clip.id != video_id
                    && clip.link_id == Some(link)
                    && matches!(clip.kind, ClipKind::Audio { .. })
                {
                    Some(clip.id)
                } else {
                    None
                }
            })
    }

    pub fn add_marker(&mut self, time: Time, name: impl Into<String>, color: u8) -> MarkerId {
        let id = MarkerId::new();
        self.markers.push(Marker {
            id,
            time: time.snap_to_frame(self.frame_rate),
            name: name.into(),
            color,
        });
        self.markers.sort_by_key(|m| m.time);
        id
    }

    pub fn remove_marker(&mut self, id: MarkerId) -> bool {
        let before = self.markers.len();
        self.markers.retain(|m| m.id != id);
        self.markers.len() < before
    }

    pub fn set_mark_in(&mut self, time: Option<Time>) {
        self.mark_in = time.map(|t| t.snap_to_frame(self.frame_rate));
    }

    pub fn set_mark_out(&mut self, time: Option<Time>) {
        self.mark_out = time.map(|t| t.snap_to_frame(self.frame_rate));
    }

    #[must_use]
    pub fn mark_range(&self) -> Option<(Time, Time)> {
        match (self.mark_in, self.mark_out) {
            (Some(a), Some(b)) if b > a => Some((a, b)),
            _ => None,
        }
    }

    /// Delete the in–out span and leave the hole.
    pub fn lift_marked(&mut self) -> Result<Duration> {
        let (start, end) = self.marked_span()?;
        self.lift_span(start, end)?;
        self.clear_marks();
        Ok(end - start)
    }

    /// Delete the in–out span and close the gap.
    pub fn extract_marked(&mut self) -> Result<Duration> {
        let (start, end) = self.marked_span()?;
        let span = end - start;
        self.lift_span(start, end)?;
        self.ripple_closed(end, span);
        self.clear_marks();
        Ok(span)
    }

    fn marked_span(&self) -> Result<(Time, Time)> {
        self.mark_range()
            .ok_or_else(|| TimelineError::Message("set in and out, with out after in".into()))
    }

    fn clear_marks(&mut self) {
        self.mark_in = None;
        self.mark_out = None;
    }

    fn lift_span(&mut self, start: Time, end: Time) -> Result<()> {
        let tracks: Vec<TrackId> = self
            .tracks
            .iter()
            .filter(|track| !track.locked)
            .map(|track| track.id)
            .collect();
        for track_id in tracks {
            for _ in 0..64 {
                let hit = self.track(track_id).and_then(|track| {
                    track
                        .clips
                        .iter()
                        .find(|clip| clip.start < end && clip.end() > start)
                        .map(|clip| clip.id)
                });
                let Some(id) = hit else {
                    break;
                };
                self.lift_one(id, start, end)?;
            }
        }
        Ok(())
    }

    fn lift_one(&mut self, id: ClipId, start: Time, end: Time) -> Result<()> {
        let (clip_start, clip_end) = {
            let Some((_, clip)) = self.find_clip(id) else {
                return Ok(());
            };
            (clip.start, clip.end())
        };
        if clip_end <= start || clip_start >= end {
            return Ok(());
        }
        if clip_start >= start && clip_end <= end {
            self.remove_clip(id)?;
            return Ok(());
        }
        if clip_start < start && clip_end > end {
            if let Some(middle) = self.split_inside(id, start)? {
                if self.split_inside(middle, end)?.is_some() {
                    if self.find_clip(middle).is_some() {
                        self.remove_clip(middle)?;
                    }
                } else {
                    self.keep_tail(middle, end)?;
                }
            } else {
                self.keep_head(id, clip_start, start)?;
            }
            return Ok(());
        }
        if clip_start < start {
            self.keep_head(id, clip_start, start)?;
            return Ok(());
        }
        self.keep_tail(id, end)?;
        Ok(())
    }

    fn keep_head(&mut self, id: ClipId, clip_start: Time, start: Time) -> Result<()> {
        let head = start - clip_start;
        if head.as_ticks() > 0 {
            self.trim(id, clip_start, head)
        } else {
            self.remove_clip(id)?;
            Ok(())
        }
    }

    fn keep_tail(&mut self, id: ClipId, end: Time) -> Result<()> {
        let Some((_, clip)) = self.find_clip(id) else {
            return Ok(());
        };
        let clip_end = clip.end();
        let tail = clip_end - end;
        if tail.as_ticks() > 0 {
            self.trim(id, end, tail)
        } else {
            self.remove_clip(id)?;
            Ok(())
        }
    }

    fn split_inside(&mut self, id: ClipId, at: Time) -> Result<Option<ClipId>> {
        match self.split(id, at) {
            Ok(right) => Ok(Some(right)),
            Err(TimelineError::SplitOutOfRange) => Ok(None),
            Err(err) => Err(err),
        }
    }

    fn ripple_closed(&mut self, end: Time, span: Duration) {
        if span.as_ticks() <= 0 {
            return;
        }
        for track in &mut self.tracks {
            if track.locked {
                continue;
            }
            for clip in &mut track.clips {
                if clip.start >= end {
                    clip.start -= span;
                }
            }
        }
    }
}

/// Insert and overwrite land on the in–out range when both marks are set.
#[must_use]
pub fn marked_place(
    start: Time,
    duration: Duration,
    marks: Option<(Time, Time)>,
) -> (Time, Duration) {
    let Some((mark_in, mark_out)) = marks else {
        return (start, duration);
    };
    let span = mark_out - mark_in;
    if span.as_ticks() <= 0 {
        return (start, duration);
    }
    let duration = if span.as_ticks() < duration.as_ticks() {
        span
    } else {
        duration
    };
    (mark_in, duration)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PlaceMode {
    #[default]
    Normal,
    Insert,
    Overwrite,
}

fn overlapping(
    timeline: &Timeline,
    track_id: TrackId,
    start: Time,
    end: Time,
    except: Option<ClipId>,
) -> Option<Time> {
    let track = timeline.track(track_id)?;
    let mut cursor = start;
    loop {
        let hit = track
            .clips
            .iter()
            .find(|c| except != Some(c.id) && c.start < end && c.end() > cursor);
        let Some(blocker) = hit else {
            return if cursor == start { None } else { Some(cursor) };
        };
        cursor = blocker.end();
        if cursor >= end {
            return Some(cursor);
        }
    }
}

fn next_gap(track: &crate::model::Track, at: Time) -> Option<(Time, Duration)> {
    let mut clips = track.clips.clone();
    clips.sort_by_key(|c| c.start);
    if clips.is_empty() {
        return None;
    }
    if at < clips[0].start {
        return Some((at, clips[0].start - at));
    }
    for window in clips.windows(2) {
        let a_end = window[0].end();
        let b_start = window[1].start;
        if at >= window[0].start && at < a_end {
            if b_start > a_end {
                return Some((a_end, b_start - a_end));
            }
        }
        if at >= a_end && at < b_start {
            return Some((at, b_start - at));
        }
    }
    None
}

fn clamp_time(value: Time, min: Time, max: Time) -> Time {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MediaId;
    use crate::model::{ClipKind, Transform};

    fn video(start: f64, dur: f64) -> Clip {
        Clip {
            id: ClipId::new(),
            media_id: Some(MediaId::new()),
            kind: ClipKind::Video {
                transform: Transform::default(),
            },
            start: Time::from_seconds(start),
            duration: Duration::from_seconds(dur),
            source_in: Time::ZERO,
            speed: 1.0,
            group_id: None,
            link_id: None,
            disabled: false,
            look: ClipLook::default(),
        }
    }

    #[test]
    fn roll_of_a_fast_clip_moves_source_by_the_rate() {
        let mut tl = Timeline::default();
        let track = tl.first_track(TrackKind::Video).unwrap().id;
        let mut left = video(0.0, 2.0);
        left.speed = 2.0;
        left.source_in = Time::from_seconds(10.0);
        let mut right = video(2.0, 2.0);
        right.speed = 2.0;
        right.source_in = Time::from_seconds(14.0);
        let id = tl.add_clip(track, left).unwrap();
        tl.add_clip(track, right).unwrap();
        tl.roll(id, Time::from_seconds(1.0)).unwrap();
        let right = &tl.first_track(TrackKind::Video).unwrap().clips[1];
        assert!(
            (right.source_in.as_seconds() - 12.0).abs() < 1e-2,
            "{}",
            right.source_in.as_seconds()
        );
    }

    #[test]
    fn roll_moves_join() {
        let mut tl = Timeline::default();
        let track = tl.first_track(TrackKind::Video).unwrap().id;
        let a = tl.add_clip(track, video(0.0, 2.0)).unwrap();
        let _b = tl.add_clip(track, video(2.0, 2.0)).unwrap();
        tl.roll(a, Time::from_seconds(1.0)).unwrap();
        let left = tl.find_clip(a).unwrap().1;
        assert!((left.duration.as_seconds() - 1.0).abs() < 1e-6);
        let right = &tl.first_track(TrackKind::Video).unwrap().clips[1];
        assert!((right.start.as_seconds() - 1.0).abs() < 1e-6);
        assert!((right.duration.as_seconds() - 3.0).abs() < 1e-6);
    }

    #[test]
    fn slide_keeps_span() {
        let mut tl = Timeline::default();
        let track = tl.first_track(TrackKind::Video).unwrap().id;
        let _a = tl.add_clip(track, video(0.0, 2.0)).unwrap();
        let b = tl.add_clip(track, video(2.0, 2.0)).unwrap();
        let _c = tl.add_clip(track, video(4.0, 2.0)).unwrap();
        tl.slide(b, Time::from_seconds(1.5)).unwrap();
        let clips = &tl.first_track(TrackKind::Video).unwrap().clips;
        assert!((clips[2].end().as_seconds() - 6.0).abs() < 1e-6);
        assert!((clips[1].start.as_seconds() - 1.5).abs() < 1e-6);
    }

    #[test]
    fn slip_only_moves_source() {
        let mut tl = Timeline::default();
        let track = tl.first_track(TrackKind::Video).unwrap().id;
        let id = tl.add_clip(track, video(0.0, 2.0)).unwrap();
        tl.slip(id, Time::from_seconds(1.5)).unwrap();
        let clip = tl.find_clip(id).unwrap().1;
        assert_eq!(clip.start, Time::ZERO);
        assert!((clip.source_in.as_seconds() - 1.5).abs() < 1e-6);
    }

    #[test]
    fn insert_pushes() {
        let mut tl = Timeline::default();
        let track = tl.first_track(TrackKind::Video).unwrap().id;
        let _a = tl.add_clip(track, video(0.0, 2.0)).unwrap();
        tl.place_clip(track, video(0.0, 1.0), PlaceMode::Insert)
            .unwrap();
        let clips = &tl.first_track(TrackKind::Video).unwrap().clips;
        assert_eq!(clips.len(), 2);
        assert!((clips[1].start.as_seconds() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn multicam_cut_matches_the_angle_you_were_watching() {
        let mut tl = Timeline::default();
        let v1 = tl.first_track(TrackKind::Video).unwrap().id;
        let v2 = tl.add_track(TrackKind::Video, "V2");
        let mut wide = video(0.0, 8.0);
        wide.source_in = Time::from_seconds(10.0);
        let mut tight = video(0.0, 8.0);
        tight.source_in = Time::from_seconds(2.0);
        tl.add_clip(v1, wide).unwrap();
        tl.add_clip(v2, tight).unwrap();
        tl.multicam_cut(v2, Time::from_seconds(4.0)).unwrap();
        let angle = tl
            .track(v2)
            .unwrap()
            .clips
            .iter()
            .find(|c| !c.disabled && c.contains(Time::from_seconds(4.0)))
            .unwrap();
        let into = (Time::from_seconds(4.0) - angle.start).as_seconds();
        let src = angle.source_in.as_seconds() + into;
        assert!((src - 14.0).abs() < 1e-3, "shared clock {src}");
        assert!(
            tl.track(v1)
                .unwrap()
                .clips
                .iter()
                .any(|c| c.start >= Time::from_seconds(4.0) && c.disabled)
        );
    }

    #[test]
    fn detach_keeps_the_offset_when_the_picture_moves() {
        let mut tl = Timeline::default();
        let video_track = tl.first_track(TrackKind::Video).unwrap().id;
        let picture = tl.add_clip(video_track, video(1.0, 4.0)).unwrap();
        let audio = tl.detach_audio(picture).unwrap();
        tl.trim(audio, Time::ZERO, Duration::from_seconds(5.0))
            .unwrap();
        tl.move_clip(picture, video_track, Time::from_seconds(3.0))
            .unwrap();
        let audio = tl.find_clip(audio).unwrap().1;
        assert!((audio.start.as_seconds() - 2.0).abs() < 1e-6);
        let picture = tl.find_clip(picture).unwrap().1;
        assert!((picture.start.as_seconds() - 3.0).abs() < 1e-6);
        assert_eq!(audio.link_id, picture.link_id);
    }

    #[test]
    fn split_cuts_the_linked_audio() {
        let mut tl = Timeline::default();
        let video_track = tl.first_track(TrackKind::Video).unwrap().id;
        let picture = tl.add_clip(video_track, video(0.0, 4.0)).unwrap();
        tl.detach_audio(picture).unwrap();
        tl.split(picture, Time::from_seconds(1.5)).unwrap();
        let audio = tl.first_track(TrackKind::Audio).unwrap();
        assert_eq!(audio.clips.len(), 2);
        assert!(
            (audio.clips[0].duration.as_seconds() - 1.5).abs() < 1e-6
                || (audio.clips[1].duration.as_seconds() - 1.5).abs() < 1e-6
        );
    }

    #[test]
    fn jl_cut_leads_and_trails_and_a_move_keeps_the_offset() {
        let mut tl = Timeline::default();
        let video_track = tl.first_track(TrackKind::Video).unwrap().id;
        let mut picture_clip = video(2.0, 4.0);
        picture_clip.source_in = Time::from_seconds(1.0);
        let picture = tl.add_clip(video_track, picture_clip).unwrap();
        let audio = tl
            .jl_cut(
                picture,
                Duration::from_seconds(0.5),
                Duration::from_seconds(0.25),
            )
            .unwrap();
        let audio_clip = tl.find_clip(audio).unwrap().1;
        assert!((audio_clip.start.as_seconds() - 1.5).abs() < 1e-4);
        assert!((audio_clip.duration.as_seconds() - 4.75).abs() < 1e-4);
        assert!((audio_clip.source_in.as_seconds() - 0.5).abs() < 1e-4);
        tl.move_clip(picture, video_track, Time::from_seconds(3.0))
            .unwrap();
        let audio_clip = tl.find_clip(audio).unwrap().1;
        assert!((audio_clip.start.as_seconds() - 2.5).abs() < 1e-4);
        assert!((audio_clip.duration.as_seconds() - 4.75).abs() < 1e-4);
        assert_eq!(audio_clip.link_id, tl.find_clip(picture).unwrap().1.link_id);
    }

    #[test]
    fn jl_cut_clamps_the_lead_inside_the_source() {
        let mut tl = Timeline::default();
        let video_track = tl.first_track(TrackKind::Video).unwrap().id;
        let mut picture_clip = video(0.2, 2.0);
        picture_clip.source_in = Time::from_seconds(0.1);
        let picture = tl.add_clip(video_track, picture_clip).unwrap();
        let audio = tl
            .jl_cut(picture, Duration::from_seconds(5.0), Duration::ZERO)
            .unwrap();
        let audio_clip = tl.find_clip(audio).unwrap().1;
        assert!((audio_clip.source_in.as_seconds()).abs() < 1e-4);
        assert!((audio_clip.start.as_seconds() - 0.1).abs() < 1e-4);
        assert!((audio_clip.duration.as_seconds() - 2.1).abs() < 1e-4);
    }

    #[test]
    fn import_cube_sets_the_grade_and_frame_rate_sticks() {
        let mut tl = Timeline::default();
        tl.set_frame_rate(oc_time::FrameRate::FPS_24);
        tl.set_background("charcoal");
        assert_eq!(tl.frame_rate, oc_time::FrameRate::FPS_24);
        assert_eq!(tl.background, "#1a1a1a");
        let track = tl.first_track(TrackKind::Video).unwrap().id;
        let id = tl.add_clip(track, video(0.0, 1.0)).unwrap();
        let cube = tl
            .import_cube(
                id,
                "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n",
            )
            .unwrap();
        assert_eq!(cube, 1);
        let grade = tl.find_clip(id).unwrap().1.look.grade;
        assert_eq!(grade.cube, Some(1));
        assert_eq!(grade.lut, crate::Lut::None);
        assert_eq!(tl.cubes.len(), 1);
    }

    #[test]
    fn overwrite_covers() {
        let mut tl = Timeline::default();
        let track = tl.first_track(TrackKind::Video).unwrap().id;
        let _a = tl.add_clip(track, video(0.0, 4.0)).unwrap();
        tl.place_clip(track, video(1.0, 1.0), PlaceMode::Overwrite)
            .unwrap();
        let clips = &tl.first_track(TrackKind::Video).unwrap().clips;
        assert_eq!(clips.len(), 3);
    }

    #[test]
    fn mark_range_needs_both_marks_in_order() {
        let mut tl = Timeline::default();
        assert!(tl.mark_range().is_none());
        tl.set_mark_in(Some(Time::from_seconds(1.0)));
        assert!(tl.mark_range().is_none());
        tl.set_mark_out(Some(Time::from_seconds(1.0)));
        assert!(tl.mark_range().is_none());
        tl.set_mark_out(Some(Time::from_seconds(0.5)));
        assert!(tl.mark_range().is_none());
        tl.set_mark_out(Some(Time::from_seconds(3.0)));
        let (start, end) = tl.mark_range().unwrap();
        assert_eq!(start, Time::from_seconds(1.0));
        assert_eq!(end, Time::from_seconds(3.0));
    }

    #[test]
    fn extract_closes_the_marked_span_and_lift_leaves_it() {
        let mut lifted = Timeline::default();
        let track = lifted.first_track(TrackKind::Video).unwrap().id;
        lifted.add_clip(track, video(0.0, 4.0)).unwrap();
        lifted.set_mark_in(Some(Time::from_seconds(1.0)));
        lifted.set_mark_out(Some(Time::from_seconds(2.0)));
        lifted.lift_marked().unwrap();
        assert!(lifted.mark_range().is_none());
        let clips = &lifted.first_track(TrackKind::Video).unwrap().clips;
        assert_eq!(clips.len(), 2);
        assert!((clips[0].duration.as_seconds() - 1.0).abs() < 1e-3);
        assert!((clips[1].start.as_seconds() - 2.0).abs() < 1e-3);
        assert!((clips[1].duration.as_seconds() - 2.0).abs() < 1e-3);

        let mut extracted = Timeline::default();
        let track = extracted.first_track(TrackKind::Video).unwrap().id;
        extracted.add_clip(track, video(0.0, 4.0)).unwrap();
        extracted.set_mark_in(Some(Time::from_seconds(1.0)));
        extracted.set_mark_out(Some(Time::from_seconds(2.0)));
        extracted.extract_marked().unwrap();
        let clips = &extracted.first_track(TrackKind::Video).unwrap().clips;
        assert_eq!(clips.len(), 2);
        assert!((clips[0].end().as_seconds() - 1.0).abs() < 1e-3);
        assert!((clips[1].start.as_seconds() - 1.0).abs() < 1e-3);
        assert!((clips[1].duration.as_seconds() - 2.0).abs() < 1e-3);
    }

    #[test]
    fn insert_and_overwrite_use_the_marked_span() {
        let (start, duration) = marked_place(
            Time::ZERO,
            Duration::from_seconds(8.0),
            Some((Time::from_seconds(1.0), Time::from_seconds(2.5))),
        );
        assert_eq!(start, Time::from_seconds(1.0));
        assert!((duration.as_seconds() - 1.5).abs() < 1e-6);

        let mut tl = Timeline::default();
        let track = tl.first_track(TrackKind::Video).unwrap().id;
        tl.add_clip(track, video(0.0, 4.0)).unwrap();
        tl.set_mark_in(Some(Time::from_seconds(1.0)));
        tl.set_mark_out(Some(Time::from_seconds(2.0)));
        tl.place_clip(track, video(0.0, 5.0), PlaceMode::Overwrite)
            .unwrap();
        let covering = tl
            .first_track(TrackKind::Video)
            .unwrap()
            .clips
            .iter()
            .find(|clip| (clip.start.as_seconds() - 1.0).abs() < 1e-3)
            .unwrap();
        assert!((covering.duration.as_seconds() - 1.0).abs() < 1e-3);
    }
}
