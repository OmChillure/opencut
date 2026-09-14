mod storage;

pub use storage::{R2, R2Config, StorageError};

use chrono::{DateTime, Utc};
use oc_timeline::{Project, ProjectId, Timeline};
use serde::{Deserialize, Serialize};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;
use std::str::FromStr;
use std::time::Duration;
use uuid::Uuid;

pub type Db = PgPool;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("missing DATABASE_URL")]
    MissingUrl,
    #[error("not found")]
    NotFound,
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error(transparent)]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("timeline json: {0}")]
    Json(#[from] serde_json::Error),
}

pub async fn connect() -> Result<Db, DbError> {
    let url = std::env::var("DATABASE_URL").map_err(|_| DbError::MissingUrl)?;
    let url = prefer_session_pooler(&url);
    // Transaction-mode poolers (Supabase :6543) leave unnamed prepared
    // statements on the backend. Next Bind then fails with
    // "supplies 0 parameters, but statement requires 1".
    let opts = PgConnectOptions::from_str(&url)?.statement_cache_capacity(0);
    let pool = PgPoolOptions::new()
        .max_connections(3)
        .acquire_timeout(Duration::from_secs(15))
        .idle_timeout(Duration::from_secs(30))
        .after_connect(|conn, _| {
            Box::pin(async move {
                let _ = sqlx::raw_sql("deallocate all").execute(&mut *conn).await;
                Ok(())
            })
        })
        .before_acquire(|conn, _| {
            Box::pin(async move {
                let _ = sqlx::raw_sql("deallocate all").execute(&mut *conn).await;
                Ok(true)
            })
        })
        .connect_with(opts)
        .await?;
    Ok(pool)
}

/// Supabase/Neon :6543 is transaction PgBouncer. sqlx needs session mode (:5432).
fn prefer_session_pooler(url: &str) -> String {
    if url.contains("pooler.supabase.com:6543") || url.contains("pooler.supabase.com:5432") {
        let next = url.replace(":6543", ":5432");
        if next != url {
            tracing::warn!("DATABASE_URL used port 6543 (transaction pooler); using 5432 (session) so sqlx binds work");
        }
        return next;
    }
    if let Some(rest) = url.split_once("-pooler.").map(|(_, rest)| rest) {
        if rest.contains(".neon.tech") && url.contains(":6543") {
            return url.replace(":6543", ":5432");
        }
    }
    url.to_string()
}

pub async fn migrate(pool: &Db) -> Result<(), DbError> {
    // Simple-query protocol so this works on a transaction-mode pooler.
    // 0001_init.sql is idempotent (`if not exists`).
    sqlx::raw_sql(include_str!("../migrations/0001_init.sql"))
        .execute(pool)
        .await?;
    sqlx::raw_sql(include_str!("../migrations/0002_media_analysis.sql"))
        .execute(pool)
        .await?;
    Ok(())
}

fn query(
    sql: &'static str,
) -> sqlx::query::Query<'static, sqlx::Postgres, sqlx::postgres::PgArguments> {
    sqlx::query(sql).persistent(false)
}

