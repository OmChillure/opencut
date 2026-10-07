use anyhow::Context;
use oc_core::{CaptionCue, Op, UndoStack, apply};
use oc_db::Db;
use std::path::Path;

use oc_db::R2;
use serde::Deserialize;
use tokio::time::{Duration, sleep};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::new("info,oc_worker=debug,oc_voice=debug,oc_media=info,oc_db=info")
        }))
        .init();

    let mut db = oc_db::connect().await.context("database")?;
    oc_db::migrate(&db).await.ok();
    let r2 = R2::from_env().await.ok();
    if r2.is_none() {
        tracing::warn!("R2 is not configured — understand and export cannot read source clips");
    }
    if oc_voice::groq_stt_configured() {
        tracing::info!("understand = ffmpeg look + Groq Whisper");
    } else {
        tracing::info!("understand = ffmpeg look (set GROQ_API_KEY for Groq Whisper)");
    }

    tracing::info!("worker polling jobs");
    let mut fail = 0u32;
    loop {
        match oc_db::claim_job(&db).await {
            Ok(Some(job)) => {
                fail = 0;
                tracing::info!(id = %job.id, kind = %job.kind, "claimed job");
                let beat = job_heartbeat(db.clone(), job.id);
                let err = handle(&db, r2.as_ref(), &job.kind, job.payload)
                    .await
                    .err()
                    .map(|e| e.to_string());
                beat.abort();
                if let Some(e) = &err {
                    tracing::error!(id = %job.id, "{e}");
                }
                if let Err(e) = oc_db::finish_job(&db, job.id, err.as_deref()).await {
                    tracing::error!("finish job: {e}");
                }
            }
            Ok(None) => {
                fail = 0;
                sleep(Duration::from_millis(750)).await;
            }
            Err(err) => {
                fail = fail.saturating_add(1);
                tracing::error!(size = db.size(), idle = db.num_idle(), fail, "claim: {err}");
                if err.is_pool_timeout() {
                    match oc_db::connect().await {
                        Ok(next) => {
                            tracing::warn!("reconnected database pool");
                            db = next;
                            fail = 0;
                        }
                        Err(e) => tracing::error!("reconnect failed: {e}"),
                    }
                }
                let wait = 2u64.saturating_mul(u64::from(fail.min(5)));
                sleep(Duration::from_secs(wait.max(2))).await;
            }
        }
    }
}

#[derive(Deserialize)]
struct TranscribePayload {
    project_id: Uuid,
    media_id: Uuid,
    r2_key: String,
}

#[derive(Deserialize)]
struct ExportPayload {
    project_id: Uuid,
    preset: oc_tools::ExportPreset,
}

fn job_heartbeat(db: Db, id: Uuid) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            sleep(Duration::from_secs(oc_db::JOB_HEARTBEAT_SECS)).await;
            if let Err(err) = oc_db::touch_job(&db, id).await {
                tracing::warn!(job = %id, "job heartbeat: {err}");
            }
        }
    })
}

async fn handle(
    db: &Db,
    r2: Option<&R2>,
    kind: &str,
    payload: serde_json::Value,
) -> anyhow::Result<()> {
    match kind {
        "transcribe" => {
            let p: TranscribePayload = serde_json::from_value(payload)?;
            transcribe(db, r2, p).await
        }
        "export" => {
            let p: ExportPayload = serde_json::from_value(payload)?;
            export(db, r2, p).await
        }
        other => anyhow::bail!("unknown job kind {other}"),
    }
}

struct OpenedMedia {
    path: std::path::PathBuf,
    /// Temp directory to delete after the job.
    cleanup: Option<std::path::PathBuf>,
}

