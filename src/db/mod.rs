use anyhow::{Context, Result};
use sqlx::{Pool, Postgres, postgres::PgPoolOptions};

pub type DbPool = Pool<Postgres>;

pub async fn connect(database_url: &str) -> Result<DbPool> {
    PgPoolOptions::new()
        .max_connections(10)
        .connect(database_url)
        .await
        .with_context(|| "failed to connect to postgres")
}

pub async fn migrate(pool: &DbPool) -> Result<()> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .with_context(|| "failed to run sqlx migrations")
}
