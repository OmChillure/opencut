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
    let (pw, ph) = preset.size();
    let width = if timeline.width >= 2 { timeline.width } else { pw };
    let height = if timeline.height >= 2 { timeline.height } else { ph };
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

    let captions = captions_for_cut(timeline);
    let srt = if captions.is_empty() {
        None
    } else {
        let path = work.join("cut.srt");
        std::fs::write(&path, to_srt(&captions)).map_err(RenderError::Io)?;
        let lab = next_label();
        fc.push_str(&format!(
            "[{vcur}]subtitles={}:force_style='Fontsize=13,Outline=1,Shadow=0,Alignment=2,MarginL=48,MarginR=48,MarginV=36,WrapStyle=0'[{lab}];",
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
        .filter(|c| !c.disabled && matches!(c.kind, ClipKind::Video { .. }) && c.media_id.is_some())
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
                    fc.push_str(&format!(
                        "[{left}][{v}]xfade=transition={}:duration={mix:.4}:offset={offset:.4}[{out}];",
                        xfade_name(kind)
                    ));
                    acc = Some((out, left_end + dur - mix));
                } else {
                    fc.push_str(&format!(
                        "[{left}][{v}]concat=n=2:v=1:a=0[{out}];"
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
    let id = clip.media_id.ok_or(RenderError::Empty)?;
    let src = media.get(&id).ok_or(RenderError::UnknownMedia(id))?;
    let idx = *index_of.get(&id).ok_or(RenderError::UnknownMedia(id))?;
    let sin = clip.source_in.as_seconds();
    let dur = clip.duration.as_seconds().max(0.04);
    let speed = if clip.speed.is_finite() && clip.speed > 0.05 {
        f64::from(clip.speed)
    } else {
        1.0
    };
    let speed_end = f64::from(clip.look.speed_to.unwrap_or(clip.speed)).clamp(0.25, 4.0);
    let speed = speed.clamp(0.25, 4.0);
    let src_dur = dur * (speed + speed_end) * 0.5;
    let mut chain = format!(
        "[{idx}:v]trim=start={sin:.4}:duration={src_dur:.4},setpts=PTS-STARTPTS"
    );
    if (speed - 1.0).abs() > 0.02 || (speed_end - speed).abs() > 0.02 {
        chain.push_str(&format!(
            ",setpts='PTS/({speed:.4}+({speed_end:.4}-{speed:.4})*T/{dur:.4})'"
        ));
    }
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
            chain.push_str(&format!(
                ",zoompan=z='{z0:.4}+({z1:.4}-{z0:.4})*on/{frames:.1}':x='(iw-iw/zoom)/2+({:.2}+({:.2}-{:.2})*on/{frames:.1})':y='(ih-ih/zoom)/2+({:.2}+({:.2}-{:.2})*on/{frames:.1})':d=1:s={width}x{height}:fps={fps}",
                transform.x, end.x, transform.x, transform.y, end.y, transform.y
            ));
        } else {
            let z = transform.scale;
            if (z - 1.0).abs() > 0.01 || transform.x.abs() > 0.5 || transform.y.abs() > 0.5 {
                let z = z.clamp(0.25, 4.0);
                chain.push_str(&format!(
                    ",scale=iw*{z:.4}:ih*{z:.4},crop={width}:{height}:(in_w-{width})/2+({:.2}):(in_h-{height})/2+({:.2})",
                    transform.x, transform.y
                ));
            }
        }
    }
    chain.push_str(&eq_filters(&clip.look.grade, &clip.look.fx));
    let fi = clip.look.fade_in.as_seconds();
    let fo = clip.look.fade_out.as_seconds();
    if fi > 0.04 {
        chain.push_str(&format!(",fade=t=in:st=0:d={fi:.3}"));
    }
    if fo > 0.04 {
        chain.push_str(&format!(",fade=t=out:st={:.3}:d={fo:.3}", (dur - fo).max(0.0)));
    }
    let _ = src;
    let lab = next_label();
    fc.push_str(&format!("{chain}[{lab}];"));
    Ok(lab)
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
            if clip.disabled || !matches!(clip.kind, ClipKind::Video { .. }) || clip.media_id.is_none()
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
            fc.push_str(&format!(
                "[{cur}][{shifted}]overlay=0:0:enable='between(t,{start:.4},{end:.4})'[{out}];"
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
    let mut beds = Vec::new();
    for track in &timeline.tracks {
        if track.muted || track.hidden {
            continue;
        }
        for clip in &track.clips {
            if clip.disabled {
                continue;
            }
            let (media_id, volume) = match &clip.kind {
                ClipKind::Audio { volume, ducked } => {
                    let v = if *ducked { *volume } else { *volume };
                    (clip.media_id?, v)
                }
                ClipKind::Video { .. } => {
                    if audio_already_on_track(timeline, clip) {
                        continue;
                    }
                    (clip.media_id?, 1.0)
                }
                _ => continue,
            };
            let src = media.get(&media_id)?;
            if !src.has_audio {
                continue;
            }
            let idx = *index_of.get(&media_id)?;
            let sin = clip.source_in.as_seconds();
            let dur = clip.duration.as_seconds().max(0.04);
            let start = clip.start.as_seconds();
            let fi = clip.look.fade_in.as_seconds();
            let fo = clip.look.fade_out.as_seconds();
            let mut chain = format!(
                "[{idx}:a]atrim=start={sin:.4}:duration={dur:.4},asetpts=PTS-STARTPTS+{start:.4}/TB,volume={volume:.3}{}"
                , audio_fx(&clip.look.audio)
            );
            if fi > 0.04 {
                chain.push_str(&format!(",afade=t=in:st={start:.3}:d={fi:.3}"));
            }
            if fo > 0.04 {
                chain.push_str(&format!(
                    ",afade=t=out:st={:.3}:d={fo:.3}",
                    start + dur - fo
                ));
            }
            let lab = next_label();
            fc.push_str(&format!("{chain}[{lab}];"));
            beds.push(lab);
        }
    }
    if beds.is_empty() {
        let lab = next_label();
        fc.push_str(&format!(
            "anullsrc=channel_layout=stereo:sample_rate=48000,atrim=duration={total:.4},asetpts=PTS-STARTPTS[{lab}];"
        ));
        return Some(lab);
    }
    if beds.len() == 1 {
        return Some(beds.remove(0));
    }
    let out = next_label();
    let ins: String = beds.iter().map(|l| format!("[{l}]")).collect();
    fc.push_str(&format!(
        "{ins}amix=inputs={}:duration=longest:normalize=0[{out}];",
        beds.len()
    ));
    Some(out)
}

fn audio_already_on_track(timeline: &Timeline, video: &Clip) -> bool {
    let Some(mid) = video.media_id else {
        return false;
    };
    timeline.tracks.iter().any(|t| {
        t.kind == TrackKind::Audio
            && t.clips.iter().any(|c| {
                c.media_id == Some(mid)
                    && c.start == video.start
                    && (c.duration.as_seconds() - video.duration.as_seconds()).abs() < 0.05
            })
    })
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