async fn open_media(r2: Option<&R2>, key: &str) -> anyhow::Result<OpenedMedia> {
    if !oc_db::is_r2_object_key(key) {
        anyhow::bail!("media is not an R2 object: {key}");
    }
    let r2 = r2.context("R2 required")?;
    let bytes = r2.get_bytes(key).await?;
    let dir = std::env::temp_dir().join(format!("oc-src-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&dir).await?;
    let path = dir.join(oc_db::object_file_name(key));
    if let Err(err) = tokio::fs::write(&path, &bytes).await {
        let _ = tokio::fs::remove_dir_all(&dir).await;
        return Err(err.into());
    }
    Ok(OpenedMedia {
        path,
        cleanup: Some(dir),
    })
}

async fn transcribe(db: &Db, r2: Option<&R2>, p: TranscribePayload) -> anyhow::Result<()> {
    let t0 = std::time::Instant::now();
    tracing::info!(
        project = %p.project_id,
        media = %p.media_id,
        key = %p.r2_key,
        "transcribe start"
    );
    let opened = match open_media(r2, &p.r2_key).await {
        Ok(opened) => opened,
        Err(e) => {
            if let Err(status) = oc_db::set_media_status(db, p.media_id, "failed").await {
                tracing::warn!(media = %p.media_id, "could not mark media failed: {status}");
            }
            return Err(e);
        }
    };
    let result = understand_file(db, &p, &opened.path, t0).await;
    if let Some(dir) = &opened.cleanup {
        let _ = tokio::fs::remove_dir_all(dir).await;
    }
    result
}

async fn understand_file(
    db: &Db,
    p: &TranscribePayload,
    path: &Path,
    t0: std::time::Instant,
) -> anyhow::Result<()> {
    let mut had_speech = oc_db::has_transcript(db, p.media_id).await.unwrap_or(false);
    if !had_speech {
        tracing::info!(media = %p.media_id, "whisper start");
        let transcript = match oc_voice::transcribe_path(path).await {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(media = %p.media_id, "whisper skipped: {e}");
                oc_voice::Transcript {
                    language: None,
                    full_text: String::new(),
                    cues: Vec::new(),
                }
            }
        };
        had_speech = !(transcript.full_text.trim().is_empty() && transcript.cues.is_empty());
        if !had_speech {
            tracing::info!(
                media = %p.media_id,
                ms = t0.elapsed().as_millis(),
                "no speech yet — look still runs"
            );
        } else {
            save_transcript(db, p, &transcript).await?;
            tracing::info!(
                media = %p.media_id,
                cues = transcript.cues.len(),
                ms = t0.elapsed().as_millis(),
                "whisper saved — cut can start"
            );
        }
    } else {
        tracing::info!(media = %p.media_id, "speech already stored");
    }

    let quiet = !had_speech;
    match oc_media::analyze_path(path).await {
        Ok(mut look) => {
            // Speech onsets are not a beat. A music file still gets a grid when the
            // words are lyrics, and a quiet picture gets one when nothing was said.
            if wants_beat_grid(quiet, look.has_video) {
                if let Some(music) = music_grid(path).await {
                    look.music = Some(music);
                }
            }
            match label_with_subscription(path, &mut look).await {
                Ok(filled) => {
                    look.vision = Some("labeled".into());
                    tracing::info!(filled, shots = look.shots.len(), "shot cards saved");
                }
                Err(LabelError::SkipLabels) => {
                    tracing::info!(
                        "shot cards skipped — sign in with `claude auth login`, `grok`, or `codex`"
                    );
                }
                Err(LabelError::LabelFailed(err)) => {
                    look.vision = Some("failed".into());
                    tracing::warn!("shot cards failed: {err}");
                }
            }
            let raw = serde_json::to_value(&look).unwrap_or(serde_json::json!({}));
            if let Err(e) = oc_db::upsert_media_analysis(
                db,
                p.media_id,
                &look.look,
                f64::from(look.motion),
                look.scenes as i32,
                f64::from(look.brightness),
                look.colorful,
                look.has_video,
                look.has_audio,
                &raw,
            )
            .await
            {
                tracing::warn!("save look: {e}");
            } else {
                tracing::info!(
                    look = %look.look,
                    scenes = look.scenes,
                    shots = look.shots.len(),
                    ms = t0.elapsed().as_millis(),
                    "look saved — chat can proceed"
                );
            }
        }
        Err(e) => tracing::warn!("local look failed: {e}"),
    }

    oc_db::set_media_status(db, p.media_id, "ready").await?;
    tracing::info!(media = %p.media_id, ms = t0.elapsed().as_millis(), "understand done");
    Ok(())
}

enum LabelError {
    SkipLabels,
    LabelFailed(String),
}

async fn label_with_subscription(
    path: &Path,
    look: &mut oc_media::VisualDigest,
) -> Result<usize, LabelError> {
    let stills = match oc_media::shot_stills(path, &look.shots).await {
        Ok(stills) => stills,
        Err(err) => return Err(LabelError::LabelFailed(err)),
    };
    if stills.is_empty() {
        return Ok(look.shots.iter().filter(|shot| shot.card.is_some()).count());
    }
    if !oc_providers::subscription_ready() {
        return Err(LabelError::SkipLabels);
    }
    let frames = stills
        .iter()
        .map(|still| oc_providers::PromptImage {
            caption: still.caption.clone(),
            jpeg: still.jpeg.clone(),
        })
        .collect::<Vec<_>>();
    let text = oc_providers::ask_with_stills(oc_media::shot_label_prompt(), &frames)
        .await
        .map_err(|err| LabelError::LabelFailed(err.to_string()))?;
    oc_media::apply_shot_reply(&mut look.shots, &stills, &text).map_err(LabelError::LabelFailed)?;
    Ok(look.shots.iter().filter(|shot| shot.card.is_some()).count())
}

async fn music_grid(path: &Path) -> Option<oc_media::MusicAnalysis> {
    let output = tokio::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-i",
            &path.to_string_lossy(),
            "-ac",
            "1",
            "-ar",
            "22050",
            "-f",
            "f32le",
            "-",
        ])
        .output()
        .await
        .ok()?;
    if !output.status.success() || output.stdout.len() < 8 {
        return None;
    }
    let mut samples = Vec::with_capacity(output.stdout.len() / 4);
    for chunk in output.stdout.chunks_exact(4) {
        samples.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    Some(oc_media::detect_beats(&samples, 22_050))
}

