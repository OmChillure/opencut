use crate::captions::{BurnedCue, captions_for_cut, to_ass};
use crate::{MediaSource, RenderError};
use oc_timeline::{Clip, ClipKind, GraphicKind, MediaId, Timeline, TrackKind, TransitionKind};
use oc_tools::ExportPreset;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct Compiled {
    pub inputs: Vec<PathBuf>,
    pub filter: String,
    pub video_label: String,
    pub audio_label: Option<String>,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub captions: Vec<BurnedCue>,
    pub srt: Option<PathBuf>,
}

pub fn compile(
    timeline: &Timeline,
    media: &HashMap<MediaId, MediaSource>,
    preset: ExportPreset,
    work: &Path,
) -> Result<Compiled, RenderError> {
    let (width, height) = preset.size();
    let fps = timeline.frame_rate.as_f64().max(1.0);

    let mut inputs: Vec<PathBuf> = Vec::new();
    let mut index_of: HashMap<MediaId, usize> = HashMap::new();
    for src in media.values() {
        if !src.path.is_file() {
            return Err(RenderError::MissingMedia(src.path.display().to_string()));
        }
        if !index_of.contains_key(&src.id) {
            index_of.insert(src.id, inputs.len());
            inputs.push(src.path.clone());
        }
    }

    let base = base_clips(timeline);
    if base.is_empty() && !has_graphics(timeline) {
        return Err(RenderError::Empty);
    }
    // Dissolves overlap, so the encoded clock runs ahead of the timeline.
    // Titles and captions have to follow that clock or they land on a later shot.
    let joins = mix_joins(&base);

    let mut fc = String::new();
    let mut n = 0u32;
    let mut next_label = || {
        let l = format!("n{n}");
        n += 1;
        l
    };

    let bg = oc_timeline::ffmpeg_color(&timeline.background);
    let cube_files = write_cubes(timeline, work)?;
    let mut video_pads = split_reused_video(&mut fc, &mut next_label, timeline, &index_of);

    let mut vcur = if base.is_empty() {
        let d = timeline.duration().as_seconds().max(0.1);
        let lab = next_label();
        fc.push_str(&format!(
            "color=c={bg}:s={width}x{height}:d={d:.4}:r={fps}[{lab}];"
        ));
        lab
    } else {
        stitch_base(
            &mut fc,
            &mut next_label,
            &base,
            media,
            &index_of,
            &mut video_pads,
            width,
            height,
            fps,
            &cube_files,
            &bg,
        )?
    };

    vcur = overlay_broll(
        &mut fc,
        &mut next_label,
        timeline,
        media,
        &index_of,
        &mut video_pads,
        &vcur,
        width,
        height,
        fps,
        &cube_files,
        &joins,
    )?;
    vcur = overlay_graphics(
        &mut fc,
        &mut next_label,
        timeline,
        &vcur,
        width,
        height,
        &joins,
    )?;
    vcur = overlay_front(
        &mut fc,
        &mut next_label,
        timeline,
        media,
        &index_of,
        &mut video_pads,
        &vcur,
        width,
        height,
        fps,
        &cube_files,
        &joins,
    )?;
    if timeline.letterbox && height > 8 {
        let bar = ((height as f32) * 0.12).round() as u32;
        let lab = next_label();
        fc.push_str(&format!(
            "[{vcur}]drawbox=x=0:y=0:w=iw:h={bar}:color={bg}@1:t=fill,drawbox=x=0:y=ih-{bar}:w=iw:h={bar}:color={bg}@1:t=fill[{lab}];"
        ));
        vcur = lab;
    }

    let mut captions = captions_for_cut(timeline);
    for cue in &mut captions {
        let shift = stream_shift(&joins, cue.start);
        cue.start = (cue.start - shift).max(0.0);
        cue.end = (cue.end - shift).max(cue.start + 0.04);
    }
    let srt = if captions.is_empty() {
        None
    } else {
        let path = work.join("cut.ass");
        std::fs::write(&path, to_ass(&captions, width, height, timeline.letterbox))
            .map_err(RenderError::Io)?;
        let lab = next_label();
        let mut sub = format!("[{vcur}]subtitles={}", escape_path(&path));
        if let Some(dir) = crate::font_dir() {
            sub.push_str(&format!(":fontsdir={}", escape_path(Path::new(dir))));
        }
        sub.push_str(&format!("[{lab}];"));
        fc.push_str(&sub);
        vcur = lab;
        Some(path)
    };

    let audio = stitch_audio(
        &mut fc,
        &mut next_label,
        timeline,
        media,
        &index_of,
        timeline.duration().as_seconds().max(0.1),
    );

    if fc.ends_with(';') {
        fc.pop();
    }

    Ok(Compiled {
        inputs,
        filter: fc,
        video_label: vcur,
        audio_label: audio,
        width,
        height,
        fps,
        captions,
        srt,
    })
}

fn is_join(a: &Clip, b: &Clip) -> bool {
    let a0 = a.start.as_seconds();
    let a1 = a.end().as_seconds();
    let b0 = b.start.as_seconds();
    b0 > a0 + 0.05 && b0 < a1 + 0.2 && b0 > a1 - 1.2
}

fn base_clips(timeline: &Timeline) -> Vec<&Clip> {
    let mut all: Vec<&Clip> = timeline
        .tracks
        .iter()
        .filter(|t| {
            t.kind == TrackKind::Video
                && !t.muted
                && !t.hidden
                && t.name != "GFX"
                && t.name != "Design"
                && t.name != "Front"
        })
        .flat_map(|t| t.clips.iter())
        .filter(|c| {
            !c.disabled
                && matches!(c.kind, ClipKind::Video { .. })
                && (c.media_id.is_some() || c.look.generator.is_some())
        })
        .collect();
    all.sort_by(|a, b| {
        a.start
            .cmp(&b.start)
            .then(a.id.as_uuid().cmp(&b.id.as_uuid()))
    });
    let mut out: Vec<&Clip> = Vec::new();
    for clip in all {
        if out
            .last()
            .is_none_or(|prev| is_join(prev, clip) || clip.start >= prev.end())
        {
            out.push(clip);
        }
    }
    out
}

fn has_graphics(timeline: &Timeline) -> bool {
    timeline.tracks.iter().any(|t| {
        !t.muted
            && !t.hidden
            && t.clips
                .iter()
                .any(|c| !c.disabled && matches!(c.kind, ClipKind::Graphic { .. }))
    })
}

fn write_cubes(timeline: &Timeline, work: &Path) -> Result<HashMap<u32, PathBuf>, RenderError> {
    let mut files = HashMap::new();
    if timeline.cubes.is_empty() {
        return Ok(files);
    }
    std::fs::create_dir_all(work)?;
    for cube in &timeline.cubes {
        let path = work.join(format!("lut-{}.cube", cube.id));
        std::fs::write(&path, oc_timeline::cube_text(cube))?;
        files.insert(cube.id, path);
    }
    Ok(files)
}

fn stitch_base(
    fc: &mut String,
    next_label: &mut impl FnMut() -> String,
    clips: &[&Clip],
    media: &HashMap<MediaId, MediaSource>,
    index_of: &HashMap<MediaId, usize>,
    video_pads: &mut HashMap<usize, Vec<String>>,
    width: u32,
    height: u32,
    fps: f64,
    cubes: &HashMap<u32, PathBuf>,
    bg: &str,
) -> Result<String, RenderError> {
    // stream_end is the real filter length. timeline_end is where the next clip sits.
    // A dissolve overlaps those, so the stream is shorter than the timeline. That
    // difference is not a hole, and it must not be filled with black.
    let mut acc: Option<(String, f64, f64)> = None;
    for clip in clips {
        let v = picture_branch(
            fc, next_label, clip, media, index_of, video_pads, width, height, fps, cubes, false,
        )?;
        let dur = clip.duration.as_seconds();
        let start = clip.start.as_seconds();
        match acc {
            None => {
                if start > 0.05 {
                    let pad = next_label();
                    let joined = next_label();
                    fc.push_str(&format!(
                        "color=c={bg}:s={width}x{height}:d={start:.4}:r={fps}[{pad}];[{pad}][{v}]concat=n=2:v=1:a=0[{joined}];"
                    ));
                    acc = Some((joined, start + dur, start + dur));
                } else {
                    let end = start + dur;
                    acc = Some((v, end, end));
                }
            }
            Some((prev, stream_end, timeline_end)) => {
                let gap = start - timeline_end;
                let mix = if gap.abs() < 0.05 {
                    clip_prev_transition(clips, clip)
                        .map(|left| {
                            left.look
                                .mix_window(left.duration.as_seconds(), clip.duration.as_seconds())
                        })
                        .unwrap_or(0.0)
                } else {
                    0.0
                };
                let xfade_kind = if mix > 0.05 {
                    clip_prev_transition(clips, clip).map(|l| l.look.transition)
                } else {
                    None
                };
                let mut left = prev;
                let mut stream = stream_end;
                let mut timeline = timeline_end;
                if gap > 0.05 {
                    let pad = next_label();
                    let joined = next_label();
                    fc.push_str(&format!(
                        "color=c={bg}:s={width}x{height}:d={gap:.4}:r={fps}[{pad}];[{left}][{pad}]concat=n=2:v=1:a=0[{joined}];"
                    ));
                    left = joined;
                    stream += gap;
                    timeline += gap;
                }
                let out = next_label();
                if let Some(kind) = xfade_kind.filter(|k| *k != TransitionKind::Cut) {
                    let offset = (stream - mix).max(0.0);
                    let left_tb = next_label();
                    let right_tb = next_label();
                    fc.push_str(&format!(
                        "[{left}]fps={fps},settb=AVTB[{left_tb}];[{v}]fps={fps},settb=AVTB[{right_tb}];[{left_tb}][{right_tb}]xfade=transition={}:duration={mix:.4}:offset={offset:.4}[{out}];",
                        xfade_name(kind)
                    ));
                    acc = Some((out, stream + dur - mix, timeline + dur));
                } else {
                    let left_tb = next_label();
                    let right_tb = next_label();
                    fc.push_str(&format!(
                        "[{left}]fps={fps},settb=AVTB[{left_tb}];[{v}]fps={fps},settb=AVTB[{right_tb}];[{left_tb}][{right_tb}]concat=n=2:v=1:a=0[{out}];"
                    ));
                    acc = Some((out, stream + dur, timeline + dur));
                }
            }
        }
    }
    Ok(acc.map(|(l, _, _)| l).unwrap_or_else(|| "null".into()))
}

