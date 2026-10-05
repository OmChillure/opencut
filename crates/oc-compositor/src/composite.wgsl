// One planned frame. The CPU packs the same numbers `draw.rs` uses.
// Eight video layers, four pictures, two cubes.

struct Frame {
    size: vec4<f32>,
    bg: vec4<f32>,
    shift: vec4<f32>,
    cubes: vec4<f32>,
    tex: array<vec4<f32>, 4>,
}

struct Layer {
    rect: vec4<f32>,
    xform: vec4<f32>,
    grade: vec4<f32>,
    grade2: vec4<f32>,
    fx: vec4<f32>,
    mask: vec4<f32>,
    mask2: vec4<f32>,
    crop: vec4<f32>,
    flags: vec4<f32>,
    extra: vec4<f32>,
    slide: vec4<f32>,
    genb: vec4<f32>,
    counts: vec4<f32>,
    curves: array<vec4<f32>, 16>,
}

struct Scene {
    frame: Frame,
    layers: array<Layer, 8>,
}

@group(0) @binding(0) var<storage, read> scene: Scene;
@group(0) @binding(1) var t0: texture_2d<f32>;
@group(0) @binding(2) var t1: texture_2d<f32>;
@group(0) @binding(3) var t2: texture_2d<f32>;
@group(0) @binding(4) var t3: texture_2d<f32>;
@group(0) @binding(5) var cube0: texture_3d<f32>;
@group(0) @binding(6) var cube1: texture_3d<f32>;

const GLYPHS: array<u32, 77> = array<u32, 77>(
    0x0eu, 0x11u, 0x13u, 0x15u, 0x19u, 0x11u, 0x0eu,
    0x04u, 0x0cu, 0x04u, 0x04u, 0x04u, 0x04u, 0x0eu,
    0x0eu, 0x11u, 0x01u, 0x02u, 0x04u, 0x08u, 0x1fu,
    0x1eu, 0x01u, 0x01u, 0x0eu, 0x01u, 0x01u, 0x1eu,
    0x02u, 0x06u, 0x0au, 0x12u, 0x1fu, 0x02u, 0x02u,
    0x1fu, 0x10u, 0x1eu, 0x01u, 0x01u, 0x11u, 0x0eu,
    0x06u, 0x08u, 0x10u, 0x1eu, 0x11u, 0x11u, 0x0eu,
    0x1fu, 0x01u, 0x02u, 0x04u, 0x08u, 0x08u, 0x08u,
    0x0eu, 0x11u, 0x11u, 0x0eu, 0x11u, 0x11u, 0x0eu,
    0x0eu, 0x11u, 0x11u, 0x0fu, 0x01u, 0x02u, 0x0cu,
    0x00u, 0x04u, 0x04u, 0x00u, 0x04u, 0x04u, 0x00u,
);

@vertex
fn vs(@builtin(vertex_index) vid: u32) -> @builtin(position) vec4<f32> {
    var p = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    return vec4<f32>(p[vid], 0.0, 1.0);
}

