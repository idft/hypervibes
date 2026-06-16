use std::sync::{Arc, OnceLock};

use pglite_oxide::PgliteServer;
use sqlx::{PgPool, postgres::PgPoolOptions};

static SERVER: OnceLock<Arc<PgliteServer>> = OnceLock::new();

fn server() -> &'static Arc<PgliteServer> {
    SERVER.get_or_init(|| {
        Arc::new(
            PgliteServer::temporary_tcp()
                .expect("failed to start embedded pglite test server"),
        )
    })
}

pub async fn pool() -> PgPool {
    let url = server().database_url();
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .expect("connect to embedded pglite test server");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("run migrations on embedded pglite test server");
    pool
}
