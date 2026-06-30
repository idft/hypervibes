use std::{env, ops::Deref, process, str::FromStr};

use futures::{future::BoxFuture, stream::BoxStream};
use sqlx::{
    Database, Describe, Either, Error, Execute, Executor, PgPool, Postgres,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use tokio::runtime::Builder;
use tokio::sync::OnceCell;
use uuid::Uuid;

static STALE_DB_CLEANUP: OnceCell<()> = OnceCell::const_new();

#[derive(Debug)]
pub struct TestDb {
    pool: PgPool,
    admin_options: PgConnectOptions,
    database_name: String,
}

impl Deref for TestDb {
    type Target = PgPool;

    fn deref(&self) -> &Self::Target {
        &self.pool
    }
}

impl AsRef<PgPool> for TestDb {
    fn as_ref(&self) -> &PgPool {
        &self.pool
    }
}

impl<'p> Executor<'p> for &'_ TestDb {
    type Database = Postgres;

    fn fetch_many<'e, 'q: 'e, E>(
        self,
        query: E,
    ) -> BoxStream<
        'e,
        Result<
            Either<<Self::Database as Database>::QueryResult, <Self::Database as Database>::Row>,
            Error,
        >,
    >
    where
        E: 'q + Execute<'q, Self::Database>,
    {
        (&self.pool).fetch_many(query)
    }

    fn fetch_optional<'e, 'q: 'e, E>(
        self,
        query: E,
    ) -> BoxFuture<'e, Result<Option<<Self::Database as Database>::Row>, Error>>
    where
        E: 'q + Execute<'q, Self::Database>,
    {
        (&self.pool).fetch_optional(query)
    }

    fn prepare_with<'e, 'q: 'e>(
        self,
        sql: &'q str,
        parameters: &'e [<Self::Database as Database>::TypeInfo],
    ) -> BoxFuture<'e, Result<<Self::Database as Database>::Statement<'q>, Error>> {
        (&self.pool).prepare_with(sql, parameters)
    }

    fn describe<'e, 'q: 'e>(
        self,
        sql: &'q str,
    ) -> BoxFuture<'e, Result<Describe<Self::Database>, Error>> {
        (&self.pool).describe(sql)
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        let pool = self.pool.clone();
        let admin_options = self.admin_options.clone();
        let database_name = self.database_name.clone();

        std::thread::spawn(move || {
            let runtime = Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build test db cleanup runtime");
            runtime.block_on(async move {
                drop_database(admin_options, database_name, Some(pool)).await;
            });
        });
    }
}

pub async fn pool() -> TestDb {
    let url = env::var("TEST_DATABASE_URL")
        .expect("TEST_DATABASE_URL must be set for tests; use the dedicated test-postgres service");
    let base_options = PgConnectOptions::from_str(&url).expect("parse test database url");

    STALE_DB_CLEANUP
        .get_or_init(|| async {
            cleanup_stale_test_databases(base_options.clone()).await;
        })
        .await;

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
        .connect_with(base_options.clone().database(&database_name))
        .await
        .expect("connect to test database");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("run migrations on test database");

    TestDb {
        pool,
        admin_options: base_options,
        database_name,
    }
}

async fn cleanup_stale_test_databases(base_options: PgConnectOptions) {
    if let Err(error) = drop_stale_test_databases(base_options).await {
        eprintln!("warning: failed to drop stale test databases: {error}");
    }
}

async fn drop_stale_test_databases(base_options: PgConnectOptions) -> Result<(), sqlx::Error> {
    let admin_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(base_options.clone())
        .await?;

    let database_names: Vec<(String,)> = sqlx::query_as(
        "SELECT datname
           FROM pg_database
          WHERE datname LIKE 'vt_test_%'",
    )
    .fetch_all(&admin_pool)
    .await?;

    for (database_name,) in database_names {
        sqlx::query(&format!(
            "DROP DATABASE IF EXISTS \"{database_name}\" WITH (FORCE)"
        ))
        .execute(&admin_pool)
        .await?;
    }

    admin_pool.close().await;
    Ok(())
}

async fn drop_database(
    admin_options: PgConnectOptions,
    database_name: String,
    pool: Option<PgPool>,
) {
    if let Some(pool) = pool {
        pool.close().await;
    }

    let Ok(admin_pool) = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(admin_options)
        .await
    else {
        return;
    };

    let _ = sqlx::query(&format!(
        "DROP DATABASE IF EXISTS \"{database_name}\" WITH (FORCE)"
    ))
    .execute(&admin_pool)
    .await;
    admin_pool.close().await;
}
