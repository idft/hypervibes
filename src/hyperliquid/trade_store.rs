use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use sqlx::AssertSqlSafe;
use uuid::Uuid;

use crate::{
    db::DbPool,
    hyperliquid::trade_cycles::{CycleFill, build_cycles},
};

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct TradeRow {
    pub id: Uuid,
    pub instrument_id: String,
    pub symbol: String,
    pub direction: String,
    pub status: String,
    pub opened_at: DateTime<Utc>,
    pub last_activity_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
    pub remaining_size: Decimal,
    pub entry_size: Decimal,
    pub exit_size: Decimal,
    pub entry_notional: Decimal,
    pub exit_notional: Decimal,
    pub gross_pnl: Decimal,
    pub usdc_fees: Decimal,
    pub net_pnl: Decimal,
    /// Signed imported funding over [opened_at, closed_at), or through now
    /// for open cycles. Unknown when the cycle's position history is incomplete.
    pub funding_usdc: Option<Decimal>,
    pub fee_coverage: bool,
    pub coverage_reason: Option<String>,
    pub note_count: i64,
}

impl TradeRow {
    pub fn entry_price(&self) -> Option<Decimal> {
        (self.entry_size > Decimal::ZERO).then(|| self.entry_notional / self.entry_size)
    }
    pub fn exit_price(&self) -> Option<Decimal> {
        (self.exit_size > Decimal::ZERO).then(|| self.exit_notional / self.exit_size)
    }
}

const TRADE_SELECT: &str = "SELECT c.id, c.instrument_id, i.name AS symbol, c.direction,
    c.status, c.opened_at, c.last_activity_at, c.closed_at, c.remaining_size,
    c.entry_size, c.exit_size, c.entry_notional, c.exit_notional, c.gross_pnl,
    c.usdc_fees, c.net_pnl, c.fee_coverage, c.coverage_reason,
    CASE WHEN c.status IN ('open', 'closed') THEN
        (SELECT COALESCE(SUM(f.usdc), 0) FROM hyperliquid.funding_events f
         WHERE f.account_address=c.account_address AND f.environment=c.environment
         AND f.instrument_id=c.instrument_id AND f.event_time >= c.opened_at
         AND f.event_time < COALESCE(c.closed_at, CURRENT_TIMESTAMP))
    END AS funding_usdc,
    (SELECT COUNT(*) FROM hyperliquid.journal_notes n WHERE n.trade_id = c.id) AS note_count
    FROM hyperliquid.trade_cycles c JOIN hyperliquid.instruments i USING(instrument_id)";

pub async fn count_trades(pool: &DbPool, account: &str, environment: &str) -> Result<i64> {
    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM hyperliquid.trade_cycles
        WHERE account_address=$1 AND environment=$2 AND status <> 'superseded'",
    )
    .bind(account)
    .bind(environment)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

