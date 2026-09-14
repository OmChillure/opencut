use crate::{TICKS_PER_SECOND, TimeError};
use serde::{Deserialize, Serialize};

/// Rational frame rate `{numerator / denominator}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FrameRate {
    pub numerator: u32,
    pub denominator: u32,
}

impl FrameRate {
    pub const FPS_24: Self = Self { numerator: 24, denominator: 1 };
    pub const FPS_25: Self = Self { numerator: 25, denominator: 1 };
    pub const FPS_30: Self = Self { numerator: 30, denominator: 1 };
    pub const FPS_50: Self = Self { numerator: 50, denominator: 1 };
    pub const FPS_60: Self = Self { numerator: 60, denominator: 1 };
    pub const FPS_23_976: Self = Self {
        numerator: 24_000,
        denominator: 1_001,
    };
    pub const FPS_29_97: Self = Self {
        numerator: 30_000,
        denominator: 1_001,
    };
    pub const FPS_59_94: Self = Self {
        numerator: 60_000,
        denominator: 1_001,
    };

    pub fn new(numerator: u32, denominator: u32) -> Result<Self, TimeError> {
        if numerator == 0 || denominator == 0 {
            return Err(TimeError::InvalidFrameRate);
        }
        Ok(Self {
            numerator,
            denominator,
        })
    }

    /// Exact ticks per frame for this rate.
    #[must_use]
    pub fn ticks_per_frame(self) -> i64 {
        let num = i64::from(self.numerator);
        let den = i64::from(self.denominator);
        (TICKS_PER_SECOND * den) / num
    }

    #[must_use]
    pub fn as_f64(self) -> f64 {
        f64::from(self.numerator) / f64::from(self.denominator)
    }

    #[must_use]
    pub const fn ticks_per_second_f64(self) -> f64 {
        TICKS_PER_SECOND as f64
    }
}

impl Default for FrameRate {
    fn default() -> Self {
        Self::FPS_30
    }
}
