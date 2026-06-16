use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::Value;

use crate::{db::DbPool, hyperliquid::sync_state::SyncStateRow};

/// A single USDC balance-impacting event from the account timeline.
#[derive(Debug, Clone, sqlx::FromRow)]
#[allow(dead_code)]
pub struct AccountTransactionRow {
    pub event_id: String,
    pub event_time: DateTime<Utc>,
    pub event_category: String,
    pub event_type: String,
    pub source_stream: String,
    pub symbol: Option<String>,
    pub asset: Option<String>,
    pub fee_usdc: Option<Decimal>,
    pub realized_pnl_usdc: Option<Decimal>,
    pub usdc_delta: Option<Decimal>,
    pub payload: Value,
}

/// Return the most recent USDC balance-impacting events for an account.
pub async fn list_account_transactions(
    pool: &DbPool,
    account_address: &str,
    environment: &str,
    limit: i64,
) -> Result<Vec<AccountTransactionRow>> {
    let rows = sqlx::query_as::<_, AccountTransactionRow>(
        "SELECT event_id,
                event_time,
                event_category,
                event_type,
                source_stream,
                symbol,
                asset,
                fee_usdc,
                realized_pnl_usdc,
                usdc_delta,
                payload
           FROM hyperliquid.account_timeline
          WHERE account_address = $1
            AND environment = $2
          ORDER BY event_time DESC
          LIMIT $3",
    )
    .bind(account_address)
    .bind(environment)
    .bind(limit)
    .fetch_all(pool)
    .await
    .context("failed to list account transactions")?;

    Ok(rows)
}

/// Return sync_state rows for an account/environment.
pub async fn list_account_sync_state(
    pool: &DbPool,
    account_address: &str,
    environment: &str,
) -> Result<Vec<SyncStateRow>> {
    let rows = sqlx::query_as::<_, SyncStateRow>(
        "SELECT account_address,
                environment,
                stream_name,
                last_event_time,
                last_event_key,
                last_synced_at,
                status,
                metadata
           FROM hyperliquid.sync_state
          WHERE account_address = $1
            AND environment = $2
          ORDER BY stream_name",
    )
    .bind(account_address)
    .bind(environment)
    .fetch_all(pool)
    .await
    .context("failed to list account sync state")?;

    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{connect, migrate};

    fn db_url() -> Option<String> {
        std::env::var("DATABASE_URL").ok()
    }

    #[tokio::test]
    async fn list_account_sync_state_returns_rows_for_account() {
        let Some(database_url) = db_url() else {
            eprintln!("DATABASE_URL not set; skipping integration test");
            return;
        };

        let pool = connect(&database_url).await.expect("connect to database");
        migrate(&pool).await.expect("run migrations");

        let account = format!("0xqueries{}", chrono::Utc::now().timestamp_millis());
        sqlx::query(
            "INSERT INTO hyperliquid.sync_state (account_address, environment, stream_name, status, metadata) VALUES ($1, 'live', 'fills', $2, '{}')",
        )
        .bind(&account)
        .bind(crate::hyperliquid::sync_state::SyncStatus::Healthy.as_str())
        .execute(&pool)
        .await
        .expect("insert sync state");

        let rows = list_account_sync_state(&pool, &account, "live")
            .await
            .expect("list sync state");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].stream_name, "fills");
    }
}