fn clip_prev_transition<'a>(clips: &[&'a Clip], current: &Clip) -> Option<&'a Clip> {
    clips
        .iter()
        .rev()
        .find(|c| {
            c.end() <= current.start + oc_time::Duration::from_seconds(0.05) && c.id != current.id
        })
        .copied()
}

/// Incoming-clip start on the timeline, and how much xfade removes there.
fn mix_joins(clips: &[&Clip]) -> Vec<(f64, f64)> {
    let mut joins = Vec::new();
    for pair in clips.windows(2) {
        let left = pair[0];
        let right = pair[1];
        let gap = right.start.as_seconds() - left.end().as_seconds();
        if gap.abs() >= 0.05 || left.look.transition == TransitionKind::Cut {
            continue;
        }
        let mix = left
            .look
            .mix_window(left.duration.as_seconds(), right.duration.as_seconds());
        if mix > 0.05 {
            joins.push((right.start.as_seconds(), mix));
        }
    }
    joins
}

fn stream_shift(joins: &[(f64, f64)], timeline_t: f64) -> f64 {
    joins
        .iter()
        .filter(|(start, _)| *start <= timeline_t + 1e-3)
        .map(|(_, mix)| *mix)
        .sum()
}

fn picture_branch(
    fc: &mut String,
    next_label: &mut impl FnMut() -> String,
    clip: &Clip,
    media: &HashMap<MediaId, MediaSource>,
    index_of: &HashMap<MediaId, usize>,
    video_pads: &mut HashMap<usize, Vec<String>>,
    width: u32,
    height: u32,
    fps: f64,
    cubes: &HashMap<u32, PathBuf>,
    matte: bool,
) -> Result<String, RenderError> {
    let dur = clip.duration.as_seconds().max(0.04);
    let mut alpha = false;
    let mut chain = if let Some(generated) = &clip.look.generator {
        generator_video(generated, width, height, fps, dur)
    } else {
        let id = clip.media_id.ok_or(RenderError::Empty)?;
        let src = media.get(&id).ok_or(RenderError::UnknownMedia(id))?;
        alpha = source_has_alpha(&src.path);
        let idx = *index_of.get(&id).ok_or(RenderError::UnknownMedia(id))?;
        let sin = clip.source_in.as_seconds();
        let src_dur = source_span(clip, dur);
        let pad = video_pads
            .get_mut(&idx)
            .and_then(|pads| pads.pop())
            .unwrap_or_else(|| format!("{idx}:v"));
        format!("[{pad}]trim=start={sin:.4}:duration={src_dur:.4},setpts=PTS-STARTPTS")
    };
    chain.push_str(&speed_filter(clip, dur));
    if clip.look.stabilize {
        chain.push_str(",deshake");
    }
    if let Some(crop) = clip.look.crop {
        let w = crop.w.clamp(0.05, 1.0);
        let h = crop.h.clamp(0.05, 1.0);
        let x = crop.x.clamp(0.0, 1.0 - w);
        let y = crop.y.clamp(0.0, 1.0 - h);
        chain.push_str(&format!(",crop=iw*{w:.4}:ih*{h:.4}:iw*{x:.4}:ih*{y:.4}"));
    }
    let (out_w, out_h) = clip
        .look
        .card
        .map(|card| {
            (
                even_px(width as f32 * card.w.clamp(0.12, 1.0)),
                even_px(height as f32 * card.h.clamp(0.12, 1.0)),
            )
        })
        .unwrap_or((width, height));
    if clip.look.generator.is_none() {
        if let Some(prefix) = clip.media_id.and_then(|id| media.get(&id)).and_then(|src| {
            if crate::still_input(&src.path) {
                return None;
            }
            let prefix = hdr_prefix(&src.path, out_w.max(out_h));
            if prefix.is_empty() {
                None
            } else {
                Some(prefix)
            }
        }) {
            chain.push(',');
            chain.push_str(&prefix);
        }
    }
    chain.push(',');
    chain.push_str(&fit_chain(
        out_w,
        out_h,
        fps,
        clip.look.card.is_some(),
        matte && !alpha,
        alpha,
    ));
    if let ClipKind::Video { transform } = &clip.kind {
        if let Some(end) = clip.look.move_to {
            let frames = (dur * fps).max(1.0);
            let z0 = transform.scale.clamp(0.25, 4.0);
            let z1 = end.scale.clamp(0.25, 4.0);
            let x0 = pan_px(transform.x, out_w);
            let x1 = pan_px(end.x, out_w);
            let y0 = pan_px(transform.y, out_h);
            let y1 = pan_px(end.y, out_h);
            let t = ease_expr(
                clip.look.move_ease.unwrap_or_default(),
                &format!("on/{frames:.1}"),
            );
            chain.push_str(&format!(
                ",zoompan=z='{z0:.4}+({z1:.4}-{z0:.4})*({t})':x='(iw-iw/zoom)/2+({x0:.2}+({x1:.2}-{x0:.2})*({t}))':y='(ih-ih/zoom)/2+({y0:.2}+({y1:.2}-{y0:.2})*({t}))':d=1:s={out_w}x{out_h}:fps={fps}"
            ));
        } else {
            let z = transform.scale;
            if (z - 1.0).abs() > 0.01 || transform.x.abs() > 0.5 || transform.y.abs() > 0.5 {
                let z = z.clamp(0.25, 4.0);
                chain.push_str(&format!(
                    ",scale=iw*{z:.4}:ih*{z:.4},crop={out_w}:{out_h}:(in_w-{out_w})/2+({:.2}):(in_h-{out_h})/2+({:.2})",
                    pan_px(transform.x, out_w),
                    pan_px(transform.y, out_h)
                ));
            }
        }
    }
    let cube_file = clip
        .look
        .grade
        .cube
        .and_then(|id| cubes.get(&id))
        .filter(|path| path.is_file());
    chain.push_str(&eq_filters(
        &clip.look.grade,
        &clip.look.fx,
        cube_file.map(PathBuf::as_path),
    ));
    chain.push_str(&curves_filter(&clip.look.curves));
    chain.push_str(&mask_filter(clip.look.mask.as_ref(), width, height));
    let fi = clip.look.fade_in.as_seconds();
    let fo = clip.look.fade_out.as_seconds();
    if clip.look.overlay && (fi > 0.04 || fo > 0.04) {
        chain.push_str(",format=yuva420p");
        if fi > 0.04 {
            chain.push_str(&format!(",fade=t=in:st=0:d={fi:.3}:alpha=1"));
        }
        if fo > 0.04 {
            chain.push_str(&format!(
                ",fade=t=out:st={:.3}:d={fo:.3}:alpha=1",
                (dur - fo).max(0.0)
            ));
        }
    } else {
        if fi > 0.04 {
            chain.push_str(&format!(",fade=t=in:st=0:d={fi:.3}"));
        }
        if fo > 0.04 {
            chain.push_str(&format!(
                ",fade=t=out:st={:.3}:d={fo:.3}",
                (dur - fo).max(0.0)
            ));
        }
    }
    let lab = next_label();
    fc.push_str(&format!("{chain}[{lab}];"));
    Ok(lab)
}

/// A design card is contained, then an opaque plate is keyed out. A cover card is cropped.
fn design_matte(timeline: &Timeline, clip: &Clip) -> bool {
    clip.look.card.is_some()
        && timeline
            .tracks
            .iter()
            .any(|track| track.name == "Design" && track.clips.iter().any(|row| row.id == clip.id))
}

fn fit_chain(out_w: u32, out_h: u32, fps: f64, card: bool, matte: bool, alpha: bool) -> String {
    if alpha {
        // The band is already transparent. Do not key a color out of the type.
        format!(
            "format=yuva420p,scale={out_w}:{out_h}:force_original_aspect_ratio=decrease,pad={out_w}:{out_h}:(ow-iw)/2:(oh-ih)/2:color=black@0,fps={fps},format=yuva420p"
        )
    } else if matte {
        // Older plates from motion/render.mjs. The number stays; the slab does not.
        format!(
            "scale={out_w}:{out_h}:force_original_aspect_ratio=decrease,pad={out_w}:{out_h}:(ow-iw)/2:(oh-ih)/2:color=0x111111,fps={fps},format=yuv420p,colorkey=0x111111:0.08:0.04,colorkey=0x1a1814:0.08:0.04,colorkey=0x0e0e0e:0.08:0.04,colorkey=0x0c1210:0.08:0.04,colorkey=0x120c18:0.08:0.04,colorkey=0x07070c:0.08:0.04,colorkey=0xf4f7fb:0.02:0.0,format=yuva420p"
        )
    } else if card {
        format!(
            "scale={out_w}:{out_h}:force_original_aspect_ratio=increase,crop={out_w}:{out_h},fps={fps},format=yuv420p"
        )
    } else {
        format!(
            "scale={out_w}:{out_h}:force_original_aspect_ratio=decrease,pad={out_w}:{out_h}:(ow-iw)/2:(oh-ih)/2,fps={fps},format=yuv420p"
        )
    }
}

