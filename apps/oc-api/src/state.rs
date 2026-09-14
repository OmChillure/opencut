use oc_db::Db;
use oc_db::R2;

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub r2: Option<R2>,
}

impl AppState {
    pub async fn connect() -> anyhow::Result<Self> {
        let db = oc_db::connect().await?;
        let r2 = match R2::from_env().await {
            Ok(r2) => Some(r2),
            Err(err) => {
                tracing::warn!("R2 disabled: {err}");
                None
            }
        };
        Ok(Self { db, r2 })
    }
}
