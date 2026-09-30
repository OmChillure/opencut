//! Adobe `.cube` 3D LUTs. Samples are stored with R changing fastest, then G, then B.

use serde::{Deserialize, Serialize};

/// One loaded look-up table. `id` is assigned when it is stored on a timeline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CubeLut {
    pub id: u32,
    pub name: String,
    pub size: u32,
    /// `size³` RGB triples, R fastest.
    pub rgb: Vec<f32>,
}

impl CubeLut {
    /// Trilinear sample. Channels are 0–1.
    #[must_use]
    pub fn sample(&self, r: f32, g: f32, b: f32) -> [f32; 3] {
        if self.size < 2 || self.rgb.len() < (self.size as usize).pow(3) * 3 {
            return [r, g, b];
        }
        let last = (self.size - 1) as f32;
        let rf = (r.clamp(0.0, 1.0) * last).clamp(0.0, last);
        let gf = (g.clamp(0.0, 1.0) * last).clamp(0.0, last);
        let bf = (b.clamp(0.0, 1.0) * last).clamp(0.0, last);
        let r0 = rf.floor() as u32;
        let g0 = gf.floor() as u32;
        let b0 = bf.floor() as u32;
        let r1 = (r0 + 1).min(self.size - 1);
        let g1 = (g0 + 1).min(self.size - 1);
        let b1 = (b0 + 1).min(self.size - 1);
        let tr = rf - r0 as f32;
        let tg = gf - g0 as f32;
        let tb = bf - b0 as f32;
        let c000 = self.at(r0, g0, b0);
        let c100 = self.at(r1, g0, b0);
        let c010 = self.at(r0, g1, b0);
        let c110 = self.at(r1, g1, b0);
        let c001 = self.at(r0, g0, b1);
        let c101 = self.at(r1, g0, b1);
        let c011 = self.at(r0, g1, b1);
        let c111 = self.at(r1, g1, b1);
        let c00 = lerp3(c000, c100, tr);
        let c10 = lerp3(c010, c110, tr);
        let c01 = lerp3(c001, c101, tr);
        let c11 = lerp3(c011, c111, tr);
        let c0 = lerp3(c00, c10, tg);
        let c1 = lerp3(c01, c11, tg);
        lerp3(c0, c1, tb)
    }

    fn at(&self, r: u32, g: u32, b: u32) -> [f32; 3] {
        let n = self.size;
        let i = ((b * n * n + g * n + r) * 3) as usize;
        [self.rgb[i], self.rgb[i + 1], self.rgb[i + 2]]
    }
}

/// Parse a `.cube` file. `id` is 0 until the timeline stores it.
pub fn parse_cube(text: &str) -> Result<CubeLut, String> {
    let mut name = String::from("LUT");
    let mut size: Option<u32> = None;
    let mut rgb = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let upper = line.to_ascii_uppercase();
        if upper.starts_with("TITLE") {
            name = title_of(line);
            continue;
        }
        if upper.starts_with("LUT_3D_SIZE") {
            let n: u32 = line
                .split_whitespace()
                .nth(1)
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| "bad LUT_3D_SIZE".to_string())?;
            if !(2..=64).contains(&n) {
                return Err(format!("LUT size {n} is outside 2..=64"));
            }
            size = Some(n);
            continue;
        }
        if upper.starts_with("LUT_1D_SIZE") {
            return Err("1D cubes are not supported".into());
        }
        if upper.starts_with("DOMAIN_") || upper.starts_with("LUT_3D_INPUT_RANGE") {
            continue;
        }
        let mut parts = line.split_whitespace();
        let r = parse_channel(parts.next())?;
        let g = parse_channel(parts.next())?;
        let b = parse_channel(parts.next())?;
        rgb.extend([r, g, b]);
    }
    let size = size.ok_or_else(|| "missing LUT_3D_SIZE".to_string())?;
    let expect = (size as usize).saturating_pow(3) * 3;
    if rgb.len() != expect {
        return Err(format!(
            "expected {} samples, got {}",
            expect / 3,
            rgb.len() / 3
        ));
    }
    Ok(CubeLut {
        id: 0,
        name,
        size,
        rgb,
    })
}