pub async fn list_trades(
    pool: &DbPool,
    account: &str,
    environment: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<TradeRow>> {
    let sql = format!("{TRADE_SELECT} WHERE c.account_address=$1 AND c.environment=$2
        AND c.status <> 'superseded' ORDER BY c.last_activity_at DESC, c.id DESC LIMIT $3 OFFSET $4");
    Ok(sqlx::query_as(AssertSqlSafe(sql))
        .bind(account)
        .bind(environment)
        .bind(limit.clamp(1, 100))
        .bind(offset.max(0))
        .fetch_all(pool)
        .await?)
}

pub async fn get_trade(
    pool: &DbPool,
    account: &str,
    environment: &str,
    id: Uuid,
) -> Result<Option<TradeRow>> {
    let sql = format!("{TRADE_SELECT} WHERE c.account_address=$1 AND c.environment=$2 AND c.id=$3");
    Ok(sqlx::query_as(AssertSqlSafe(sql))
        .bind(account)
        .bind(environment)
        .bind(id)
        .fetch_optional(pool)
        .await?)
}

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct TradeFillRow {
    pub hash: String,
    pub trade_id: String,
    pub segment: String,
    pub fill_time: DateTime<Utc>,
    pub side: String,
    pub price: Decimal,
    pub quantity: Decimal,
    pub fee_usdc: Option<Decimal>,
    pub realized_pnl_usdc: Decimal,
}

pub async fn trade_fills(
    pool: &DbPool,
    account: &str,
    environment: &str,
    id: Uuid,
) -> Result<Vec<TradeFillRow>> {
    Ok(sqlx::query_as(
        "SELECT p.hash, p.trade_id, p.segment, f.fill_time, f.side, f.price,
        p.quantity, p.fee_usdc, p.realized_pnl_usdc
        FROM hyperliquid.trade_cycle_fills p JOIN hyperliquid.trade_cycles c ON c.id=p.cycle_id
        JOIN hyperliquid.trade_fills f ON f.hash=p.hash AND f.trade_id=p.trade_id
        WHERE c.account_address=$1 AND c.environment=$2 AND c.id=$3
        ORDER BY f.fill_time, f.trade_id, f.hash, p.segment",
    )
    .bind(account)
    .bind(environment)
    .bind(id)
    .fetch_all(pool)
    .await?)
}

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct JournalNote {
    pub id: Uuid,
    pub target_kind: String,
    pub trade_id: Option<Uuid>,
    pub event_id: Option<String>,
    pub author_kind: String,
    pub author_id: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
}

pub async fn list_notes(
    pool: &DbPool,
    account: &str,
    environment: &str,
    target_kind: &str,
    target_id: &str,
) -> Result<Vec<JournalNote>> {
    let trade_id = if target_kind == "trade" {
        Some(Uuid::parse_str(target_id)?)
    } else {
        None
    };
    let event_id = if target_kind == "trade" {
        None
    } else {
        Some(target_id)
    };
    Ok(sqlx::query_as(
        "SELECT id, target_kind, trade_id, event_id, author_kind, author_id, body, created_at
        FROM hyperliquid.journal_notes WHERE account_address=$1 AND environment=$2
        AND target_kind=$3 AND trade_id IS NOT DISTINCT FROM $4 AND event_id IS NOT DISTINCT FROM $5
        ORDER BY created_at, id LIMIT 500",
    )
    .bind(account)
    .bind(environment)
    .bind(target_kind)
    .bind(trade_id)
    .bind(event_id)
    .fetch_all(pool)
    .await?)
}

pub async fn target_exists(
    pool: &DbPool,
    account: &str,
    environment: &str,
    kind: &str,
    id: &str,
) -> Result<bool> {
    if kind == "trade" {
        let Ok(uuid) = Uuid::parse_str(id) else {
            return Ok(false);
        };
        return Ok(get_trade(pool, account, environment, uuid).await?.is_some());
    }
    let (exists,): (bool,) = sqlx::query_as(
        "SELECT EXISTS(SELECT 1 FROM hyperliquid.account_timeline
        WHERE account_address=$1 AND environment=$2 AND event_category=$3 AND event_id=$4)",
    )
    .bind(account)
    .bind(environment)
    .bind(kind)
    .bind(id)
    .fetch_one(pool)
    .await?;
    Ok(exists)
}

pub struct JournalNoteInput<'a> {
    pub account: &'a str,
    pub environment: &'a str,
    pub kind: &'a str,
    pub target: &'a str,
    pub author_kind: &'a str,
    pub author_id: &'a str,
    pub body: &'a str,
    pub source_run_id: Option<i64>,
    pub source_conversation_id: Option<Uuid>,
}

