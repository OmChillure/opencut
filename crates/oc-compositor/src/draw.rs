//! Draw one planned frame into straight RGBA. Callers supply each decoded picture.

use oc_timeline::{
    AlphaShape, CubeLut, CurvePoint, Curves, FrameCard, Fx, Generator, Grade, Lut, MaskShape,
    MediaId, Transform, TransitionKind,
};

use crate::{FramePlan, Layer};

#[derive(Clone, Debug)]
pub struct FrameSource {
    pub media_id: MediaId,
    /// Seconds into the file this picture was decoded from.
    pub source_time: f64,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct Surface {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[must_use]
pub fn composite(plan: &FramePlan, sources: &[FrameSource], cubes: &[CubeLut]) -> Surface {
    let width = plan.width.max(2);
    let height = plan.height.max(2);
    let mut dst = vec![0u8; (width as usize) * (height as usize) * 4];
    fill(&mut dst, parse_hex(&plan.background));

    let mut index = 0;
    while index < plan.layers.len() {
        let layer = &plan.layers[index];
        let Layer::Video {
            incoming, overlay, ..
        } = layer
        else {
            index += 1;
            continue;
        };
        if *incoming {
            index += 1;
            continue;
        }
        if !*overlay
            && matches!(
                plan.layers.get(index + 1),
                Some(Layer::Video { incoming: true, .. })
            )
        {
            paint_mix(
                &mut dst,
                width,
                height,
                layer,
                &plan.layers[index + 1],
                sources,
                cubes,
            );
            index += 2;
            continue;
        }
        paint_over(&mut dst, width, height, layer, sources, cubes);
        index += 1;
    }

    if plan.letterbox {
        let bar = ((height as f32) * 0.12).round() as u32;
        let bg = parse_hex(&plan.background);
        for y in 0..height {
            if y >= bar && y + bar < height {
                continue;
            }
            for x in 0..width {
                put(&mut dst, width, x, y, bg, 1.0);
            }
        }
    }

    Surface {
        width,
        height,
        rgba: dst,
    }
}

fn paint_mix(
    dst: &mut [u8],
    width: u32,
    height: u32,
    outgoing: &Layer,
    incoming: &Layer,
    sources: &[FrameSource],
    cubes: &[CubeLut],
) {
    let Layer::Video {
        transition: kind,
        mix,
        ..
    } = outgoing
    else {
        return;
    };
    let kind = *kind;
    let p = mix.clamp(0.0, 1.0);
    let slide = slide_shift(kind, p, width, height);
    let mosaic = kind == TransitionKind::Pixelize;
    let (cell_w, cell_h) = mosaic_cell(p, width, height);
    for y in 0..height {
        for x in 0..width {
            let (sx, sy) = if mosaic {
                let cx = (x / cell_w) * cell_w + cell_w / 2;
                let cy = (y / cell_h) * cell_h + cell_h / 2;
                (cx.min(width - 1) as f32, cy.min(height - 1) as f32)
            } else {
                (x as f32, y as f32)
            };
            let (ox, oy, ix, iy) =
                slide.map_or((0.0, 0.0, 0.0, 0.0), |(o, i)| (o.0, o.1, i.0, i.1));
            let out = sample_layer(outgoing, sx - ox, sy - oy, width, height, sources, cubes);
            let inc = sample_layer(incoming, sx - ix, sy - iy, width, height, sources, cubes);
            let (wo, wi) = mix_weights(kind, p, x, y, width, height);
            let mut rgb = [
                out[0] * wo * out[3] + inc[0] * wi * inc[3],
                out[1] * wo * out[3] + inc[1] * wi * inc[3],
                out[2] * wo * out[3] + inc[2] * wi * inc[3],
            ];
            let alpha = (out[3] * wo + inc[3] * wi).clamp(0.0, 1.0);
            if alpha > 1e-4 {
                rgb[0] /= alpha;
                rgb[1] /= alpha;
                rgb[2] /= alpha;
            }
            if matches!(kind, TransitionKind::FadeBlack | TransitionKind::FadeWhite) {
                let dip = 1.0 - (p * 2.0 - 1.0).abs();
                let keep = 1.0 - dip;
                let lift = if kind == TransitionKind::FadeWhite {
                    dip
                } else {
                    0.0
                };
                rgb[0] = rgb[0] * keep + lift;
                rgb[1] = rgb[1] * keep + lift;
                rgb[2] = rgb[2] * keep + lift;
            }
            put(dst, width, x, y, rgb, alpha);
        }
    }
}

fn mix_weights(
    kind: TransitionKind,
    p: f32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> (f32, f32) {
    let u = (x as f32 + 0.5) / width as f32;
    let v = (y as f32 + 0.5) / height as f32;
    // Same edge as `TransitionKind::wipe_inset`: the outgoing plate is clipped
    // from that side, so the incoming plate shows there.
    let incoming = match kind {
        TransitionKind::Cut => 0.0,
        TransitionKind::FadeBlack | TransitionKind::FadeWhite => p,
        TransitionKind::Wipe | TransitionKind::WipeLeft | TransitionKind::SmoothLeft => {
            f32::from(u > 1.0 - p)
        }
        TransitionKind::WipeRight | TransitionKind::SmoothRight => f32::from(u < p),
        TransitionKind::WipeUp | TransitionKind::SmoothUp => f32::from(v > 1.0 - p),
        TransitionKind::WipeDown | TransitionKind::SmoothDown => f32::from(v < p),
        TransitionKind::WipeTl => f32::from(u > 1.0 - p || v > 1.0 - p),
        TransitionKind::WipeTr => f32::from(u < p || v > 1.0 - p),
        TransitionKind::WipeBl => f32::from(v < p || u > 1.0 - p),
        TransitionKind::WipeBr => f32::from(u < p || v < p),
        TransitionKind::HorzOpen => {
            let h = p * 0.5;
            f32::from(u < h || u > 1.0 - h)
        }
        TransitionKind::VertOpen => {
            let h = p * 0.5;
            f32::from(v < h || v > 1.0 - h)
        }
        TransitionKind::CircleOpen => f32::from(center_dist(u, v) < p * 0.75),
        TransitionKind::CircleClose => f32::from(center_dist(u, v) > (1.0 - p) * 0.75),
        TransitionKind::Radial => f32::from(center_dist(u, v) < p),
        TransitionKind::Pixelize => p,
        _ => p,
    };
    (1.0 - incoming, incoming)
}

/// Cell size is large at the start of a pixelize and small at the end.
pub fn mosaic_cell(p: f32, width: u32, height: u32) -> (u32, u32) {
    let cells = 6.0 + p.clamp(0.0, 1.0) * 40.0;
    let cw = (width as f32 / cells).round().max(1.0) as u32;
    let ch = (height as f32 / cells).round().max(1.0) as u32;
    (cw.max(1), ch.max(1))
}

fn center_dist(u: f32, v: f32) -> f32 {
    (u - 0.5).hypot(v - 0.5)
}

fn slide_shift(
    kind: TransitionKind,
    p: f32,
    width: u32,
    height: u32,
) -> Option<((f32, f32), (f32, f32))> {
    let (dx, dy) = kind.slide_delta()?;
    let ow = dx as f32 * p * width as f32;
    let oh = dy as f32 * p * height as f32;
    let iw = -dx as f32 * (1.0 - p) * width as f32;
    let ih = -dy as f32 * (1.0 - p) * height as f32;
    Some(((ow, oh), (iw, ih)))
}

fn paint_over(
    dst: &mut [u8],
    width: u32,
    height: u32,
    layer: &Layer,
    sources: &[FrameSource],
    cubes: &[CubeLut],
) {
    let (x0, y0, x1, y1) = paint_bounds(layer, width, height);
    for y in y0..y1 {
        for x in x0..x1 {
            let px = sample_layer(layer, x as f32, y as f32, width, height, sources, cubes);
            if px[3] <= 0.001 {
                continue;
            }
            let i = ((y * width + x) * 4) as usize;
            over(&mut dst[i..i + 4], px[0], px[1], px[2], px[3]);
        }
    }
}

fn paint_bounds(layer: &Layer, width: u32, height: u32) -> (u32, u32, u32, u32) {
    let Layer::Video {
        card, transform, ..
    } = layer
    else {
        return (0, 0, width, height);
    };
    if transform.rotation.abs() > 0.4 {
        return (0, 0, width, height);
    }
    let Some(card) = card else {
        return (0, 0, width, height);
    };
    let (x, y, w, h) = placement(Some(*card), width, height);
    let pad =
        2.0 + transform.x.abs().max(transform.y.abs()) + (transform.scale - 1.0).abs() * w.max(h);
    let x0 = (x - pad).floor().max(0.0) as u32;
    let y0 = (y - pad).floor().max(0.0) as u32;
    let x1 = (x + w + pad).ceil().min(width as f32) as u32;
    let y1 = (y + h + pad).ceil().min(height as f32) as u32;
    (x0, y0, x1.max(x0), y1.max(y0))
}

fn sample_layer(
    layer: &Layer,
    x: f32,
    y: f32,
    width: u32,
    height: u32,
    sources: &[FrameSource],
    cubes: &[CubeLut],
) -> [f32; 4] {
    let Layer::Video {
        media_id,
        source_time,
        transform,
        grade,
        fx,
        opacity,
        curves,
        mask,
        crop,
        card,
        generator,
        ..
    } = layer
    else {
        return [0.0, 0.0, 0.0, 0.0];
    };
    if x < 0.0 || y < 0.0 || x >= width as f32 || y >= height as f32 {
        return [0.0, 0.0, 0.0, 0.0];
    }
    let (rect_x, rect_y, rect_w, rect_h) = placement(*card, width, height);
    let (u, v) = dest_to_uv(x, y, rect_x, rect_y, rect_w, rect_h, transform);
    let (u, v) = match crop {
        Some(crop) => (
            crop.x.clamp(0.0, 1.0) + u * crop.w.clamp(0.02, 1.0),
            crop.y.clamp(0.0, 1.0) + v * crop.h.clamp(0.02, 1.0),
        ),
        None => (u, v),
    };
    if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
        return [0.0, 0.0, 0.0, 0.0];
    }
    let mut rgb = if let Some(generated) = generator {
        generator_rgb(generated, source_time.as_seconds(), u, x, y, width, height)
    } else {
        let Some(source) = nearest_source(sources, media_id, source_time.as_seconds()) else {
            return [0.0, 0.0, 0.0, 0.0];
        };
        sample_source(source, u, v, fx.blur)
    };
    rgb = grade_rgb(rgb, grade, cubes);
    rgb = curves_rgb(rgb, curves);
    rgb = vignette_rgb(rgb, fx, x, y, width, height);
    let mut alpha = *opacity;
    if let Some(mask) = mask {
        alpha *= mask_alpha(*mask, (x + 0.5) / width as f32, (y + 0.5) / height as f32);
    }
    [rgb[0], rgb[1], rgb[2], alpha.clamp(0.0, 1.0)]
}

fn placement(card: Option<FrameCard>, width: u32, height: u32) -> (f32, f32, f32, f32) {
    if let Some(card) = card {
        (
            card.x.clamp(0.0, 1.0) * width as f32,
            card.y.clamp(0.0, 1.0) * height as f32,
            (card.w.clamp(0.02, 1.0) * width as f32).max(1.0),
            (card.h.clamp(0.02, 1.0) * height as f32).max(1.0),
        )
    } else {
        (0.0, 0.0, width as f32, height as f32)
    }
}

fn dest_to_uv(
    x: f32,
    y: f32,
    rect_x: f32,
    rect_y: f32,
    rect_w: f32,
    rect_h: f32,
    transform: &Transform,
) -> (f32, f32) {
    let pan_x = pan_px(transform.x, rect_w);
    let pan_y = pan_px(transform.y, rect_h);
    let cx = rect_x + rect_w * 0.5 + pan_x;
    let cy = rect_y + rect_h * 0.5 + pan_y;
    let dx = x + 0.5 - cx;
    let dy = y + 0.5 - cy;
    let rad = -transform.rotation.to_radians();
    let (sin, cos) = rad.sin_cos();
    let rx = dx * cos - dy * sin;
    let ry = dx * sin + dy * cos;
    let scale = transform.scale.abs().max(0.05);
    let sx = rx / scale;
    let sy = ry / scale;
    ((sx + rect_w * 0.5) / rect_w, (sy + rect_h * 0.5) / rect_h)
}

pub(crate) fn pan_px(value: f32, span: f32) -> f32 {
    if value.abs() <= 1.5 {
        value * span
    } else {
        value
    }
}

fn sample_source(source: &FrameSource, u: f32, v: f32, blur: f32) -> [f32; 3] {
    if source.width == 0 || source.height == 0 || source.rgba.len() < 4 {
        return [0.0, 0.0, 0.0];
    }
    if blur <= 0.02 {
        return sample_bilinear(source, u, v);
    }
    let step = (blur * 0.02).clamp(0.002, 0.02);
    let mut acc = [0.0, 0.0, 0.0];
    for oy in [-1.0, 0.0, 1.0] {
        for ox in [-1.0, 0.0, 1.0] {
            let px = sample_bilinear(source, u + ox * step, v + oy * step);
            acc[0] += px[0];
            acc[1] += px[1];
            acc[2] += px[2];
        }
    }
    [acc[0] / 9.0, acc[1] / 9.0, acc[2] / 9.0]
}

fn sample_bilinear(source: &FrameSource, u: f32, v: f32) -> [f32; 3] {
    if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
        return [0.0, 0.0, 0.0];
    }
    let x = u * (source.width.saturating_sub(1) as f32);
    let y = v * (source.height.saturating_sub(1) as f32);
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(source.width - 1);
    let y1 = (y0 + 1).min(source.height - 1);
    let tx = x - x0 as f32;
    let ty = y - y0 as f32;
    let c00 = texel(source, x0, y0);
    let c10 = texel(source, x1, y0);
    let c01 = texel(source, x0, y1);
    let c11 = texel(source, x1, y1);
    [
        lerp(lerp(c00[0], c10[0], tx), lerp(c01[0], c11[0], tx), ty),
        lerp(lerp(c00[1], c10[1], tx), lerp(c01[1], c11[1], tx), ty),
        lerp(lerp(c00[2], c10[2], tx), lerp(c01[2], c11[2], tx), ty),
    ]
}

fn texel(source: &FrameSource, x: u32, y: u32) -> [f32; 3] {
    let i = ((y * source.width + x) * 4) as usize;
    if i + 2 >= source.rgba.len() {
        return [0.0, 0.0, 0.0];
    }
    [
        source.rgba[i] as f32 / 255.0,
        source.rgba[i + 1] as f32 / 255.0,
        source.rgba[i + 2] as f32 / 255.0,
    ]
}

fn nearest_source<'a>(
    sources: &'a [FrameSource],
    media_id: &MediaId,
    source_time: f64,
) -> Option<&'a FrameSource> {
    let mut best: Option<&FrameSource> = None;
    let mut best_distance = f64::MAX;
    for source in sources {
        if source.media_id != *media_id {
            continue;
        }
        let distance = (source.source_time - source_time).abs();
        if best.is_none() || distance < best_distance {
            best = Some(source);
            best_distance = distance;
        }
    }
    best
}