/// PQ and HLG look gray if they are scaled straight into 8-bit. Shrink in 10-bit, then tonemap.
fn hdr_prefix(path: &Path, limit: u32) -> String {
    tonemap_prefix(&color_transfer(path), limit)
}

fn tonemap_prefix(transfer: &str, limit: u32) -> String {
    let chain = hdr_chain(transfer);
    if chain.is_empty() {
        return String::new();
    }
    let limit = limit.clamp(2, 1920);
    format!("scale={limit}:{limit}:force_original_aspect_ratio=decrease:flags=lanczos,{chain}")
}

fn hdr_chain(transfer: &str) -> &'static str {
    match transfer.trim() {
        "smpte2084" => {
            "zscale=tin=smpte2084:t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv,format=yuv420p"
        }
        "arib-std-b67" => {
            "zscale=tin=arib-std-b67:t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv,format=yuv420p"
        }
        _ => "",
    }
}

/// A lower-band webm keeps its alpha in a side plane. ffprobe's built-in vp9
/// decoder still prints `pix_fmt=yuv420p`. The `alpha_mode=1` tag is the signal,
/// and that input has to be decoded with libvpx or the plane is thrown away.
#[derive(Clone, Debug, Default)]
struct StreamInfo {
    pix_fmt: String,
    codec: String,
    alpha_mode: bool,
}

fn source_has_alpha(path: &Path) -> bool {
    stream_has_alpha(&read_stream(path))
}

/// Decoder for a webm whose alpha lives in the container tag. Still images with
/// a real alpha pixel format keep the default decoder.
pub(crate) fn vpx_alpha_decoder(path: &Path) -> Option<&'static str> {
    decoder_for_stream(&read_stream(path))
}

fn stream_has_alpha(info: &StreamInfo) -> bool {
    info.alpha_mode || pix_fmt_has_alpha(&info.pix_fmt)
}

fn decoder_for_stream(info: &StreamInfo) -> Option<&'static str> {
    if !info.alpha_mode {
        return None;
    }
    match info.codec.as_str() {
        "vp8" => Some("libvpx"),
        _ => Some("libvpx-vp9"),
    }
}

fn pix_fmt_has_alpha(fmt: &str) -> bool {
    let fmt = fmt.trim().to_ascii_lowercase();
    fmt.contains("yuva")
        || fmt.contains("rgba")
        || fmt.contains("argb")
        || fmt.contains("bgra")
        || fmt.contains("abgr")
        || fmt.contains("gbra")
}

fn read_stream(path: &Path) -> StreamInfo {
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, StreamInfo>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let key = path.to_path_buf();
    if let Ok(guard) = cache.lock() {
        if let Some(found) = guard.get(&key) {
            return found.clone();
        }
    }
    let found = probe_stream(path);
    if let Ok(mut guard) = cache.lock() {
        guard.insert(key, found.clone());
    }
    found
}

fn probe_stream(path: &Path) -> StreamInfo {
    let Ok(out) = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=codec_name,pix_fmt:stream_tags=alpha_mode",
            "-of",
            "default=nw=1",
        ])
        .arg(path)
        .output()
    else {
        return StreamInfo::default();
    };
    if !out.status.success() {
        return StreamInfo::default();
    }
    parse_stream_probe(&String::from_utf8_lossy(&out.stdout))
}

fn parse_stream_probe(text: &str) -> StreamInfo {
    let mut info = StreamInfo::default();
    for raw in text.lines() {
        let line = raw.trim();
        let lower = line.to_ascii_lowercase();
        if let Some(value) = lower.strip_prefix("pix_fmt=") {
            info.pix_fmt = value.trim().to_string();
        } else if let Some(value) = lower.strip_prefix("codec_name=") {
            info.codec = value.trim().to_string();
        } else if alpha_mode_is_one(&lower) {
            info.alpha_mode = true;
        }
    }
    if info.pix_fmt.is_empty() && !text.contains('=') {
        info.pix_fmt = text.trim().to_ascii_lowercase();
    }
    info
}

fn alpha_mode_is_one(lower: &str) -> bool {
    let Some(rest) = lower.split("alpha_mode=").nth(1) else {
        return false;
    };
    let digits: String = rest
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits == "1"
}

fn color_transfer(path: &Path) -> String {
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, String>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let key = path.to_path_buf();
    if let Ok(guard) = cache.lock() {
        if let Some(found) = guard.get(&key) {
            return found.clone();
        }
    }
    let found = read_color_transfer(path);
    if let Ok(mut guard) = cache.lock() {
        guard.insert(key, found.clone());
    }
    found
}

fn read_color_transfer(path: &Path) -> String {
    let Ok(out) = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=color_transfer",
            "-of",
            "default=nw=1:nk=1",
        ])
        .arg(path)
        .output()
    else {
        return String::new();
    };
    if !out.status.success() {
        return String::new();
    }
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn source_span(clip: &Clip, dur: f64) -> f64 {
    if clip.look.speed_keys.len() >= 2 {
        let mut span = 0.0;
        let keys = &clip.look.speed_keys;
        for pair in keys.windows(2) {
            let dt = f64::from((pair[1].at - pair[0].at).max(0.0)) * dur;
            let rate = (f64::from(pair[0].speed) + f64::from(pair[1].speed)) * 0.5;
            span += dt * rate.clamp(0.25, 4.0);
        }
        let tail = (1.0 - f64::from(keys.last().map(|k| k.at).unwrap_or(1.0))).max(0.0) * dur;
        span +=
            tail * f64::from(keys.last().map(|k| k.speed).unwrap_or(clip.speed)).clamp(0.25, 4.0);
        return span.max(0.04);
    }
    let speed = speed_of(clip);
    let speed_end = f64::from(clip.look.speed_to.unwrap_or(clip.speed)).clamp(0.25, 4.0);
    dur * (speed + speed_end) * 0.5
}

fn speed_filter(clip: &Clip, dur: f64) -> String {
    if clip.look.speed_keys.len() >= 2 {
        let expr = speed_expr(&clip.look.speed_keys, dur);
        return format!(",setpts='PTS/({expr})'");
    }
    let speed = speed_of(clip);
    let speed_end = f64::from(clip.look.speed_to.unwrap_or(clip.speed)).clamp(0.25, 4.0);
    if (speed - 1.0).abs() > 0.02 || (speed_end - speed).abs() > 0.02 {
        format!(",setpts='PTS/({speed:.4}+({speed_end:.4}-{speed:.4})*T/{dur:.4})'")
    } else {
        String::new()
    }
}

/// Piecewise linear speed, the same interpolation Kdenlive uses between linear keyframes.
fn speed_expr(keys: &[oc_timeline::SpeedKey], dur: f64) -> String {
    let dur = dur.max(0.04);
    let mut expr = format!(
        "{:.4}",
        f64::from(keys.last().map(|k| k.speed).unwrap_or(1.0)).clamp(0.25, 4.0)
    );
    for pair in keys.windows(2).rev() {
        let t1 = f64::from(pair[1].at).clamp(0.0, 1.0) * dur;
        let s0 = f64::from(pair[0].speed).clamp(0.25, 4.0);
        let s1 = f64::from(pair[1].speed).clamp(0.25, 4.0);
        let span = (t1 - f64::from(pair[0].at).clamp(0.0, 1.0) * dur).max(0.04);
        expr = format!(
            "if(lt(T\\,{t1:.4})\\,{s0:.4}+({s1:.4}-{s0:.4})*(T-{:.4})/{span:.4}\\,{expr})",
            f64::from(pair[0].at).clamp(0.0, 1.0) * dur
        );
    }
    expr
}

fn generator_video(
    generated: &oc_timeline::Generator,
    width: u32,
    height: u32,
    fps: f64,
    dur: f64,
) -> String {
    match generated {
        oc_timeline::Generator::Color { color } => {
            let hex = color.trim().trim_start_matches('#').to_string();
            format!("color=c=0x{hex}:s={width}x{height}:r={fps}:d={dur:.3},format=yuv420p")
        }
        oc_timeline::Generator::ColorBars => {
            format!("smptebars=s={width}x{height}:r={fps}:d={dur:.3},format=yuv420p")
        }
        oc_timeline::Generator::WhiteNoise => {
            format!(
                "color=c=black:s={width}x{height}:r={fps}:d={dur:.3},noise=alls=40:allf=t+u,format=yuv420p"
            )
        }
        oc_timeline::Generator::Counter => {
            format!(
                "color=c=0x202020:s={width}x{height}:r={fps}:d={dur:.3},drawtext=text='%{{pts\\:hms}}':fontsize={}:fontcolor=white:x=(w-text_w)/2:y=(h-text_h)/2,format=yuv420p",
                (height / 8).max(24)
            )
        }
    }
}

fn curves_filter(curves: &oc_timeline::Curves) -> String {
    if curves.is_identity() {
        return String::new();
    }
    let mut parts = Vec::new();
    if let Some(p) = curve_points(&curves.all) {
        parts.push(format!("master='{p}'"));
    }
    if let Some(p) = curve_points(&curves.red) {
        parts.push(format!("red='{p}'"));
    }
    if let Some(p) = curve_points(&curves.green) {
        parts.push(format!("green='{p}'"));
    }
    if let Some(p) = curve_points(&curves.blue) {
        parts.push(format!("blue='{p}'"));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(",curves={}", parts.join(":"))
    }
}