pub async fn add_note(pool: &DbPool, input: JournalNoteInput<'_>) -> Result<Option<JournalNote>> {
    let JournalNoteInput {
        account,
        environment,
        kind,
        target,
        author_kind,
        author_id,
        body,
        source_run_id,
        source_conversation_id,
    } = input;
    if !matches!(kind, "trade" | "fill" | "funding" | "ledger")
        || body.trim().is_empty()
        || body.len() > 4000
        || !target_exists(pool, account, environment, kind, target).await?
    {
        return Ok(None);
    }
    let trade_id = if kind == "trade" {
        Some(Uuid::parse_str(target)?)
    } else {
        None
    };
    let event_id = if kind == "trade" { None } else { Some(target) };
    let row = sqlx::query_as("INSERT INTO hyperliquid.journal_notes
        (id,account_address,environment,target_kind,trade_id,event_id,author_kind,author_id,body,source_run_id,source_conversation_id)
        VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
        RETURNING id,target_kind,trade_id,event_id,author_kind,author_id,body,created_at")
        .bind(Uuid::new_v4()).bind(account).bind(environment).bind(kind).bind(trade_id)
        .bind(event_id).bind(author_kind).bind(author_id).bind(body.trim())
        .bind(source_run_id).bind(source_conversation_id).fetch_one(pool).await?;
    Ok(Some(row))
}

/// Claim one dirty instrument at a time. The row lock and all projection writes
/// commit together; a crash leaves the dirty marker available for retry.
pub async fn rebuild_dirty(pool: &DbPool, account: &str, environment: &str) -> Result<()> {
    loop {
        let mut tx = pool.begin().await?;
        let instrument: Option<(String,)> = sqlx::query_as(
            "SELECT instrument_id FROM hyperliquid.trade_projection_dirty
            WHERE account_address=$1 AND environment=$2 ORDER BY changed_at, instrument_id
            LIMIT 1 FOR UPDATE SKIP LOCKED",
        )
        .bind(account)
        .bind(environment)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((instrument,)) = instrument else {
            tx.commit().await?;
            break;
        };
        let (market_type,): (String,) = sqlx::query_as(
            "SELECT market_type FROM hyperliquid.instruments WHERE instrument_id=$1",
        )
        .bind(&instrument)
        .fetch_one(&mut *tx)
        .await?;
        if market_type != "perp" {
            sqlx::query(
                "DELETE FROM hyperliquid.trade_projection_dirty WHERE account_address=$1
                AND environment=$2 AND instrument_id=$3",
            )
            .bind(account)
            .bind(environment)
            .bind(&instrument)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            continue;
        }
        let mut fills: Vec<CycleFill> = Vec::new();
        loop {
            let batch: Vec<CycleFill> = sqlx::query_as(
                "SELECT hash, trade_id, fill_time, side,
                size, price, start_position, fee, fee_token, realized_pnl_usdc
                FROM hyperliquid.trade_fills WHERE account_address=$1 AND environment=$2
                AND instrument_id=$3 ORDER BY fill_time, trade_id, hash LIMIT 500 OFFSET $4",
            )
            .bind(account)
            .bind(environment)
            .bind(&instrument)
            .bind(fills.len() as i64)
            .fetch_all(&mut *tx)
            .await?;
            let done = batch.len() < 500;
            fills.extend(batch);
            if done {
                break;
            }
        }
        let cycles = build_cycles(account, environment, &instrument, &mut fills)
            .context("failed to build trade cycles")?;
        // Preserve old IDs (and their notes) even when a late fill changes a
        // cycle boundary. Such cycles remain superseded for human review.
        sqlx::query("UPDATE hyperliquid.trade_cycles SET status='superseded', updated_at=now()
            WHERE account_address=$1 AND environment=$2 AND instrument_id=$3 AND status <> 'superseded'")
            .bind(account).bind(environment).bind(&instrument).execute(&mut *tx).await?;
        for cycle in cycles {
            sqlx::query("INSERT INTO hyperliquid.trade_cycles
                (id,account_address,environment,instrument_id,opening_hash,opening_trade_id,opening_segment,
                 direction,status,opened_at,last_activity_at,closed_at,remaining_size,entry_size,exit_size,
                 entry_notional,exit_notional,gross_pnl,usdc_fees,net_pnl,fee_coverage,coverage_reason)
                VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22)
                ON CONFLICT (id) DO UPDATE SET status=EXCLUDED.status,last_activity_at=EXCLUDED.last_activity_at,
                closed_at=EXCLUDED.closed_at,remaining_size=EXCLUDED.remaining_size,entry_size=EXCLUDED.entry_size,
                exit_size=EXCLUDED.exit_size,entry_notional=EXCLUDED.entry_notional,exit_notional=EXCLUDED.exit_notional,
                gross_pnl=EXCLUDED.gross_pnl,usdc_fees=EXCLUDED.usdc_fees,net_pnl=EXCLUDED.net_pnl,
                fee_coverage=EXCLUDED.fee_coverage,coverage_reason=EXCLUDED.coverage_reason,
                revision=hyperliquid.trade_cycles.revision+1,updated_at=now()")
                .bind(cycle.id).bind(account).bind(environment).bind(&instrument)
                .bind(&cycle.opening_hash).bind(&cycle.opening_trade_id).bind(cycle.opening_segment)
                .bind(cycle.direction).bind(cycle.status).bind(cycle.opened_at).bind(cycle.last_activity_at)
                .bind(cycle.closed_at).bind(cycle.remaining_size).bind(cycle.entry_size).bind(cycle.exit_size)
                .bind(cycle.entry_notional).bind(cycle.exit_notional).bind(cycle.gross_pnl)
                .bind(cycle.usdc_fees).bind(cycle.net_pnl).bind(cycle.fee_coverage).bind(&cycle.coverage_reason)
                .execute(&mut *tx).await?;
            sqlx::query("DELETE FROM hyperliquid.trade_cycle_fills WHERE cycle_id=$1")
                .bind(cycle.id)
                .execute(&mut *tx)
                .await?;
            for part in cycle.parts {
                sqlx::query(
                    "INSERT INTO hyperliquid.trade_cycle_fills
                    (cycle_id,hash,trade_id,segment,quantity,fee_usdc,realized_pnl_usdc)
                    VALUES ($1,$2,$3,$4,$5,$6,$7)",
                )
                .bind(cycle.id)
                .bind(part.hash)
                .bind(part.trade_id)
                .bind(part.segment)
                .bind(part.quantity)
                .bind(part.fee_usdc)
                .bind(part.realized_pnl_usdc)
                .execute(&mut *tx)
                .await?;
            }
        }
        sqlx::query(
            "DELETE FROM hyperliquid.trade_projection_dirty WHERE account_address=$1
            AND environment=$2 AND instrument_id=$3",
        )
        .bind(account)
        .bind(environment)
        .bind(&instrument)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_db;

    #[tokio::test]
    async fn funding_follows_holding_intervals_and_refreshes_without_rebuilding() {
        use rust_decimal_macros::dec;

        let pool = test_db::pool().await;
        let account = "0xfunding-cycle-test";
        let other = "0xother-funding-account";
        for (key, address) in [
            ("funding-cycle-test", account),
            ("other-funding-test", other),
        ] {
            sqlx::query(
                "INSERT INTO agents(agent_key,user_id,created_at,updated_at,display_name,
                trading_account_address,environment,api_key,lifecycle)
                VALUES ($1,$2,now(),now(),'Funding test',$3,'live',$1,'active')",
            )
            .bind(key)
            .bind(test_db::test_user_id())
            .bind(address)
            .execute(&*pool)
            .await
            .expect("agent");
        }
        for instrument in ["FUNDING-PERP", "OTHER-PERP"] {
            sqlx::query("INSERT INTO hyperliquid.instruments(instrument_id,name,market_type,base_asset,
                quote_asset,settlement_asset,price_decimals,size_decimals,lot_size,is_hip3,active,created_at,updated_at)
                VALUES ($1,$1,'perp',$1,'USDC','USDC',2,2,0.01,false,true,now(),now())")
                .bind(instrument).execute(&*pool).await.expect("instrument");
        }
        let base = Utc::now() - chrono::Duration::hours(8);
        for (id, side, size, start, hour) in [
            ("1", "B", dec!(1), dec!(0), 0),
            ("2", "B", dec!(1), dec!(1), 1),
            ("3", "A", dec!(3), dec!(2), 2),
            ("4", "B", dec!(1), dec!(-1), 3),
            ("5", "B", dec!(1), dec!(0), 4),
        ] {
            sqlx::query("INSERT INTO hyperliquid.trade_fills(hash,trade_id,account_address,environment,
                event_time,source_stream,instrument_id,fill_time,direction,side,price,size,start_position,
                fee,fee_token,realized_pnl_usdc,ingest_source,inserted_at)
                VALUES ($1,$1,$2,'live',$3,'test','FUNDING-PERP',$3,$4,$4,100,$5,$6,0,'USDC',0,'test',now())")
                .bind(id).bind(account).bind(base + chrono::Duration::hours(hour))
                .bind(side).bind(size).bind(start).execute(&*pool).await.expect("fill");
        }
        rebuild_dirty(&pool, account, "live")
            .await
            .expect("projection");
        let before = list_trades(&pool, account, "live", 10, 0)
            .await
            .expect("trades");
        assert_eq!(before.len(), 3);
        assert!(
            before
                .iter()
                .all(|trade| trade.funding_usdc == Some(Decimal::ZERO))
        );

        for (address, instrument, time, amount) in [
            (
                account,
                "FUNDING-PERP",
                base - chrono::Duration::seconds(1),
                dec!(100),
            ),
            (account, "FUNDING-PERP", base, dec!(0.1)),
            (
                account,
                "FUNDING-PERP",
                base + chrono::Duration::hours(1),
                dec!(-0.03),
            ),
            (
                account,
                "FUNDING-PERP",
                base + chrono::Duration::hours(2),
                dec!(-0.02),
            ),
            (
                account,
                "FUNDING-PERP",
                base + chrono::Duration::hours(3),
                dec!(100),
            ),
            (
                account,
                "FUNDING-PERP",
                base + chrono::Duration::hours(4),
                dec!(0.04),
            ),
            (
                account,
                "FUNDING-PERP",
                base + chrono::Duration::hours(5),
                dec!(0.05),
            ),
            (
                account,
                "FUNDING-PERP",
                Utc::now() + chrono::Duration::hours(1),
                dec!(100),
            ),
            (
                account,
                "OTHER-PERP",
                base + chrono::Duration::hours(1),
                dec!(100),
            ),
            (
                other,
                "FUNDING-PERP",
                base + chrono::Duration::hours(1),
                dec!(100),
            ),
        ] {
            sqlx::query(
                "INSERT INTO hyperliquid.funding_events(account_address,environment,instrument_id,
                event_time,source_stream,usdc,ingest_source,inserted_at)
                VALUES ($1,'live',$2,$3,'test',$4,'test',now())",
            )
            .bind(address)
            .bind(instrument)
            .bind(time)
            .bind(amount)
            .execute(&*pool)
            .await
            .expect("funding");
        }
        let after = list_trades(&pool, account, "live", 10, 0)
            .await
            .expect("updated trades");
        for trade in &after {
            let expected = match (trade.direction.as_str(), trade.status.as_str()) {
                ("long", "closed") => dec!(0.07),
                ("short", "closed") => dec!(-0.02),
                ("long", "open") => dec!(0.09),
                _ => panic!("unexpected projected cycle"),
            };
            assert_eq!(trade.funding_usdc, Some(expected));
            assert_eq!(
                trade.net_pnl,
                Decimal::ZERO,
                "funding stays separate from trading P&L"
            );
            let detail = get_trade(&pool, account, "live", trade.id)
                .await
                .expect("detail")
                .expect("trade exists");
            assert_eq!(detail.funding_usdc, trade.funding_usdc);
        }
        sqlx::query("UPDATE hyperliquid.trade_cycles SET status='incomplete' WHERE id=$1")
            .bind(after[0].id)
            .execute(&*pool)
            .await
            .expect("incomplete cycle");
        let incomplete = get_trade(&pool, account, "live", after[0].id)
            .await
            .expect("detail")
            .expect("incomplete trade");
        assert_eq!(incomplete.funding_usdc, None);
    }

    #[tokio::test]
    async fn rebuild_is_idempotent_and_preserves_trade_notes() {
        let pool = test_db::pool().await;
        let account = "0xprojection-test";
        sqlx::query("INSERT INTO agents(agent_key,user_id,created_at,updated_at,display_name,
            trading_account_address,environment,api_key,lifecycle)
            VALUES ('projection-test',$1,now(),now(),'Projection test',$2,'live','projection-test-api','active')")
            .bind(test_db::test_user_id()).bind(account).execute(&*pool).await.expect("agent");
        sqlx::query("INSERT INTO hyperliquid.instruments(instrument_id,name,market_type,base_asset,quote_asset,
            settlement_asset,price_decimals,size_decimals,lot_size,is_hip3,active,created_at,updated_at)
            VALUES ('TEST-PERP','TEST-PERP','perp','TEST','USDC','USDC',2,2,0.01,false,true,now(),now())")
            .execute(&*pool).await.expect("instrument");
        for (hash, side, start, pnl) in [("open", "B", 0, 0), ("close", "A", 1, 5)] {
            sqlx::query("INSERT INTO hyperliquid.trade_fills(hash,trade_id,account_address,environment,event_time,
                source_stream,instrument_id,fill_time,direction,side,price,size,start_position,fee,fee_token,
                realized_pnl_usdc,ingest_source,inserted_at)
                VALUES ($1,$1,$2,'live',now() + CASE WHEN $3='A' THEN interval '1 second' ELSE interval '0 second' END,
                'test','TEST-PERP',now() + CASE WHEN $3='A' THEN interval '1 second' ELSE interval '0 second' END,
                $3,$4,100,1,$5,1,'USDC',$6,'test',now())")
                .bind(hash).bind(account).bind(side).bind(side).bind(Decimal::new(start,0))
                .bind(Decimal::new(pnl,0)).execute(&*pool).await.expect("fill");
        }
        rebuild_dirty(&pool, account, "live")
            .await
            .expect("initial projection");
        let trades = list_trades(&pool, account, "live", 10, 0)
            .await
            .expect("list");
        assert_eq!(trades.len(), 1);
        assert_eq!(trades[0].status, "closed");
        assert_eq!(trades[0].net_pnl, Decimal::new(3, 0));
        add_note(
            &pool,
            JournalNoteInput {
                account,
                environment: "live",
                kind: "trade",
                target: &trades[0].id.to_string(),
                author_kind: "human",
                author_id: "test-user",
                body: "Keep this note",
                source_run_id: None,
                source_conversation_id: None,
            },
        )
        .await
        .expect("add note")
        .expect("saved");
        sqlx::query("INSERT INTO hyperliquid.trade_projection_dirty(account_address,environment,instrument_id)
            VALUES ($1,'live','TEST-PERP')").bind(account).execute(&*pool).await.expect("dirty");
        rebuild_dirty(&pool, account, "live")
            .await
            .expect("rebuild");
        let again = list_trades(&pool, account, "live", 10, 0)
            .await
            .expect("list again");
        assert_eq!(trades[0].id, again[0].id);
        assert_eq!(
            list_notes(&pool, account, "live", "trade", &trades[0].id.to_string())
                .await
                .expect("notes")
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn migration_reverts_populated_journal_and_reapplies_over_existing_fills() {
        let pool = test_db::pool().await;
        let account = "0xrollback-test";
        sqlx::query("INSERT INTO agents(agent_key,user_id,created_at,updated_at,display_name,
            trading_account_address,environment,api_key,lifecycle)
            VALUES ('rollback-test',$1,now(),now(),'Rollback test',$2,'live','rollback-test-api','active')")
            .bind(test_db::test_user_id()).bind(account).execute(&*pool).await.expect("agent");
        sqlx::query("INSERT INTO hyperliquid.instruments(instrument_id,name,market_type,base_asset,quote_asset,
            settlement_asset,price_decimals,size_decimals,lot_size,is_hip3,active,created_at,updated_at)
            VALUES ('ROLLBACK-PERP','ROLLBACK-PERP','perp','RB','USDC','USDC',2,2,0.01,false,true,now(),now())")
            .execute(&*pool).await.expect("instrument");
        sqlx::query("INSERT INTO hyperliquid.trade_fills(hash,trade_id,account_address,environment,event_time,
            source_stream,instrument_id,fill_time,direction,side,price,size,start_position,fee,fee_token,
            realized_pnl_usdc,ingest_source,inserted_at)
            VALUES ('rollback-fill','1',$1,'live',now(),'test','ROLLBACK-PERP',now(),'Open Long','B',100,1,0,1,'USDC',0,'test',now())")
            .bind(account).execute(&*pool).await.expect("fill");
        sqlx::query("INSERT INTO hyperliquid.ledger_events(hash,account_address,environment,event_time,
            event_type,source_stream,ledger_type,token,amount,fee,details,ingest_source,inserted_at)
            VALUES ('rollback-send',$1,'live',now(),'send','test','send','USDC',7,0,$2,'test',now())")
            .bind(account).bind(serde_json::json!({"user":"0xother","destination":account}))
            .execute(&*pool).await.expect("send");
        let (delta,): (Option<Decimal>,) = sqlx::query_as(
            "SELECT usdc_delta FROM hyperliquid.account_timeline
            WHERE event_id='rollback-send'",
        )
        .fetch_one(&*pool)
        .await
        .expect("classified send");
        assert_eq!(delta, Some(Decimal::new(7, 0)));
        rebuild_dirty(&pool, account, "live")
            .await
            .expect("backfill");
        let trades = list_trades(&pool, account, "live", 10, 0)
            .await
            .expect("trades");
        assert_eq!(trades.len(), 1);

        let down = include_str!("../../migrations/0026_transaction_trade_journal.down.sql");
        let up = include_str!("../../migrations/0026_transaction_trade_journal.sql");
        add_note(
            &pool,
            JournalNoteInput {
                account,
                environment: "live",
                kind: "trade",
                target: &trades[0].id.to_string(),
                author_kind: "human",
                author_id: "test-user",
                body: "Protect this note",
                source_run_id: None,
                source_conversation_id: None,
            },
        )
        .await
        .expect("insert note")
        .expect("note");
        let conversation = Uuid::new_v4();
        sqlx::query("INSERT INTO agent_conversations(id,agent_key,opencode_session_id,title,model_provider_id,model_id)
            VALUES ($1,'rollback-test',$2,'Rollback test','test','test')")
            .bind(conversation).bind(format!("rollback-{conversation}"))
            .execute(&*pool).await.expect("conversation");
        sqlx::query(
            "INSERT INTO agent_conversation_tool_policies(conversation_id,tool_group,policy)
            VALUES ($1,'journal_writes','allow')",
        )
        .bind(conversation)
        .execute(&*pool)
        .await
        .expect("journal permission");
        sqlx::raw_sql(down)
            .execute(&*pool)
            .await
            .expect("revert 0026");
        let (cycles_table,): (Option<String>,) =
            sqlx::query_as("SELECT to_regclass('hyperliquid.trade_cycles')::text")
                .fetch_one(&*pool)
                .await
                .expect("table check");
        assert_eq!(cycles_table, None);
        let (notes_table,): (Option<String>,) =
            sqlx::query_as("SELECT to_regclass('hyperliquid.journal_notes')::text")
                .fetch_one(&*pool)
                .await
                .expect("notes table check");
        assert_eq!(notes_table, None);
        let (source_fills,): (i64,) =
            sqlx::query_as("SELECT count(*) FROM hyperliquid.trade_fills WHERE account_address=$1")
                .bind(account)
                .fetch_one(&*pool)
                .await
                .expect("fills preserved");
        assert_eq!(source_fills, 1);
        let (view_columns,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM information_schema.columns
            WHERE table_schema='hyperliquid' AND table_name='account_timeline'",
        )
        .fetch_one(&*pool)
        .await
        .expect("view columns");
        assert_eq!(view_columns, 16);
        assert!(
            sqlx::query(
                "INSERT INTO agent_conversation_tool_policies(conversation_id,tool_group,policy)
            VALUES ($1,'journal_writes','deny')"
            )
            .bind(conversation)
            .execute(&*pool)
            .await
            .is_err()
        );
        let (delta,): (Option<Decimal>,) = sqlx::query_as(
            "SELECT usdc_delta FROM hyperliquid.account_timeline
            WHERE event_id='rollback-send'",
        )
        .fetch_one(&*pool)
        .await
        .expect("legacy send");
        assert_eq!(delta, Some(Decimal::ZERO));

        sqlx::raw_sql(up)
            .execute(&*pool)
            .await
            .expect("reapply 0026 over populated journal");
        let (journal_policy,): (String,) = sqlx::query_as(
            "SELECT policy FROM agent_conversation_tool_policies
            WHERE conversation_id=$1 AND tool_group='journal_writes'",
        )
        .bind(conversation)
        .fetch_one(&*pool)
        .await
        .expect("restored default policy");
        assert_eq!(journal_policy, "deny");
        let (note_count,): (i64,) =
            sqlx::query_as("SELECT count(*) FROM hyperliquid.journal_notes")
                .fetch_one(&*pool)
                .await
                .expect("notes after rollback and reapply");
        assert_eq!(note_count, 0);
        rebuild_dirty(&pool, account, "live")
            .await
            .expect("backfill again");
        let (delta,): (Option<Decimal>,) = sqlx::query_as(
            "SELECT usdc_delta FROM hyperliquid.account_timeline
            WHERE event_id='rollback-send'",
        )
        .fetch_one(&*pool)
        .await
        .expect("reclassified send");
        assert_eq!(delta, Some(Decimal::new(7, 0)));
        assert_eq!(
            list_trades(&pool, account, "live", 10, 0)
                .await
                .expect("trades")
                .len(),
            1
        );
    }
}
