use crate::captions::{captions_for_cut, to_srt, BurnedCue};
use crate::{MediaSource, RenderError};
use oc_timeline::{
    Clip, ClipKind, GraphicKind, MediaId, Timeline, TrackKind, TransitionKind,
};
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

    let mut fc = String::new();
    let mut n = 0u32;
    let mut next_label = || {
        let l = format!("n{n}");
        n += 1;
        l
    };

    let mut vcur = if base.is_empty() {
        let d = timeline.duration().as_seconds().max(0.1);
        let lab = next_label();
        fc.push_str(&format!(
            "color=c=black:s={width}x{height}:d={d:.4}:r={fps}[{lab}];"
        ));
        lab
    } else {
        stitch_base(
            &mut fc,
            &mut next_label,
            &base,
            media,
            &index_of,
            width,
            height,
            fps,
        )?
    };

    vcur = overlay_broll(
        &mut fc,
        &mut next_label,
        timeline,
        media,
        &index_of,
        &vcur,
        width,
        height,
        fps,
    )?;
    vcur = overlay_graphics(&mut fc, &mut next_label, timeline, &vcur, width, height)?;
    if timeline.letterbox && height > 8 {
        let bar = ((height as f32) * 0.12).round() as u32;
        let lab = next_label();
        fc.push_str(&format!(
            "[{vcur}]drawbox=x=0:y=0:w=iw:h={bar}:color=black@1:t=fill,drawbox=x=0:y=ih-{bar}:w=iw:h={bar}:color=black@1:t=fill[{lab}];"
        ));
        vcur = lab;
    }

    let captions = captions_for_cut(timeline);
    let srt = if captions.is_empty() {
        None
    } else {
        let path = work.join("cut.srt");
        std::fs::write(&path, to_srt(&captions)).map_err(RenderError::Io)?;
        let lab = next_label();
        let margin_v = if timeline.letterbox {
            ((height as f32) * 0.12).round() as u32 + 28
        } else {
            36
        };
        fc.push_str(&format!(
            "[{vcur}]subtitles={}:force_style='Fontsize=13,Outline=1,Shadow=0,Alignment=2,MarginL=48,MarginR=48,MarginV={margin_v},WrapStyle=0'[{lab}];",
            escape_path(&path)
        ));
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

    // drop trailing ;
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
        .filter(|t| t.kind == TrackKind::Video && !t.muted && !t.hidden && t.name != "GFX")
        .flat_map(|t| t.clips.iter())
        .filter(|c| {
            !c.disabled
                && matches!(c.kind, ClipKind::Video { .. })
                && (c.media_id.is_some() || c.look.generator.is_some())
        })
        .collect();
    all.sort_by(|a, b| a.start.cmp(&b.start).then(a.id.as_uuid().cmp(&b.id.as_uuid())));
    let mut out: Vec<&Clip> = Vec::new();
    for clip in all {
        if out.last().is_none_or(|prev| is_join(prev, clip) || clip.start >= prev.end()) {
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

fn stitch_base(
    fc: &mut String,
    next_label: &mut impl FnMut() -> String,
    clips: &[&Clip],
    media: &HashMap<MediaId, MediaSource>,
    index_of: &HashMap<MediaId, usize>,
    width: u32,
    height: u32,
    fps: f64,
) -> Result<String, RenderError> {
    let mut acc: Option<(String, f64)> = None;
    for clip in clips {
        let v = picture_branch(fc, next_label, clip, media, index_of, width, height, fps)?;
        let dur = clip.duration.as_seconds();
        match acc {
            None => {
                let start = clip.start.as_seconds();
                if start > 0.05 {
                    let pad = next_label();
                    let joined = next_label();
                    fc.push_str(&format!(
                        "color=c=black:s={width}x{height}:d={start:.4}:r={fps}[{pad}];[{pad}][{v}]concat=n=2:v=1:a=0[{joined}];"
                    ));
                    acc = Some((joined, start + dur));
                } else {
                    acc = Some((v, dur.max(start + dur)));
                }
            }
            Some((prev, t_end)) => {
                let gap = clip.start.as_seconds() - t_end;
                let mix = if gap.abs() < 0.05 {
                    clip_prev_transition(clips, clip)
                        .map(|left| {
                            left.look.mix_window(
                                left.duration.as_seconds(),
                                clip.duration.as_seconds(),
                            )
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
                let mut left_end = t_end;
                if gap > 0.05 {
                    let pad = next_label();
                    let joined = next_label();
                    fc.push_str(&format!(
                        "color=c=black:s={width}x{height}:d={gap:.4}:r={fps}[{pad}];[{left}][{pad}]concat=n=2:v=1:a=0[{joined}];"
                    ));
                    left = joined;
                    left_end += gap;
                }
                let out = next_label();
                if let Some(kind) = xfade_kind.filter(|k| *k != TransitionKind::Cut) {
                    let offset = (left_end - mix).max(0.0);
                    let left_tb = next_label();
                    let right_tb = next_label();
                    fc.push_str(&format!(
                        "[{left}]fps={fps},settb=AVTB[{left_tb}];[{v}]fps={fps},settb=AVTB[{right_tb}];[{left_tb}][{right_tb}]xfade=transition={}:duration={mix:.4}:offset={offset:.4}[{out}];",
                        xfade_name(kind)
                    ));
                    acc = Some((out, left_end + dur - mix));
                } else {
                    let left_tb = next_label();
                    let right_tb = next_label();
                    fc.push_str(&format!(
                        "[{left}]fps={fps},settb=AVTB[{left_tb}];[{v}]fps={fps},settb=AVTB[{right_tb}];[{left_tb}][{right_tb}]concat=n=2:v=1:a=0[{out}];"
                    ));
                    acc = Some((out, left_end + dur));
                }
            }
        }
    }
    Ok(acc.map(|(l, _)| l).unwrap_or_else(|| "null".into()))
}

fn clip_prev_transition<'a>(clips: &[&'a Clip], current: &Clip) -> Option<&'a Clip> {
    clips.iter().rev().find(|c| c.end() <= current.start + oc_time::Duration::from_seconds(0.05) && c.id != current.id).copied()
}

fn picture_branch(
    fc: &mut String,
    next_label: &mut impl FnMut() -> String,
    clip: &Clip,
    media: &HashMap<MediaId, MediaSource>,
    index_of: &HashMap<MediaId, usize>,
    width: u32,
    height: u32,
    fps: f64,
) -> Result<String, RenderError> {
    let dur = clip.duration.as_seconds().max(0.04);
    let mut chain = if let Some(generated) = &clip.look.generator {
        generator_video(generated, width, height, fps, dur)
    } else {
        let id = clip.media_id.ok_or(RenderError::Empty)?;
        let _src = media.get(&id).ok_or(RenderError::UnknownMedia(id))?;
        let idx = *index_of.get(&id).ok_or(RenderError::UnknownMedia(id))?;
        let sin = clip.source_in.as_seconds();
        let src_dur = source_span(clip, dur);
        format!("[{idx}:v]trim=start={sin:.4}:duration={src_dur:.4},setpts=PTS-STARTPTS")
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
        chain.push_str(&format!(
            ",crop=iw*{w:.4}:ih*{h:.4}:iw*{x:.4}:ih*{y:.4}"
        ));
    }
    chain.push_str(&format!(
        ",scale={width}:{height}:force_original_aspect_ratio=decrease,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2,fps={fps},format=yuv420p"
    ));
    if let ClipKind::Video { transform } = &clip.kind {
        if let Some(end) = clip.look.move_to {
            let frames = (dur * fps).max(1.0);
            let z0 = transform.scale.clamp(0.25, 4.0);
            let z1 = end.scale.clamp(0.25, 4.0);
            let x0 = pan_px(transform.x, width);
            let x1 = pan_px(end.x, width);
            let y0 = pan_px(transform.y, height);
            let y1 = pan_px(end.y, height);
            let t = ease_expr(clip.look.move_ease.unwrap_or_default(), &format!("on/{frames:.1}"));
            chain.push_str(&format!(
                ",zoompan=z='{z0:.4}+({z1:.4}-{z0:.4})*({t})':x='(iw-iw/zoom)/2+({x0:.2}+({x1:.2}-{x0:.2})*({t}))':y='(ih-ih/zoom)/2+({y0:.2}+({y1:.2}-{y0:.2})*({t}))':d=1:s={width}x{height}:fps={fps}"
            ));
        } else {
            let z = transform.scale;
            if (z - 1.0).abs() > 0.01 || transform.x.abs() > 0.5 || transform.y.abs() > 0.5 {
                let z = z.clamp(0.25, 4.0);
                chain.push_str(&format!(
                    ",scale=iw*{z:.4}:ih*{z:.4},crop={width}:{height}:(in_w-{width})/2+({:.2}):(in_h-{height})/2+({:.2})",
                    pan_px(transform.x, width),
                    pan_px(transform.y, height)
                ));
            }
        }
    }
    chain.push_str(&eq_filters(&clip.look.grade, &clip.look.fx));
    chain.push_str(&curves_filter(&clip.look.curves));
    chain.push_str(&mask_filter(clip.look.mask.as_ref(), width, height));
    let fi = clip.look.fade_in.as_seconds();
    let fo = clip.look.fade_out.as_seconds();
    if fi > 0.04 {
        chain.push_str(&format!(",fade=t=in:st=0:d={fi:.3}"));
    }
    if fo > 0.04 {
        chain.push_str(&format!(",fade=t=out:st={:.3}:d={fo:.3}", (dur - fo).max(0.0)));
    }
    let lab = next_label();
    fc.push_str(&format!("{chain}[{lab}];"));
    Ok(lab)
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
        span += tail * f64::from(keys.last().map(|k| k.speed).unwrap_or(clip.speed)).clamp(0.25, 4.0);
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
    let mut expr = format!("{:.4}", f64::from(keys.last().map(|k| k.speed).unwrap_or(1.0)).clamp(0.25, 4.0));
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

fn generator_video(generated: &oc_timeline::Generator, width: u32, height: u32, fps: f64, dur: f64) -> String {
    match generated {
        oc_timeline::Generator::Color { color } => {
            let hex = color.trim().trim_start_matches('#').to_string();
            format!("color=c=0x{hex}:s={width}x{height}:r={fps}:d={dur:.3},format=yuv420p")
        }
        oc_timeline::Generator::ColorBars => {
            format!("smptebars=s={width}x{height}:r={fps}:d={dur:.3},format=yuv420p")
        }
        oc_timeline::Generator::WhiteNoise => {
            format!("color=c=black:s={width}x{height}:r={fps}:d={dur:.3},noise=alls=40:allf=t+u,format=yuv420p")
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
        format!(",gblur=sigma={:.2}:planes=7", (feather / 8.0).clamp(0.4, 12.0))
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

fn eq_filters(grade: &oc_timeline::Grade, fx: &oc_timeline::Fx) -> String {
    let mut s = String::new();
    if !grade.is_identity() {
        let b = (grade.exposure * 0.45).clamp(-1.0, 1.0);
        let c = (1.0 + grade.contrast).clamp(0.2, 3.0);
        let sat = (1.0 + grade.saturation).clamp(0.0, 3.0);
        s.push_str(&format!(
            ",eq=brightness={b:.3}:contrast={c:.3}:saturation={sat:.3}"
        ));
        if grade.temperature.abs() > 1e-3 {
            let t = grade.temperature.clamp(-1.0, 1.0);
            s.push_str(&format!(",colorbalance=rs={t:.3}:bs={:.3}", -t));
        }
        if grade.lift.abs() > 1e-3 || grade.gamma.abs() > 1e-3 || grade.gain.abs() > 1e-3 {
            let lift = grade.lift.clamp(-1.0, 1.0);
            let gamma = grade.gamma.clamp(-1.0, 1.0);
            let gain = grade.gain.clamp(-1.0, 1.0);
            s.push_str(&format!(
                ",colorbalance=rs={lift:.3}:gs={lift:.3}:bs={lift:.3}:rm={gamma:.3}:gm={gamma:.3}:bm={gamma:.3}:rh={gain:.3}:gh={gain:.3}:bh={gain:.3}"
            ));
        }
        s.push_str(lut_filter(grade.lut));
    }
    if fx.grain > 0.02 {
        let alls = (fx.grain * 28.0).clamp(1.0, 40.0);
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

fn lut_filter(lut: oc_timeline::Lut) -> &'static str {
    use oc_timeline::Lut;
    match lut {
        Lut::None => "",
        Lut::Film => ",eq=saturation=0.9:contrast=1.05,colorbalance=rs=0.04:bs=-0.03",
        Lut::Cool => ",colorbalance=bs=0.08:bh=0.06:rs=-0.04",
        Lut::Warm => ",colorbalance=rs=0.08:rh=0.05:bs=-0.04",
        Lut::TealOrange => ",colorbalance=bs=0.07:rs=0.06:rm=0.04:bh=-0.02",
        Lut::Mono => ",hue=s=0",
    }
}

fn audio_fx(fx: &oc_timeline::AudioFx) -> String {
    let mut s = String::new();
    if fx.denoise {
        s.push_str(",afftdn=nr=12:nf=-25");
    }
    if fx.low.abs() > 0.05 {
        s.push_str(&format!(",equalizer=f=120:t=q:w=1:g={:.2}", fx.low.clamp(-12.0, 12.0)));
    }
    if fx.mid.abs() > 0.05 {
        s.push_str(&format!(",equalizer=f=1000:t=q:w=1:g={:.2}", fx.mid.clamp(-12.0, 12.0)));
    }
    if fx.high.abs() > 0.05 {
        s.push_str(&format!(",equalizer=f=8000:t=q:w=1:g={:.2}", fx.high.clamp(-12.0, 12.0)));
    }
    if fx.compressor {
        s.push_str(",acompressor=threshold=-18dB:ratio=3:attack=20:release=200");
    }
    if fx.normalize {
        s.push_str(",dynaudnorm=f=150:g=15");
    }
    s
}

fn overlay_broll(
    fc: &mut String,
    next_label: &mut impl FnMut() -> String,
    timeline: &Timeline,
    media: &HashMap<MediaId, MediaSource>,
    index_of: &HashMap<MediaId, usize>,
    base: &str,
    width: u32,
    height: u32,
    fps: f64,
) -> Result<String, RenderError> {
    let program = base_clips(timeline);
    let mut cur = base.to_string();
    for track in &timeline.tracks {
        if track.kind != TrackKind::Video || track.muted || track.hidden || track.name == "GFX" {
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
            let v = picture_branch(fc, next_label, clip, media, index_of, width, height, fps)?;
            let shifted = next_label();
            let start = clip.start.as_seconds();
            fc.push_str(&format!(
                "[{v}]setpts=PTS-STARTPTS+{start:.4}/TB[{shifted}];"
            ));
            let out = next_label();
            let end = clip.end().as_seconds();
            let alpha = if clip.look.mask.is_some() {
                ":format=auto"
            } else {
                ""
            };
            fc.push_str(&format!(
                "[{cur}][{shifted}]overlay=0:0{alpha}:enable='between(t,{start:.4},{end:.4})'[{out}];"
            ));
            cur = out;
        }
    }
    Ok(cur)
}

fn overlay_graphics(
    fc: &mut String,
    next_label: &mut impl FnMut() -> String,
    timeline: &Timeline,
    base: &str,
    width: u32,
    height: u32,
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
            let start = clip.start.as_seconds();
            let end = clip.end().as_seconds();
            let enable = format!("enable='between(t,{start:.4},{end:.4})'");
            let alpha = clip
                .look
                .fade_gain(0.5 * clip.duration.as_seconds(), clip.duration.as_seconds());
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
                    let y = format!("h-{}", height / 6);
                    fc.push_str(&drawtext(
                        &cur,
                        &out,
                        &graphic.text,
                        width / 22,
                        "w/16",
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
                    fc.push_str(&drawtext(
                        &cur,
                        &out,
                        &graphic.text,
                        width / 12,
                        "(w-text_w)/2",
                        "(h-text_h)/2",
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

    let mut pieces = Vec::new();
    let mut cursor = 0.0;
    for clip in &program {
        let start = clip.start.as_seconds();
        let dur = clip.duration.as_seconds().max(0.04);
        if start - cursor > 0.05 {
            pieces.push(silence(fc, next_label, start - cursor));
        }
        pieces.push(voice_piece(
            fc,
            next_label,
            clip,
            media,
            index_of,
            &mut pools,
            dur,
        ));
        cursor = start + dur;
    }
    if total - cursor > 0.05 {
        pieces.push(silence(fc, next_label, total - cursor));
    }
    if pieces.is_empty() {
        pieces.push(silence(fc, next_label, total));
    }
    let mut program_label = concat_audio(fc, next_label, pieces);

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
                let Some(raw) = pools.get_mut(&audio_index(clip, media, index_of).unwrap_or(usize::MAX))
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
        chain.push_str(&format!(",afade=t=out:st={:.3}:d={fo:.3}", (dur - fo).max(0.0)));
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
    fc.push_str(&format!(
        "{ins}concat=n={}:v=0:a=1[{out}];",
        pieces.len()
    ));
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