fn generator_rgb(
    kind: &Generator,
    source_seconds: f64,
    u: f32,
    x: f32,
    y: f32,
    width: u32,
    height: u32,
) -> [f32; 3] {
    match kind {
        Generator::Color { color } => parse_hex(color),
        Generator::ColorBars => color_bars(u),
        Generator::WhiteNoise => {
            let n = hash(x as u32, y as u32, (u * 1000.0) as u32);
            [n, n, n]
        }
        Generator::Counter => counter_rgb(source_seconds, x, y, width, height),
    }
}

fn counter_rgb(seconds: f64, x: f32, y: f32, width: u32, height: u32) -> [f32; 3] {
    let dark = [0.12, 0.12, 0.12];
    let light = [0.92, 0.92, 0.92];
    if width < 8 || height < 8 {
        return dark;
    }
    let total = seconds.max(0.0).floor() as u32;
    let mm = (total / 60) % 100;
    let ss = total % 60;
    let glyphs = [mm / 10, mm % 10, 10, ss / 10, ss % 10];
    // Integer cells. A float cell lands one ulp under a column edge, and the
    // GPU division rounds the other way.
    let xi = x.floor().max(0.0) as u32;
    let yi = y.floor().max(0.0) as u32;
    let cell = (height * 28 / 100).max(7) / 7;
    if cell == 0 {
        return dark;
    }
    let glyph_w = cell * 5;
    let digit_h = cell * 7;
    let run = glyph_w * 5 + cell * 4;
    if run > width || digit_h > height {
        return dark;
    }
    let x0 = (width - run) / 2;
    let y0 = (height - digit_h) / 2;
    if xi < x0 || yi < y0 || xi >= x0 + run || yi >= y0 + digit_h {
        return dark;
    }
    let mut cursor = x0;
    for glyph in glyphs {
        if xi >= cursor && xi < cursor + glyph_w {
            let col = (xi - cursor) / cell;
            let row = (yi - y0) / cell;
            if glyph_bit(glyph, col, row) {
                return light;
            }
            return dark;
        }
        cursor += glyph_w + cell;
    }
    dark
}

