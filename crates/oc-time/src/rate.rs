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

    /// Next rate in the settings cycle: 23.976, 24, 25, 29.97, 30, 50, 59.94, 60.
    #[must_use]
    pub fn cycle(self) -> Self {
        let rates = Self::common();
        let i = rates.iter().position(|rate| *rate == self).unwrap_or(0);
        rates[(i + 1) % rates.len()]
    }

    /// Closest common rate. Used when a tool passes a plain fps number.
    #[must_use]
    pub fn nearest(fps: f64) -> Self {
        Self::common()
            .into_iter()
            .min_by(|a, b| {
                let da = (a.as_f64() - fps).abs();
                let db = (b.as_f64() - fps).abs();
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or(Self::FPS_30)
    }

    #[must_use]
    pub fn label(self) -> String {
        match (self.numerator, self.denominator) {
            (24, 1) => "24".into(),
            (25, 1) => "25".into(),
            (30, 1) => "30".into(),
            (50, 1) => "50".into(),
            (60, 1) => "60".into(),
            (24_000, 1_001) => "23.976".into(),
            (30_000, 1_001) => "29.97".into(),
            (60_000, 1_001) => "59.94".into(),
            _ => format!("{:.3}", self.as_f64()),
        }
    }

    #[must_use]
    pub fn common() -> [Self; 8] {
        [
            Self::FPS_23_976,
            Self::FPS_24,
            Self::FPS_25,
            Self::FPS_29_97,
            Self::FPS_30,
            Self::FPS_50,
            Self::FPS_59_94,
            Self::FPS_60,
        ]
    }
}

impl Default for FrameRate {
    fn default() -> Self {
        Self::FPS_30
    }
}
