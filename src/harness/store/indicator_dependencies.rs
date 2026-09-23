use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::{
    db::DbPool,
    harness::{
        model::{IndicatorRunSnapshot, IndicatorSnapshot, MAX_ANALYSIS_INDICATOR_DEPENDENCIES},
        timeframe::canonical_boundary_at_or_before,
    },
};

#[derive(Debug, sqlx::FromRow)]
struct ApplicableTarget {
    definition_id: Uuid,
    version_id: Uuid,
    instrument_id: String,
    timeframe: String,
}

#[derive(Debug, sqlx::FromRow)]
struct DependencyRow {
    definition_id: Uuid,
    version_id: Uuid,
    instrument_id: String,
    timeframe: String,
    indicator_boundary: DateTime<Utc>,
    run_id: Uuid,
    snapshot_status: Option<String>,
}

#[derive(Debug)]
pub struct IndicatorSetState {
    pub wait_deadline_at: DateTime<Utc>,
    pub frozen: bool,
    pub all_terminal: bool,
}

pub async fn prepare_indicator_dependencies(
    pool: &DbPool,
    analysis_run_id: i64,
    agent_key: &str,
    analysis_instruments: &[String],
    as_of_boundary: DateTime<Utc>,
    wait_timeout: std::time::Duration,
) -> Result<()> {
    let wait_timeout = chrono::Duration::from_std(wait_timeout)
        .context("indicator Analysis wait timeout is out of range")?;
    let mut tx = pool
        .begin()
        .await
        .context("failed to begin indicator dependency preparation")?;
    let inserted: Option<(i64,)> = sqlx::query_as(
        "INSERT INTO harness_run_indicator_sets (
             analysis_run_id, as_of_boundary, wait_deadline_at
         )
         SELECT runs.id, $3, now() + $4
           FROM harness_sub_agent_runs runs
          WHERE runs.id = $1
            AND runs.agent_key = $2
            AND runs.sub_agent_kind = 'analysis'
         ON CONFLICT (analysis_run_id) DO NOTHING
         RETURNING analysis_run_id",
    )
    .bind(analysis_run_id)
    .bind(agent_key)
    .bind(as_of_boundary)
    .bind(wait_timeout)
    .fetch_optional(&mut *tx)
    .await
    .context("failed to create indicator dependency set")?;

    let parent: Option<(i64,)> = sqlx::query_as(
        "SELECT analysis_run_id
           FROM harness_run_indicator_sets
          WHERE analysis_run_id = $1
          FOR UPDATE",
    )
    .bind(analysis_run_id)
    .fetch_optional(&mut *tx)
    .await
    .context("failed to lock indicator dependency set")?;
    if parent.is_none() {
        bail!("Analysis run is unavailable for indicator dependency preparation");
    }
    if inserted.is_none() {
        tx.commit()
            .await
            .context("failed to commit existing indicator dependency set")?;
        return Ok(());
    }

    let targets: Vec<ApplicableTarget> = sqlx::query_as(
        "SELECT d.id AS definition_id,
                d.active_version_id AS version_id,
                target.instrument_id,
                timeframe.timeframe
           FROM agent_indicator_definitions d
           JOIN agent_indicator_definition_timeframes timeframe
             ON timeframe.indicator_definition_id = d.id
           JOIN agent_indicator_definition_instruments target
             ON target.indicator_definition_id = d.id
           JOIN hyperliquid.instruments instrument
             ON instrument.instrument_id = target.instrument_id
          WHERE d.agent_key = $1
            AND d.enabled
            AND d.archived_at IS NULL
            AND d.active_version_id IS NOT NULL
            AND target.instrument_id = ANY($2)
            AND instrument.active
          ORDER BY d.id, timeframe.timeframe, target.instrument_id",
    )
    .bind(agent_key)
    .bind(analysis_instruments)
    .fetch_all(&mut *tx)
    .await
    .context("failed to load applicable indicator dependency targets")?;
    if targets.len() > MAX_ANALYSIS_INDICATOR_DEPENDENCIES {
        bail!(
            "Analysis indicator dependency set exceeds the {} dependency limit",
            MAX_ANALYSIS_INDICATOR_DEPENDENCIES
        );
    }

    for target in targets {
        let indicator_boundary =
            canonical_boundary_at_or_before(as_of_boundary, &target.timeframe)?;
        let run_id =
            insert_or_get_indicator_run(&mut tx, agent_key, &target, indicator_boundary).await?;
        sqlx::query(
            "INSERT INTO harness_run_indicator_dependencies (
                 analysis_run_id, indicator_run_id, indicator_boundary
             ) VALUES ($1, $2, $3)
             ON CONFLICT DO NOTHING",
        )
        .bind(analysis_run_id)
        .bind(run_id)
        .bind(indicator_boundary)
        .execute(&mut *tx)
        .await
        .context("failed to insert indicator dependency")?;
    }

    tx.commit()
        .await
        .context("failed to commit indicator dependency preparation")?;
    crate::indicators::coordination::notify_queue();
    Ok(())
}

