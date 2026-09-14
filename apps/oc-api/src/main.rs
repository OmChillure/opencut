mod routes;
mod state;

use anyhow::Context;
use axum::Router;
use axum::routing::{get, patch, post};
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

use crate::routes::{
    apply_ops, chat, complete_upload, create_project, delete_project, get_project, health,
    list_ai_providers, list_media, list_projects, patch_media, put_media_bytes, register_media,
    request_upload, transcribe_media,
    update_project,
};
use crate::state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let state = AppState::connect().await.context("connect dependencies")?;
    oc_db::migrate(&state.db)
        .await
        .context("run migrations")?;

    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/projects", get(list_projects).post(create_project))
        .route(
            "/v1/projects/{id}",
            get(get_project).patch(update_project).delete(delete_project),
        )
        .route("/v1/projects/{id}/ops", post(apply_ops))
        .route("/v1/ai/providers", get(list_ai_providers))
        .route("/v1/projects/{id}/chat", post(chat))
        .route("/v1/projects/{id}/media", get(list_media).post(register_media))
        .route("/v1/projects/{id}/media/upload", post(request_upload))
        .route(
            "/v1/projects/{id}/media/{media_id}",
            patch(patch_media),
        )
        .route(
            "/v1/projects/{id}/media/{media_id}/bytes",
            axum::routing::put(put_media_bytes),
        )
        .route(
            "/v1/projects/{id}/media/{media_id}/complete",
            post(complete_upload),
        )
        .route(
            "/v1/projects/{id}/media/{media_id}/transcribe",
            post(transcribe_media),
        )
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let bind = std::env::var("API_BIND").unwrap_or_else(|_| "0.0.0.0:8787".into());
    let addr: SocketAddr = bind.parse().context("API_BIND")?;
    tracing::info!("api listening on {addr}");
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
