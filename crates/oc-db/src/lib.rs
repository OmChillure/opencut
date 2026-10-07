mod pass;
mod storage;

pub use storage::{ObjectStat, R2, R2Config, StorageError};

use chrono::{DateTime, Utc};
use oc_timeline::{Project, ProjectId, Timeline};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
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
    #[error("bad user")]
    BadUser,
    #[error("wrong password")]
    WrongPassword,
}

impl DbError {
    #[must_use]
    pub fn is_pool_timeout(&self) -> bool {
        matches!(
            self,
            Self::Sqlx(sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed)
        )
    }
}

pub async fn connect() -> Result<Db, DbError> {
    let url = std::env::var("DATABASE_URL").map_err(|_| DbError::MissingUrl)?;
    let url = prefer_session_pooler(&url);
    // Transaction-mode poolers (Supabase :6543) leave unnamed prepared
    // statements on the backend. Next Bind then fails with
    // "supplies 0 parameters, but statement requires 1".
    let opts = PgConnectOptions::from_str(&url)?
        .statement_cache_capacity(0)
        .application_name("opencut");
    let host = opts.get_host().to_string();
    let port = opts.get_port();
    // Do not run SQL in before_acquire. Dead sockets from the Supabase
    // pooler make `deallocate all` hang until acquire_timeout, which is
    // the "pool timed out while waiting for an open connection" log.
    // sqlx already pings (test_before_acquire defaults to true).
    let max = env_u32("DB_MAX_CONNECTIONS", 4);
    let pool = PgPoolOptions::new()
        .max_connections(max)
        .acquire_timeout(Duration::from_secs(env_secs("DB_ACQUIRE_TIMEOUT_SECS", 8)))
        .idle_timeout(Duration::from_secs(env_secs("DB_IDLE_TIMEOUT_SECS", 90)))
        .max_lifetime(Duration::from_secs(env_secs("DB_MAX_LIFETIME_SECS", 240)))
        .after_connect(|conn, _| {
            Box::pin(async move {
                let _ = sqlx::raw_sql("deallocate all").execute(&mut *conn).await;
                Ok(())
            })
        })
        .connect_with(opts)
        .await?;
    tracing::info!(host, port, max, "postgres pool ready");
    Ok(pool)
}

fn env_secs(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|n| *n > 0)
        .unwrap_or(default)
}

fn env_u32(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|n| *n > 0)
        .unwrap_or(default)
}