async fn insert_or_get_indicator_run(
    tx: &mut Transaction<'_, Postgres>,
    agent_key: &str,
    target: &ApplicableTarget,
    boundary: DateTime<Utc>,
) -> Result<Uuid> {
    let proposed_id = Uuid::new_v4();
    let inserted: Option<(Uuid,)> = sqlx::query_as(
        "INSERT INTO agent_indicator_runs (
             id, agent_key, indicator_definition_id, indicator_version_id,
             instrument_id, timeframe, scheduled_for, status
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, 'queued')
         ON CONFLICT (indicator_version_id, instrument_id, timeframe, scheduled_for)
         DO NOTHING
         RETURNING id",
    )
    .bind(proposed_id)
    .bind(agent_key)
    .bind(target.definition_id)
    .bind(target.version_id)
    .bind(&target.instrument_id)
    .bind(&target.timeframe)
    .bind(boundary)
    .fetch_optional(&mut **tx)
    .await
    .context("failed to enqueue indicator dependency run")?;
    if let Some((run_id,)) = inserted {
        return Ok(run_id);
    }
    let (run_id,): (Uuid,) = sqlx::query_as(
        "SELECT id
           FROM agent_indicator_runs
          WHERE indicator_version_id = $1
            AND instrument_id = $2
            AND timeframe = $3
            AND scheduled_for = $4",
    )
    .bind(target.version_id)
    .bind(&target.instrument_id)
    .bind(&target.timeframe)
    .bind(boundary)
    .fetch_one(&mut **tx)
    .await
    .context("failed to load reused indicator dependency run")?;
    Ok(run_id)
}

pub async fn get_indicator_set_state(
    pool: &DbPool,
    analysis_run_id: i64,
) -> Result<IndicatorSetState> {
    let row: (DateTime<Utc>, Option<DateTime<Utc>>, bool) = sqlx::query_as(
        "SELECT sets.wait_deadline_at,
                sets.frozen_at,
                NOT EXISTS (
                    SELECT 1
                      FROM harness_run_indicator_dependencies dependency
                      JOIN agent_indicator_runs run ON run.id = dependency.indicator_run_id
                     WHERE dependency.analysis_run_id = sets.analysis_run_id
                       AND run.status NOT IN ('succeeded', 'failed', 'skipped')
                ) AS all_terminal
           FROM harness_run_indicator_sets sets
          WHERE sets.analysis_run_id = $1",
    )
    .bind(analysis_run_id)
    .fetch_one(pool)
    .await
    .context("failed to load indicator dependency set state")?;
    Ok(IndicatorSetState {
        wait_deadline_at: row.0,
        frozen: row.1.is_some(),
        all_terminal: row.2,
    })
}

pub async fn freeze_indicator_dependencies(
    pool: &DbPool,
    analysis_run_id: i64,
) -> Result<IndicatorSnapshot> {
    let mut tx = pool
        .begin()
        .await
        .context("failed to begin indicator dependency freeze")?;
    let parent: (DateTime<Utc>, Option<DateTime<Utc>>) = sqlx::query_as(
        "SELECT as_of_boundary, frozen_at
           FROM harness_run_indicator_sets
          WHERE analysis_run_id = $1
          FOR UPDATE",
    )
    .bind(analysis_run_id)
    .fetch_one(&mut *tx)
    .await
    .context("failed to lock indicator dependency set for freeze")?;
    if parent.1.is_none() {
        sqlx::query(
            "UPDATE harness_run_indicator_dependencies dependency
                SET snapshot_status = CASE run.status
                    WHEN 'succeeded' THEN 'succeeded'
                    WHEN 'failed' THEN 'failed'
                    WHEN 'skipped' THEN 'skipped'
                    ELSE 'timed_out'
                END
               FROM agent_indicator_runs run
              WHERE dependency.analysis_run_id = $1
                AND run.id = dependency.indicator_run_id",
        )
        .bind(analysis_run_id)
        .execute(&mut *tx)
        .await
        .context("failed to freeze indicator dependency statuses")?;
        sqlx::query(
            "UPDATE harness_run_indicator_sets SET frozen_at = now()
              WHERE analysis_run_id = $1 AND frozen_at IS NULL",
        )
        .bind(analysis_run_id)
        .execute(&mut *tx)
        .await
        .context("failed to mark indicator dependency set frozen")?;
    }
    let rows = load_dependency_rows(&mut tx, analysis_run_id).await?;
    tx.commit()
        .await
        .context("failed to commit indicator dependency freeze")?;
    Ok(snapshot_from_rows(parent.0, rows))
}