async fn save_transcript(
    db: &Db,
    p: &TranscribePayload,
    transcript: &oc_voice::Transcript,
) -> anyhow::Result<()> {
    let raw = serde_json::to_value(transcript)?;
    let cue_rows: Vec<(i64, i64, String, Option<String>)> = transcript
        .cues
        .iter()
        .map(|c| {
            (
                c.start.as_ticks(),
                c.end.as_ticks(),
                c.text.clone(),
                c.speaker.clone(),
            )
        })
        .collect();
    let refs: Vec<(i64, i64, &str, Option<&str>)> = cue_rows
        .iter()
        .map(|(a, b, t, s)| (*a, *b, t.as_str(), s.as_deref()))
        .collect();
    oc_db::insert_transcript(
        db,
        p.media_id,
        transcript.language.as_deref(),
        &transcript.full_text,
        &raw,
        &refs,
    )
    .await?;

    let mut cues: Vec<CaptionCue> = transcript
        .cues
        .iter()
        .cloned()
        .map(|c| c.into_timeline())
        .collect();
    let mut project = oc_db::get_project(db, p.project_id).await?;
    dress_safe(&mut cues, &project.timeline);
    let mut undo = UndoStack::new();
    apply(
        &mut project.timeline,
        &mut undo,
        Op::AddCaptions {
            style: oc_timeline::CaptionStyle::Stacked,
            cues,
        },
    )?;
    oc_db::save_timeline(db, p.project_id, &project.timeline).await?;
    oc_db::set_media_status(db, p.media_id, "ready").await?;
    Ok(())
}

fn dress_safe(cues: &mut [oc_core::CaptionCue], timeline: &oc_core::Timeline) {
    let recipe = oc_core::caption_recipe_for(timeline);
    let faces = vec![true; cues.len()];
    oc_core::dress_cues(cues, &recipe, &faces);
}

async fn lay_captions(
    db: &Db,
    project_id: Uuid,
    timeline: &mut oc_core::Timeline,
) -> anyhow::Result<()> {
    if oc_tools::has_burnable_captions(timeline) {
        if oc_tools::redress_unset_captions(timeline) {
            oc_db::save_timeline(db, project_id, timeline).await?;
            tracing::info!(project = %project_id, "restyled captions onto one theme");
        }
        return Ok(());
    }
    let rows = oc_db::list_transcripts_for_project(db, project_id).await?;
    let lines: Vec<oc_tools::SpokenLine> = rows
        .iter()
        .map(|row| oc_tools::SpokenLine {
            media: oc_core::MediaId::from_uuid(row.media_id),
            start: oc_core::Time::from_ticks(row.start_ticks).as_seconds(),
            end: oc_core::Time::from_ticks(row.end_ticks).as_seconds(),
            text: row.text.clone(),
        })
        .collect();
    let clips = oc_tools::program_clips(timeline);
    let mut cues = oc_tools::mapped_cues(&clips, &lines);
    if cues.is_empty() {
        return Ok(());
    }
    dress_safe(&mut cues, timeline);
    let n = cues.len();
    let mut undo = UndoStack::new();
    apply(
        timeline,
        &mut undo,
        Op::AddCaptions {
            style: oc_core::CaptionStyle::Stacked,
            cues,
        },
    )?;
    oc_db::save_timeline(db, project_id, timeline).await?;
    tracing::info!(project = %project_id, cues = n, "captions laid on the cut for export");
    Ok(())
}