fn query_as<T>(
    sql: &'static str,
) -> sqlx::query::QueryAs<'static, sqlx::Postgres, T, sqlx::postgres::PgArguments>
where
    T: for<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow>,
{
    sqlx::query_as(sql).persistent(false)
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ProjectRow {
    pub id: Uuid,
    pub name: String,
    pub timeline: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ProjectRow {
    pub fn into_project(self) -> Result<Project, DbError> {
        Ok(Project {
            id: ProjectId::from_uuid(self.id),
            name: self.name,
            timeline: serde_json::from_value(self.timeline)?,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct MediaRow {
    pub id: Uuid,
    pub project_id: Uuid,
    pub r2_key: String,
    pub filename: String,
    pub content_type: String,
    pub byte_size: Option<i64>,
    pub duration_ticks: Option<i64>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct JobRow {
    pub id: Uuid,
    pub kind: String,
    pub status: String,
    pub payload: serde_json::Value,
    pub error: Option<String>,
}

pub async fn create_project(pool: &Db, name: &str) -> Result<Project, DbError> {
    let project = Project::new(name);
    let timeline = serde_json::to_value(&project.timeline)?;
    query(
        "insert into projects (id, name, timeline) values ($1, $2, $3)",
    )
    .bind(project.id.as_uuid())
    .bind(&project.name)
    .bind(&timeline)
    .execute(pool)
    .await?;
    Ok(project)
}

pub async fn list_projects(pool: &Db) -> Result<Vec<ProjectRow>, DbError> {
    let rows = query_as::<ProjectRow>(
        "select id, name, timeline, created_at, updated_at from projects order by updated_at desc",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn get_project(pool: &Db, id: Uuid) -> Result<Project, DbError> {
    let row = query_as::<ProjectRow>(
        "select id, name, timeline, created_at, updated_at from projects where id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or(DbError::NotFound)?;
    row.into_project()
}

pub async fn rename_project(pool: &Db, id: Uuid, name: &str) -> Result<Project, DbError> {
    let res = query("update projects set name = $2, updated_at = now() where id = $1")
        .bind(id)
        .bind(name)
        .execute(pool)
        .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    get_project(pool, id).await
}

/// One round-trip: collect media keys, drop jobs, cascade-delete the project.
/// Already-gone projects return an empty key list (idempotent).
pub async fn delete_project(pool: &Db, id: Uuid) -> Result<Vec<String>, DbError> {
    let rows: Vec<(Uuid, Option<String>)> = query_as(
        "with keys as (
            select r2_key from media where project_id = $1
         ),
         _jobs as (
            delete from jobs where payload->>'project_id' = $2
         ),
         gone as (
            delete from projects where id = $1 returning id
         )
         select g.id, k.r2_key
         from gone g
         left join keys k on true",
    )
    .bind(id)
    .bind(id.to_string())
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().filter_map(|(_, key)| key).collect())
}

pub async fn save_timeline(pool: &Db, id: Uuid, timeline: &Timeline) -> Result<(), DbError> {
    let value = serde_json::to_value(timeline)?;
    let res = query(
        "update projects set timeline = $2, updated_at = now() where id = $1",
    )
    .bind(id)
    .bind(&value)
    .execute(pool)
    .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

pub async fn insert_media(
    pool: &Db,
    project_id: Uuid,
    media_id: Uuid,
    r2_key: &str,
    filename: &str,
    content_type: &str,
) -> Result<MediaRow, DbError> {
    let row = query_as::<MediaRow>(
        "insert into media (id, project_id, r2_key, filename, content_type, status)
         values ($1, $2, $3, $4, $5, 'uploading')
         returning id, project_id, r2_key, filename, content_type, byte_size, duration_ticks, width, height, status, created_at",
    )
    .bind(media_id)
    .bind(project_id)
    .bind(r2_key)
    .bind(filename)
    .bind(content_type)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// Register a clip that lives in the editor even if R2 upload never ran.
pub async fn upsert_workspace_media(
    pool: &Db,
    project_id: Uuid,
    media_id: Uuid,
    filename: &str,
    content_type: &str,
    duration_ticks: Option<i64>,
) -> Result<(), DbError> {
    let key = format!("workspace/{media_id}");
    query(
        "insert into media (id, project_id, r2_key, filename, content_type, duration_ticks, status)
         values ($1, $2, $3, $4, $5, $6, 'ready')
         on conflict (id) do update set
            filename = excluded.filename,
            content_type = excluded.content_type,
            duration_ticks = coalesce(excluded.duration_ticks, media.duration_ticks)",
    )
    .bind(media_id)
    .bind(project_id)
    .bind(&key)
    .bind(filename)
    .bind(content_type)
    .bind(duration_ticks)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn set_media_r2_key(pool: &Db, id: Uuid, r2_key: &str) -> Result<(), DbError> {
    query("update media set r2_key = $2, status = 'ready' where id = $1")
        .bind(id)
        .bind(r2_key)
        .execute(pool)
        .await?;
    Ok(())
}

pub fn is_r2_object_key(key: &str) -> bool {
    !key.is_empty() && !key.starts_with("workspace/")
}

pub async fn list_media(pool: &Db, project_id: Uuid) -> Result<Vec<MediaRow>, DbError> {
    let rows = query_as::<MediaRow>(
        "select id, project_id, r2_key, filename, content_type, byte_size, duration_ticks, width, height, status, created_at
         from media where project_id = $1 order by created_at desc",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn get_media(pool: &Db, id: Uuid) -> Result<MediaRow, DbError> {
    query_as::<MediaRow>(
        "select id, project_id, r2_key, filename, content_type, byte_size, duration_ticks, width, height, status, created_at
         from media where id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or(DbError::NotFound)
}

pub async fn set_media_status(pool: &Db, id: Uuid, status: &str) -> Result<(), DbError> {
    query("update media set status = $2 where id = $1")
        .bind(id)
        .bind(status)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_media_duration(pool: &Db, id: Uuid, duration_ticks: i64) -> Result<(), DbError> {
    query("update media set duration_ticks = $2 where id = $1")
        .bind(id)
        .bind(duration_ticks)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn enqueue_job(
    pool: &Db,
    kind: &str,
    payload: serde_json::Value,
) -> Result<Uuid, DbError> {
    let id = Uuid::now_v7();
    query("insert into jobs (id, kind, payload) values ($1, $2, $3)")
        .bind(id)
        .bind(kind)
        .bind(&payload)
        .execute(pool)
        .await?;
    Ok(id)
}

pub async fn claim_job(pool: &Db) -> Result<Option<JobRow>, DbError> {
    let row = query_as::<JobRow>(
        "update jobs
         set status = 'running', updated_at = now()
         where id = (
            select id from jobs
            where status = 'queued'
            order by created_at
            for update skip locked
            limit 1
         )
         returning id, kind, status, payload, error",
    )
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn finish_job(pool: &Db, id: Uuid, error: Option<&str>) -> Result<(), DbError> {
    let status = if error.is_some() { "failed" } else { "done" };
    query("update jobs set status = $2, error = $3, updated_at = now() where id = $1")
        .bind(id)
        .bind(status)
        .bind(error)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn insert_transcript(
    pool: &Db,
    media_id: Uuid,
    language: Option<&str>,
    full_text: &str,
    raw: &serde_json::Value,
    cues: &[(i64, i64, &str, Option<&str>)],
) -> Result<Uuid, DbError> {
    let id = Uuid::now_v7();
    query(
        "insert into transcripts (id, media_id, language, full_text, raw) values ($1, $2, $3, $4, $5)",
    )
    .bind(id)
    .bind(media_id)
    .bind(language)
    .bind(full_text)
    .bind(raw)
    .execute(pool)
    .await?;
    for (i, (start, end, text, speaker)) in cues.iter().enumerate() {
        query(
            "insert into cues (transcript_id, start_ticks, end_ticks, text, speaker, idx)
             values ($1, $2, $3, $4, $5, $6)",
        )
        .bind(id)
        .bind(start)
        .bind(end)
        .bind(text)
        .bind(speaker)
        .bind(i as i32)
        .execute(pool)
        .await?;
    }
    Ok(id)
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TranscriptCueRow {
    pub media_id: Uuid,
    pub full_text: String,
    pub start_ticks: i64,
    pub end_ticks: i64,
    pub text: String,
}

pub async fn list_transcripts_for_project(
    pool: &Db,
    project_id: Uuid,
) -> Result<Vec<TranscriptCueRow>, DbError> {
    let rows = query_as::<TranscriptCueRow>(
        "select t.media_id, t.full_text, c.start_ticks, c.end_ticks, c.text
         from transcripts t
         join media m on m.id = t.media_id
         join cues c on c.transcript_id = t.id
         where m.project_id = $1
           and t.created_at = (
             select max(t2.created_at) from transcripts t2 where t2.media_id = t.media_id
           )
         order by t.media_id, c.idx",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AnalysisRow {
    pub media_id: Uuid,
    pub look: String,
    pub motion: f64,
    pub scenes: i32,
    pub brightness: f64,
    pub colorful: bool,
    pub has_video: bool,
    pub has_audio: bool,
}

pub async fn upsert_media_analysis(
    pool: &Db,
    media_id: Uuid,
    look: &str,
    motion: f64,
    scenes: i32,
    brightness: f64,
    colorful: bool,
    has_video: bool,
    has_audio: bool,
    raw: &serde_json::Value,
) -> Result<(), DbError> {
    query(
        "insert into media_analysis
            (media_id, look, motion, scenes, brightness, colorful, has_video, has_audio, raw, updated_at)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, now())
         on conflict (media_id) do update set
            look = excluded.look,
            motion = excluded.motion,
            scenes = excluded.scenes,
            brightness = excluded.brightness,
            colorful = excluded.colorful,
            has_video = excluded.has_video,
            has_audio = excluded.has_audio,
            raw = excluded.raw,
            updated_at = now()",
    )
    .bind(media_id)
    .bind(look)
    .bind(motion)
    .bind(scenes)
    .bind(brightness)
    .bind(colorful)
    .bind(has_video)
    .bind(has_audio)
    .bind(raw)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_analysis_for_project(
    pool: &Db,
    project_id: Uuid,
) -> Result<Vec<AnalysisRow>, DbError> {
    let rows = query_as::<AnalysisRow>(
        "select a.media_id, a.look, a.motion, a.scenes, a.brightness, a.colorful, a.has_video, a.has_audio
         from media_analysis a
         join media m on m.id = a.media_id
         where m.project_id = $1",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