fn curve_points(points: &[oc_timeline::CurvePoint]) -> Option<String> {
    if points.is_empty() {
        return None;
    }
    let mut pts: Vec<_> = points
        .iter()
        .map(|p| (p.x.clamp(0.0, 1.0), p.y.clamp(0.0, 1.0)))
        .collect();
    pts.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    if pts.first().is_none_or(|p| p.0 > 0.01) {
        pts.insert(0, (0.0, 0.0));
    }
    if pts.last().is_none_or(|p| p.0 < 0.99) {
        pts.push((1.0, 1.0));
    }
    Some(
        pts.iter()
            .map(|(x, y)| format!("{x:.3}/{y:.3}"))
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn mask_filter(mask: Option<&oc_timeline::AlphaShape>, width: u32, height: u32) -> String {
    let Some(mask) = mask else {
        return String::new();
    };
    let w = f64::from(width);
    let h = f64::from(height);
    let cx = f64::from(mask.x.clamp(0.0, 1.0)) * w;
    let cy = f64::from(mask.y.clamp(0.0, 1.0)) * h;
    let rw = (f64::from(mask.w.clamp(0.02, 1.0)) * w * 0.5).max(2.0);
    let rh = (f64::from(mask.h.clamp(0.02, 1.0)) * h * 0.5).max(2.0);
    let feather = (f64::from(mask.feather.clamp(0.0, 1.0)) * rw.min(rh) * 0.5).max(0.0);
    let inside = match mask.shape {
        oc_timeline::MaskShape::Rectangle | oc_timeline::MaskShape::Diamond => {
            // Diamond uses a rotated box via manhattan distance.
            if matches!(mask.shape, oc_timeline::MaskShape::Diamond) {
                format!("lte(abs(X-{cx:.1})/{rw:.1}+abs(Y-{cy:.1})/{rh:.1}\\,1)")
            } else {
                format!("lte(abs(X-{cx:.1})\\,{rw:.1})*lte(abs(Y-{cy:.1})\\,{rh:.1})")
            }
        }
        oc_timeline::MaskShape::Ellipse => {
            format!("lte(pow((X-{cx:.1})/{rw:.1}\\,2)+pow((Y-{cy:.1})/{rh:.1}\\,2)\\,1)")
        }
        oc_timeline::MaskShape::Triangle => {
            format!(
                "gte(Y\\,{:.1})*lte(Y\\,{:.1})*lte(abs(X-{cx:.1})\\,{rw:.1}*(Y-{:.1})/{:.1})",
                cy - rh,
                cy + rh,
                cy - rh,
                (2.0 * rh).max(1.0)
            )
        }
    };
    let alpha = if feather > 1.0 {
        format!("clip(255*(1-({inside}))\\,0\\,255)")
    } else if mask.invert {
        format!("if({inside}\\,0\\,255)")
    } else {
        format!("if({inside}\\,255\\,0)")
    };
    // Feather is a blur of the alpha plane after the hard shape.
    let blur = if feather > 1.0 {
        format!(
            ",gblur=sigma={:.2}:planes=7",
            (feather / 8.0).clamp(0.4, 12.0)
        )
    } else {
        String::new()
    };
    let hard = if feather > 1.0 {
        let solid = if mask.invert {
            format!("if({inside}\\,0\\,255)")
        } else {
            format!("if({inside}\\,255\\,0)")
        };
        format!(",format=yuva420p,geq=lum='p(X,Y)':a='{solid}'{blur}")
    } else {
        format!(",format=yuva420p,geq=lum='p(X,Y)':a='{alpha}'")
    };
    let _ = alpha;
    hard
}

/// Fractions of the frame (−1..1) become pixels. Values already in pixels are left alone.
fn pan_px(value: f32, span: u32) -> f32 {
    if value.abs() <= 1.5 {
        value * span as f32
    } else {
        value
    }
}

fn ease_expr(ease: oc_timeline::Ease, p: &str) -> String {
    match ease {
        oc_timeline::Ease::Linear => p.to_string(),
        oc_timeline::Ease::In => format!("({p})*({p})"),
        oc_timeline::Ease::Out => format!("1-(1-({p}))*(1-({p}))"),
        oc_timeline::Ease::InOut => {
            format!("if(lt({p}\\,0.5)\\,2*({p})*({p})\\,1-2*(1-({p}))*(1-({p})))")
        }
    }
}

fn eq_filters(
    grade: &oc_timeline::Grade,
    fx: &oc_timeline::Fx,
    cube_file: Option<&Path>,
) -> String {
    let mut s = String::new();
    // The monitor grades in RGB. `colorbalance` on yuv420p is a different picture:
    // it lifts the blacks and knocks the color out. Do the same math here, in RGB,
    // then convert to video levels once.
    let procedural = grade_moves(grade);
    if procedural || cube_file.is_some() {
        let affine = grade_affine(grade);
        if procedural {
            s.push_str(&grade_mixer(&affine));
        } else {
            s.push_str(",format=gbrp");
        }
        if let Some(path) = cube_file {
            // A cube is sampled on the clamped grade. The float path can sit above 1.
            if procedural && !mixer_fits(&affine) {
                s.push_str(",format=gbrp");
            }
            s.push_str(&format!(",lut3d=file={}", escape_path(path)));
        }
        s.push_str(",format=yuv420p");
    }
    let alls = fx.noise_alls();
    if alls > 0.0 {
        s.push_str(&format!(",noise=alls={alls:.1}:allf=t"));
    }
    if fx.vignette > 0.02 {
        let angle = (std::f32::consts::PI / 5.0) * fx.vignette.clamp(0.0, 1.0);
        s.push_str(&format!(",vignette=angle={angle:.3}"));
    }
    if fx.blur > 0.02 {
        let r = (fx.blur * 6.0).clamp(0.5, 8.0);
        s.push_str(&format!(",gblur=sigma={r:.2}"));
    }
    s
}

fn grade_moves(grade: &oc_timeline::Grade) -> bool {
    grade.exposure.abs() > 1e-4
        || grade.contrast.abs() > 1e-4
        || grade.saturation.abs() > 1e-4
        || grade.temperature.abs() > 1e-3
        || grade.lift.abs() > 1e-3
        || grade.gamma.abs() > 1e-3
        || grade.gain.abs() > 1e-3
        || grade.lut != oc_timeline::Lut::None
}

/// One RGB affine step. Rows mix r, g, b. `b` is added after the mix.
#[derive(Clone, Copy)]
struct Affine {
    m: [[f64; 3]; 3],
    b: [f64; 3],
}

fn ident_affine() -> Affine {
    Affine {
        m: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        b: [0.0, 0.0, 0.0],
    }
}

/// `outer` runs after `inner`: `outer * (inner * x) + bias`.
fn compose(outer: Affine, inner: Affine) -> Affine {
    let mut m = [[0.0; 3]; 3];
    for r in 0..3 {
        for c in 0..3 {
            m[r][c] = outer.m[r][0] * inner.m[0][c]
                + outer.m[r][1] * inner.m[1][c]
                + outer.m[r][2] * inner.m[2][c];
        }
    }
    let mut b = [0.0; 3];
    for r in 0..3 {
        b[r] = outer.m[r][0] * inner.b[0]
            + outer.m[r][1] * inner.b[1]
            + outer.m[r][2] * inner.b[2]
            + outer.b[r];
    }
    Affine { m, b }
}

fn scale_affine(k: f64) -> Affine {
    Affine {
        m: [[k, 0.0, 0.0], [0.0, k, 0.0], [0.0, 0.0, k]],
        b: [0.0, 0.0, 0.0],
    }
}

fn bias_affine(b: [f64; 3]) -> Affine {
    let mut affine = ident_affine();
    affine.b = b;
    affine
}

const LUMA_R: f64 = 0.2126;
const LUMA_G: f64 = 0.7152;
const LUMA_B: f64 = 0.0722;

fn saturation_affine(sat: f64) -> Affine {
    let w = 1.0 - sat;
    Affine {
        m: [
            [sat + w * LUMA_R, w * LUMA_G, w * LUMA_B],
            [w * LUMA_R, sat + w * LUMA_G, w * LUMA_B],
            [w * LUMA_R, w * LUMA_G, sat + w * LUMA_B],
        ],
        b: [0.0, 0.0, 0.0],
    }
}

/// Same formula as `oc_compositor` `grade_rgb`, as one matrix plus a bias.
fn grade_affine(grade: &oc_timeline::Grade) -> Affine {
    let contrast = 1.0 + f64::from(grade.contrast) + f64::from(grade.gamma) * 0.45;
    let brightness = 1.0
        + f64::from(grade.exposure)
        + f64::from(grade.gain) * 0.35
        + f64::from(grade.lift) * 0.15;
    let sat = (1.0 + f64::from(grade.saturation)).max(0.0);
    let k = contrast * brightness;
    let off = 0.5 * brightness * (1.0 - contrast);
    let contrast_step = Affine {
        m: [[k, 0.0, 0.0], [0.0, k, 0.0], [0.0, 0.0, k]],
        b: [off, off, off],
    };
    let temperature = f64::from(grade.temperature) * 0.15;
    let temp_step = bias_affine([temperature, 0.0, -temperature]);
    let lut_step = lut_affine(grade.lut);
    compose(
        lut_step,
        compose(temp_step, compose(saturation_affine(sat), contrast_step)),
    )
}

fn lut_affine(lut: oc_timeline::Lut) -> Affine {
    use oc_timeline::Lut;
    match lut {
        Lut::None => ident_affine(),
        Lut::Warm => bias_affine([0.08, 0.03, -0.04]),
        Lut::Cool => bias_affine([-0.04, 0.0, 0.08]),
        Lut::TealOrange => bias_affine([0.06, 0.0, 0.05]),
        Lut::Film => compose(bias_affine([0.03, 0.0, -0.015]), saturation_affine(1.16)),
        Lut::Mono => Affine {
            m: [
                [LUMA_R, LUMA_G, LUMA_B],
                [LUMA_R, LUMA_G, LUMA_B],
                [LUMA_R, LUMA_G, LUMA_B],
            ],
            b: [0.0, 0.0, 0.0],
        },
    }
}

/// ffmpeg rejects a `colorchannelmixer` coefficient outside this range.
fn mixer_fits(affine: &Affine) -> bool {
    affine
        .m
        .iter()
        .flatten()
        .chain(affine.b.iter())
        .all(|c| c.abs() <= 2.0)
}

fn mixer_args(affine: &Affine) -> String {
    format!(
        "colorchannelmixer=rr={:.6}:rg={:.6}:rb={:.6}:ra={:.6}:gr={:.6}:gg={:.6}:gb={:.6}:ga={:.6}:br={:.6}:bg={:.6}:bb={:.6}:ba={:.6}",
        affine.m[0][0],
        affine.m[0][1],
        affine.m[0][2],
        affine.b[0],
        affine.m[1][0],
        affine.m[1][1],
        affine.m[1][2],
        affine.b[1],
        affine.m[2][0],
        affine.m[2][1],
        affine.m[2][2],
        affine.b[2]
    )
}

/// Kdenlive's color grade is this filter: a compiled 3×3 mix, not a `geq` script.
/// `geq` runs the formula on every pixel and is what makes a short export take hours.
/// Coefficients have to stay in [-2, 2]. A stronger grade is the same mix on float
/// pixels, split into legal steps, so the highlight is clipped once at the end.
fn grade_mixer(affine: &Affine) -> String {
    if mixer_fits(affine) {
        return format!(",format=gbrap,{}", mixer_args(affine));
    }
    let mut chain = vec!["format=gbrapf32le".to_string()];
    let mut linear = *affine;
    linear.b = [0.0; 3];
    let mut ups = 0u32;
    while !mixer_fits(&linear) && ups < 16 {
        for row in &mut linear.m {
            for coeff in row.iter_mut() {
                *coeff /= 2.0;
            }
        }
        ups += 1;
    }
    chain.push(mixer_args(&linear));
    for _ in 0..ups {
        chain.push(mixer_args(&scale_affine(2.0)));
    }
    let mut left = affine.b;
    let mut guard = 0;
    while left.iter().any(|c| c.abs() > 2.0) && guard < 16 {
        let mut step = [0.0; 3];
        for i in 0..3 {
            if left[i] > 2.0 {
                step[i] = 2.0;
                left[i] -= 2.0;
            } else if left[i] < -2.0 {
                step[i] = -2.0;
                left[i] += 2.0;
            }
        }
        chain.push(mixer_args(&bias_affine(step)));
        guard += 1;
    }
    if left.iter().any(|c| c.abs() > 1e-6) {
        chain.push(mixer_args(&bias_affine(left)));
    }
    format!(",{}", chain.join(","))
}

fn audio_fx(fx: &oc_timeline::AudioFx) -> String {
    let mut s = String::new();
    if fx.denoise {
        s.push_str(&format!(
            ",afftdn=nr={:.0}:nf={:.0}",
            oc_timeline::DENOISE_NR_DB,
            oc_timeline::DENOISE_NF_DB
        ));
    }
    if fx.low.abs() > 0.05 {
        s.push_str(&format!(
            ",equalizer=f=120:t=q:w=1:g={:.2}",
            fx.low.clamp(-12.0, 12.0)
        ));
    }
    if fx.mid.abs() > 0.05 {
        s.push_str(&format!(
            ",equalizer=f=1000:t=q:w=1:g={:.2}",
            fx.mid.clamp(-12.0, 12.0)
        ));
    }
    if fx.high.abs() > 0.05 {
        s.push_str(&format!(
            ",equalizer=f=8000:t=q:w=1:g={:.2}",
            fx.high.clamp(-12.0, 12.0)
        ));
    }
    if fx.compressor {
        s.push_str(",acompressor=threshold=-18dB:ratio=3:attack=20:release=200");
    }
    if fx.normalize {
        s.push_str(",dynaudnorm=f=150:g=15");
    }
    s
}

fn even_px(value: f32) -> u32 {
    let n = value.round().max(2.0) as u32;
    (n + n % 2).max(2)
}

fn broll_clips(timeline: &Timeline) -> Vec<&Clip> {
    let program = base_clips(timeline);
    let mut out = Vec::new();
    for track in &timeline.tracks {
        if track.kind != TrackKind::Video
            || track.muted
            || track.hidden
            || track.name == "GFX"
            || track.name == "Front"
        {
            continue;
        }
        for clip in &track.clips {
            if clip.disabled
                || !matches!(clip.kind, ClipKind::Video { .. })
                || (clip.media_id.is_none() && clip.look.generator.is_none())
            {
                continue;
            }
            if program.iter().any(|c| c.id == clip.id) {
                continue;
            }
            out.push(clip);
        }
    }
    out
}

fn front_clips(timeline: &Timeline) -> Vec<&Clip> {
    let mut out = Vec::new();
    for track in &timeline.tracks {
        if track.kind != TrackKind::Video || track.muted || track.hidden || track.name != "Front" {
            continue;
        }
        for clip in &track.clips {
            if clip.disabled
                || !matches!(clip.kind, ClipKind::Video { .. })
                || clip.media_id.is_none()
            {
                continue;
            }
            out.push(clip);
        }
    }
    out
}

/// One input pad can feed the filter graph once. The speaker is also the corner window.
fn split_reused_video(
    fc: &mut String,
    next_label: &mut impl FnMut() -> String,
    timeline: &Timeline,
    index_of: &HashMap<MediaId, usize>,
) -> HashMap<usize, Vec<String>> {
    let mut uses: HashMap<usize, usize> = HashMap::new();
    let mut note = |clip: &Clip| {
        if clip.look.generator.is_some() {
            return;
        }
        let Some(id) = clip.media_id else { return };
        let Some(&idx) = index_of.get(&id) else {
            return;
        };
        *uses.entry(idx).or_insert(0) += 1;
    };
    for clip in base_clips(timeline) {
        note(clip);
    }
    for clip in broll_clips(timeline) {
        note(clip);
    }
    for clip in front_clips(timeline) {
        note(clip);
    }
    let mut pads = HashMap::new();
    for (idx, n) in uses {
        if n <= 1 {
            continue;
        }
        let labels: Vec<String> = (0..n).map(|_| next_label()).collect();
        let outs: String = labels.iter().map(|label| format!("[{label}]")).collect();
        fc.push_str(&format!("[{idx}:v]split={n}{outs};"));
        pads.insert(idx, labels);
    }
    pads
}

fn overlay_broll(
    fc: &mut String,
    next_label: &mut impl FnMut() -> String,
    timeline: &Timeline,
    media: &HashMap<MediaId, MediaSource>,
    index_of: &HashMap<MediaId, usize>,
    video_pads: &mut HashMap<usize, Vec<String>>,
    base: &str,
    width: u32,
    height: u32,
    fps: f64,
    cubes: &HashMap<u32, PathBuf>,
    joins: &[(f64, f64)],
) -> Result<String, RenderError> {
    let mut cur = base.to_string();
    for clip in broll_clips(timeline) {
        let matte = design_matte(timeline, clip);
        let v = picture_branch(
            fc, next_label, clip, media, index_of, video_pads, width, height, fps, cubes, matte,
        )?;
        cur = paste_overlay(fc, next_label, clip, &v, &cur, width, height, joins);
    }
    Ok(cur)
}

fn overlay_front(
    fc: &mut String,
    next_label: &mut impl FnMut() -> String,
    timeline: &Timeline,
    media: &HashMap<MediaId, MediaSource>,
    index_of: &HashMap<MediaId, usize>,
    video_pads: &mut HashMap<usize, Vec<String>>,
    base: &str,
    width: u32,
    height: u32,
    fps: f64,
    cubes: &HashMap<u32, PathBuf>,
    joins: &[(f64, f64)],
) -> Result<String, RenderError> {
    let mut cur = base.to_string();
    for clip in front_clips(timeline) {
        let v = picture_branch(
            fc, next_label, clip, media, index_of, video_pads, width, height, fps, cubes, false,
        )?;
        cur = paste_overlay(fc, next_label, clip, &v, &cur, width, height, joins);
    }
    Ok(cur)
}

fn paste_overlay(
    fc: &mut String,
    next_label: &mut impl FnMut() -> String,
    clip: &Clip,
    branch: &str,
    base: &str,
    width: u32,
    height: u32,
    joins: &[(f64, f64)],
) -> String {
    let shifted = next_label();
    let raw_start = clip.start.as_seconds();
    let shift = stream_shift(joins, raw_start);
    let start = (raw_start - shift).max(0.0);
    let end = (clip.end().as_seconds() - shift).max(start);
    fc.push_str(&format!(
        "[{branch}]setpts=PTS-STARTPTS+{start:.4}/TB[{shifted}];"
    ));
    let out = next_label();
    let (ox, oy) = clip.look.card.map_or((0, 0), |card| {
        (
            (f64::from(card.x.clamp(0.0, 1.0)) * f64::from(width)).round() as i32,
            (f64::from(card.y.clamp(0.0, 1.0)) * f64::from(height)).round() as i32,
        )
    });
    let alpha = if clip.look.mask.is_some() || clip.look.overlay {
        ":format=auto"
    } else {
        ""
    };
    fc.push_str(&format!(
        "[{base}][{shifted}]overlay={ox}:{oy}{alpha}:enable='between(t,{start:.4},{end:.4})'[{out}];"
    ));
    out
}

fn overlay_graphics(
    fc: &mut String,
    next_label: &mut impl FnMut() -> String,
    timeline: &Timeline,
    base: &str,
    width: u32,
    height: u32,
    joins: &[(f64, f64)],
) -> Result<String, RenderError> {
    let mut cur = base.to_string();
    let font = crate::font_path();
    for track in &timeline.tracks {
        if track.muted || track.hidden {
            continue;
        }
        for clip in &track.clips {
            if clip.disabled {
                continue;
            }
            let ClipKind::Graphic { graphic } = &clip.kind else {
                continue;
            };
            let raw_start = clip.start.as_seconds();
            let shift = stream_shift(joins, raw_start);
            let start = (raw_start - shift).max(0.0);
            let end = (clip.end().as_seconds() - shift).max(start);
            let enable = format!("enable='between(t,{start:.4},{end:.4})'");
            let alpha = clip
                .look
                .fade_gain(0.5 * clip.duration.as_seconds(), clip.duration.as_seconds());
            let placed = graphic.x.zip(graphic.y).map(|(x, y)| {
                (
                    format!("(w*{x:.4}-text_w/2)"),
                    format!("(h*{y:.4}-text_h/2)"),
                )
            });
            let out = next_label();
            match graphic.kind {
                GraphicKind::Shape => {
                    let w = width / 4;
                    let h = height / 4;
                    let x = (width - w) / 2;
                    let y = (height - h) / 2;
                    fc.push_str(&format!(
                        "[{cur}]drawbox=x={x}:y={y}:w={w}:h={h}:color=0x4db8ff@{alpha:.2}:t=fill:{enable}[{out}];"
                    ));
                }
                GraphicKind::Sticker => {
                    let text = if graphic.text.trim().is_empty() {
                        "*".into()
                    } else {
                        graphic.text.clone()
                    };
                    fc.push_str(&drawtext(
                        &cur,
                        &out,
                        &text,
                        width / 8,
                        "(w-text_w)/2",
                        "(h-text_h)/2",
                        &enable,
                        font.as_deref(),
                    ));
                }
                GraphicKind::LowerThird => {
                    let (x, y) =
                        placed.unwrap_or_else(|| ("w/16".into(), format!("h-{}", height / 6)));
                    fc.push_str(&drawtext(
                        &cur,
                        &out,
                        &graphic.text,
                        width / 22,
                        &x,
                        &y,
                        &enable,
                        font.as_deref(),
                    ));
                }
                GraphicKind::Card => {
                    let box_out = next_label();
                    fc.push_str(&format!(
                        "[{cur}]drawbox=x=w/8:y=h/4:w=w*3/4:h=h/3:color=black@0.7:t=fill:{enable}[{box_out}];"
                    ));
                    fc.push_str(&drawtext(
                        &box_out,
                        &out,
                        &graphic.text,
                        width / 18,
                        "(w-text_w)/2",
                        "(h-text_h)/2",
                        &enable,
                        font.as_deref(),
                    ));
                }
                GraphicKind::Title => {
                    let (x, y) =
                        placed.unwrap_or_else(|| ("(w-text_w)/2".into(), "(h-text_h)/2".into()));
                    fc.push_str(&drawtext(
                        &cur,
                        &out,
                        &graphic.text,
                        width / 12,
                        &x,
                        &y,
                        &enable,
                        font.as_deref(),
                    ));
                }
            }
            cur = out;
        }
    }
    Ok(cur)
}

fn drawtext(
    input: &str,
    output: &str,
    text: &str,
    fontsize: u32,
    x: &str,
    y: &str,
    enable: &str,
    font: Option<&str>,
) -> String {
    let mut d = format!(
        "[{input}]drawtext=text='{}':fontsize={fontsize}:fontcolor=white:x={x}:y={y}:shadowcolor=black:shadowx=2:shadowy=2:{enable}",
        escape_text(text)
    );
    if let Some(font) = font {
        d.push_str(&format!(":fontfile={}", escape_path(Path::new(font))));
    }
    d.push_str(&format!("[{output}];"));
    d
}

fn stitch_audio(
    fc: &mut String,
    next_label: &mut impl FnMut() -> String,
    timeline: &Timeline,
    media: &HashMap<MediaId, MediaSource>,
    index_of: &HashMap<MediaId, usize>,
    total: f64,
) -> Option<String> {
    let total = total.max(0.1);
    let program = base_clips(timeline);
    let mut need: HashMap<usize, usize> = HashMap::new();
    let mut note = |idx: usize| {
        *need.entry(idx).or_insert(0) += 1;
    };
    for clip in &program {
        if linked_audio_voice(timeline, clip) {
            continue;
        }
        if let Some(idx) = audio_index(clip, media, index_of) {
            note(idx);
        }
    }
    for track in &timeline.tracks {
        if track.muted || track.hidden || track.kind != TrackKind::Audio {
            continue;
        }
        for clip in &track.clips {
            if clip.disabled {
                continue;
            }
            let ClipKind::Audio { volume, .. } = &clip.kind else {
                continue;
            };
            if *volume <= 0.02 {
                continue;
            }
            if let Some(idx) = audio_index(clip, media, index_of) {
                note(idx);
            }
        }
    }
    let mut pools: HashMap<usize, Vec<String>> = HashMap::new();
    for (idx, n) in need {
        if n <= 1 {
            pools.insert(idx, vec![format!("{idx}:a")]);
            continue;
        }
        let labels: Vec<String> = (0..n).map(|_| next_label()).collect();
        let outs: String = labels.iter().map(|l| format!("[{l}]")).collect();
        fc.push_str(&format!("[{idx}:a]asplit={n}{outs};"));
        pools.insert(idx, labels);
    }

    let mut program_label: Option<String> = None;
    let mut timeline_at = 0.0;
    let mut previous: Option<&Clip> = None;
    for clip in &program {
        let start = clip.start.as_seconds();
        let dur = clip.duration.as_seconds().max(0.04);
        let mut parts = Vec::new();
        if start - timeline_at > 0.05 {
            parts.push(silence(fc, next_label, start - timeline_at));
        }
        if linked_audio_voice(timeline, clip) {
            parts.push(silence(fc, next_label, dur));
        } else {
            parts.push(voice_piece(
                fc, next_label, clip, media, index_of, &mut pools, dur,
            ));
        }
        let piece = concat_audio(fc, next_label, parts);
        let mix = if (start - timeline_at).abs() < 0.05 {
            previous
                .map(|left| {
                    left.look
                        .mix_window(left.duration.as_seconds(), clip.duration.as_seconds())
                })
                .filter(|mix| {
                    *mix > 0.05
                        && previous.is_some_and(|left| left.look.transition != TransitionKind::Cut)
                })
                .unwrap_or(0.0)
        } else {
            0.0
        };
        program_label = Some(match program_label {
            None => piece,
            Some(prev) if mix > 0.05 => {
                let out = next_label();
                fc.push_str(&format!(
                    "[{prev}][{piece}]acrossfade=d={mix:.4}:c1=tri:c2=tri[{out}];"
                ));
                out
            }
            Some(prev) => concat_audio(fc, next_label, vec![prev, piece]),
        });
        timeline_at = start + dur;
        previous = Some(clip);
    }
    if total - timeline_at > 0.05 {
        let gap = silence(fc, next_label, total - timeline_at);
        program_label = Some(match program_label {
            None => gap,
            Some(prev) => concat_audio(fc, next_label, vec![prev, gap]),
        });
    }
    let mut program_label = program_label.unwrap_or_else(|| silence(fc, next_label, total));

    let solo = timeline
        .tracks
        .iter()
        .any(|t| t.kind == TrackKind::Audio && t.mix.solo && !t.muted);
    let mut beds = vec![program_label.clone()];
    for track in &timeline.tracks {
        if track.muted || track.hidden || track.kind != TrackKind::Audio {
            continue;
        }
        if solo && !track.mix.solo {
            continue;
        }
        let (left, right) = track.mix.balance();
        let track_gain = f64::from(track.mix.linear());
        for clip in &track.clips {
            if clip.disabled {
                continue;
            }
            let ClipKind::Audio { volume, .. } = &clip.kind else {
                continue;
            };
            if *volume <= 0.02 && clip.look.generator.is_none() {
                continue;
            }
            let start = clip.start.as_seconds();
            let dur = clip.duration.as_seconds().max(0.04);
            let ms = (start * 1000.0).round().max(0.0) as i64;
            let gain = (*volume as f64 * track_gain).max(0.0);
            let lab = next_label();
            if let Some(generated) = &clip.look.generator {
                let src = match generated {
                    oc_timeline::Generator::WhiteNoise => {
                        format!("anoisesrc=color=white:sample_rate=48000:d={dur:.3}")
                    }
                    oc_timeline::Generator::Counter => {
                        format!("sine=frequency=1000:sample_rate=48000:d={dur:.3}")
                    }
                    _ => format!("anullsrc=channel_layout=stereo:sample_rate=48000:d={dur:.3}"),
                };
                fc.push_str(&format!(
                    "{src},volume={gain:.3},pan=stereo|c0={left:.3}*c0|c1={right:.3}*c0,aformat=sample_fmts=fltp:sample_rates=48000:channel_layouts=stereo,adelay={ms}|{ms}:all=1,apad=whole_dur={total:.4}[{lab}];"
                ));
            } else {
                let Some(raw) =
                    pools.get_mut(&audio_index(clip, media, index_of).unwrap_or(usize::MAX))
                else {
                    continue;
                };
                let Some(src) = raw.pop() else { continue };
                fc.push_str(&format!(
                    "[{src}]atrim=start={:.4}:duration={:.4},asetpts=PTS-STARTPTS,volume={gain:.3}{},pan=stereo|c0={left:.3}*c0|c1={right:.3}*c1,aformat=sample_fmts=fltp:sample_rates=48000:channel_layouts=stereo,adelay={ms}|{ms}:all=1,apad=whole_dur={total:.4}[{lab}];",
                    clip.source_in.as_seconds(),
                    dur * speed_of(clip),
                    audio_fx(&clip.look.audio)
                ));
            }
            beds.push(lab);
        }
    }
    if beds.len() > 1 {
        program_label = {
            let out = next_label();
            let ins: String = beds.iter().map(|l| format!("[{l}]")).collect();
            fc.push_str(&format!(
                "{ins}amix=inputs={}:duration=first:normalize=0[{out}];",
                beds.len()
            ));
            out
        };
    }
    let master = f64::from(timeline.master.linear());
    if (master - 1.0).abs() > 0.01 {
        let out = next_label();
        fc.push_str(&format!("[{program_label}]volume={master:.4}[{out}];"));
        program_label = out;
    }
    Some(program_label)
}

fn linked_audio_voice(timeline: &Timeline, clip: &Clip) -> bool {
    let Some(link) = clip.link_id else {
        return false;
    };
    timeline
        .tracks
        .iter()
        .flat_map(|track| track.clips.iter())
        .any(|other| {
            other.id != clip.id
                && other.link_id == Some(link)
                && !other.disabled
                && matches!(other.kind, ClipKind::Audio { volume, .. } if volume > 0.02)
        })
}

fn audio_index(
    clip: &Clip,
    media: &HashMap<MediaId, MediaSource>,
    index_of: &HashMap<MediaId, usize>,
) -> Option<usize> {
    let id = clip.media_id?;
    if !media.get(&id)?.has_audio {
        return None;
    }
    index_of.get(&id).copied()
}

fn speed_of(clip: &Clip) -> f64 {
    if clip.speed.is_finite() && clip.speed > 0.05 {
        f64::from(clip.speed).clamp(0.25, 4.0)
    } else {
        1.0
    }
}

fn silence(fc: &mut String, next_label: &mut impl FnMut() -> String, dur: f64) -> String {
    let lab = next_label();
    fc.push_str(&format!(
        "anullsrc=channel_layout=stereo:sample_rate=48000,atrim=duration={:.4},asetpts=PTS-STARTPTS[{lab}];",
        dur.max(0.04)
    ));
    lab
}

fn voice_piece(
    fc: &mut String,
    next_label: &mut impl FnMut() -> String,
    clip: &Clip,
    media: &HashMap<MediaId, MediaSource>,
    index_of: &HashMap<MediaId, usize>,
    pools: &mut HashMap<usize, Vec<String>>,
    dur: f64,
) -> String {
    let Some(idx) = audio_index(clip, media, index_of) else {
        return silence(fc, next_label, dur);
    };
    let Some(src) = pools.get_mut(&idx).and_then(|labels| labels.pop()) else {
        return silence(fc, next_label, dur);
    };
    let speed = speed_of(clip);
    let src_dur = dur * speed;
    let mut chain = format!(
        "[{src}]atrim=start={:.4}:duration={src_dur:.4},asetpts=PTS-STARTPTS{}",
        clip.source_in.as_seconds(),
        audio_fx(&clip.look.audio)
    );
    if (speed - 1.0).abs() > 0.02 {
        let tempo = speed.clamp(0.5, 2.0);
        chain.push_str(&format!(",atempo={tempo:.4}"));
    }
    let fi = clip.look.fade_in.as_seconds();
    let fo = clip.look.fade_out.as_seconds();
    if fi > 0.04 {
        chain.push_str(&format!(",afade=t=in:st=0:d={fi:.3}"));
    }
    if fo > 0.04 {
        chain.push_str(&format!(
            ",afade=t=out:st={:.3}:d={fo:.3}",
            (dur - fo).max(0.0)
        ));
    }
    chain.push_str(",aformat=sample_fmts=fltp:sample_rates=48000:channel_layouts=stereo");
    let lab = next_label();
    fc.push_str(&format!("{chain}[{lab}];"));
    lab
}

fn concat_audio(
    fc: &mut String,
    next_label: &mut impl FnMut() -> String,
    pieces: Vec<String>,
) -> String {
    if pieces.len() <= 1 {
        return pieces.into_iter().next().unwrap_or_else(|| "a".into());
    }
    let out = next_label();
    let ins: String = pieces.iter().map(|l| format!("[{l}]")).collect();
    fc.push_str(&format!("{ins}concat=n={}:v=0:a=1[{out}];", pieces.len()));
    out
}

fn xfade_name(kind: TransitionKind) -> &'static str {
    kind.xfade()
}

fn escape_text(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('\'', "\u{2019}")
        .replace(':', "\\:")
        .replace('\n', " ")
}

fn escape_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace(':', "\\:")
        .replace('\'', "\\'")
}