/// Supabase/Neon :6543 is transaction PgBouncer. sqlx needs session mode (:5432).
fn prefer_session_pooler(url: &str) -> String {
    if url.contains("pooler.supabase.com:6543") || url.contains("pooler.supabase.com:5432") {
        let next = url.replace(":6543", ":5432");
        if next != url {
            tracing::warn!(
                "DATABASE_URL used port 6543 (transaction pooler); using 5432 (session) so sqlx binds work"
            );
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
    sqlx::raw_sql(include_str!("../migrations/0003_chats.sql"))
        .execute(pool)
        .await?;
    sqlx::raw_sql(include_str!("../migrations/0004_projects_owner.sql"))
        .execute(pool)
        .await?;
    sqlx::raw_sql(include_str!("../migrations/0005_sessions.sql"))
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
    pub owner_email: Option<String>,
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

pub async fn create_project(pool: &Db, name: &str, owner: &str) -> Result<Project, DbError> {
    let email = normalize_user_email(owner)?;
    let project = Project::new(name);
    let timeline = serde_json::to_value(&project.timeline)?;
    query("insert into projects (id, name, timeline, owner_email) values ($1, $2, $3, $4)")
        .bind(project.id.as_uuid())
        .bind(&project.name)
        .bind(&timeline)
        .bind(&email)
        .execute(pool)
        .await?;
    Ok(project)
}

/// Projects with no owner yet belong to the first signed-in email that lists them.
pub async fn list_projects(pool: &Db, owner: &str) -> Result<Vec<ProjectRow>, DbError> {
    let email = normalize_user_email(owner)?;
    query("update projects set owner_email = $1 where owner_email is null")
        .bind(&email)
        .execute(pool)
        .await?;
    let rows = query_as::<ProjectRow>(
        "select id, name, timeline, created_at, updated_at, owner_email from projects where owner_email = $1 order by updated_at desc",
    )
    .bind(&email)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn get_project(pool: &Db, id: Uuid) -> Result<Project, DbError> {
    let row = query_as::<ProjectRow>(
        "select id, name, timeline, created_at, updated_at, owner_email from projects where id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or(DbError::NotFound)?;
    row.into_project()
}

/// Claim a project that has no owner, then refuse a different email.
pub async fn require_project_owner(pool: &Db, id: Uuid, owner: &str) -> Result<(), DbError> {
    let email = normalize_user_email(owner)?;
    query("update projects set owner_email = $2 where id = $1 and owner_email is null")
        .bind(id)
        .bind(&email)
        .execute(pool)
        .await?;
    let found: Option<(Uuid,)> =
        query_as("select id from projects where id = $1 and owner_email = $2")
            .bind(id)
            .bind(&email)
            .fetch_optional(pool)
            .await?;
    found.map(|_| ()).ok_or(DbError::NotFound)
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
    let res = query("update projects set timeline = $2, updated_at = now() where id = $1")
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

/// Register a clip before its bytes are on R2. The key stays `workspace/{id}` until the put stores the object key.
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
         values ($1, $2, $3, $4, $5, $6, 'uploading')
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
    !key.is_empty() && !key.starts_with("workspace/") && !key.starts_with("local/")
}

/// Last path segment of an object key, safe to join onto a temp directory.
/// `..` and an empty name become `media.bin` so a key cannot escape that directory.
#[must_use]
pub fn object_file_name(key: &str) -> String {
    let raw = key.rsplit(['/', '\\']).next().unwrap_or("").trim();
    let raw = raw.trim_matches('.');
    if raw.is_empty() || raw.contains('\0') {
        return "media.bin".into();
    }
    let mut out = String::new();
    for ch in raw.chars().take(120) {
        if ch.is_control() {
            continue;
        }
        out.push(ch);
    }
    if out.is_empty() {
        "media.bin".into()
    } else {
        out
    }
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

/// A live worker updates `updated_at` this often. A crash stops the updates.
pub const JOB_HEARTBEAT_SECS: u64 = 30;
/// Reclaim a running job after this many seconds without a heartbeat.
pub const JOB_STALE_AFTER_SECS: u64 = 120;

const CLAIM_JOB_SQL: &str = "update jobs
         set status = 'running', updated_at = now()
         where id = (
            select id from jobs
            where status = 'queued'
               or (status = 'running' and updated_at < now() - interval '120 seconds')
            order by created_at
            for update skip locked
            limit 1
         )
         returning id, kind, status, payload, error";

const LATEST_EXPORT_JOB: &str = "select created_at from jobs
         where kind = 'export'
           and payload->>'project_id' = $1
           and payload->>'preset' = $2
         order by created_at desc
         limit 1";

pub async fn claim_job(pool: &Db) -> Result<Option<JobRow>, DbError> {
    let row = query_as::<JobRow>(CLAIM_JOB_SQL)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

/// Keep a running job from being claimed by another worker.
pub async fn touch_job(pool: &Db, id: Uuid) -> Result<(), DbError> {
    query("update jobs set updated_at = now() where id = $1 and status = 'running'")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// When the newest export job for this preset was queued, in unix milliseconds.
pub async fn latest_export_job_ms(
    pool: &Db,
    project_id: Uuid,
    preset: &str,
) -> Result<Option<i64>, DbError> {
    let row = query_as::<(DateTime<Utc>,)>(LATEST_EXPORT_JOB)
        .bind(project_id.to_string())
        .bind(preset)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|(created,)| created.timestamp_millis()))
}

pub async fn job_finished(pool: &Db, id: Uuid) -> Result<bool, DbError> {
    let row = query_as::<(String,)>("select status from jobs where id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(matches!(
        row.as_ref().map(|r| r.0.as_str()),
        Some("done" | "failed") | None
    ))
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

pub async fn has_transcript(pool: &Db, media_id: Uuid) -> Result<bool, DbError> {
    let row = query_as::<(Uuid,)>("select id from transcripts where media_id = $1 limit 1")
        .bind(media_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.is_some())
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
    pub raw: Option<serde_json::Value>,
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
        "select a.media_id, a.look, a.motion, a.scenes, a.brightness, a.colorful, a.has_video, a.has_audio, a.raw
         from media_analysis a
         join media m on m.id = a.media_id
         where m.project_id = $1",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ChatRow {
    pub id: Uuid,
    pub title: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ChatMessageInput {
    pub role: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub tool_id: String,
    #[serde(default)]
    pub tool_name: String,
    #[serde(default)]
    pub tool_status: String,
    #[serde(default)]
    pub tool_args: String,
    #[serde(default)]
    pub tool_result: String,
}

#[derive(sqlx::FromRow)]
struct PasswordRow {
    password_hash: String,
}

#[derive(sqlx::FromRow)]
struct EmailRow {
    email: String,
}

/// First sign-in claims the email. A later sign-in must match that password.
/// Returns `(email, token)`.
pub async fn open_session(
    pool: &Db,
    email: &str,
    password: &str,
) -> Result<(String, String), DbError> {
    let email = normalize_user_email(email)?;
    if password.chars().count() < 4 || password.len() > 200 {
        return Err(DbError::BadUser);
    }
    let salt = *Uuid::new_v4().as_bytes();
    let hash = pass::hash_password(password, &salt, pass::PBKDF2_ITERS);
    query(
        "insert into users (email, password_hash) values ($1, $2)
         on conflict (email) do nothing",
    )
    .bind(&email)
    .bind(&hash)
    .execute(pool)
    .await?;
    let stored = query_as::<PasswordRow>("select password_hash from users where email = $1")
        .bind(&email)
        .fetch_optional(pool)
        .await?
        .ok_or(DbError::NotFound)?;
    if !pass::password_matches(password, &stored.password_hash) {
        return Err(DbError::WrongPassword);
    }
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    query("insert into sessions (token, email) values ($1, $2)")
        .bind(&token)
        .bind(&email)
        .execute(pool)
        .await?;
    Ok((email, token))
}

pub async fn email_for_session(pool: &Db, token: &str) -> Result<String, DbError> {
    let token = token.trim();
    if token.len() < 16 || token.len() > 200 {
        return Err(DbError::NotFound);
    }
    query_as::<EmailRow>("select email from sessions where token = $1")
        .bind(token)
        .fetch_optional(pool)
        .await?
        .map(|row| row.email)
        .ok_or(DbError::NotFound)
}

pub fn normalize_user_email(raw: &str) -> Result<String, DbError> {
    let email = raw.trim().to_ascii_lowercase();
    if email.len() < 3
        || email.len() > 200
        || !email.contains('@')
        || email.starts_with('@')
        || email.ends_with('@')
        || email.contains(char::is_whitespace)
    {
        return Err(DbError::BadUser);
    }
    Ok(email)
}

/// Keep a title the person set. Otherwise use the first thing they asked.
pub fn chat_title(current: &str, first_user_text: Option<&str>) -> String {
    let current = current.trim();
    if !current.is_empty() && current != "New chat" {
        return current.to_string();
    }
    let Some(text) = first_user_text
        .map(str::trim)
        .filter(|text| !text.is_empty())
    else {
        return "New chat".into();
    };
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = String::new();
    for (i, ch) in flat.chars().enumerate() {
        if i == 48 {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    if out.is_empty() {
        "New chat".into()
    } else {
        out
    }
}

/// CLI tracing must not land in the database.
pub fn is_cli_noise(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("sampling.request")
        || lower.contains("cli-chat-proxy")
        || lower.contains("auth_prefix")
        || lower.contains("encrypted_content")
        || lower.contains("sse_chunk")
        || lower.contains("api_backend")
        || text.contains("[0m")
        || text.contains("[32m")
        || text.contains("[2m")
        || text.contains('\u{1b}')
}

pub fn prepare_chat_messages(messages: &[ChatMessageInput]) -> Vec<ChatMessageInput> {
    let mut out = Vec::new();
    for msg in messages {
        let Some(role) = canonical_chat_role(&msg.role) else {
            continue;
        };
        if is_cli_noise(&msg.text) || is_cli_noise(&msg.tool_result) {
            continue;
        }
        let text = clip_chars(&msg.text, 100_000);
        if text.trim().is_empty() && role != "tool" {
            continue;
        }
        out.push(ChatMessageInput {
            role: role.into(),
            text,
            tool_id: clip_chars(&msg.tool_id, 200),
            tool_name: clip_chars(&msg.tool_name, 200),
            tool_status: clip_chars(&msg.tool_status, 40),
            tool_args: clip_chars(&msg.tool_args, 20_000),
            tool_result: clip_chars(&msg.tool_result, 100_000),
        });
    }
    if out.len() > 400 {
        out = out.split_off(out.len() - 400);
    }
    out
}

fn canonical_chat_role(role: &str) -> Option<&'static str> {
    match role {
        "user" => Some("user"),
        "assistant" | "bot" => Some("assistant"),
        "tool" => Some("tool"),
        "thought" => Some("thought"),
        _ => None,
    }
}

fn clip_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    text.chars().take(max).collect()
}

pub async fn list_chats(pool: &Db, project_id: Uuid, user: &str) -> Result<Vec<ChatRow>, DbError> {
    let user = normalize_user_email(user)?;
    let rows = query_as::<ChatRow>(
        "select id, title, updated_at from chats
         where project_id = $1 and user_email = $2
         order by updated_at desc
         limit 100",
    )
    .bind(project_id)
    .bind(user)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn create_chat(
    pool: &Db,
    project_id: Uuid,
    user: &str,
    title: Option<&str>,
) -> Result<ChatRow, DbError> {
    let user = normalize_user_email(user)?;
    let title = chat_title("", title);
    let id = Uuid::now_v7();
    let row = query_as::<ChatRow>(
        "insert into chats (id, project_id, user_email, title)
         values ($1, $2, $3, $4)
         returning id, title, updated_at",
    )
    .bind(id)
    .bind(project_id)
    .bind(user)
    .bind(title)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn get_chat(
    pool: &Db,
    project_id: Uuid,
    chat_id: Uuid,
    user: &str,
) -> Result<(ChatRow, Vec<ChatMessageInput>), DbError> {
    let user = normalize_user_email(user)?;
    let chat = chat_for_user(pool, project_id, chat_id, &user).await?;
    let messages = query_as::<ChatMessageInput>(
        "select role, text, tool_id, tool_name, tool_status, tool_args, tool_result
         from chat_messages
         where chat_id = $1
         order by idx",
    )
    .bind(chat_id)
    .fetch_all(pool)
    .await?;
    Ok((chat, messages))
}

pub async fn save_chat_messages(
    pool: &Db,
    project_id: Uuid,
    chat_id: Uuid,
    user: &str,
    messages: &[ChatMessageInput],
) -> Result<ChatRow, DbError> {
    let user = normalize_user_email(user)?;
    let chat = chat_for_user(pool, project_id, chat_id, &user).await?;
    let messages = prepare_chat_messages(messages);
    let title = chat_title(
        &chat.title,
        messages
            .iter()
            .find(|msg| msg.role == "user")
            .map(|msg| msg.text.as_str()),
    );
    let mut tx = pool.begin().await?;
    query("delete from chat_messages where chat_id = $1")
        .bind(chat_id)
        .execute(&mut *tx)
        .await?;
    for (idx, msg) in messages.iter().enumerate() {
        query(
            "insert into chat_messages
                (id, chat_id, idx, role, text, tool_id, tool_name, tool_status, tool_args, tool_result)
             values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(Uuid::now_v7())
        .bind(chat_id)
        .bind(idx as i32)
        .bind(&msg.role)
        .bind(&msg.text)
        .bind(&msg.tool_id)
        .bind(&msg.tool_name)
        .bind(&msg.tool_status)
        .bind(&msg.tool_args)
        .bind(&msg.tool_result)
        .execute(&mut *tx)
        .await?;
    }
    let row = query_as::<ChatRow>(
        "update chats set title = $2, updated_at = now()
         where id = $1
         returning id, title, updated_at",
    )
    .bind(chat_id)
    .bind(&title)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

async fn chat_for_user(
    pool: &Db,
    project_id: Uuid,
    chat_id: Uuid,
    user: &str,
) -> Result<ChatRow, DbError> {
    query_as::<ChatRow>(
        "select id, title, updated_at from chats
         where id = $1 and project_id = $2 and user_email = $3",
    )
    .bind(chat_id)
    .bind(project_id)
    .bind(user)
    .fetch_optional(pool)
    .await?
    .ok_or(DbError::NotFound)
}

#[cfg(test)]
mod tests {
    use super::prefer_session_pooler;

    #[test]
    fn an_object_name_cannot_leave_its_temp_directory() {
        assert_eq!(super::object_file_name("raw/p/m/clip.mp4"), "clip.mp4");
        assert_eq!(super::object_file_name("raw/p/m/.."), "media.bin");
        assert_eq!(super::object_file_name("raw/p/m/"), "media.bin");
        assert_eq!(super::object_file_name(".."), "media.bin");
        assert_eq!(
            super::object_file_name("raw/p/m/my take.mp4"),
            "my take.mp4"
        );
    }

    #[test]
    fn claim_reclaims_a_job_that_stopped_heartbeating() {
        assert!(super::CLAIM_JOB_SQL.contains("status = 'queued'"));
        assert!(super::CLAIM_JOB_SQL.contains("status = 'running'"));
        assert!(super::CLAIM_JOB_SQL.contains("interval '120 seconds'"));
        assert!(super::CLAIM_JOB_SQL.contains("for update skip locked"));
        assert_eq!(super::JOB_STALE_AFTER_SECS, 120);
        assert!(super::JOB_HEARTBEAT_SECS * 3 < super::JOB_STALE_AFTER_SECS);
        assert!(super::LATEST_EXPORT_JOB.contains("kind = 'export'"));
        assert!(super::LATEST_EXPORT_JOB.contains("payload->>'project_id'"));
        assert!(super::LATEST_EXPORT_JOB.contains("payload->>'preset'"));
    }

    #[test]
    fn rewrites_supabase_transaction_port() {
        let url = "postgres://u:p@aws-0-ap-northeast-2.pooler.supabase.com:6543/postgres";
        let next = prefer_session_pooler(url);
        assert!(next.contains(":5432"), "{next}");
        assert!(!next.contains(":6543"), "{next}");
    }

    #[test]
    fn chat_title_uses_the_first_request() {
        assert_eq!(super::chat_title("", None), "New chat");
        assert_eq!(super::chat_title("New chat", None), "New chat");
        assert_eq!(
            super::chat_title("New chat", Some("edit like a pro editor")),
            "edit like a pro editor"
        );
        assert_eq!(
            super::chat_title("Rough cut", Some("ignore this")),
            "Rough cut"
        );
        let long = "a".repeat(60);
        let title = super::chat_title("New chat", Some(&long));
        assert_eq!(title.chars().count(), 49);
        assert!(title.ends_with('…'));
    }

    #[test]
    fn saved_messages_drop_status_and_cli_logs() {
        use super::ChatMessageInput;
        let rows = super::prepare_chat_messages(&[
            ChatMessageInput {
                role: "user".into(),
                text: "edit like a pro editor".into(),
                tool_id: String::new(),
                tool_name: String::new(),
                tool_status: String::new(),
                tool_args: String::new(),
                tool_result: String::new(),
            },
            ChatMessageInput {
                role: "status".into(),
                text: "Sending to xai".into(),
                tool_id: String::new(),
                tool_name: String::new(),
                tool_status: String::new(),
                tool_args: String::new(),
                tool_result: String::new(),
            },
            ChatMessageInput {
                role: "assistant".into(),
                text: "acp: [2m INFO sampling.request auth_prefix=hidden encrypted_content".into(),
                tool_id: String::new(),
                tool_name: String::new(),
                tool_status: String::new(),
                tool_args: String::new(),
                tool_result: String::new(),
            },
            ChatMessageInput {
                role: "bot".into(),
                text: "I'll cut a 40 second vlog from the clips you imported.".into(),
                tool_id: String::new(),
                tool_name: String::new(),
                tool_status: String::new(),
                tool_args: String::new(),
                tool_result: String::new(),
            },
        ]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].role, "user");
        assert_eq!(rows[1].role, "assistant");
        assert!(rows[1].text.contains("40 second"));
    }

    #[test]
    fn email_is_the_chat_owner() {
        assert_eq!(
            super::normalize_user_email("  A@Studio.com ").unwrap(),
            "a@studio.com"
        );
        assert!(super::normalize_user_email("not-an-email").is_err());
    }
}