fn glyph_bit(glyph: u32, col: u32, row: u32) -> bool {
    if col >= 5 || row >= 7 {
        return false;
    }
    let rows: [u8; 7] = match glyph {
        0 => [
            0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
        ],
        1 => [
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        2 => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
        ],
        3 => [
            0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        4 => [
            0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
        ],
        5 => [
            0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110,
        ],
        6 => [
            0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
        ],
        7 => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
        ],
        8 => [
            0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
        ],
        9 => [
            0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100,
        ],
        _ => [
            0b00000, 0b00100, 0b00100, 0b00000, 0b00100, 0b00100, 0b00000,
        ],
    };
    rows[row as usize] & (1 << (4 - col)) != 0
}

fn color_bars(u: f32) -> [f32; 3] {
    const BARS: [[f32; 3]; 8] = [
        [0.75, 0.75, 0.75],
        [0.75, 0.75, 0.0],
        [0.0, 0.75, 0.75],
        [0.0, 0.75, 0.0],
        [0.75, 0.0, 0.75],
        [0.75, 0.0, 0.0],
        [0.0, 0.0, 0.75],
        [0.0, 0.0, 0.0],
    ];
    BARS[((u.clamp(0.0, 0.999)) * 8.0) as usize]
}

fn grade_rgb(rgb: [f32; 3], grade: &Grade, cubes: &[CubeLut]) -> [f32; 3] {
    let brightness = 1.0 + grade.exposure + grade.gain * 0.35 + grade.lift * 0.15;
    let contrast = 1.0 + grade.contrast + grade.gamma * 0.45;
    let sat = (1.0 + grade.saturation).max(0.0);
    let mut out = rgb.map(|c| ((c - 0.5) * contrast + 0.5) * brightness);
    let y = luma(out);
    out[0] = y + (out[0] - y) * sat;
    out[1] = y + (out[1] - y) * sat;
    out[2] = y + (out[2] - y) * sat;
    let t = grade.temperature;
    out[0] += t * 0.15;
    out[2] -= t * 0.15;
    out = match grade.lut {
        Lut::None => out,
        Lut::Film => {
            let gray = luma(out);
            [
                gray + (out[0] - gray) * 1.16 + 0.03,
                gray + (out[1] - gray) * 1.16,
                gray + (out[2] - gray) * 1.16 - 0.015,
            ]
        }
        Lut::Cool => [out[0] - 0.04, out[1], out[2] + 0.08],
        Lut::Warm => [out[0] + 0.08, out[1] + 0.03, out[2] - 0.04],
        Lut::TealOrange => [out[0] + 0.06, out[1], out[2] + 0.05],
        Lut::Mono => {
            let gray = luma(out);
            [gray, gray, gray]
        }
    };
    if let Some(id) = grade.cube
        && let Some(cube) = cubes.iter().find(|cube| cube.id == id)
    {
        out = cube.sample(
            out[0].clamp(0.0, 1.0),
            out[1].clamp(0.0, 1.0),
            out[2].clamp(0.0, 1.0),
        );
    }
    out.map(|c| c.clamp(0.0, 1.0))
}

