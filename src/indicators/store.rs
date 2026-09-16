use anyhow::{Context, Result, anyhow, ensure};
use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{AssertSqlSafe, Postgres, Transaction};
use uuid::Uuid;

use crate::{
    db::DbPool,
    indicators::model::{IndicatorDefinition, IndicatorRun, IndicatorVersion},
};

const RUN_COLUMNS: &str = "id, agent_key, indicator_definition_id, indicator_version_id, instrument_id, timeframe, scheduled_for, status, candle_data, plot_data, latest_values, diagnostics, error_summary, started_at, finished_at, created_at, updated_at";
const RUN_COLUMNS_QUALIFIED: &str = "r.id, r.agent_key, r.indicator_definition_id, r.indicator_version_id, r.instrument_id, r.timeframe, r.scheduled_for, r.status, r.candle_data, r.plot_data, r.latest_values, r.diagnostics, r.error_summary, r.started_at, r.finished_at, r.created_at, r.updated_at";
const DEFINITION_COLUMNS: &str = "id, agent_key, name, description, timeframe, enabled, active_version_id, created_at, updated_at";
const VERSION_COLUMNS: &str = "id, indicator_definition_id, version_number, source, source_sha256, compiler_version, metadata, input_values, created_by_kind, created_by_run_id, created_by_conversation_id, created_at";
const VERSION_COLUMNS_QUALIFIED: &str = "v.id, v.indicator_definition_id, v.version_number, v.source, v.source_sha256, v.compiler_version, v.metadata, v.input_values, v.created_by_kind, v.created_by_run_id, v.created_by_conversation_id, v.created_at";

#[derive(Debug, Clone)]
pub struct NewIndicatorVersion {
    pub source: String,
    pub compiler_version: String,
    pub metadata: Value,
    pub input_values: Value,
    pub created_by_kind: String,
    pub created_by_run_id: Option<i64>,
    pub created_by_conversation_id: Option<Uuid>,
}

#[derive(Debug, Clone)]
pub struct CreateIndicatorDefinition {
    pub name: String,
    pub description: String,
    pub timeframe: String,
    pub enabled: bool,
    pub instrument_ids: Vec<String>,
    pub version: NewIndicatorVersion,
}