/// Text ffmpeg `lut3d` can read back.
#[must_use]
pub fn cube_text(lut: &CubeLut) -> String {
    let mut out = format!(
        "TITLE \"{}\"\nLUT_3D_SIZE {}\n",
        lut.name.replace('"', ""),
        lut.size
    );
    for triple in lut.rgb.chunks(3) {
        if triple.len() == 3 {
            out.push_str(&format!(
                "{:.6} {:.6} {:.6}\n",
                triple[0], triple[1], triple[2]
            ));
        }
    }
    out
}

fn title_of(line: &str) -> String {
    if let Some(start) = line.find('"') {
        if let Some(end) = line[start + 1..].find('"') {
            let name = line[start + 1..start + 1 + end].trim();
            if !name.is_empty() {
                return name.to_string();
            }
        }
    }
    let rest = line
        .split_once(char::is_whitespace)
        .map(|(_, rest)| rest.trim().trim_matches('"'))
        .unwrap_or("LUT");
    if rest.is_empty() {
        "LUT".into()
    } else {
        rest.to_string()
    }
}

fn parse_channel(raw: Option<&str>) -> Result<f32, String> {
    let raw = raw.ok_or_else(|| "sample needs three numbers".to_string())?;
    raw.parse::<f32>()
        .map_err(|_| format!("bad sample {raw}"))
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// `#rrggbb` for CSS. Unknown text becomes black.
#[must_use]
pub fn canonical_color(color: &str) -> String {
    match color.trim().to_ascii_lowercase().as_str() {
        "black" => return "#000000".into(),
        "white" => return "#ffffff".into(),
        "gray" | "grey" => return "#808080".into(),
        "charcoal" => return "#1a1a1a".into(),
        _ => {}
    }
    let hex = color
        .trim()
        .trim_start_matches('#')
        .trim_start_matches("0x")
        .trim_start_matches("0X");
    let expanded = if hex.len() == 3 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        let mut out = String::with_capacity(6);
        for c in hex.chars() {
            out.push(c);
            out.push(c);
        }
        out
    } else {
        hex.to_string()
    };
    if expanded.len() == 6 && expanded.chars().all(|c| c.is_ascii_hexdigit()) {
        format!("#{}", expanded.to_ascii_lowercase())
    } else {
        "#000000".into()
    }
}

/// ffmpeg `0xRRGGBB`.
#[must_use]
pub fn ffmpeg_color(color: &str) -> String {
    let hex = canonical_color(color);
    format!("0x{}", &hex[1..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_size_two_samples_the_corners() {
        let text = "\
TITLE \"identity\"
# comment
DOMAIN_MIN 0.0 0.0 0.0
DOMAIN_MAX 1.0 1.0 1.0
LUT_3D_SIZE 2
0 0 0
1 0 0
0 1 0
1 1 0
0 0 1
1 0 1
0 1 1
1 1 1
";
        let lut = parse_cube(text).unwrap();
        assert_eq!(lut.size, 2);
        assert_eq!(lut.name, "identity");
        let black = lut.sample(0.0, 0.0, 0.0);
        assert!(black.iter().all(|c| c.abs() < 1e-4));
        let red = lut.sample(1.0, 0.0, 0.0);
        assert!((red[0] - 1.0).abs() < 1e-4 && red[1].abs() < 1e-4);
        let mid = lut.sample(0.5, 0.0, 0.0);
        assert!((mid[0] - 0.5).abs() < 1e-3);
        let gray = lut.sample(0.5, 0.5, 0.5);
        assert!(gray.iter().all(|c| (*c - 0.5).abs() < 1e-3));
        let white = lut.sample(1.0, 1.0, 1.0);
        assert!(white.iter().all(|c| (*c - 1.0).abs() < 1e-4));
        let again = parse_cube(&cube_text(&lut)).unwrap();
        assert_eq!(again.rgb.len(), lut.rgb.len());
    }

    #[test]
    fn rejects_a_short_cube() {
        let err = parse_cube("LUT_3D_SIZE 2\n0 0 0\n").unwrap_err();
        assert!(err.contains("expected"));
    }

    #[test]
    fn color_names_and_short_hex() {
        assert_eq!(canonical_color("Charcoal"), "#1a1a1a");
        assert_eq!(canonical_color("#ABC"), "#aabbcc");
        assert_eq!(ffmpeg_color("white"), "0xffffff");
        assert_eq!(canonical_color("nope"), "#000000");
    }
}
