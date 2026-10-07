use crate::TICKS_PER_SECOND;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::{Add, Sub};

/// A signed span on the media clock.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
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
    pub const fn as_ticks(self) -> i64 {
        self.0
    }

    #[must_use]
    pub fn as_seconds(self) -> f64 {
        self.0 as f64 / TICKS_PER_SECOND as f64
    }
}

impl Add for Duration {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl Sub for Duration {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl fmt::Display for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.3}s", self.as_seconds())
    }
}
