use std::{env, process, str::FromStr};

use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use uuid::Uuid;

pub async fn pool() -> PgPool {
    let url = env::var("TEST_DATABASE_URL")
        .expect("TEST_DATABASE_URL must be set for tests; use the dedicated test-postgres service");
    let base_options = PgConnectOptions::from_str(&url).expect("parse test database url");
    let database_name = format!("vt_test_{}_{}", process::id(), Uuid::new_v4().simple());

    let admin_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(base_options.clone())
        .await
        .expect("connect to test postgres admin database");
    sqlx::query(&format!("CREATE DATABASE \"{database_name}\""))
        .execute(&admin_pool)
        .await
        .expect("create test database");
    admin_pool.close().await;

    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect_with(base_options.database(&database_name))
        .await
        .expect("connect to test database");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("run migrations on test database");
    pool
}
