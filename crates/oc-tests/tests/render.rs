//! Graph compile + ffmpeg bake (skipped if ffmpeg is missing).

use oc_core::{
    Duration, Graphic, MediaId, Op, Time, Timeline, TrackKind, TransitionKind, UndoStack, apply,
};
use oc_render::{MediaSource, RenderRequest, captions_for_cut, compile, ffmpeg_available, render};
use oc_tests::video_on;
use oc_tools::ExportPreset;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

fn make_color_clip(path: &std::path::Path, color: &str, seconds: f64) {
    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("color=c={color}:s=320x180:d={seconds}:r=30"),
            "-f",
            "lavfi",
            "-i",
            &format!("sine=f=440:d={seconds}"),
            "-shortest",
            "-pix_fmt",
            "yuv420p",
            "-c:v",
            "libx264",
            "-c:a",
            "aac",
            path.to_str().unwrap(),
        ])
        .status()
        .expect("spawn ffmpeg");
    assert!(status.success(), "lavfi encode failed");
}

fn source(id: MediaId, path: PathBuf) -> MediaSource {
    MediaSource {
        id,
        path,
        has_video: true,
        has_audio: true,
    }
}

#[test]
fn graph_uses_xfade_for_dissolve() {
    let mut tl = Timeline::default();
    let mut undo = UndoStack::new();
    let track = tl.first_track(TrackKind::Video).unwrap().id;
    let a_id = MediaId::new();
    let b_id = MediaId::new();
    let a = tl.add_clip(track, video_on(a_id, 0.0, 2.0)).unwrap();
    let _b = tl.add_clip(track, video_on(b_id, 2.0, 2.0)).unwrap();
    apply(
        &mut tl,
        &mut undo,
        Op::SetTransition {
            clip_id: a,
            kind: TransitionKind::Dissolve,
        },
    )
    .unwrap();
    let dir = std::env::temp_dir().join("oc-render-graph");
    let _ = std::fs::create_dir_all(&dir);
    let a_path = dir.join("a.mp4");
    let b_path = dir.join("b.mp4");
    std::fs::write(&a_path, b"x").unwrap();
    std::fs::write(&b_path, b"x").unwrap();
    let mut media = HashMap::new();
    media.insert(a_id, source(a_id, a_path));
    media.insert(b_id, source(b_id, b_path));
    let compiled = compile(&tl, &media, ExportPreset::Youtube1080, &dir).unwrap();
    assert!(
        compiled.filter.contains("xfade=transition=fade"),
        "{}",
        compiled.filter
    );
    assert!(compiled.filter.contains("eq=") == false || compiled.filter.contains("scale="));
    assert!(compiled.filter.contains("1920") && compiled.filter.contains("1080"));
}

#[test]
fn graph_draws_title_and_grade() {
    let mut tl = Timeline::default();
    let mut undo = UndoStack::new();
    let track = tl.first_track(TrackKind::Video).unwrap().id;
    let mid = MediaId::new();
    let clip = tl.add_clip(track, video_on(mid, 0.0, 3.0)).unwrap();
    apply(
        &mut tl,
        &mut undo,
        Op::SetGrade {
            clip_id: clip,
            grade: oc_core::Grade::punchy(),
        },
    )
    .unwrap();
    apply(
        &mut tl,
        &mut undo,
        Op::AddGraphic {
            graphic: Graphic::title("HELLO"),
            start: Time::from_seconds(0.2),
            duration: Duration::from_seconds(2.0),
            track_id: None,
        },
    )
    .unwrap();
    let dir = std::env::temp_dir().join("oc-render-title");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("a.mp4");
    std::fs::write(&path, b"x").unwrap();
    let mut media = HashMap::new();
    media.insert(mid, source(mid, path));
    let compiled = compile(&tl, &media, ExportPreset::Vertical1080, &dir).unwrap();
    assert!(compiled.filter.contains("drawtext"), "{}", compiled.filter);
    assert!(compiled.filter.contains("eq="), "{}", compiled.filter);
    assert!(compiled.filter.contains("1080") && compiled.filter.contains("1920"));
}

#[test]
fn bakes_two_shots_with_ffmpeg() {
    if !ffmpeg_available() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("oc-render-bake-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let a_path = dir.join("a.mp4");
    let b_path = dir.join("b.mp4");
    make_color_clip(&a_path, "red", 2.0);
    make_color_clip(&b_path, "blue", 2.0);

    let mut tl = Timeline::default();
    let mut undo = UndoStack::new();
    let track = tl.first_track(TrackKind::Video).unwrap().id;
    let a_id = MediaId::new();
    let b_id = MediaId::new();
    let a = tl.add_clip(track, video_on(a_id, 0.0, 2.0)).unwrap();
    let _b = tl.add_clip(track, video_on(b_id, 2.0, 2.0)).unwrap();
    apply(
        &mut tl,
        &mut undo,
        Op::SetTransition {
            clip_id: a,
            kind: TransitionKind::Dissolve,
        },
    )
    .unwrap();
    apply(
        &mut tl,
        &mut undo,
        Op::AddGraphic {
            graphic: Graphic::title("CUT"),
            start: Time::from_seconds(0.4),
            duration: Duration::from_seconds(1.2),
            track_id: None,
        },
    )
    .unwrap();

    let mut media = HashMap::new();
    media.insert(a_id, source(a_id, a_path));
    media.insert(b_id, source(b_id, b_path));
    let output = dir.join("out.mp4");
    let result = render(&RenderRequest {
        timeline: tl,
        media,
        output: output.clone(),
        preset: ExportPreset::Youtube1080,
    })
    .expect("render");
    assert!(result.output.is_file());
    let meta = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
            result.output.to_str().unwrap(),
        ])
        .output()
        .expect("ffprobe");
    let dur: f64 = String::from_utf8_lossy(&meta.stdout)
        .trim()
        .parse()
        .unwrap_or(0.0);
    assert!(dur > 2.8 && dur < 3.6, "expected ~3.2s dissolve, got {dur}");
}

#[test]
fn empty_timeline_errors() {
    let tl = Timeline::default();
    let dir = std::env::temp_dir();
    let err = compile(&tl, &HashMap::new(), ExportPreset::Square1080, &dir).unwrap_err();
    assert!(err.to_string().contains("nothing") || matches!(err, oc_render::RenderError::Empty));
}

#[test]
fn captions_for_cut_is_public() {
    let tl = Timeline::default();
    assert!(captions_for_cut(&tl).is_empty());
}