#[cfg(test)]
mod tests {
    use super::eq_filters;
    use oc_timeline::{Grade, Lut};

    fn monitor(rgb: [f32; 3], grade: &Grade) -> [f32; 3] {
        let brightness = 1.0 + grade.exposure + grade.gain * 0.35 + grade.lift * 0.15;
        let contrast = 1.0 + grade.contrast + grade.gamma * 0.45;
        let sat = (1.0 + grade.saturation).max(0.0);
        let mut out = rgb.map(|c| ((c - 0.5) * contrast + 0.5) * brightness);
        let y = 0.2126 * out[0] + 0.7152 * out[1] + 0.0722 * out[2];
        out[0] = y + (out[0] - y) * sat;
        out[1] = y + (out[1] - y) * sat;
        out[2] = y + (out[2] - y) * sat;
        out[0] += grade.temperature * 0.15;
        out[2] -= grade.temperature * 0.15;
        out = match grade.lut {
            Lut::None => out,
            Lut::Film => {
                let gray = 0.2126 * out[0] + 0.7152 * out[1] + 0.0722 * out[2];
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
                let gray = 0.2126 * out[0] + 0.7152 * out[1] + 0.0722 * out[2];
                [gray, gray, gray]
            }
        };
        out.map(|c| c.clamp(0.0, 1.0))
    }

