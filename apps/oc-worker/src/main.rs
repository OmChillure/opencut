use anyhow::Context;
use oc_core::{CaptionCue, Op, UndoStack, apply};
use oc_db::Db;
use oc_voice::transcribe_local;
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
        tracing::warn!("R2 not configured — transcribe jobs will fail");
    }
    if oc_voice::groq_stt_configured() {
        tracing::info!("understand = ffmpeg look + Groq Whisper (free, ~8h audio/day)");
    } else if oc_voice::grok_stt_configured() {
        tracing::info!("understand = ffmpeg look + Grok STT ($0.10/hour)");
    } else {
        tracing::info!(
            "understand = ffmpeg look + local Whisper (set GROQ_API_KEY for free hosted Whisper)"
        );
    }

    tracing::info!("worker polling jobs");
    let mut fail = 0u32;
    loop {
        match oc_db::claim_job(&db).await {
            Ok(Some(job)) => {
                fail = 0;
                tracing::info!(id = %job.id, kind = %job.kind, "claimed job");
                let err = handle(&db, r2.as_ref(), &job.kind, job.payload)
                    .await
                    .err()
                    .map(|e| e.to_string());
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
                tracing::error!(
                    size = db.size(),
                    idle = db.num_idle(),
                    fail,
                    "claim: {err}"
                );
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
        other => anyhow::bail!("unknown job kind {other}"),
    }
}

async fn transcribe(
    db: &Db,
    r2: Option<&R2>,
    p: TranscribePayload,
) -> anyhow::Result<()> {
    let t0 = std::time::Instant::now();
    tracing::info!(media = %p.media_id, key = %p.r2_key, "transcribe start");
    if !oc_db::is_r2_object_key(&p.r2_key) {
        oc_db::set_media_status(db, p.media_id, "ready").await?;
        anyhow::bail!("clip not in R2 yet (workspace-only). Re-import or wait for upload.");
    }
    let r2 = r2.context("R2 required")?;
    let bytes = match r2.get_bytes(&p.r2_key).await {
        Ok(b) => b,
        Err(e) => {
            oc_db::set_media_status(db, p.media_id, "ready").await?;
            anyhow::bail!("R2 missing object {}: {e}", p.r2_key);
        }
    };
    tracing::info!(
        media = %p.media_id,
        bytes = bytes.len(),
        ms = t0.elapsed().as_millis(),
        "r2 download"
    );
    let filename = p.r2_key.rsplit('/').next().unwrap_or("audio.bin");

    match oc_media::analyze_local(&bytes, filename).await {
        Ok(look) => {
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
                    ms = t0.elapsed().as_millis(),
                    "look saved — chat can proceed"
                );
            }
        }
        Err(e) => tracing::warn!("local look failed: {e}"),
    }

    tracing::info!(media = %p.media_id, "whisper start");
    let transcript = match transcribe_local(&bytes, filename).await {
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
    if transcript.full_text.trim().is_empty() && transcript.cues.is_empty() {
        tracing::info!(
            media = %p.media_id,
            ms = t0.elapsed().as_millis(),
            "no speech (silent or whisper empty)"
        );
        oc_db::set_media_status(db, p.media_id, "ready").await?;
        return Ok(());
    }
    tracing::info!(
        media = %p.media_id,
        words = transcript.full_text.split_whitespace().count(),
        cues = transcript.cues.len(),
        ms = t0.elapsed().as_millis(),
        "whisper saved"
    );

    let raw = serde_json::to_value(&transcript)?;
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

    let cues: Vec<CaptionCue> = transcript.cues.into_iter().map(|c| c.into_timeline()).collect();
    let mut project = oc_db::get_project(db, p.project_id).await?;
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