async fn export(db: &Db, r2: Option<&R2>, p: ExportPayload) -> anyhow::Result<()> {
    let t0 = std::time::Instant::now();
    let mut project = oc_db::get_project(db, p.project_id).await?;
    let rows = oc_db::list_media(db, p.project_id).await?;
    if project.timeline.duration().as_seconds() < 0.04 {
        anyhow::bail!("timeline is empty");
    }
    lay_captions(db, p.project_id, &mut project.timeline).await?;
    let work = std::env::temp_dir().join(format!("oc-export-{}", p.project_id));
    tokio::fs::create_dir_all(&work).await?;
    let mut media = std::collections::HashMap::new();
    for row in &rows {
        let dest = work.join(&row.filename);
        if !oc_db::is_r2_object_key(&row.r2_key) {
            tracing::warn!(media = %row.id, key = %row.r2_key, "skip source that is not on R2");
            continue;
        }
        let r2 = r2.context("R2 required to fetch source clips")?;
        let bytes = r2.get_bytes(&row.r2_key).await?;
        tokio::fs::write(&dest, bytes).await?;
        let has_video =
            row.content_type.starts_with("video/") || row.content_type.starts_with("image/");
        let has_audio =
            row.content_type.starts_with("audio/") || row.content_type.starts_with("video/");
        media.insert(
            oc_timeline::MediaId::from_uuid(row.id),
            oc_render::MediaSource {
                id: oc_timeline::MediaId::from_uuid(row.id),
                path: dest,
                has_video,
                has_audio,
            },
        );
    }
    let out_dir = std::env::var("OPENCUT_EXPORT_DIR").unwrap_or_else(|_| "data/exports".into());
    tokio::fs::create_dir_all(&out_dir).await?;
    let filename = p.preset.file_name(p.project_id);
    let output = std::path::PathBuf::from(&out_dir).join(&filename);
    let req = oc_render::RenderRequest {
        timeline: project.timeline,
        media,
        output: output.clone(),
        preset: p.preset,
    };
    let rendered = tokio::task::spawn_blocking(move || oc_render::render(&req))
        .await
        .map_err(|e| anyhow::anyhow!("render join: {e}"))??;
    if let Some(r2) = r2 {
        let bytes = tokio::fs::read(&rendered.output).await?;
        let key =
            oc_media::export_object_key(oc_timeline::ProjectId::from_uuid(p.project_id), &filename);
        // The editor plays the local file. A denied upload must not fail that render.
        match r2.put_bytes(&key, bytes, "video/mp4").await {
            Ok(()) => tracing::info!(
                project = %p.project_id,
                key,
                ms = t0.elapsed().as_millis(),
                "export uploaded"
            ),
            Err(err) => tracing::error!(
                project = %p.project_id,
                path = %rendered.output.display(),
                "export file is local; R2 upload failed: {err}"
            ),
        }
    } else {
        tracing::info!(
            project = %p.project_id,
            path = %rendered.output.display(),
            ms = t0.elapsed().as_millis(),
            "export written locally"
        );
    }
    Ok(())
}

/// A talking picture's onsets are speech, not a beat grid. An audio file still
/// gets a grid when the words are lyrics. A quiet picture gets one too.
fn wants_beat_grid(quiet: bool, has_video: bool) -> bool {
    quiet || !has_video
}

#[cfg(test)]
mod tests {
    use super::{dress_safe, wants_beat_grid};

    #[test]
    fn dress_safe_uses_the_vertical_theme() {
        let mut cues = vec![oc_core::CaptionCue {
            start: oc_core::Time::ZERO,
            end: oc_core::Time::from_seconds(2.0),
            text: "hello there".into(),
            speaker: None,
            place: oc_core::CaptionPlace::Bottom,
            font: oc_core::CaptionFont::Sans,
            effect: oc_core::CaptionEffect::None,
        }];
        let timeline = oc_core::Timeline::new(oc_core::FrameRate::FPS_30, 1080, 1920);
        dress_safe(&mut cues, &timeline);
        assert_eq!(cues[0].font, oc_core::CaptionFont::Display);
        assert_eq!(cues[0].effect, oc_core::CaptionEffect::Pop);
        assert_eq!(cues[0].place, oc_core::CaptionPlace::Lower);
    }

    #[test]
    fn beat_grid_skips_talking_video_and_keeps_music_files() {
        assert!(!wants_beat_grid(false, true));
        assert!(wants_beat_grid(true, true));
        assert!(wants_beat_grid(false, false));
        assert!(wants_beat_grid(true, false));
    }
}
