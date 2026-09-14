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
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let db = oc_db::connect().await.context("database")?;
    oc_db::migrate(&db).await.ok();
    let r2 = R2::from_env().await.ok();
    if r2.is_none() {
        tracing::warn!("R2 not configured — transcribe jobs will fail");
    }
    tracing::info!("transcribe uses local ffmpeg + Whisper (no Sarvam)");

    tracing::info!("worker polling jobs");
    loop {
        match oc_db::claim_job(&db).await {
            Ok(Some(job)) => {
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
            Ok(None) => sleep(Duration::from_millis(750)).await,
            Err(err) => {
                tracing::error!("claim: {err}");
                sleep(Duration::from_secs(2)).await;
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
    let r2 = r2.context("R2 required")?;
    let bytes = r2.get_bytes(&p.r2_key).await?;
    let filename = p.r2_key.rsplit('/').next().unwrap_or("audio.bin");
    let transcript = transcribe_local(&bytes, filename)
        .await
        .context("local whisper")?;

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
