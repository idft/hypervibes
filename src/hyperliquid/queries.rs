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
    /// Cumulative net USDC flow as of this event, computed across the
    /// account's full history and anchored at 0 at the first journaled
    /// event. This is realized cash flow only — unrealized PnL is not
    /// included.
    pub running_balance: Option<Decimal>,
}

/// Return the most recent USDC balance-impacting events for an account,
/// with a running cumulative `usdc_delta` total that is computed over the
/// account's full history (so the value on each row equals the realized
/// cash-flow balance immediately after that event was applied) before
/// slicing the newest `limit` rows for display.
pub async fn list_account_transactions(
    pool: &DbPool,
    account_address: &str,
    environment: &str,
    limit: i64,
) -> Result<Vec<AccountTransactionRow>> {
    let rows = sqlx::query_as::<_, AccountTransactionRow>(
        "WITH ordered AS (
             SELECT event_id,
                    event_time,
                    event_category,
                    event_type,
                    source_stream,
                    symbol,
                    asset,
                    fee_usdc,
                    realized_pnl_usdc,
                    usdc_delta,
                    payload,
                    SUM(COALESCE(usdc_delta, 0))
                      OVER (ORDER BY event_time, event_id
                            ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW)
                      AS running_balance
               FROM hyperliquid.account_timeline
              WHERE account_address = $1
                AND environment = $2
         )
         SELECT event_id,
                event_time,
                event_category,
                event_type,
                source_stream,
                symbol,
                asset,
                fee_usdc,
                realized_pnl_usdc,
                usdc_delta,
                payload,
                running_balance
           FROM ordered
          ORDER BY event_time DESC, event_id DESC
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

/// Return all USDC balance-impacting events for an account, newest first,
/// with the same running-balance calculation as [`list_account_transactions`].
pub async fn list_all_account_transactions(
    pool: &DbPool,
    account_address: &str,
    environment: &str,
) -> Result<Vec<AccountTransactionRow>> {
    let rows = sqlx::query_as::<_, AccountTransactionRow>(
        "WITH ordered AS (
             SELECT event_id,
                    event_time,
                    event_category,
                    event_type,
                    source_stream,
                    symbol,
                    asset,
                    fee_usdc,
                    realized_pnl_usdc,
                    usdc_delta,
                    payload,
                    SUM(COALESCE(usdc_delta, 0))
                      OVER (ORDER BY event_time, event_id
                            ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW)
                      AS running_balance
               FROM hyperliquid.account_timeline
              WHERE account_address = $1
                AND environment = $2
         )
         SELECT event_id,
                event_time,
                event_category,
                event_type,
                source_stream,
                symbol,
                asset,
                fee_usdc,
                realized_pnl_usdc,
                usdc_delta,
                payload,
                running_balance
           FROM ordered
          ORDER BY event_time DESC, event_id DESC",
    )
    .bind(account_address)
    .bind(environment)
    .fetch_all(pool)
    .await
    .context("failed to list all account transactions")?;

    Ok(rows)
}

/// A single point on a balance-history series. `bucket` is the truncated
/// time-bucket boundary and `balance` is the cumulative net USDC flow
/// closing value for that bucket.
#[derive(Debug, Clone, sqlx::FromRow)]
#[allow(dead_code)]
pub struct BalancePoint {
    pub bucket: DateTime<Utc>,
    pub balance: Decimal,
}

/// Whitelisted bucket units for [`fetch_balance_series`]. Passing any
/// other string causes the function to return an error — the value is
/// never interpolated into SQL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BalanceSeriesBucket {
    Hour,
    Day,
}

impl BalanceSeriesBucket {
    fn as_str(self) -> &'static str {
        match self {
            BalanceSeriesBucket::Hour => "hour",
            BalanceSeriesBucket::Day => "day",
        }
    }
}