fn luma(rgb: vec3<f32>) -> f32 {
    return dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn over(dst: vec3<f32>, src: vec3<f32>, a: f32) -> vec3<f32> {
    let src_c = clamp(src, vec3<f32>(0.0), vec3<f32>(1.0));
    let alpha = clamp(a, 0.0, 1.0);
    return src_c * alpha + dst * (1.0 - alpha);
}

fn load_tex(index: i32, x: i32, y: i32) -> vec3<f32> {
    let c = vec2<i32>(x, y);
    if index == 0 { return textureLoad(t0, c, 0).rgb; }
    if index == 1 { return textureLoad(t1, c, 0).rgb; }
    if index == 2 { return textureLoad(t2, c, 0).rgb; }
    if index == 3 { return textureLoad(t3, c, 0).rgb; }
    return vec3<f32>(0.0);
}

fn sample_bilinear(index: i32, u: f32, v: f32) -> vec3<f32> {
    if u < 0.0 || v < 0.0 || u > 1.0 || v > 1.0 {
        return vec3<f32>(0.0);
    }
    let size = scene.frame.tex[index].xy;
    let x = u * (size.x - 1.0);
    let y = v * (size.y - 1.0);
    let x0 = i32(floor(x));
    let y0 = i32(floor(y));
    let x1 = min(x0 + 1, i32(size.x) - 1);
    let y1 = min(y0 + 1, i32(size.y) - 1);
    let tx = x - f32(x0);
    let ty = y - f32(y0);
    let c00 = load_tex(index, x0, y0);
    let c10 = load_tex(index, x1, y0);
    let c01 = load_tex(index, x0, y1);
    let c11 = load_tex(index, x1, y1);
    let top = mix(c00, c10, tx);
    let bot = mix(c01, c11, tx);
    return mix(top, bot, ty);
}

fn sample_picture(index: i32, u: f32, v: f32, blur: f32) -> vec3<f32> {
    if blur <= 0.02 {
        return sample_bilinear(index, u, v);
    }
    let step = clamp(blur * 0.02, 0.002, 0.02);
    var acc = vec3<f32>(0.0);
    for (var oy = -1; oy <= 1; oy = oy + 1) {
        for (var ox = -1; ox <= 1; ox = ox + 1) {
            acc = acc + sample_bilinear(index, u + f32(ox) * step, v + f32(oy) * step);
        }
    }
    return acc / 9.0;
}

fn cube_texel(slot: i32, r: i32, g: i32, b: i32) -> vec3<f32> {
    let c = vec3<i32>(r, g, b);
    if slot == 0 {
        return textureLoad(cube0, c, 0).rgb;
    }
    return textureLoad(cube1, c, 0).rgb;
}

fn cube_sample(slot: i32, rgb: vec3<f32>, size: f32) -> vec3<f32> {
    let last = size - 1.0;
    let rf = clamp(rgb.r, 0.0, 1.0) * last;
    let gf = clamp(rgb.g, 0.0, 1.0) * last;
    let bf = clamp(rgb.b, 0.0, 1.0) * last;
    let r0 = i32(floor(rf));
    let g0 = i32(floor(gf));
    let b0 = i32(floor(bf));
    let r1 = min(r0 + 1, i32(last));
    let g1 = min(g0 + 1, i32(last));
    let b1 = min(b0 + 1, i32(last));
    let tr = rf - f32(r0);
    let tg = gf - f32(g0);
    let tb = bf - f32(b0);
    let c000 = cube_texel(slot, r0, g0, b0);
    let c100 = cube_texel(slot, r1, g0, b0);
    let c010 = cube_texel(slot, r0, g1, b0);
    let c110 = cube_texel(slot, r1, g1, b0);
    let c001 = cube_texel(slot, r0, g0, b1);
    let c101 = cube_texel(slot, r1, g0, b1);
    let c011 = cube_texel(slot, r0, g1, b1);
    let c111 = cube_texel(slot, r1, g1, b1);
    let c00 = mix(c000, c100, tr);
    let c10 = mix(c010, c110, tr);
    let c01 = mix(c001, c101, tr);
    let c11 = mix(c011, c111, tr);
    return mix(mix(c00, c10, tg), mix(c01, c11, tg), tb);
}

fn grade_rgb(rgb: vec3<f32>, layer: Layer) -> vec3<f32> {
    let brightness = 1.0 + layer.grade.x + layer.grade2.z * 0.35 + layer.grade2.x * 0.15;
    let contrast = 1.0 + layer.grade.y + layer.grade2.y * 0.45;
    let sat = max(1.0 + layer.grade.z, 0.0);
    var out = ((rgb - vec3<f32>(0.5)) * contrast + vec3<f32>(0.5)) * brightness;
    let y = luma(out);
    out = vec3<f32>(y) + (out - vec3<f32>(y)) * sat;
    out.r = out.r + layer.grade.w * 0.15;
    out.b = out.b - layer.grade.w * 0.15;
    let lut = u32(layer.grade2.w);
    if lut == 1u {
        let gray = luma(out);
        out = vec3<f32>(
            gray + (out.r - gray) * 1.16 + 0.03,
            gray + (out.g - gray) * 1.16,
            gray + (out.b - gray) * 1.16 - 0.015,
        );
    } else if lut == 2u {
        out = vec3<f32>(out.r - 0.04, out.g, out.b + 0.08);
    } else if lut == 3u {
        out = vec3<f32>(out.r + 0.08, out.g + 0.03, out.b - 0.04);
    } else if lut == 4u {
        out = vec3<f32>(out.r + 0.06, out.g, out.b + 0.05);
    } else if lut == 5u {
        let gray = luma(out);
        out = vec3<f32>(gray);
    }
    out = clamp(out, vec3<f32>(0.0), vec3<f32>(1.0));
    let slot = i32(layer.extra.y);
    if slot >= 0 {
        let cube_size = select(scene.frame.cubes.y, scene.frame.cubes.x, slot == 0);
        if cube_size >= 2.0 {
            out = clamp(cube_sample(slot, out, cube_size), vec3<f32>(0.0), vec3<f32>(1.0));
        }
    }
    return out;
}

fn curve_point(curves: array<vec4<f32>, 16>, base: i32, i: i32) -> vec2<f32> {
    let v = curves[base + (i / 2)];
    if (i & 1) == 0 {
        return v.xy;
    }
    return v.zw;
}

fn curve_at(curves: array<vec4<f32>, 16>, base: i32, n: i32, x_in: f32) -> f32 {
    let x = clamp(x_in, 0.0, 1.0);
    if n <= 0 {
        return x;
    }
    var left = vec2<f32>(0.0, 0.0);
    var right = vec2<f32>(1.0, 1.0);
    var have_l = false;
    var have_r = false;
    for (var i = 0; i < 8; i = i + 1) {
        if i >= n {
            break;
        }
        let p = curve_point(curves, base, i);
        if p.x <= x && (!have_l || p.x >= left.x) {
            left = p;
            have_l = true;
        }
        if p.x >= x && (!have_r || p.x <= right.x) {
            right = p;
            have_r = true;
        }
    }
    let span = right.x - left.x;
    if span < 0.0001 {
        return left.y;
    }
    return left.y + (right.y - left.y) * ((x - left.x) / span);
}

fn curves_rgb(rgb: vec3<f32>, layer: Layer) -> vec3<f32> {
    let all_n = i32(layer.counts.x);
    let r = curve_at(layer.curves, 4, i32(layer.counts.y), curve_at(layer.curves, 0, all_n, rgb.r));
    let g = curve_at(layer.curves, 8, i32(layer.counts.z), curve_at(layer.curves, 0, all_n, rgb.g));
    let b = curve_at(layer.curves, 12, i32(layer.counts.w), curve_at(layer.curves, 0, all_n, rgb.b));
    return vec3<f32>(r, g, b);
}

fn vignette_rgb(rgb: vec3<f32>, layer: Layer, x: f32, y: f32, w: f32, h: f32) -> vec3<f32> {
    if layer.fx.y <= 0.02 {
        return rgb;
    }
    let u = (x + 0.5) / w - 0.5;
    let v = (y + 0.5) / h - 0.5;
    let d = sqrt(u * u * 1.4 + v * v);
    let fall = clamp(1.0 - d * layer.fx.y * 1.6, 0.35, 1.0);
    return rgb * fall;
}

fn mask_alpha(layer: Layer, u: f32, v: f32) -> f32 {
    if layer.mask2.w < 0.5 {
        return 1.0;
    }
    let hw = max(layer.mask.z * 0.5, 0.001);
    let hh = max(layer.mask.w * 0.5, 0.001);
    let nx = (u - layer.mask.x) / hw;
    let ny = (v - layer.mask.y) / hh;
    let shape = u32(layer.mask2.x);
    var dist = 0.0;
    if shape == 0u {
        dist = max(abs(nx), abs(ny));
    } else if shape == 1u {
        dist = sqrt(nx * nx + ny * ny);
    } else if shape == 2u {
        dist = abs(nx) + abs(ny);
    } else {
        if ny < -1.0 || ny > 1.0 {
            dist = 2.0;
        } else {
            let half = max((ny + 1.0) * 0.5, 0.001);
            dist = abs(nx) / half;
        }
    }
    let feather = clamp(layer.mask2.y, 0.0, 1.0);
    var inside = 0.0;
    if feather < 0.01 {
        inside = select(0.0, 1.0, dist <= 1.0);
    } else {
        let t = clamp((1.0 + feather - dist) / (2.0 * feather), 0.0, 1.0);
        inside = t * t * (3.0 - 2.0 * t);
    }
    if layer.mask2.z > 0.5 {
        return 1.0 - inside;
    }
    return inside;
}

fn hash_px(x: u32, y: u32, salt: u32) -> f32 {
    var n = x * 374761393u + y * 668265263u + salt * 1440671u;
    n = (n ^ (n >> 13u)) * 1274126177u;
    return f32(n & 0xffffu) / 65535.0;
}

fn glyph_bit(glyph: u32, col: u32, row: u32) -> bool {
    if col >= 5u || row >= 7u || glyph >= 11u {
        return false;
    }
    let bits = GLYPHS[glyph * 7u + row];
    return (bits & (1u << (4u - col))) != 0u;
}

fn color_bars(u: f32) -> vec3<f32> {
    var bars = array<vec3<f32>, 8>(
        vec3<f32>(0.75, 0.75, 0.75),
        vec3<f32>(0.75, 0.75, 0.0),
        vec3<f32>(0.0, 0.75, 0.75),
        vec3<f32>(0.0, 0.75, 0.0),
        vec3<f32>(0.75, 0.0, 0.75),
        vec3<f32>(0.75, 0.0, 0.0),
        vec3<f32>(0.0, 0.0, 0.75),
        vec3<f32>(0.0, 0.0, 0.0),
    );
    let i = i32(clamp(u, 0.0, 0.999) * 8.0);
    return bars[i];
}

fn counter_rgb(seconds: f32, x: f32, y: f32, w: f32, h: f32) -> vec3<f32> {
    let dark = vec3<f32>(0.12, 0.12, 0.12);
    let light = vec3<f32>(0.92, 0.92, 0.92);
    let width = u32(w);
    let height = u32(h);
    if width < 8u || height < 8u {
        return dark;
    }
    let total = u32(max(floor(seconds), 0.0));
    let mm = (total / 60u) % 100u;
    let ss = total % 60u;
    var glyphs = array<u32, 5>(mm / 10u, mm % 10u, 10u, ss / 10u, ss % 10u);
    // Same integer grid as `counter_rgb` in draw.rs.
    let xi = u32(max(floor(x), 0.0));
    let yi = u32(max(floor(y), 0.0));
    let cell = max(height * 28u / 100u, 7u) / 7u;
    if cell == 0u {
        return dark;
    }
    let glyph_w = cell * 5u;
    let digit_h = cell * 7u;
    let run = glyph_w * 5u + cell * 4u;
    if run > width || digit_h > height {
        return dark;
    }
    let x0 = (width - run) / 2u;
    let y0 = (height - digit_h) / 2u;
    if xi < x0 || yi < y0 || xi >= x0 + run || yi >= y0 + digit_h {
        return dark;
    }
    var cursor = x0;
    for (var i = 0u; i < 5u; i = i + 1u) {
        if xi >= cursor && xi < cursor + glyph_w {
            let col = (xi - cursor) / cell;
            let row = (yi - y0) / cell;
            if glyph_bit(glyphs[i], col, row) {
                return light;
            }
            return dark;
        }
        cursor = cursor + glyph_w + cell;
    }
    return dark;
}

fn generator_rgb(layer: Layer, u: f32, x: f32, y: f32, w: f32, h: f32) -> vec3<f32> {
    let kind = u32(layer.flags.y);
    if kind == 1u {
        return vec3<f32>(layer.extra.z, layer.extra.w, layer.genb.x);
    }
    if kind == 2u {
        return color_bars(u);
    }
    if kind == 3u {
        let n = hash_px(u32(x), u32(y), u32(u * 1000.0));
        return vec3<f32>(n);
    }
    if kind == 4u {
        return counter_rgb(layer.extra.x, x, y, w, h);
    }
    return vec3<f32>(0.0);
}

fn dest_uv(x: f32, y: f32, layer: Layer) -> vec2<f32> {
    let rect = layer.rect;
    let scale = max(abs(layer.xform.w), 0.05);
    let cx = rect.x + rect.z * 0.5 + layer.xform.x;
    let cy = rect.y + rect.w * 0.5 + layer.xform.y;
    let dx = x + 0.5 - cx;
    let dy = y + 0.5 - cy;
    let s = sin(layer.xform.z);
    let c = cos(layer.xform.z);
    let rx = dx * c - dy * s;
    let ry = dx * s + dy * c;
    let sx = rx / scale;
    let sy = ry / scale;
    return vec2<f32>((sx + rect.z * 0.5) / rect.z, (sy + rect.w * 0.5) / rect.w);
}

fn shade_layer(layer: Layer, x: f32, y: f32, w: f32, h: f32) -> vec4<f32> {
    if x < 0.0 || y < 0.0 || x >= w || y >= h {
        return vec4<f32>(0.0);
    }
    var uv = dest_uv(x, y, layer);
    uv = vec2<f32>(layer.crop.x + uv.x * layer.crop.z, layer.crop.y + uv.y * layer.crop.w);
    if uv.x < 0.0 || uv.y < 0.0 || uv.x > 1.0 || uv.y > 1.0 {
        return vec4<f32>(0.0);
    }
    let tex = i32(layer.flags.x);
    let gen = u32(layer.flags.y);
    var rgb: vec3<f32>;
    if gen > 0u {
        rgb = generator_rgb(layer, uv.x, x, y, w, h);
    } else if tex >= 0 {
        // genb.w marks a frame the CPU already composited. Copy that texel.
        // Bilinear would resample it and drift from the reference.
        if layer.genb.w > 0.5 {
            let ix = i32(floor(x));
            let iy = i32(floor(y));
            let size = scene.frame.tex[tex].xy;
            if ix < 0 || iy < 0 || f32(ix) >= size.x || f32(iy) >= size.y {
                rgb = vec3<f32>(0.0);
            } else {
                rgb = load_tex(tex, ix, iy);
            }
        } else {
            rgb = sample_picture(tex, uv.x, uv.y, layer.fx.x);
        }
    } else {
        return vec4<f32>(0.0);
    }
    rgb = grade_rgb(rgb, layer);
    rgb = curves_rgb(rgb, layer);
    rgb = vignette_rgb(rgb, layer, x, y, w, h);
    var alpha = clamp(layer.fx.z, 0.0, 1.0);
    alpha = alpha * mask_alpha(layer, (x + 0.5) / w, (y + 0.5) / h);
    return vec4<f32>(rgb, alpha);
}

fn center_dist(u: f32, v: f32) -> f32 {
    return length(vec2<f32>(u - 0.5, v - 0.5));
}

fn mix_weights(mode: u32, p: f32, x: f32, y: f32, w: f32, h: f32) -> vec2<f32> {
    let u = (x + 0.5) / w;
    let v = (y + 0.5) / h;
    var incoming = p;
    if mode == 0u {
        incoming = 0.0;
    } else if mode == 4u {
        incoming = select(0.0, 1.0, u > 1.0 - p);
    } else if mode == 5u {
        incoming = select(0.0, 1.0, u < p);
    } else if mode == 6u {
        incoming = select(0.0, 1.0, v > 1.0 - p);
    } else if mode == 7u {
        incoming = select(0.0, 1.0, v < p);
    } else if mode == 8u {
        incoming = select(0.0, 1.0, u > 1.0 - p || v > 1.0 - p);
    } else if mode == 9u {
        incoming = select(0.0, 1.0, u < p || v > 1.0 - p);
    } else if mode == 10u {
        incoming = select(0.0, 1.0, v < p || u > 1.0 - p);
    } else if mode == 11u {
        incoming = select(0.0, 1.0, u < p || v < p);
    } else if mode == 12u {
        let half = p * 0.5;
        incoming = select(0.0, 1.0, u < half || u > 1.0 - half);
    } else if mode == 13u {
        let half = p * 0.5;
        incoming = select(0.0, 1.0, v < half || v > 1.0 - half);
    } else if mode == 14u {
        incoming = select(0.0, 1.0, center_dist(u, v) < p * 0.75);
    } else if mode == 15u {
        incoming = select(0.0, 1.0, center_dist(u, v) > (1.0 - p) * 0.75);
    } else if mode == 16u {
        incoming = select(0.0, 1.0, center_dist(u, v) < p);
    }
    return vec2<f32>(1.0 - incoming, incoming);
}

fn mix_pair(outgoing: Layer, incoming: Layer, x: f32, y: f32, w: f32, h: f32) -> vec4<f32> {
    let mode = u32(outgoing.flags.z);
    let p = clamp(outgoing.fx.w, 0.0, 1.0);
    var sx = x;
    var sy = y;
    if mode == 17u {
        let cw = max(outgoing.genb.y, 1.0);
        let ch = max(outgoing.genb.z, 1.0);
        let cx = floor(x / cw) * cw + floor(cw * 0.5);
        let cy = floor(y / ch) * ch + floor(ch * 0.5);
        sx = min(cx, w - 1.0);
        sy = min(cy, h - 1.0);
    }
    let out_px = shade_layer(outgoing, sx - outgoing.slide.x, sy - outgoing.slide.y, w, h);
    let in_px = shade_layer(incoming, sx - outgoing.slide.z, sy - outgoing.slide.w, w, h);
    let weight = mix_weights(mode, p, x, y, w, h);
    var rgb = out_px.rgb * weight.x * out_px.a + in_px.rgb * weight.y * in_px.a;
    let alpha = clamp(out_px.a * weight.x + in_px.a * weight.y, 0.0, 1.0);
    if alpha > 0.0001 {
        rgb = rgb / alpha;
    }
    if mode == 2u || mode == 3u {
        let dip = 1.0 - abs(p * 2.0 - 1.0);
        let keep = 1.0 - dip;
        let lift = select(0.0, dip, mode == 3u);
        rgb = rgb * keep + vec3<f32>(lift);
    }
    return vec4<f32>(rgb, alpha);
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let w = scene.frame.size.x;
    let h = scene.frame.size.y;
    let x = floor(pos.x) - scene.frame.shift.x;
    let y = floor(pos.y) - scene.frame.shift.y;
    if x < 0.0 || y < 0.0 || x >= w || y >= h {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    var color = scene.frame.bg.rgb;
    let count = i32(scene.frame.size.z);
    var i = 0;
    loop {
        if i >= count {
            break;
        }
        if scene.layers[i].flags.w > 0.5 {
            i = i + 1;
            continue;
        }
        var px: vec4<f32>;
        var step = 1;
        if i + 1 < count && scene.layers[i + 1].flags.w > 0.5 {
            px = mix_pair(scene.layers[i], scene.layers[i + 1], x, y, w, h);
            step = 2;
        } else {
            px = shade_layer(scene.layers[i], x, y, w, h);
        }
        if px.a > 0.001 {
            color = over(color, px.rgb, px.a);
        }
        i = i + step;
    }
    if scene.frame.size.w > 0.5 {
        let bar = round(h * 0.12);
        if y < bar || y + bar >= h {
            color = scene.frame.bg.rgb;
        }
    }
    return vec4<f32>(color, 1.0);
}
