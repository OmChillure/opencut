//! Picture frame. Phone files store a sideways coded size plus a display matrix.

/// Snap an upright picture to the editor presets: 9:16, 1:1, 4:3, 16:9.
///
/// `rotation` is degrees. 90 and 270 swap width and height. Pass 0 when
/// `width` and `height` are already the upright displayed size.
#[must_use]
pub fn frame_for_picture(width: u32, height: u32, rotation: i32) -> (u32, u32) {
    let mut w = width.max(1);
    let mut h = height.max(1);
    let turns = rotation.rem_euclid(360);
    if turns == 90 || turns == 270 {
        std::mem::swap(&mut w, &mut h);
    }
    let ratio = w as f64 / h as f64;
    if ratio < 0.92 {
        (1080, 1920)
    } else if ratio < 1.08 {
        (1080, 1080)
    } else if ratio < 1.5 {
        (1440, 1080)
    } else {
        (1920, 1080)
    }
}

/// Display size from an MP4, or from a head slice joined to a tail slice.
///
/// The first video `tkhd` wins. A zero-size track (audio) is skipped. A quarter
/// turn in the display matrix swaps the coded size. The last matrix cell is
/// ignored: Apple stores 1.0 there as 2.30, not 16.16.
#[must_use]
pub fn mp4_display_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut from = 0;
    while from + 4 <= bytes.len() {
        let Some(rel) = bytes[from..]
            .windows(4)
            .position(|window| window == b"tkhd")
        else {
            break;
        };
        let at = from + rel;
        if let Some(size) = tkhd_display(&bytes[at + 4..]) {
            return Some(size);
        }
        from = at + 4;
    }
    None
}

fn tkhd_display(body: &[u8]) -> Option<(u32, u32)> {
    let version = *body.first()?;
    let (matrix_at, wh_at) = match version {
        0 => (40usize, 76usize),
        1 => (52, 88),
        _ => return None,
    };
    if body.len() < wh_at + 8 {
        return None;
    }
    let a = i32::from_be_bytes(body[matrix_at..matrix_at + 4].try_into().ok()?);
    let b = i32::from_be_bytes(body[matrix_at + 4..matrix_at + 8].try_into().ok()?);
    let c = i32::from_be_bytes(body[matrix_at + 12..matrix_at + 16].try_into().ok()?);
    let d = i32::from_be_bytes(body[matrix_at + 16..matrix_at + 20].try_into().ok()?);
    let mut w = fixed_px(&body[wh_at..wh_at + 4])?;
    let mut h = fixed_px(&body[wh_at + 4..wh_at + 8])?;
    if quarter_turn(a, b, c, d) {
        std::mem::swap(&mut w, &mut h);
    }
    Some((w, h))
}

fn fixed_px(bytes: &[u8]) -> Option<u32> {
    let raw = u32::from_be_bytes(bytes.try_into().ok()?);
    let px = u64::from(raw) / 65536;
    if (2..16385).contains(&px) {
        Some(px as u32)
    } else {
        None
    }
}

fn quarter_turn(a: i32, b: i32, c: i32, d: i32) -> bool {
    near_zero(a) && near_zero(d) && near_one(b) && near_one(c)
}

fn near_zero(value: i32) -> bool {
    value.unsigned_abs() < 13_107
}

fn near_one(value: i32) -> bool {
    value.unsigned_abs().abs_diff(65_536) < 13_107
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tkhd(version: u8, matrix: [i32; 9], width: u32, height: u32) -> Vec<u8> {
        let (matrix_at, wh_at, len) = if version == 1 {
            (52, 88, 96)
        } else {
            (40, 76, 84)
        };
        let mut body = vec![0u8; len];
        body[0] = version;
        for (i, value) in matrix.iter().enumerate() {
            body[matrix_at + i * 4..matrix_at + i * 4 + 4].copy_from_slice(&value.to_be_bytes());
        }
        let wide = u64::from(width) * 65536;
        let high = u64::from(height) * 65536;
        body[wh_at..wh_at + 4].copy_from_slice(&(wide as u32).to_be_bytes());
        body[wh_at + 4..wh_at + 8].copy_from_slice(&(high as u32).to_be_bytes());
        let mut bytes = b"----ftypjunk".to_vec();
        bytes.extend_from_slice(b"tkhd");
        bytes.extend(body);
        bytes
    }

    const IDENTITY: [i32; 9] = [65536, 0, 0, 0, 65536, 0, 0, 0, 0x4000_0000];
    const TURN_LEFT: [i32; 9] = [0, 65536, 0, -65536, 0, 0, 0, 0, 0x4000_0000];

    #[test]
    fn presets_follow_the_upright_picture() {
        assert_eq!(frame_for_picture(3840, 2160, 0), (1920, 1080));
        assert_eq!(frame_for_picture(3840, 2160, -90), (1080, 1920));
        assert_eq!(frame_for_picture(2160, 3840, 0), (1080, 1920));
        assert_eq!(frame_for_picture(2160, 3840, 90), (1920, 1080));
        assert_eq!(frame_for_picture(1080, 1080, 0), (1080, 1080));
        assert_eq!(frame_for_picture(1440, 1080, 0), (1440, 1080));
        assert_eq!(frame_for_picture(4000, 3000, 0), (1440, 1080));
    }

    #[test]
    fn a_sideways_tkhd_reports_the_upright_size() {
        let bytes = tkhd(0, TURN_LEFT, 3840, 2160);
        assert_eq!(mp4_display_size(&bytes), Some((2160, 3840)));
        assert_eq!(frame_for_picture(2160, 3840, 0), (1080, 1920));
    }

    #[test]
    fn an_audio_track_is_skipped_and_version_one_still_reads() {
        let mut bytes = tkhd(0, IDENTITY, 0, 0);
        bytes.extend(tkhd(1, IDENTITY, 1920, 1080));
        assert_eq!(mp4_display_size(&bytes), Some((1920, 1080)));
    }

    #[test]
    fn a_joined_head_and_tail_still_finds_tkhd() {
        let track = tkhd(0, TURN_LEFT, 3840, 2160);
        let mut bytes = vec![0u8; 64];
        bytes.extend_from_slice(b"mdat");
        bytes.extend_from_slice(&track);
        assert_eq!(mp4_display_size(&bytes), Some((2160, 3840)));
    }

    #[test]
    fn the_phone_clip_is_portrait_when_it_is_on_disk() {
        let path = std::path::Path::new(
            "/home/omchillure/Downloads/IIITN-ReelClips-20261003T031556Z-1-001/IIITN-ReelClips/20260926_163655.mp4",
        );
        let Ok(mut file) = std::fs::File::open(path) else {
            return;
        };
        use std::io::{Read, Seek, SeekFrom};
        let Ok(len) = file.metadata().map(|meta| meta.len()) else {
            return;
        };
        if len < 64 {
            return;
        }
        let head_len = 1_048_576.min(len as usize);
        let mut bytes = vec![0u8; head_len];
        if file.read_exact(&mut bytes).is_err() {
            return;
        }
        let tail_len = 8_388_608.min(len as usize);
        if (tail_len as u64) < len {
            let mut tail = vec![0u8; tail_len];
            if file
                .seek(SeekFrom::End(-(tail_len as i64)))
                .and_then(|_| file.read_exact(&mut tail))
                .is_err()
            {
                return;
            }
            bytes.extend(tail);
        }
        let (width, height) = mp4_display_size(&bytes).expect("tkhd");
        assert!(height > width, "{width}x{height}");
        assert_eq!(frame_for_picture(width, height, 0), (1080, 1920));
    }
}