async fn load_dependency_rows(
    tx: &mut Transaction<'_, Postgres>,
    analysis_run_id: i64,
) -> Result<Vec<DependencyRow>> {
    sqlx::query_as(
        "SELECT run.indicator_definition_id AS definition_id,
                run.indicator_version_id AS version_id,
                run.instrument_id,
                run.timeframe,
                dependency.indicator_boundary,
                run.id AS run_id,
                dependency.snapshot_status
           FROM harness_run_indicator_dependencies dependency
           JOIN agent_indicator_runs run ON run.id = dependency.indicator_run_id
          WHERE dependency.analysis_run_id = $1
          ORDER BY run.indicator_definition_id, run.timeframe,
                   run.instrument_id, dependency.indicator_boundary",
    )
    .bind(analysis_run_id)
    .fetch_all(&mut **tx)
    .await
    .context("failed to load frozen indicator dependencies")
}

fn snapshot_from_rows(
    as_of_boundary: DateTime<Utc>,
    rows: Vec<DependencyRow>,
) -> IndicatorSnapshot {
    IndicatorSnapshot {
        as_of_boundary_ms: as_of_boundary.timestamp_millis(),
        runs: rows
            .into_iter()
            .map(|row| IndicatorRunSnapshot {
                definition_id: row.definition_id,
                version_id: row.version_id,
                instrument_id: row.instrument_id,
                timeframe: row.timeframe,
                indicator_boundary_ms: row.indicator_boundary.timestamp_millis(),
                run_id: row.run_id,
                status: row.snapshot_status.unwrap_or_else(|| "missing".to_string()),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};
    use serde_json::json;

    use super::*;
    use crate::{
        agents::{
            model::AgentRegistryRow,
            store::{insert_agent, replace_agent_analysis_instruments},
        },
        indicators::store::{
            CreateIndicatorDefinition, FrozenDependencyResultFilter, NewIndicatorVersion,
            create_definition_with_initial_version, delete_definition,
            list_frozen_dependency_results,
        },
        test_db,
    };

    async fn seed(pool: &DbPool, key: &str) -> (i64, Uuid) {
        let now = Utc::now();
        insert_agent(
            pool,
            &AgentRegistryRow {
                agent_key: key.to_string(),
                user_id: test_db::test_user_id(),
                created_at: now,
                updated_at: now,
                enabled: true,
                lifecycle: "active".to_string(),
                display_name: key.to_string(),
                trading_account_address: Some(format!("0x{:040x}", Uuid::new_v4().as_u128())),
                environment: "live".to_string(),
                api_key: format!("key-{key}"),
                api_key_last_used_at: None,
                runtime_config: json!({}),
            },
        )
        .await
        .expect("insert agent");
        sqlx::query("INSERT INTO hyperliquid.instruments (instrument_id, name, market_type, base_asset, quote_asset, settlement_asset, asset_index, price_decimals, size_decimals, lot_size, max_leverage, is_hip3, active, created_at, updated_at) VALUES ('BTC', 'BTC', 'perp', 'BTC', 'USD', 'USDC', 1, 2, 3, 0.001, 50, false, true, now(), now()) ON CONFLICT (instrument_id) DO NOTHING")
            .execute(pool)
            .await
            .expect("seed instrument");
        replace_agent_analysis_instruments(pool, key, &["BTC".to_string()])
            .await
            .expect("select instrument");
        let (sub_agent_id,): (i64,) = sqlx::query_as(
            "INSERT INTO harness_sub_agents (agent_key, sub_agent_key, sub_agent_kind, timeframe, trigger_delay_seconds, next_run_at, timeout_seconds)
             VALUES ($1, 'analysis-15m', 'analysis', '15m', 2, now(), 60) RETURNING id",
        )
        .bind(key)
        .fetch_one(pool)
        .await
        .expect("insert Analysis job");
        let definition = create_definition_with_initial_version(
            pool,
            key,
            &CreateIndicatorDefinition {
                name: "Hourly".to_string(),
                description: String::new(),
                timeframes: vec!["1h".to_string()],
                enabled: true,
                instrument_ids: vec!["BTC".to_string()],
                version: NewIndicatorVersion {
                    source: "indicator(\"Hourly\")\nplot(close)".to_string(),
                    compiler_version: "test".to_string(),
                    metadata: json!({}),
                    input_values: json!({}),
                    created_by_kind: "operator".to_string(),
                    created_by_run_id: None,
                    created_by_conversation_id: None,
                },
            },
        )
        .await
        .expect("create indicator");
        (sub_agent_id, definition.id)
    }

    async fn analysis_run(
        pool: &DbPool,
        key: &str,
        sub_agent_id: i64,
        boundary: DateTime<Utc>,
    ) -> i64 {
        let (run_id,): (i64,) = sqlx::query_as(
            "INSERT INTO harness_sub_agent_runs (sub_agent_id, agent_key, sub_agent_key, sub_agent_kind, timeframe, status, scheduled_for, timeout_seconds)
             VALUES ($1, $2, 'analysis-15m', 'analysis', '15m', 'queued', $3, 60) RETURNING id",
        )
        .bind(sub_agent_id)
        .bind(key)
        .bind(boundary)
        .fetch_one(pool)
        .await
        .expect("insert Analysis run");
        run_id
    }

    #[tokio::test]
    async fn preparation_is_idempotent_and_reuses_canonical_slower_runs() {
        let pool = test_db::pool().await;
        let key = "dependency-canonical";
        let (sub_agent_id, definition_id) = seed(&pool, key).await;
        let at_1015 = Utc.with_ymd_and_hms(2026, 1, 1, 10, 15, 0).unwrap();
        let at_1045 = Utc.with_ymd_and_hms(2026, 1, 1, 10, 45, 0).unwrap();
        let first = analysis_run(&pool, key, sub_agent_id, at_1015).await;
        let second = analysis_run(&pool, key, sub_agent_id, at_1045).await;
        for (run_id, boundary) in [(first, at_1015), (second, at_1045)] {
            prepare_indicator_dependencies(
                &pool,
                run_id,
                key,
                &["BTC".to_string()],
                boundary,
                std::time::Duration::from_secs(30),
            )
            .await
            .expect("prepare dependencies");
        }
        prepare_indicator_dependencies(
            &pool,
            first,
            key,
            &[],
            at_1045,
            std::time::Duration::from_secs(1),
        )
        .await
        .expect("repeat preparation");
        let dependencies: Vec<(i64, Uuid, DateTime<Utc>)> = sqlx::query_as(
            "SELECT analysis_run_id, indicator_run_id, indicator_boundary FROM harness_run_indicator_dependencies ORDER BY analysis_run_id",
        )
        .fetch_all(&pool)
        .await
        .expect("load dependencies");
        assert_eq!(dependencies.len(), 2);
        assert_eq!(dependencies[0].1, dependencies[1].1);
        assert_eq!(
            dependencies[0].2,
            Utc.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).unwrap()
        );

        assert!(
            delete_definition(&pool, key, definition_id)
                .await
                .expect("archive definition")
        );
        prepare_indicator_dependencies(
            &pool,
            first,
            key,
            &[],
            at_1045,
            std::time::Duration::from_secs(1),
        )
        .await
        .expect("reuse frozen applicability");
        let count: (i64,) = sqlx::query_as(
            "SELECT count(*) FROM harness_run_indicator_dependencies WHERE analysis_run_id = $1",
        )
        .bind(first)
        .fetch_one(&pool)
        .await
        .expect("count dependencies");
        assert_eq!(count.0, 1);
    }

    #[tokio::test]
    async fn timed_out_freeze_remains_hidden_after_late_completion_and_survives_cleanup() {
        let pool = test_db::pool().await;
        let key = "dependency-timeout";
        let (sub_agent_id, definition_id) = seed(&pool, key).await;
        let boundary = Utc.with_ymd_and_hms(2026, 1, 1, 10, 15, 0).unwrap();
        let analysis_run_id = analysis_run(&pool, key, sub_agent_id, boundary).await;
        prepare_indicator_dependencies(
            &pool,
            analysis_run_id,
            key,
            &["BTC".to_string()],
            boundary,
            std::time::Duration::from_secs(1),
        )
        .await
        .expect("prepare dependencies");
        let snapshot = freeze_indicator_dependencies(&pool, analysis_run_id)
            .await
            .expect("freeze dependencies");
        assert_eq!(snapshot.runs[0].status, "timed_out");
        let run_id = snapshot.runs[0].run_id;
        sqlx::query("UPDATE agent_indicator_runs SET status = 'succeeded', finished_at = now() - interval '40 days' WHERE id = $1")
            .bind(run_id)
            .execute(&pool)
            .await
            .expect("complete dependency late");
        assert!(
            list_frozen_dependency_results(
                &pool,
                analysis_run_id,
                key,
                definition_id,
                FrozenDependencyResultFilter {
                    timeframe: Some("1h"),
                    instrument_id: None,
                    run_id: None,
                    limit: 10,
                },
            )
            .await
            .expect("read frozen results")
            .is_empty()
        );
        assert_eq!(
            crate::indicators::store::cleanup_expired_runs(
                &pool,
                Utc::now() + Duration::days(1),
                100,
            )
            .await
            .expect("clean old runs"),
            0
        );
        let still_exists: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM agent_indicator_runs WHERE id = $1)")
                .bind(run_id)
                .fetch_one(&pool)
                .await
                .expect("check retained run");
        assert!(still_exists);
    }
}