    #[test]
    fn a_clear_frame_adds_no_grade_filter() {
        let filter = eq_filters(&Grade::default(), &oc_timeline::Fx::default(), None);
        assert!(filter.is_empty(), "{filter}");
    }

    #[test]
    fn export_grade_uses_the_monitor_math_not_yuv_colorbalance() {
        let filter = eq_filters(&Grade::punchy(), &oc_timeline::Fx::default(), None);
        assert!(filter.contains("format=gbrap"), "{filter}");
        assert!(filter.contains("colorchannelmixer="), "{filter}");
        assert!(!filter.contains("geq="), "{filter}");
        assert!(
            !filter.contains("colorbalance"),
            "yuv colorbalance washes the picture: {filter}"
        );
        let ffmpeg = std::process::Command::new("ffmpeg")
            .arg("-version")
            .output();
        let Ok(version) = ffmpeg else {
            return;
        };
        if !version.status.success() {
            return;
        }
        let grades = [
            Grade {
                exposure: 0.28,
                contrast: 0.18,
                saturation: 0.22,
                temperature: 0.12,
                lift: 0.15,
                gamma: 0.08,
                gain: 0.10,
                lut: Lut::Warm,
                cube: None,
            },
            // Pushes a coefficient past 2 so the float split path is the one that runs.
            Grade {
                exposure: 0.4,
                contrast: 1.4,
                saturation: 0.8,
                temperature: -0.4,
                lift: 0.2,
                gamma: 0.3,
                gain: 0.5,
                lut: Lut::Film,
                cube: None,
            },
        ];
        let samples: [[u8; 3]; 4] = [
            [16, 16, 16],
            [128, 128, 128],
            [190, 140, 115],
            [235, 235, 235],
        ];
        let dir = std::env::temp_dir().join("oc-grade-match");
        let _ = std::fs::create_dir_all(&dir);
        for (g, grade) in grades.iter().enumerate() {
            let chain = eq_filters(grade, &oc_timeline::Fx::default(), None);
            assert!(chain.contains("colorchannelmixer="), "{chain}");
            assert!(!chain.contains("geq="), "{chain}");
            for (i, px) in samples.iter().enumerate() {
                let mut raw = Vec::new();
                for _ in 0..4 {
                    raw.extend_from_slice(px);
                    raw.push(255);
                }
                let input = dir.join(format!("in{g}-{i}.rgba"));
                std::fs::write(&input, &raw).unwrap();
                let rgb_grade = chain.trim_end_matches(",format=yuv420p");
                let filter = format!("[0:v]scale=2:2:flags=neighbor{rgb_grade},format=rgb24");
                let out = std::process::Command::new("ffmpeg")
                    .args([
                        "-y",
                        "-hide_banner",
                        "-loglevel",
                        "error",
                        "-f",
                        "rawvideo",
                        "-pix_fmt",
                        "rgba",
                        "-s",
                        "2x2",
                        "-i",
                    ])
                    .arg(&input)
                    .args([
                        "-filter_complex",
                        &filter,
                        "-frames:v",
                        "1",
                        "-f",
                        "rawvideo",
                        "-pix_fmt",
                        "rgb24",
                        "pipe:1",
                    ])
                    .output()
                    .expect("ffmpeg");
                assert!(
                    out.status.success(),
                    "{}",
                    String::from_utf8_lossy(&out.stderr)
                );
                let rgb = [
                    px[0] as f32 / 255.0,
                    px[1] as f32 / 255.0,
                    px[2] as f32 / 255.0,
                ];
                let want = monitor(rgb, grade);
                let got = &out.stdout[..3];
                for c in 0..3 {
                    let expect = (want[c] * 255.0).round() as i32;
                    let actual = got[c] as i32;
                    assert!(
                        (expect - actual).abs() <= 1,
                        "grade {g} sample {i} channel {c}: export {actual} monitor {expect}\n{chain}"
                    );
                }
            }
        }
    }