fn luma(rgb: [f32; 3]) -> f32 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}

fn curves_rgb(rgb: [f32; 3], curves: &Curves) -> [f32; 3] {
    if curves.is_identity() {
        return rgb;
    }
    [
        curve_at(&curves.red, curve_at(&curves.all, rgb[0])),
        curve_at(&curves.green, curve_at(&curves.all, rgb[1])),
        curve_at(&curves.blue, curve_at(&curves.all, rgb[2])),
    ]
}

fn curve_at(points: &[CurvePoint], x: f32) -> f32 {
    if points.is_empty() {
        return x.clamp(0.0, 1.0);
    }
    let x = x.clamp(0.0, 1.0);
    let mut left: Option<(f32, f32)> = None;
    let mut right: Option<(f32, f32)> = None;
    for point in points {
        let px = point.x.clamp(0.0, 1.0);
        let py = point.y.clamp(0.0, 1.0);
        if px <= x && left.is_none_or(|(lx, _)| px >= lx) {
            left = Some((px, py));
        }
        if px >= x && right.is_none_or(|(rx, _)| px <= rx) {
            right = Some((px, py));
        }
    }
    let left = left.unwrap_or((0.0, 0.0));
    let right = right.unwrap_or((1.0, 1.0));
    let span = right.0 - left.0;
    if span < 1e-4 {
        return left.1;
    }
    left.1 + (right.1 - left.1) * ((x - left.0) / span)
}