#[derive(Debug, Clone)]
pub struct UpdateIndicatorDefinition {
    pub expected_active_version_id: Uuid,
    pub name: String,
    pub description: String,
    pub timeframe: String,
    pub enabled: bool,
    pub instrument_ids: Vec<String>,
    pub version: NewIndicatorVersion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndicatorUpdateResult {
    Updated,
    NotFound,
    VersionConflict,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ScheduledIndicatorTarget {
    pub agent_key: String,
    pub definition_id: Uuid,
    pub version_id: Uuid,
    pub instrument_id: String,
    pub timeframe: String,
}

fn source_sha256(source: &str) -> String {
    hex::encode(Sha256::digest(source.as_bytes()))
}

fn validate_provenance(version: &NewIndicatorVersion) -> Result<()> {
    match version.created_by_kind.as_str() {
        "operator"
            if version.created_by_run_id.is_none()
                && version.created_by_conversation_id.is_none() =>
        {
            Ok(())
        }
        "chat"
            if version.created_by_run_id.is_none()
                && version.created_by_conversation_id.is_some() =>
        {
            Ok(())
        }
        "review"
            if version.created_by_run_id.is_some()
                && version.created_by_conversation_id.is_none() =>
        {
            Ok(())
        }
        _ => Err(anyhow!("invalid indicator version creator provenance")),
    }
}

async fn validate_targets(
    tx: &mut Transaction<'_, Postgres>,
    agent_key: &str,
    targets: &[String],
) -> Result<Vec<String>> {
    let targets: Vec<String> = targets
        .iter()
        .map(|target| target.trim())
        .filter(|target| !target.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    ensure!(
        targets.len() <= super::runtime::MAX_INDICATOR_TARGETS,
        "indicator has too many instrument targets"
    );
    let distinct: std::collections::BTreeSet<_> = targets.iter().collect();
    ensure!(
        distinct.len() == targets.len(),
        "indicator instrument targets must be unique"
    );
    let valid: Vec<(String,)> = sqlx::query_as("SELECT selection.instrument_id FROM agent_analysis_instruments AS selection JOIN hyperliquid.instruments AS instruments ON instruments.instrument_id = selection.instrument_id WHERE selection.agent_key = $1 AND selection.instrument_id = ANY($2) AND instruments.active = true")
        .bind(agent_key).bind(&targets).fetch_all(&mut **tx).await.context("failed to validate indicator targets")?;
    ensure!(
        valid.len() == targets.len(),
        "indicator targets must be active instruments selected for analysis"
    );
    Ok(targets)
}

async fn insert_version(
    tx: &mut Transaction<'_, Postgres>,
    definition_id: Uuid,
    version_number: i32,
    version: &NewIndicatorVersion,
) -> Result<IndicatorVersion> {
    validate_provenance(version)?;
    sqlx::query_as(AssertSqlSafe(format!("INSERT INTO agent_indicator_versions (id, indicator_definition_id, version_number, source, source_sha256, compiler_version, metadata, input_values, created_by_kind, created_by_run_id, created_by_conversation_id) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) RETURNING {VERSION_COLUMNS}")))
        .bind(Uuid::new_v4()).bind(definition_id).bind(version_number).bind(&version.source).bind(source_sha256(&version.source)).bind(&version.compiler_version).bind(&version.metadata).bind(&version.input_values).bind(&version.created_by_kind).bind(version.created_by_run_id).bind(version.created_by_conversation_id).fetch_one(&mut **tx).await.context("failed to insert indicator version")
}

async fn replace_targets(
    tx: &mut Transaction<'_, Postgres>,
    definition_id: Uuid,
    targets: &[String],
) -> Result<()> {
    sqlx::query(
        "DELETE FROM agent_indicator_definition_instruments WHERE indicator_definition_id = $1",
    )
    .bind(definition_id)
    .execute(&mut **tx)
    .await?;
    for target in targets {
        sqlx::query("INSERT INTO agent_indicator_definition_instruments (indicator_definition_id, instrument_id) VALUES ($1, $2)").bind(definition_id).bind(target).execute(&mut **tx).await?;
    }
    Ok(())
}

pub async fn create_definition_with_initial_version(
    pool: &DbPool,
    agent_key: &str,
    input: &CreateIndicatorDefinition,
) -> Result<IndicatorDefinition> {
    let mut tx = pool
        .begin()
        .await
        .context("failed to begin indicator create transaction")?;
    let agent_exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM agents WHERE agent_key = $1)")
            .bind(agent_key)
            .fetch_one(&mut *tx)
            .await
            .context("failed to verify indicator owner agent")?;
    ensure!(agent_exists, "indicator owner agent does not exist");
    let targets = validate_targets(&mut tx, agent_key, &input.instrument_ids).await?;
    let definition_id = Uuid::new_v4();
    sqlx::query("INSERT INTO agent_indicator_definitions (id, agent_key, name, description, timeframe, enabled) SELECT $1, agent_key, $2, $3, $4, $5 FROM agents WHERE agent_key = $6")
        .bind(definition_id).bind(input.name.trim()).bind(input.description.trim()).bind(input.timeframe.trim()).bind(input.enabled).bind(agent_key).execute(&mut *tx).await.context("failed to insert indicator definition")?;
    let version = insert_version(&mut tx, definition_id, 1, &input.version).await?;
    replace_targets(&mut tx, definition_id, &targets).await?;
    let definition = sqlx::query_as(AssertSqlSafe(format!("UPDATE agent_indicator_definitions SET active_version_id = $1, updated_at = now() WHERE id = $2 AND agent_key = $3 RETURNING {DEFINITION_COLUMNS}")))
        .bind(version.id).bind(definition_id).bind(agent_key).fetch_one(&mut *tx).await.context("failed to activate initial indicator version")?;
    tx.commit()
        .await
        .context("failed to commit indicator create transaction")?;
    Ok(definition)
}

pub async fn update_definition_with_new_version(
    pool: &DbPool,
    agent_key: &str,
    definition_id: Uuid,
    input: &UpdateIndicatorDefinition,
) -> Result<IndicatorUpdateResult> {
    let mut tx = pool
        .begin()
        .await
        .context("failed to begin indicator update transaction")?;
    let existing: Option<(Uuid, i32)> = sqlx::query_as("SELECT active_version_id, (SELECT version_number FROM agent_indicator_versions WHERE id = active_version_id) FROM agent_indicator_definitions WHERE id = $1 AND agent_key = $2 FOR UPDATE").bind(definition_id).bind(agent_key).fetch_optional(&mut *tx).await?;
    let Some((active_version_id, version_number)) = existing else {
        return Ok(IndicatorUpdateResult::NotFound);
    };
    if active_version_id != input.expected_active_version_id {
        return Ok(IndicatorUpdateResult::VersionConflict);
    }
    let targets = validate_targets(&mut tx, agent_key, &input.instrument_ids).await?;
    let version =
        insert_version(&mut tx, definition_id, version_number + 1, &input.version).await?;
    replace_targets(&mut tx, definition_id, &targets).await?;
    sqlx::query("UPDATE agent_indicator_definitions SET name = $1, description = $2, timeframe = $3, enabled = $4, active_version_id = $5, updated_at = now() WHERE id = $6 AND agent_key = $7").bind(input.name.trim()).bind(input.description.trim()).bind(input.timeframe.trim()).bind(input.enabled).bind(version.id).bind(definition_id).bind(agent_key).execute(&mut *tx).await?;
    tx.commit()
        .await
        .context("failed to commit indicator update transaction")?;
    Ok(IndicatorUpdateResult::Updated)
}

pub async fn list_definitions(pool: &DbPool, agent_key: &str) -> Result<Vec<IndicatorDefinition>> {
    sqlx::query_as(AssertSqlSafe(format!("SELECT {DEFINITION_COLUMNS} FROM agent_indicator_definitions WHERE agent_key = $1 ORDER BY name"))).bind(agent_key).fetch_all(pool).await.context("failed to list indicator definitions")
}

/// Lists only targets still selected in the agent's active analysis universe.
pub async fn list_enabled_targets(pool: &DbPool) -> Result<Vec<ScheduledIndicatorTarget>> {
    sqlx::query_as("SELECT d.agent_key, d.id AS definition_id, d.active_version_id AS version_id, target.instrument_id, d.timeframe FROM agent_indicator_definitions d JOIN agent_indicator_definition_instruments target ON target.indicator_definition_id = d.id JOIN agent_analysis_instruments selected ON selected.agent_key = d.agent_key AND selected.instrument_id = target.instrument_id JOIN hyperliquid.instruments instruments ON instruments.instrument_id = target.instrument_id WHERE d.enabled AND instruments.active ORDER BY d.agent_key, d.id, target.instrument_id")
        .fetch_all(pool)
        .await
        .context("failed to list enabled indicator targets")
}
pub async fn get_definition(
    pool: &DbPool,
    agent_key: &str,
    definition_id: Uuid,
) -> Result<Option<IndicatorDefinition>> {
    sqlx::query_as(AssertSqlSafe(format!("SELECT {DEFINITION_COLUMNS} FROM agent_indicator_definitions WHERE agent_key = $1 AND id = $2"))).bind(agent_key).bind(definition_id).fetch_optional(pool).await.context("failed to get indicator definition")
}
pub async fn get_active_version(
    pool: &DbPool,
    agent_key: &str,
    definition_id: Uuid,
) -> Result<Option<IndicatorVersion>> {
    sqlx::query_as(AssertSqlSafe(format!("SELECT {VERSION_COLUMNS_QUALIFIED} FROM agent_indicator_versions v JOIN agent_indicator_definitions d ON d.active_version_id = v.id WHERE d.agent_key = $1 AND d.id = $2"))).bind(agent_key).bind(definition_id).fetch_optional(pool).await.context("failed to get active indicator version")
}
pub async fn get_version(
    pool: &DbPool,
    agent_key: &str,
    definition_id: Uuid,
    version_id: Uuid,
) -> Result<Option<IndicatorVersion>> {
    sqlx::query_as(AssertSqlSafe(format!("SELECT {VERSION_COLUMNS_QUALIFIED} FROM agent_indicator_versions v JOIN agent_indicator_definitions d ON d.id = v.indicator_definition_id WHERE d.agent_key = $1 AND d.id = $2 AND v.id = $3")))
        .bind(agent_key)
        .bind(definition_id)
        .bind(version_id)
        .fetch_optional(pool)
        .await
        .context("failed to get indicator version")
}
pub async fn list_definition_versions(
    pool: &DbPool,
    agent_key: &str,
    definition_id: Uuid,
) -> Result<Vec<IndicatorVersion>> {
    sqlx::query_as(AssertSqlSafe(format!("SELECT {VERSION_COLUMNS_QUALIFIED} FROM agent_indicator_versions v JOIN agent_indicator_definitions d ON d.id = v.indicator_definition_id WHERE d.agent_key = $1 AND d.id = $2 ORDER BY v.version_number DESC"))).bind(agent_key).bind(definition_id).fetch_all(pool).await.context("failed to list indicator versions")
}
pub async fn list_definition_instruments(
    pool: &DbPool,
    agent_key: &str,
    definition_id: Uuid,
) -> Result<Vec<String>> {
    let rows: Vec<(String,)> = sqlx::query_as("SELECT i.instrument_id FROM agent_indicator_definition_instruments i JOIN agent_indicator_definitions d ON d.id = i.indicator_definition_id WHERE d.agent_key = $1 AND d.id = $2 ORDER BY i.instrument_id").bind(agent_key).bind(definition_id).fetch_all(pool).await?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}
pub async fn list_latest_results(
    pool: &DbPool,
    agent_key: &str,
    definition_id: Uuid,
    limit: i64,
) -> Result<Vec<IndicatorRun>> {
    sqlx::query_as(AssertSqlSafe(format!("SELECT {RUN_COLUMNS} FROM agent_indicator_runs WHERE agent_key = $1 AND indicator_definition_id = $2 ORDER BY scheduled_for DESC LIMIT $3"))).bind(agent_key).bind(definition_id).bind(limit.clamp(1, 100)).fetch_all(pool).await.context("failed to list indicator results")
}
pub async fn get_chart_run(
    pool: &DbPool,
    agent_key: &str,
    definition_id: Uuid,
    instrument_id: &str,
) -> Result<Option<IndicatorRun>> {
    sqlx::query_as(AssertSqlSafe(format!("SELECT {RUN_COLUMNS} FROM agent_indicator_runs WHERE agent_key = $1 AND indicator_definition_id = $2 AND instrument_id = $3 AND status = 'succeeded' ORDER BY scheduled_for DESC LIMIT 1"))).bind(agent_key).bind(definition_id).bind(instrument_id).fetch_optional(pool).await.context("failed to get indicator chart run")
}

pub async fn enqueue_run(
    pool: &DbPool,
    agent_key: &str,
    definition_id: Uuid,
    version_id: Uuid,
    instrument_id: &str,
    timeframe: &str,
    scheduled_for: DateTime<Utc>,
) -> Result<Option<Uuid>> {
    let id = Uuid::new_v4();
    let inserted: Option<(Uuid,)> = sqlx::query_as("INSERT INTO agent_indicator_runs (id, agent_key, indicator_definition_id, indicator_version_id, instrument_id, timeframe, scheduled_for, status) SELECT $1, d.agent_key, d.id, $3, $4, $5, $6, 'queued' FROM agent_indicator_definitions d JOIN agent_indicator_definition_instruments i ON i.indicator_definition_id = d.id WHERE d.id = $2 AND d.agent_key = $7 AND i.instrument_id = $4 ON CONFLICT (indicator_version_id, instrument_id, timeframe, scheduled_for) DO NOTHING RETURNING id").bind(id).bind(definition_id).bind(version_id).bind(instrument_id).bind(timeframe).bind(scheduled_for).bind(agent_key).fetch_optional(pool).await.context("failed to enqueue indicator run")?;
    Ok(inserted.map(|(id,)| id))
}
pub async fn claim_next_queued_run(pool: &DbPool) -> Result<Option<IndicatorRun>> {
    sqlx::query_as(AssertSqlSafe(format!("WITH next AS (SELECT id FROM agent_indicator_runs WHERE status = 'queued' ORDER BY scheduled_for FOR UPDATE SKIP LOCKED LIMIT 1) UPDATE agent_indicator_runs r SET status = 'running', started_at = now(), updated_at = now() FROM next WHERE r.id = next.id RETURNING {RUN_COLUMNS_QUALIFIED}"))) .fetch_optional(pool).await.context("failed to claim indicator run")
}
pub async fn finish_run_succeeded(
    pool: &DbPool,
    agent_key: &str,
    run_id: Uuid,
    candle_data: Value,
    plot_data: Value,
    latest_values: Value,
    diagnostics: Value,
) -> Result<bool> {
    let result = sqlx::query("UPDATE agent_indicator_runs SET status = 'succeeded', candle_data = $1, plot_data = $2, latest_values = $3, diagnostics = $4, finished_at = now(), updated_at = now() WHERE id = $5 AND agent_key = $6 AND status = 'running'").bind(candle_data).bind(plot_data).bind(latest_values).bind(diagnostics).bind(run_id).bind(agent_key).execute(pool).await?;
    Ok(result.rows_affected() == 1)
}
pub async fn finish_run_failed(
    pool: &DbPool,
    agent_key: &str,
    run_id: Uuid,
    diagnostics: Value,
    error_summary: &str,
) -> Result<bool> {
    let result = sqlx::query("UPDATE agent_indicator_runs SET status = 'failed', diagnostics = $1, error_summary = $2, finished_at = now(), updated_at = now() WHERE id = $3 AND agent_key = $4 AND status IN ('queued', 'running')").bind(diagnostics).bind(error_summary).bind(run_id).bind(agent_key).execute(pool).await?;
    Ok(result.rows_affected() == 1)
}
pub async fn cleanup_expired_runs(pool: &DbPool, now: DateTime<Utc>, limit: i64) -> Result<u64> {
    let cutoff = now - Duration::days(30);
    let result = sqlx::query("DELETE FROM agent_indicator_runs WHERE id IN (SELECT r.id FROM agent_indicator_runs r WHERE r.finished_at < $1 AND NOT (r.status = 'succeeded' AND NOT EXISTS (SELECT 1 FROM agent_indicator_runs newer WHERE newer.indicator_definition_id = r.indicator_definition_id AND newer.instrument_id = r.instrument_id AND newer.status = 'succeeded' AND newer.scheduled_for > r.scheduled_for)) ORDER BY r.finished_at LIMIT $2)").bind(cutoff).bind(limit.clamp(1, 1000)).execute(pool).await.context("failed to clean up expired indicator runs")?;
    Ok(result.rows_affected())
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde_json::json;

    use super::*;
    use crate::{
        agents::{
            model::AgentRegistryRow,
            store::{insert_agent, replace_agent_analysis_instruments},
        },
        test_db,
    };

    async fn seed_agent(pool: &DbPool, key: &str) {
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
            .execute(pool).await.expect("seed instrument");
        replace_agent_analysis_instruments(pool, key, &["BTC".to_string()])
            .await
            .expect("select analysis instrument");
    }

    fn version() -> NewIndicatorVersion {
        NewIndicatorVersion {
            source: "indicator(\"Test\")".to_string(),
            compiler_version: "test".to_string(),
            metadata: json!({}),
            input_values: json!({}),
            created_by_kind: "operator".to_string(),
            created_by_run_id: None,
            created_by_conversation_id: None,
        }
    }

    fn definition() -> CreateIndicatorDefinition {
        CreateIndicatorDefinition {
            name: "Test".to_string(),
            description: String::new(),
            timeframe: "1h".to_string(),
            enabled: true,
            instrument_ids: vec!["BTC".to_string()],
            version: version(),
        }
    }

    #[tokio::test]
    async fn definitions_and_versions_are_scoped_to_the_owner_agent() {
        let pool = test_db::pool().await;
        seed_agent(&pool, "indicator-owner").await;
        seed_agent(&pool, "indicator-other").await;
        let created =
            create_definition_with_initial_version(&pool, "indicator-owner", &definition())
                .await
                .expect("create definition");
        assert_eq!(
            list_definition_instruments(&pool, "indicator-owner", created.id)
                .await
                .expect("targets"),
            ["BTC"]
        );
        assert!(
            get_definition(&pool, "indicator-other", created.id)
                .await
                .expect("get foreign")
                .is_none()
        );
        assert!(
            get_active_version(&pool, "indicator-other", created.id)
                .await
                .expect("get foreign version")
                .is_none()
        );
        assert!(
            list_definition_versions(&pool, "indicator-other", created.id)
                .await
                .expect("list foreign versions")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn update_creates_an_immutable_version_and_checks_the_active_version() {
        let pool = test_db::pool().await;
        seed_agent(&pool, "indicator-update").await;
        let created =
            create_definition_with_initial_version(&pool, "indicator-update", &definition())
                .await
                .expect("create definition");
        let active = get_active_version(&pool, "indicator-update", created.id)
            .await
            .expect("active")
            .expect("version");
        let update = UpdateIndicatorDefinition {
            expected_active_version_id: active.id,
            name: "Test two".to_string(),
            description: "changed".to_string(),
            timeframe: "4h".to_string(),
            enabled: true,
            instrument_ids: vec!["BTC".to_string()],
            version: version(),
        };
        assert_eq!(
            update_definition_with_new_version(&pool, "indicator-update", created.id, &update)
                .await
                .expect("update"),
            IndicatorUpdateResult::Updated
        );
        let versions = list_definition_versions(&pool, "indicator-update", created.id)
            .await
            .expect("versions");
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].version_number, 2);
        assert_eq!(
            update_definition_with_new_version(&pool, "indicator-update", created.id, &update)
                .await
                .expect("stale update"),
            IndicatorUpdateResult::VersionConflict
        );
    }

    #[tokio::test]
    async fn runs_are_claimed_once_and_retention_preserves_the_latest_success() {
        let pool = test_db::pool().await;
        seed_agent(&pool, "indicator-runs").await;
        let definition =
            create_definition_with_initial_version(&pool, "indicator-runs", &definition())
                .await
                .expect("create definition");
        let version = get_active_version(&pool, "indicator-runs", definition.id)
            .await
            .expect("active")
            .expect("version");
        let older = Utc::now() - Duration::days(40);
        let newer = older + Duration::hours(1);
        enqueue_run(
            &pool,
            "indicator-runs",
            definition.id,
            version.id,
            "BTC",
            "1h",
            older,
        )
        .await
        .expect("enqueue old");
        enqueue_run(
            &pool,
            "indicator-runs",
            definition.id,
            version.id,
            "BTC",
            "1h",
            newer,
        )
        .await
        .expect("enqueue new");
        let first = claim_next_queued_run(&pool)
            .await
            .expect("claim")
            .expect("first run");
        assert!(
            claim_next_queued_run(&pool)
                .await
                .expect("claim second")
                .is_some()
        );
        assert!(
            finish_run_succeeded(
                &pool,
                "indicator-runs",
                first.id,
                json!([]),
                json!([]),
                json!({}),
                json!([])
            )
            .await
            .expect("finish first")
        );
        let second = list_latest_results(&pool, "indicator-runs", definition.id, 2)
            .await
            .expect("runs")
            .into_iter()
            .find(|run| run.status == "running")
            .expect("second run");
        assert!(
            finish_run_succeeded(
                &pool,
                "indicator-runs",
                second.id,
                json!([]),
                json!([]),
                json!({}),
                json!([])
            )
            .await
            .expect("finish second")
        );
        sqlx::query("UPDATE agent_indicator_runs SET finished_at = scheduled_for")
            .execute(&pool)
            .await
            .expect("age runs");
        assert_eq!(
            cleanup_expired_runs(&pool, Utc::now(), 10)
                .await
                .expect("clean up"),
            1
        );
        assert_eq!(
            list_latest_results(&pool, "indicator-runs", definition.id, 10)
                .await
                .expect("remaining")
                .len(),
            1
        );
    }
}