    #[test]
    fn pq_is_tonemapped_and_a_plain_transfer_is_not() {
        let pq = super::hdr_chain("smpte2084");
        assert!(pq.contains("tin=smpte2084"), "{pq}");
        assert!(pq.contains("tonemap=hable"), "{pq}");
        assert!(super::hdr_chain("arib-std-b67").contains("tin=arib-std-b67"));
        assert!(super::hdr_chain("bt709").is_empty());
        let prefix = super::tonemap_prefix("smpte2084", 1920);
        assert!(prefix.contains("scale=1920:1920"), "{prefix}");
        assert!(prefix.contains("tonemap=hable"), "{prefix}");
        assert!(super::tonemap_prefix("bt709", 1080).is_empty());
    }

    #[test]
    fn a_design_card_is_contained_and_keyed_and_a_cover_card_is_cropped() {
        use oc_timeline::{FrameCard, TrackKind};
        use std::collections::HashMap;
        let mut timeline = oc_timeline::Timeline::default();
        let base_id = timeline.first_track(TrackKind::Video).unwrap().id;
        let media_id = oc_timeline::MediaId::new();
        timeline
            .add_clip(base_id, clip_with(media_id, 0.0))
            .unwrap();
        let cover_track = timeline.add_track(TrackKind::Video, "Broll");
        let mut cover = clip_with(media_id, 0.2);
        cover.look.card = Some(FrameCard {
            x: 0.1,
            y: 0.1,
            w: 0.4,
            h: 0.4,
        });
        cover.look.overlay = true;
        timeline.add_clip(cover_track, cover).unwrap();
        let design_track = timeline.add_track(TrackKind::Video, "Design");
        let mut design = clip_with(media_id, 0.4);
        design.look.card = Some(FrameCard {
            x: 0.52,
            y: 0.1,
            w: 0.44,
            h: 0.8,
        });
        design.look.overlay = true;
        timeline.add_clip(design_track, design).unwrap();
        let dir = std::env::temp_dir().join("oc-design-matte");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("a.mp4");
        std::fs::write(&path, b"x").unwrap();
        let mut media = HashMap::new();
        media.insert(
            media_id,
            crate::MediaSource {
                id: media_id,
                path,
                has_video: true,
                has_audio: false,
            },
        );
        let compiled = super::compile(&timeline, &media, oc_tools::ExportPreset::Youtube1080, &dir)
            .expect("compile");
        let filter = compiled.filter;
        assert!(
            filter.contains("force_original_aspect_ratio=increase,crop="),
            "{filter}"
        );
        let key_at = filter
            .find("colorkey=0x111111")
            .expect("design plate is keyed");
        let before = &filter[..key_at];
        let last = before
            .rfind("force_original_aspect_ratio=")
            .expect("scale before the key");
        assert!(
            before[last..].starts_with("force_original_aspect_ratio=decrease"),
            "{before}"
        );
        assert!(filter.contains("format=yuva420p"), "{filter}");
    }

