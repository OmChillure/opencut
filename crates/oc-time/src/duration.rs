use crate::{TICKS_PER_SECOND, Time, TimeError};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::{Add, AddAssign, Div, Mul, Sub, SubAssign};

/// A signed span on the media clock.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Duration(i64);

impl Duration {
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
        Self(ms.saturating_mul(TICKS_PER_SECOND) / 1_000)
    }

    pub fn try_from_ticks_non_negative(ticks: i64) -> Result<Self, TimeError> {
        if ticks < 0 {
            return Err(TimeError::NegativeDuration);
        }
        Ok(Self(ticks))
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
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    #[must_use]
    pub const fn is_negative(self) -> bool {
        self.0 < 0
    }

    #[must_use]
    pub const fn saturating_sub(self, other: Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }

    #[must_use]
    pub const fn min(self, other: Self) -> Self {
        if self.0 <= other.0 { self } else { other }
    }

    #[must_use]
    pub const fn max(self, other: Self) -> Self {
        if self.0 >= other.0 { self } else { other }
    }

    #[must_use]
    pub fn after(self, start: Time) -> Time {
        start + self
    }
}

impl Add for Duration {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl AddAssign for Duration {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl Sub for Duration {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl SubAssign for Duration {
    fn sub_assign(&mut self, rhs: Self) {
        self.0 -= rhs.0;
    }
}

impl Mul<i64> for Duration {
    type Output = Self;
    fn mul(self, rhs: i64) -> Self::Output {
        Self(self.0 * rhs)
    }
}

impl Div<i64> for Duration {
    type Output = Self;
    fn div(self, rhs: i64) -> Self::Output {
        Self(self.0 / rhs)
    }
}

impl fmt::Display for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.3}s", self.as_seconds())
    }
}