fn vignette_rgb(rgb: [f32; 3], fx: &Fx, x: f32, y: f32, width: u32, height: u32) -> [f32; 3] {
    if fx.vignette <= 0.02 {
        return rgb;
    }
    let u = (x + 0.5) / width as f32 - 0.5;
    let v = (y + 0.5) / height as f32 - 0.5;
    let d = (u * u * 1.4 + v * v).sqrt();
    let fall = (1.0 - (d * fx.vignette * 1.6)).clamp(0.35, 1.0);
    rgb.map(|c| c * fall)
}

fn mask_alpha(mask: AlphaShape, u: f32, v: f32) -> f32 {
    let hw = (mask.w * 0.5).max(0.001);
    let hh = (mask.h * 0.5).max(0.001);
    let nx = (u - mask.x) / hw;
    let ny = (v - mask.y) / hh;
    let dist = match mask.shape {
        MaskShape::Rectangle => nx.abs().max(ny.abs()),
        MaskShape::Ellipse => (nx * nx + ny * ny).sqrt(),
        MaskShape::Diamond => nx.abs() + ny.abs(),
        MaskShape::Triangle => {
            if !(-1.0..=1.0).contains(&ny) {
                2.0
            } else {
                let half = ((ny + 1.0) * 0.5).max(0.001);
                nx.abs() / half
            }
        }
    };
    let feather = mask.feather.clamp(0.0, 1.0);
    let inside = if feather < 0.01 {
        if dist <= 1.0 { 1.0 } else { 0.0 }
    } else {
        let t = ((1.0 + feather - dist) / (2.0 * feather)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    if mask.invert { 1.0 - inside } else { inside }
}

pub(crate) fn parse_hex(color: &str) -> [f32; 3] {
    let hex = oc_timeline::canonical_color(color);
    let bytes = hex.trim_start_matches('#');
    let parse =
        |start: usize| u8::from_str_radix(&bytes[start..start + 2], 16).unwrap_or(0) as f32 / 255.0;
    if bytes.len() >= 6 {
        [parse(0), parse(2), parse(4)]
    } else {
        [0.0, 0.0, 0.0]
    }
}

fn hash(x: u32, y: u32, salt: u32) -> f32 {
    let mut n = x
        .wrapping_mul(374761393)
        .wrapping_add(y.wrapping_mul(668265263))
        .wrapping_add(salt.wrapping_mul(1440671));
    n = (n ^ (n >> 13)).wrapping_mul(1274126177);
    (n & 0xffff) as f32 / 65535.0
}

fn fill(dst: &mut [u8], rgb: [f32; 3]) {
    for px in dst.chunks_exact_mut(4) {
        px[0] = (rgb[0] * 255.0).round() as u8;
        px[1] = (rgb[1] * 255.0).round() as u8;
        px[2] = (rgb[2] * 255.0).round() as u8;
        px[3] = 255;
    }
}

fn put(dst: &mut [u8], width: u32, x: u32, y: u32, rgb: [f32; 3], alpha: f32) {
    let i = ((y * width + x) * 4) as usize;
    if i + 3 >= dst.len() {
        return;
    }
    over(&mut dst[i..i + 4], rgb[0], rgb[1], rgb[2], alpha);
}

fn over(px: &mut [u8], r: f32, g: f32, b: f32, a: f32) {
    let a = a.clamp(0.0, 1.0);
    if a <= 0.001 {
        return;
    }
    let inv = 1.0 - a;
    px[0] = ((r.clamp(0.0, 1.0) * a + px[0] as f32 / 255.0 * inv) * 255.0).round() as u8;
    px[1] = ((g.clamp(0.0, 1.0) * a + px[1] as f32 / 255.0 * inv) * 255.0).round() as u8;
    px[2] = ((b.clamp(0.0, 1.0) * a + px[2] as f32 / 255.0 * inv) * 255.0).round() as u8;
    px[3] = 255;
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;
    use oc_time::{Duration, Time};
    use oc_timeline::{
        AlphaShape, ClipId, ClipKind, ClipLook, CurvePoint, FrameCard, MaskShape, MediaId,
        Timeline, TrackKind, Transform,
    };

    fn solid(media: MediaId, rgb: [u8; 3], n: u32) -> FrameSource {
        let mut rgba = Vec::with_capacity((n * n * 4) as usize);
        for _ in 0..n * n {
            rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
        FrameSource {
            media_id: media,
            source_time: 0.0,
            width: n,
            height: n,
            rgba,
        }
    }

    fn solid_at(media: MediaId, rgb: [u8; 3], n: u32, source_time: f64) -> FrameSource {
        let mut source = solid(media, rgb, n);
        source.source_time = source_time;
        source
    }

    fn pixel(surface: &Surface, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * surface.width + x) * 4) as usize;
        [surface.rgba[i], surface.rgba[i + 1], surface.rgba[i + 2]]
    }

    #[test]
    fn a_crop_keeps_the_right_half_of_the_source() {
        let media = MediaId::new();
        let mut rgba = Vec::new();
        for _y in 0..4 {
            for x in 0..8 {
                let v = if x >= 4 { 255 } else { 0 };
                rgba.extend_from_slice(&[v, v, v, 255]);
            }
        }
        let mut tl = Timeline::new(oc_timeline::FrameRate::FPS_30, 8, 4);
        let track = tl.first_track(TrackKind::Video).unwrap().id;
        let look = ClipLook {
            crop: Some(oc_timeline::Crop {
                x: 0.5,
                y: 0.0,
                w: 0.5,
                h: 1.0,
            }),
            ..ClipLook::default()
        };
        tl.add_clip(
            track,
            oc_timeline::Clip {
                id: ClipId::new(),
                media_id: Some(media),
                kind: ClipKind::Video {
                    transform: Transform::default(),
                },
                start: Time::ZERO,
                duration: Duration::from_seconds(1.0),
                source_in: Time::ZERO,
                speed: 1.0,
                group_id: None,
                link_id: None,
                disabled: false,
                look,
            },
        )
        .unwrap();
        let plan = crate::plan_frame(&tl, Time::from_seconds(0.2));
        assert!(plan.needs_paint);
        let surface = composite(
            &plan,
            &[FrameSource {
                media_id: media,
                source_time: 0.0,
                width: 8,
                height: 4,
                rgba,
            }],
            &[],
        );
        let left = pixel(&surface, 1, 2);
        assert!(left[0] > 200, "{left:?}");
    }

    #[test]
    fn a_mask_clears_the_corners() {
        let media = MediaId::new();
        let mut tl = Timeline::new(oc_timeline::FrameRate::FPS_30, 16, 16);
        tl.background = "#000000".into();
        let track = tl.first_track(TrackKind::Video).unwrap().id;
        let look = ClipLook {
            mask: Some(AlphaShape {
                shape: MaskShape::Ellipse,
                x: 0.5,
                y: 0.5,
                w: 0.5,
                h: 0.5,
                feather: 0.0,
                invert: false,
            }),
            ..ClipLook::default()
        };
        tl.add_clip(
            track,
            oc_timeline::Clip {
                id: ClipId::new(),
                media_id: Some(media),
                kind: ClipKind::Video {
                    transform: Transform::default(),
                },
                start: Time::ZERO,
                duration: Duration::from_seconds(1.0),
                source_in: Time::ZERO,
                speed: 1.0,
                group_id: None,
                link_id: None,
                disabled: false,
                look,
            },
        )
        .unwrap();
        let plan = crate::plan_frame(&tl, Time::from_seconds(0.2));
        let surface = composite(&plan, &[solid(media, [255, 0, 0], 16)], &[]);
        let center = pixel(&surface, 8, 8);
        let corner = pixel(&surface, 0, 0);
        assert!(center[0] > 200, "{center:?}");
        assert!(corner[0] < 20, "{corner:?}");
    }

    #[test]
    fn an_overlay_card_covers_only_its_rect() {
        let speaker = MediaId::new();
        let design = MediaId::new();
        let mut tl = Timeline::new(oc_timeline::FrameRate::FPS_30, 20, 10);
        let v1 = tl.first_track(TrackKind::Video).unwrap().id;
        tl.add_clip(
            v1,
            oc_timeline::Clip {
                id: ClipId::new(),
                media_id: Some(speaker),
                kind: ClipKind::Video {
                    transform: Transform::default(),
                },
                start: Time::ZERO,
                duration: Duration::from_seconds(1.0),
                source_in: Time::ZERO,
                speed: 1.0,
                group_id: None,
                link_id: None,
                disabled: false,
                look: ClipLook::default(),
            },
        )
        .unwrap();
        let design_track = tl.add_track(TrackKind::Video, "Design");
        let look = ClipLook {
            card: Some(FrameCard {
                x: 0.5,
                y: 0.0,
                w: 0.5,
                h: 1.0,
            }),
            overlay: true,
            ..ClipLook::default()
        };
        tl.add_clip(
            design_track,
            oc_timeline::Clip {
                id: ClipId::new(),
                media_id: Some(design),
                kind: ClipKind::Video {
                    transform: Transform::default(),
                },
                start: Time::ZERO,
                duration: Duration::from_seconds(1.0),
                source_in: Time::ZERO,
                speed: 1.0,
                group_id: None,
                link_id: None,
                disabled: false,
                look,
            },
        )
        .unwrap();
        let plan = crate::plan_frame(&tl, Time::from_seconds(0.2));
        let surface = composite(
            &plan,
            &[
                solid(speaker, [200, 0, 0], 8),
                solid(design, [0, 180, 0], 8),
            ],
            &[],
        );
        let left = pixel(&surface, 2, 5);
        let right = pixel(&surface, 16, 5);
        assert!(left[0] > left[1], "speaker stays on the open side {left:?}");
        assert!(right[1] > right[0], "design covers the card {right:?}");
    }

    #[test]
    fn curves_pull_white_down() {
        let media = MediaId::new();
        let mut tl = Timeline::new(oc_timeline::FrameRate::FPS_30, 4, 4);
        let track = tl.first_track(TrackKind::Video).unwrap().id;
        let mut look = ClipLook::default();
        look.curves.all = vec![CurvePoint { x: 0.0, y: 0.0 }, CurvePoint { x: 1.0, y: 0.4 }];
        tl.add_clip(
            track,
            oc_timeline::Clip {
                id: ClipId::new(),
                media_id: Some(media),
                kind: ClipKind::Video {
                    transform: Transform::default(),
                },
                start: Time::ZERO,
                duration: Duration::from_seconds(1.0),
                source_in: Time::ZERO,
                speed: 1.0,
                group_id: None,
                link_id: None,
                disabled: false,
                look,
            },
        )
        .unwrap();
        let plan = crate::plan_frame(&tl, Time::from_seconds(0.2));
        let surface = composite(&plan, &[solid(media, [255, 255, 255], 4)], &[]);
        let px = pixel(&surface, 1, 1);
        assert!(px[0] < 140, "{px:?}");
    }

    #[test]
    fn rotation_moves_a_marked_corner() {
        let media = MediaId::new();
        let mut rgba = vec![0u8; 8 * 8 * 4];
        for y in 0..3 {
            for x in 0..3 {
                let i = ((y * 8 + x) * 4) as usize;
                rgba[i] = 255;
                rgba[i + 3] = 255;
            }
        }
        let mut tl = Timeline::new(oc_timeline::FrameRate::FPS_30, 8, 8);
        let track = tl.first_track(TrackKind::Video).unwrap().id;
        tl.add_clip(
            track,
            oc_timeline::Clip {
                id: ClipId::new(),
                media_id: Some(media),
                kind: ClipKind::Video {
                    transform: Transform {
                        x: 0.0,
                        y: 0.0,
                        scale: 1.0,
                        rotation: 180.0,
                    },
                },
                start: Time::ZERO,
                duration: Duration::from_seconds(1.0),
                source_in: Time::ZERO,
                speed: 1.0,
                group_id: None,
                link_id: None,
                disabled: false,
                look: ClipLook::default(),
            },
        )
        .unwrap();
        let plan = crate::plan_frame(&tl, Time::from_seconds(0.2));
        let surface = composite(
            &plan,
            &[FrameSource {
                media_id: media,
                source_time: 0.0,
                width: 8,
                height: 8,
                rgba,
            }],
            &[],
        );
        let opposite = pixel(&surface, 7, 7);
        assert!(
            opposite[0] > 200,
            "180° parks the mark on the far corner {opposite:?}"
        );
    }

    #[test]
    fn a_left_wipe_opens_from_the_right() {
        let outgoing = MediaId::new();
        let incoming = MediaId::new();
        let mut tl = Timeline::new(oc_timeline::FrameRate::FPS_30, 20, 8);
        let track = tl.first_track(TrackKind::Video).unwrap().id;
        let look = ClipLook {
            transition: oc_timeline::TransitionKind::WipeLeft,
            ..ClipLook::default()
        };
        tl.add_clip(track, plate(outgoing, look, Time::ZERO))
            .unwrap();
        tl.add_clip(
            track,
            plate(incoming, ClipLook::default(), Time::from_seconds(4.0)),
        )
        .unwrap();
        let plan = crate::plan_frame(&tl, Time::from_seconds(3.6));
        let surface = composite(
            &plan,
            &[
                solid(outgoing, [200, 0, 0], 8),
                solid(incoming, [0, 180, 0], 8),
            ],
            &[],
        );
        let left = pixel(&surface, 2, 4);
        let right = pixel(&surface, 17, 4);
        assert!(left[0] > left[1], "outgoing stays on the left {left:?}");
        assert!(
            right[1] > right[0],
            "incoming enters from the right {right:?}"
        );
    }

    #[test]
    fn pixelize_snaps_a_cell_to_one_sample() {
        let outgoing = MediaId::new();
        let incoming = MediaId::new();
        let mut tl = Timeline::new(oc_timeline::FrameRate::FPS_30, 78, 8);
        let track = tl.first_track(TrackKind::Video).unwrap().id;
        let look = ClipLook {
            transition: oc_timeline::TransitionKind::Pixelize,
            ..ClipLook::default()
        };
        tl.add_clip(track, plate(outgoing, look, Time::ZERO))
            .unwrap();
        tl.add_clip(
            track,
            plate(incoming, ClipLook::default(), Time::from_seconds(4.0)),
        )
        .unwrap();
        let plan = crate::plan_frame(&tl, Time::from_seconds(3.6));
        assert!(plan.needs_paint);
        let stripe = |media: MediaId| {
            let mut rgba = Vec::new();
            for _y in 0..8 {
                for x in 0..78 {
                    rgba.extend_from_slice(&[x as u8, 0, 40, 255]);
                }
            }
            FrameSource {
                media_id: media,
                source_time: 0.0,
                width: 78,
                height: 8,
                rgba,
            }
        };
        let surface = composite(&plan, &[stripe(outgoing), stripe(incoming)], &[]);
        let a = pixel(&surface, 0, 4);
        let b = pixel(&surface, 1, 4);
        let next = pixel(&surface, 3, 4);
        assert_eq!(a, b, "one cell is one sample {a:?} {b:?}");
        assert_ne!(a, next, "the next cell samples somewhere else");
    }

    #[test]
    fn a_counter_draws_digits_from_the_source_clock() {
        let mut tl = Timeline::new(oc_timeline::FrameRate::FPS_30, 96, 54);
        let track = tl.first_track(TrackKind::Video).unwrap().id;
        let mut clip = plate(MediaId::new(), ClipLook::default(), Time::ZERO);
        clip.media_id = None;
        clip.look.generator = Some(oc_timeline::Generator::Counter);
        clip.source_in = Time::from_seconds(61.0);
        tl.add_clip(track, clip).unwrap();
        let plan = crate::plan_frame(&tl, Time::from_seconds(0.2));
        assert!(plan.needs_paint);
        let surface = composite(&plan, &[], &[]);
        let margin = pixel(&surface, 2, 27);
        assert!(margin[0] < 40, "digits stay off the frame edge {margin:?}");
        let mut lit = 0u32;
        for y in 0..54 {
            for x in 0..96 {
                if pixel(&surface, x, y)[0] > 200 {
                    lit += 1;
                }
            }
        }
        assert!(lit > 8, "bitmap digits are lit, got {lit}");
        let mut other = plate(MediaId::new(), ClipLook::default(), Time::ZERO);
        other.media_id = None;
        other.look.generator = Some(oc_timeline::Generator::Counter);
        other.source_in = Time::ZERO;
        let mut tl0 = Timeline::new(oc_timeline::FrameRate::FPS_30, 96, 54);
        let track0 = tl0.first_track(TrackKind::Video).unwrap().id;
        tl0.add_clip(track0, other).unwrap();
        let zero = composite(&crate::plan_frame(&tl0, Time::from_seconds(0.2)), &[], &[]);
        assert_ne!(surface.rgba, zero.rgba, "01:01 differs from 00:00");
    }

    #[test]
    fn the_same_file_can_show_two_source_times() {
        let media = MediaId::new();
        let mut tl = Timeline::new(oc_timeline::FrameRate::FPS_30, 20, 10);
        let v1 = tl.first_track(TrackKind::Video).unwrap().id;
        tl.add_clip(v1, plate(media, ClipLook::default(), Time::ZERO))
            .unwrap();
        let front = tl.add_track(TrackKind::Video, "Front");
        let mut corner = plate(media, ClipLook::default(), Time::ZERO);
        corner.source_in = Time::from_seconds(5.0);
        corner.look.card = Some(FrameCard {
            x: 0.5,
            y: 0.0,
            w: 0.5,
            h: 1.0,
        });
        tl.add_clip(front, corner).unwrap();
        let plan = crate::plan_frame(&tl, Time::from_seconds(0.2));
        let surface = composite(
            &plan,
            &[
                solid_at(media, [200, 0, 0], 8, 0.0),
                solid_at(media, [0, 180, 0], 8, 5.0),
            ],
            &[],
        );
        let left = pixel(&surface, 2, 5);
        let right = pixel(&surface, 16, 5);
        assert!(left[0] > left[1], "program keeps the early frame {left:?}");
        assert!(
            right[1] > right[0],
            "corner keeps the later frame {right:?}"
        );
    }

    fn plate(media: MediaId, look: ClipLook, start: Time) -> oc_timeline::Clip {
        oc_timeline::Clip {
            id: ClipId::new(),
            media_id: Some(media),
            kind: ClipKind::Video {
                transform: Transform::default(),
            },
            start,
            duration: Duration::from_seconds(4.0),
            source_in: Time::ZERO,
            speed: 1.0,
            group_id: None,
            link_id: None,
            disabled: false,
            look,
        }
    }
}