    #[test]
    fn ffmpeg_accepts_the_design_key() {
        let ready = std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false);
        if !ready {
            return;
        }
        let chain = super::fit_chain(320, 180, 30.0, true, true, false);
        assert!(chain.contains("colorkey=0x111111"), "{chain}");
        let clear = super::fit_chain(320, 180, 30.0, true, true, true);
        assert!(clear.contains("black@0"), "{clear}");
        assert!(!clear.contains("colorkey"), "{clear}");
        assert!(super::pix_fmt_has_alpha("yuva420p"));
        assert!(super::pix_fmt_has_alpha("rgba"));
        assert!(!super::pix_fmt_has_alpha("yuv420p"));
        assert!(!super::pix_fmt_has_alpha(""));
        let tagged = super::parse_stream_probe("pix_fmt=yuv420p\nTAG:ALPHA_MODE=1\n");
        assert!(tagged.alpha_mode, "{tagged:?}");
        assert!(super::stream_has_alpha(&tagged));
        assert_eq!(super::decoder_for_stream(&tagged), Some("libvpx-vp9"));
        let plain = super::parse_stream_probe("pix_fmt=yuv420p\n");
        assert!(!super::stream_has_alpha(&plain));
        assert_eq!(super::decoder_for_stream(&plain), None);
        let vp8 = super::parse_stream_probe("codec_name=vp8\npix_fmt=yuv420p\nTAG:alpha_mode=1\n");
        assert_eq!(super::decoder_for_stream(&vp8), Some("libvpx"));
        let still = super::parse_stream_probe("codec_name=png\npix_fmt=rgba\n");
        assert!(super::stream_has_alpha(&still));
        assert_eq!(super::decoder_for_stream(&still), None);
        let off = super::parse_stream_probe("pix_fmt=yuv420p\nTAG:alpha_mode=0\n");
        assert!(!off.alpha_mode);
        let out = std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=0x111111:s=640x360:d=0.1:r=30",
                "-vf",
                &chain,
                "-frames:v",
                "1",
                "-f",
                "null",
                "-",
            ])
            .output()
            .expect("ffmpeg");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let cleared = std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=640x360:d=0.1:r=30",
                "-vf",
                &clear,
                "-frames:v",
                "1",
                "-f",
                "null",
                "-",
            ])
            .output()
            .expect("ffmpeg");
        assert!(
            cleared.status.success(),
            "{}",
            String::from_utf8_lossy(&cleared.stderr)
        );
    }

    #[test]
    fn an_alpha_webm_is_alpha_even_when_pix_fmt_is_plain() {
        let ready = std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false);
        if !ready {
            return;
        }
        let dir = std::env::temp_dir().join("oc-alpha-tag-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("band.webm");
        let made = std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=black@0.0:s=64x64:d=0.2:r=10,format=yuva420p",
                "-f",
                "lavfi",
                "-i",
                "color=c=orange:s=16x16:d=0.2:r=10,format=yuv420p",
                "-filter_complex",
                "[0:v][1:v]overlay=8:8:format=auto,format=yuva420p",
                "-c:v",
                "libvpx-vp9",
                "-pix_fmt",
                "yuva420p",
                "-auto-alt-ref",
                "0",
                "-lag-in-frames",
                "0",
                "-an",
                "-metadata:s:v:0",
                "alpha_mode=1",
            ])
            .arg(&path)
            .output()
            .expect("ffmpeg");
        assert!(
            made.status.success(),
            "{}",
            String::from_utf8_lossy(&made.stderr)
        );
        assert!(
            super::source_has_alpha(&path),
            "alpha_mode tag was not read"
        );
        assert_eq!(super::vpx_alpha_decoder(&path), Some("libvpx-vp9"));
        let junk = dir.join("not-a-video.mp4");
        std::fs::write(&junk, b"x").unwrap();
        assert!(!super::source_has_alpha(&junk));
        assert_eq!(super::vpx_alpha_decoder(&junk), None);

        use oc_timeline::{FrameCard, TrackKind};
        use std::collections::HashMap;
        let mut timeline = oc_timeline::Timeline::default();
        timeline.width = 720;
        timeline.height = 1280;
        let base_id = timeline.first_track(TrackKind::Video).unwrap().id;
        let speaker = oc_timeline::MediaId::new();
        let graphic = oc_timeline::MediaId::new();
        timeline.add_clip(base_id, clip_with(speaker, 0.0)).unwrap();
        let design_track = timeline.add_track(TrackKind::Video, "Design");
        let mut design = clip_with(graphic, 0.2);
        design.look.overlay = true;
        design.look.fade_in = oc_timeline::Duration::from_seconds(0.35);
        design.look.fade_out = oc_timeline::Duration::from_seconds(0.35);
        design.look.card = Some(FrameCard {
            x: 0.08,
            y: 0.66,
            w: 0.84,
            h: 0.14,
        });
        timeline.add_clip(design_track, design).unwrap();
        let mut media = HashMap::new();
        media.insert(
            speaker,
            crate::MediaSource {
                id: speaker,
                path: junk,
                has_video: true,
                has_audio: false,
            },
        );
        media.insert(
            graphic,
            crate::MediaSource {
                id: graphic,
                path: path.clone(),
                has_video: true,
                has_audio: false,
            },
        );
        let compiled = super::compile(
            &timeline,
            &media,
            oc_tools::ExportPreset::Vertical1080,
            &dir,
        )
        .expect("compile");
        assert!(
            compiled.filter.contains("color=black@0"),
            "{}",
            compiled.filter
        );
        assert!(
            !compiled.filter.contains("colorkey="),
            "{}",
            compiled.filter
        );
        assert!(compiled.filter.contains("alpha=1"), "{}", compiled.filter);
    }

    fn clip_with(media_id: oc_timeline::MediaId, start: f64) -> oc_timeline::Clip {
        oc_timeline::Clip {
            id: oc_timeline::ClipId::new(),
            media_id: Some(media_id),
            kind: oc_timeline::ClipKind::Video {
                transform: Default::default(),
            },
            start: oc_timeline::Time::from_seconds(start),
            duration: oc_timeline::Duration::from_seconds(1.5),
            source_in: oc_timeline::Time::ZERO,
            speed: 1.0,
            group_id: None,
            link_id: None,
            disabled: false,
            look: oc_timeline::ClipLook::default(),
        }
    }
}