/// Fetch a time-bucketed historical balance series for an account.
///
/// The series is the running cumulative net USDC flow over the account's
/// full history, sampled at the closing value of each `bucket_unit`-
/// truncated bucket from `since` to the present. The result always
/// includes a single anchor point at `since` carrying the running balance
/// as of the latest event strictly before `since`, so a window with no
/// events in the trailing range still renders a flat line at the correct
/// starting level. Output is ascending by `bucket`.
pub async fn fetch_balance_series(
    pool: &DbPool,
    account_address: &str,
    environment: &str,
    since: DateTime<Utc>,
    bucket_unit: BalanceSeriesBucket,
) -> Result<Vec<BalancePoint>> {
    let rows = sqlx::query_as::<_, BalancePoint>(
        "WITH ordered AS (
             SELECT event_time,
                    SUM(COALESCE(usdc_delta, 0))
                      OVER (ORDER BY event_time, event_id
                            ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW)
                      AS running_balance
               FROM hyperliquid.account_timeline
              WHERE account_address = $1
                AND environment = $2
         ),
         anchor AS (
             SELECT $3::timestamptz AS bucket, running_balance AS balance
               FROM ordered
              WHERE event_time < $3
              ORDER BY event_time DESC
              LIMIT 1
         ),
         windowed AS (
             SELECT DISTINCT ON (date_trunc($4, event_time))
                    date_trunc($4, event_time) AS bucket,
                    running_balance AS balance
               FROM ordered
              WHERE event_time >= $3
              ORDER BY date_trunc($4, event_time), event_time DESC
         )
         SELECT bucket, balance FROM anchor
         UNION ALL
         SELECT bucket, balance FROM windowed
         ORDER BY bucket",
    )
    .bind(account_address)
    .bind(environment)
    .bind(since)
    .bind(bucket_unit.as_str())
    .fetch_all(pool)
    .await
    .context("failed to fetch balance series")?;

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
    use crate::test_db;
    use chrono::Duration;

    /// Insert the minimum parent rows a journal event needs. Returns the
    /// shared `instrument_id` that the test's events can reference.
    async fn seed_instrument(pool: &DbPool, suffix: &str) -> String {
        let id = format!("inst-{suffix}");
        sqlx::query(
            "INSERT INTO hyperliquid.instruments
                (instrument_id, name, market_type, base_asset, quote_asset,
                 settlement_asset, asset_index, price_decimals, size_decimals,
                 lot_size, max_leverage, is_hip3, active, created_at, updated_at)
             VALUES ($1, $1, 'perp', 'BASE', 'USDC', 'USDC', 0, 2, 4,
                     0.0001, 50, false, true, NOW(), NOW())",
        )
        .bind(&id)
        .execute(pool)
        .await
        .expect("insert instrument");
        id
    }

    /// Insert a single ledger event with a deterministic identity. Ledger
    /// events have a nullable `instrument_id`, so they are the easiest way
    /// to seed account_timeline rows in isolation.
    async fn seed_ledger_event(
        pool: &DbPool,
        hash: &str,
        account: &str,
        event_time: DateTime<Utc>,
        usdc: Decimal,
    ) {
        sqlx::query(
            "INSERT INTO hyperliquid.ledger_events
                (hash, account_address, environment, event_time, event_type,
                 source_stream, ledger_type, usdc, ingest_source, inserted_at)
             VALUES ($1, $2, 'live', $3, 'ledger', 'test', 'deposit', $4, 'test', NOW())",
        )
        .bind(hash)
        .bind(account)
        .bind(event_time)
        .bind(usdc)
        .execute(pool)
        .await
        .expect("insert ledger event");
    }

    #[tokio::test]
    async fn list_account_sync_state_returns_rows_for_account() {
        let pool = test_db::pool().await;

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

    #[tokio::test]
    async fn running_balance_accumulates_in_event_time_order() {
        let pool = test_db::pool().await;
        let suffix = format!(
            "rb-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        let account = format!("0x{suffix}");
        let _ = seed_instrument(&pool, &suffix).await;

        // Three events at distinct, ordered timestamps with deltas
        // 100, -25, 7.5 (cumulative: 100, 75, 82.5).
        let t0 = Utc::now() - Duration::hours(3);
        let t1 = Utc::now() - Duration::hours(2);
        let t2 = Utc::now() - Duration::hours(1);
        seed_ledger_event(
            &pool,
            &format!("{suffix}-0"),
            &account,
            t0,
            Decimal::new(100, 0),
        )
        .await;
        seed_ledger_event(
            &pool,
            &format!("{suffix}-1"),
            &account,
            t1,
            Decimal::new(-25, 0),
        )
        .await;
        seed_ledger_event(
            &pool,
            &format!("{suffix}-2"),
            &account,
            t2,
            Decimal::new(75, 1),
        )
        .await;

        // Pass a high limit so all three rows are returned.
        let rows = list_account_transactions(&pool, &account, "live", 100)
            .await
            .expect("list transactions");
        assert_eq!(rows.len(), 3);

        // The query returns rows in event_time DESC order, so the
        // most-recent event (t2) is first; t0 is last.
        assert!(rows[0].event_time > rows[1].event_time);
        assert!(rows[1].event_time > rows[2].event_time);

        // The newest row's running balance is the cumulative total
        // across all events in this account.
        assert_eq!(rows[0].running_balance, Some(Decimal::new(825, 1)));
        // The middle row's running balance is the cumulative total
        // through t1.
        assert_eq!(rows[1].running_balance, Some(Decimal::new(75, 0)));
        // The oldest row's running balance equals the first delta.
        assert_eq!(rows[0].running_balance, Some(Decimal::new(825, 1)));
        assert_eq!(rows[2].running_balance, Some(Decimal::new(100, 0)));
    }

    #[tokio::test]
    async fn fetch_balance_series_buckets_and_anchors() {
        let pool = test_db::pool().await;
        let suffix = format!(
            "bs-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        let account = format!("0x{suffix}");
        let _ = seed_instrument(&pool, &suffix).await;

        // Anchor: one event well before the window.
        let anchor_time = Utc::now() - Duration::days(10);
        seed_ledger_event(
            &pool,
            &format!("{suffix}-anchor"),
            &account,
            anchor_time,
            Decimal::new(50, 0),
        )
        .await;

        // Inside a 24h window: three events at distinct hour buckets with
        // deltas +5, -2, +3 (cumulative: 55, 53, 56).
        let base = Utc::now() - Duration::hours(20);
        for (i, delta) in [Decimal::new(5, 0), Decimal::new(-2, 0), Decimal::new(3, 0)]
            .into_iter()
            .enumerate()
        {
            let ts = base + Duration::hours(i as i64 * 2);
            seed_ledger_event(&pool, &format!("{suffix}-win-{i}"), &account, ts, delta).await;
        }

        let since = truncate_to_micros(Utc::now() - Duration::hours(24));
        let series =
            fetch_balance_series(&pool, &account, "live", since, BalanceSeriesBucket::Hour)
                .await
                .expect("fetch hourly series");

        // First point is the anchor at the `since` boundary.
        assert_eq!(series.len(), 4);
        assert_eq!(series[0].bucket, since);
        assert_eq!(series[0].balance, Decimal::new(50, 0));

        // Subsequent points are ascending by bucket and represent the
        // closing running balance for each hour bucket.
        for w in series.windows(2) {
            assert!(w[0].bucket <= w[1].bucket);
        }

        // Last point equals the cumulative total across all seeded
        // events: 50 + 5 - 2 + 3 = 56.
        assert_eq!(
            series.last().expect("non-empty").balance,
            Decimal::new(56, 0)
        );

        // Daily anchor test: with the same data and a 30d window, only
        // the anchor point is present (the in-window events are within
        // the trailing 24h, so a 30d daily bucket still covers them).
        let series_day = fetch_balance_series(
            &pool,
            &account,
            "live",
            Utc::now() - Duration::days(30),
            BalanceSeriesBucket::Day,
        )
        .await
        .expect("fetch daily series");

        // Anchor at the 30d boundary, then one day bucket (today) with
        // the closing balance, plus possibly a second day bucket if the
        // anchor event was in a previous UTC day.
        assert!(!series_day.is_empty());
        assert_eq!(series_day[0].balance, Decimal::new(50, 0));
        assert_eq!(
            series_day.last().expect("non-empty").balance,
            Decimal::new(56, 0)
        );

        // Anchor in an empty trailing window: a query with `since` set
        // to a point after all events yields only the anchor point.
        let future_since = truncate_to_micros(Utc::now() + Duration::hours(1));
        let series_empty = fetch_balance_series(
            &pool,
            &account,
            "live",
            future_since,
            BalanceSeriesBucket::Hour,
        )
        .await
        .expect("fetch series with future since");

        // With no events at or after `future_since` and no events
        // strictly before it (since the latest event is also after
        // the original `since` but still before `future_since`), the
        // anchor query will select the most-recent event. The series
        // therefore contains exactly the anchor row.
        assert_eq!(series_empty.len(), 1);
        assert_eq!(series_empty[0].bucket, future_since);
        assert_eq!(series_empty[0].balance, Decimal::new(56, 0));
    }

    /// PostgreSQL `timestamptz` only has microsecond precision, so any
    /// nanosecond-precision `DateTime<Utc>` we pass through it is
    /// truncated on round-trip. Helpers below let tests bind values
    /// that compare equal after the round-trip.
    fn truncate_to_micros(at: DateTime<Utc>) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp_micros(at.timestamp_micros())
            .expect("timestamp_micros in range")
    }
}
