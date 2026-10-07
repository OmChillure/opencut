//! Frame-accurate media clock.
//!
//! One second is exactly [`TICKS_PER_SECOND`] ticks (120_000). That rate
//! divides evenly by every common frame-rate denominator, including
//! drop-frame families:
//!
//! | Rate    | Ticks / frame |
//! |---------|---------------|
//! | 24      | 5_000         |
//! | 23.976  | 5_005         |
//! | 25      | 4_800         |
//! | 30      | 4_000         |
//! | 29.97   | 4_004         |
//! | 60      | 2_000         |
//! | 59.94   | 2_002         |

mod duration;
mod rate;
mod time;

pub use duration::Duration;
pub use rate::FrameRate;
pub use time::Time;

/// Ticks in one second of media time.
pub const TICKS_PER_SECOND: i64 = 120_000;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum TimeError {
    #[error("frame rate numerator and denominator must be > 0")]
    InvalidFrameRate,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_per_frame_table() {
        assert_eq!(FrameRate::FPS_24.ticks_per_frame(), 5_000);
        assert_eq!(FrameRate::FPS_23_976.ticks_per_frame(), 5_005);
        assert_eq!(FrameRate::FPS_25.ticks_per_frame(), 4_800);
        assert_eq!(FrameRate::FPS_30.ticks_per_frame(), 4_000);
        assert_eq!(FrameRate::FPS_29_97.ticks_per_frame(), 4_004);
        assert_eq!(FrameRate::FPS_60.ticks_per_frame(), 2_000);
        assert_eq!(FrameRate::FPS_59_94.ticks_per_frame(), 2_002);
    }

    #[test]
    fn second_roundtrip() {
        let t = Time::from_seconds(1.5);
        assert_eq!(t.as_ticks(), 180_000);
        assert!((t.as_seconds() - 1.5).abs() < f64::EPSILON);
    }

    #[test]
    fn snap_to_frame() {
        let t = Time::from_ticks(7_000);
        assert_eq!(t.snap_to_frame(FrameRate::FPS_24).as_ticks(), 5_000);
        let t = Time::from_ticks(8_000);
        assert_eq!(t.snap_to_frame(FrameRate::FPS_24).as_ticks(), 10_000);
    }

    #[test]
    fn timecode_24fps() {
        let t = Time::from_ticks(FrameRate::FPS_24.ticks_per_frame() * (24 * 3661 + 7));
        assert_eq!(t.timecode(FrameRate::FPS_24), "01:01:01:07");
    }

    #[test]
    fn add_sub() {
        let a = Time::from_seconds(2.0);
        let d = Duration::from_seconds(0.5);
        assert_eq!((a + d).as_seconds(), 2.5);
        assert_eq!((a - d).as_seconds(), 1.5);
        assert_eq!((a - Time::from_seconds(0.5)).as_seconds(), 1.5);
    }

    #[test]
    fn serde_ticks() {
        let t = Time::from_ticks(1234);
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(json, "1234");
        let back: Time = serde_json::from_str(&json).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn nearest_rate_cycles_and_rejects_zero() {
        assert_eq!(FrameRate::nearest(23.976), FrameRate::FPS_23_976);
        assert_eq!(FrameRate::nearest(30.0), FrameRate::FPS_30);
        assert_eq!(FrameRate::FPS_60.cycle(), FrameRate::FPS_23_976);
        assert_eq!(FrameRate::FPS_24.label(), "24");
        assert_eq!(FrameRate::FPS_29_97.label(), "29.97");
        assert!(FrameRate::new(0, 1).is_err());
        assert!(FrameRate::new(24, 0).is_err());
    }

    #[test]
    fn duration_adds_and_prints_seconds() {
        let span = Duration::from_seconds(1.5) + Duration::from_seconds(0.5);
        assert_eq!(span.as_ticks(), 240_000);
        assert_eq!(span.to_string(), "2.000s");
        assert_eq!((span - Duration::from_seconds(0.5)).as_seconds(), 1.5);
    }
}
