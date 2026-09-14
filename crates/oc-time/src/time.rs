use crate::{Duration, FrameRate, TICKS_PER_SECOND};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::{Add, AddAssign, Sub, SubAssign};

/// A point on the media clock, stored as an integer tick count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Time(i64);

impl Time {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn from_ticks(ticks: i64) -> Self {
        Self(ticks)
    }

    #[must_use]
    pub fn from_seconds(seconds: f64) -> Self {
        Self((seconds * TICKS_PER_SECOND as f64).round() as i64)
    }

    #[must_use]
    pub fn from_millis(ms: i64) -> Self {
        Self::from_ticks(ms.saturating_mul(TICKS_PER_SECOND) / 1_000)
    }

    #[must_use]
    pub const fn as_ticks(self) -> i64 {
        self.0
    }

    #[must_use]
    pub fn as_seconds(self) -> f64 {
        self.0 as f64 / TICKS_PER_SECOND as f64
    }

    #[must_use]
    pub fn as_millis(self) -> i64 {
        self.0.saturating_mul(1_000) / TICKS_PER_SECOND
    }

    #[must_use]
    pub fn saturating_add(self, duration: Duration) -> Self {
        Self(self.0.saturating_add(duration.as_ticks()))
    }

    #[must_use]
    pub fn saturating_sub(self, duration: Duration) -> Self {
        Self(self.0.saturating_sub(duration.as_ticks()))
    }

    #[must_use]
    pub fn duration_since(self, earlier: Self) -> Duration {
        Duration::from_ticks(self.0.saturating_sub(earlier.0).max(0))
    }

    #[must_use]
    pub fn snap_to_frame(self, rate: FrameRate) -> Self {
        let tpf = rate.ticks_per_frame();
        if tpf <= 0 {
            return self;
        }
        let half = tpf / 2;
        let q = self.0.saturating_add(half).div_euclid(tpf);
        Self(q.saturating_mul(tpf))
    }

    #[must_use]
    pub fn frame_index(self, rate: FrameRate) -> i64 {
        let tpf = rate.ticks_per_frame();
        if tpf <= 0 {
            return 0;
        }
        self.0.div_euclid(tpf)
    }

    /// Non-drop timecode `HH:MM:SS:FF`.
    #[must_use]
    pub fn timecode(self, rate: FrameRate) -> String {
        let fps = rate.ticks_per_second_f64() / rate.ticks_per_frame() as f64;
        let fps_i = fps.round().max(1.0) as i64;
        let total_frames = self.frame_index(rate).max(0);
        let ff = total_frames % fps_i;
        let total_secs = total_frames / fps_i;
        let ss = total_secs % 60;
        let total_mins = total_secs / 60;
        let mm = total_mins % 60;
        let hh = total_mins / 60;
        format!("{hh:02}:{mm:02}:{ss:02}:{ff:02}")
    }
}

impl Add<Duration> for Time {
    type Output = Self;
    fn add(self, rhs: Duration) -> Self::Output {
        Self(self.0 + rhs.as_ticks())
    }
}

impl AddAssign<Duration> for Time {
    fn add_assign(&mut self, rhs: Duration) {
        self.0 += rhs.as_ticks();
    }
}

impl Sub<Duration> for Time {
    type Output = Self;
    fn sub(self, rhs: Duration) -> Self::Output {
        Self(self.0 - rhs.as_ticks())
    }
}

impl SubAssign<Duration> for Time {
    fn sub_assign(&mut self, rhs: Duration) {
        self.0 -= rhs.as_ticks();
    }
}

impl Sub for Time {
    type Output = Duration;
    fn sub(self, rhs: Self) -> Self::Output {
        Duration::from_ticks(self.0 - rhs.0)
    }
}

impl fmt::Display for Time {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.timecode(FrameRate::FPS_30))
    }
}
