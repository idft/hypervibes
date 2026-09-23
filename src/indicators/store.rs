use anyhow::{Context, Result, anyhow, ensure};
use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{AssertSqlSafe, Postgres, Transaction};
use uuid::Uuid;

use crate::{
    db::DbPool,
    harness::timeframe::{
        DEFAULT_TRIGGER_DELAY_SECONDS, boundary_for_due_at, latest_due_at_or_before,
        parse_timeframe_seconds,
    },
    indicators::model::{IndicatorDefinition, IndicatorRun, IndicatorVersion},
};

const RUN_COLUMNS: &str = "id, agent_key, indicator_definition_id, indicator_version_id, instrument_id, timeframe, scheduled_for, status, candle_data, plot_data, visual_data, latest_values, diagnostics, error_summary, attempt_count, next_attempt_at, claim_token, lease_expires_at, started_at, finished_at, created_at, updated_at";
const RUN_COLUMNS_QUALIFIED: &str = "r.id, r.agent_key, r.indicator_definition_id, r.indicator_version_id, r.instrument_id, r.timeframe, r.scheduled_for, r.status, r.candle_data, r.plot_data, r.visual_data, r.latest_values, r.diagnostics, r.error_summary, r.attempt_count, r.next_attempt_at, r.claim_token, r.lease_expires_at, r.started_at, r.finished_at, r.created_at, r.updated_at";
const DEFINITION_COLUMNS: &str = "d.id, d.agent_key, d.name, d.description, ARRAY(SELECT timeframe.timeframe FROM agent_indicator_definition_timeframes timeframe WHERE timeframe.indicator_definition_id = d.id ORDER BY CASE right(timeframe.timeframe, 1) WHEN 'm' THEN 60 WHEN 'h' THEN 3600 WHEN 'd' THEN 86400 END * left(timeframe.timeframe, -1)::bigint, timeframe.timeframe) AS timeframes, d.enabled, d.active_version_id, d.created_at, d.updated_at";
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
    pub timeframes: Vec<String>,
    pub enabled: bool,
    pub instrument_ids: Vec<String>,
    pub version: NewIndicatorVersion,
}

#[derive(Debug, Clone)]
pub struct UpdateIndicatorDefinition {
    pub expected_active_version_id: Uuid,
    pub name: String,
    pub description: String,
    pub timeframes: Vec<String>,
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

pub struct PersistedIndicatorOutput {
    pub candle_data: Value,
    pub plot_data: Value,
    pub visual_data: Value,
    pub latest_values: Value,
    pub diagnostics: Value,
}

#[derive(Debug, sqlx::FromRow)]
pub struct IndicatorQueueHealth {
    pub ready_count: i64,
    pub oldest_ready_age_seconds: Option<f64>,
    pub running_count: i64,
    pub retry_count: i64,
    pub timed_out_dependency_count: i64,
}

pub struct FrozenDependencyResultFilter<'a> {
    pub timeframe: Option<&'a str>,
    pub instrument_id: Option<&'a str>,
    pub run_id: Option<Uuid>,
    pub limit: i64,
}

fn source_sha256(source: &str) -> String {
    hex::encode(Sha256::digest(source.as_bytes()))
}

pub fn normalize_timeframes(timeframes: &[String]) -> Result<Vec<String>> {
    ensure!(
        (1..=8).contains(&timeframes.len()),
        "indicator must have between 1 and 8 timeframes"
    );
    let mut normalized = timeframes
        .iter()
        .map(|timeframe| {
            let timeframe = timeframe.trim().to_string();
            let seconds = parse_timeframe_seconds(&timeframe)
                .with_context(|| format!("invalid indicator timeframe {timeframe:?}"))?;
            Ok((seconds, timeframe))
        })
        .collect::<Result<Vec<_>>>()?;
    let distinct: std::collections::HashSet<_> = normalized
        .iter()
        .map(|(_, timeframe)| timeframe.as_str())
        .collect();
    ensure!(
        distinct.len() == normalized.len(),
        "indicator timeframes must be unique"
    );
    normalized.sort();
    Ok(normalized
        .into_iter()
        .map(|(_, timeframe)| timeframe)
        .collect())
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

async fn replace_timeframes(
    tx: &mut Transaction<'_, Postgres>,
    definition_id: Uuid,
    timeframes: &[String],
) -> Result<()> {
    sqlx::query(
        "DELETE FROM agent_indicator_definition_timeframes WHERE indicator_definition_id = $1",
    )
    .bind(definition_id)
    .execute(&mut **tx)
    .await?;
    for timeframe in timeframes {
        sqlx::query("INSERT INTO agent_indicator_definition_timeframes (indicator_definition_id, timeframe) VALUES ($1, $2)")
            .bind(definition_id)
            .bind(timeframe)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

pub async fn create_definition_with_initial_version(
    pool: &DbPool,
    agent_key: &str,
    input: &CreateIndicatorDefinition,
) -> Result<IndicatorDefinition> {
    let timeframes = normalize_timeframes(&input.timeframes)?;
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
    sqlx::query("INSERT INTO agent_indicator_definitions (id, agent_key, name, description, enabled) SELECT $1, agent_key, $2, $3, $4 FROM agents WHERE agent_key = $5")
        .bind(definition_id).bind(input.name.trim()).bind(input.description.trim()).bind(input.enabled).bind(agent_key).execute(&mut *tx).await.context("failed to insert indicator definition")?;
    let version = insert_version(&mut tx, definition_id, 1, &input.version).await?;
    replace_targets(&mut tx, definition_id, &targets).await?;
    replace_timeframes(&mut tx, definition_id, &timeframes).await?;
    sqlx::query("UPDATE agent_indicator_definitions SET active_version_id = $1, updated_at = now() WHERE id = $2 AND agent_key = $3")
        .bind(version.id).bind(definition_id).bind(agent_key).execute(&mut *tx).await.context("failed to activate initial indicator version")?;
    let definition: IndicatorDefinition = sqlx::query_as(AssertSqlSafe(format!("SELECT {DEFINITION_COLUMNS} FROM agent_indicator_definitions d WHERE d.id = $1 AND d.agent_key = $2")))
        .bind(definition_id).bind(agent_key).fetch_one(&mut *tx).await.context("failed to load created indicator definition")?;
    tx.commit()
        .await
        .context("failed to commit indicator create transaction")?;
    enqueue_immediate_runs(pool, agent_key, definition.id).await?;
    Ok(definition)
}

pub async fn update_definition_with_new_version(
    pool: &DbPool,
    agent_key: &str,
    definition_id: Uuid,
    input: &UpdateIndicatorDefinition,
) -> Result<IndicatorUpdateResult> {
    let timeframes = normalize_timeframes(&input.timeframes)?;
    let mut tx = pool
        .begin()
        .await
        .context("failed to begin indicator update transaction")?;
    let existing: Option<(Uuid, i32)> = sqlx::query_as("SELECT active_version_id, (SELECT version_number FROM agent_indicator_versions WHERE id = active_version_id) FROM agent_indicator_definitions WHERE id = $1 AND agent_key = $2 AND archived_at IS NULL FOR UPDATE").bind(definition_id).bind(agent_key).fetch_optional(&mut *tx).await?;
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
    replace_timeframes(&mut tx, definition_id, &timeframes).await?;
    sqlx::query("UPDATE agent_indicator_definitions SET name = $1, description = $2, enabled = $3, active_version_id = $4, updated_at = now() WHERE id = $5 AND agent_key = $6").bind(input.name.trim()).bind(input.description.trim()).bind(input.enabled).bind(version.id).bind(definition_id).bind(agent_key).execute(&mut *tx).await?;
    tx.commit()
        .await
        .context("failed to commit indicator update transaction")?;
    enqueue_immediate_runs(pool, agent_key, definition_id).await?;
    Ok(IndicatorUpdateResult::Updated)
}

async fn enqueue_immediate_runs(pool: &DbPool, agent_key: &str, definition_id: Uuid) -> Result<()> {
    let definition = get_definition(pool, agent_key, definition_id)
        .await?
        .ok_or_else(|| anyhow!("indicator definition disappeared after activation"))?;
    if !definition.enabled {
        return Ok(());
    }
    let version_id = definition
        .active_version_id
        .ok_or_else(|| anyhow!("activated indicator has no active version"))?;
    let instruments = list_definition_instruments(pool, agent_key, definition_id).await?;
    for timeframe in &definition.timeframes {
        let Some(due_at) =
            latest_due_at_or_before(Utc::now(), timeframe, DEFAULT_TRIGGER_DELAY_SECONDS)?
        else {
            continue;
        };
        let boundary = boundary_for_due_at(due_at, DEFAULT_TRIGGER_DELAY_SECONDS);
        for instrument_id in &instruments {
            enqueue_run(
                pool,
                agent_key,
                definition_id,
                version_id,
                instrument_id,
                timeframe,
                boundary,
            )
            .await?;
        }
    }
    Ok(())
}

pub async fn list_definitions(pool: &DbPool, agent_key: &str) -> Result<Vec<IndicatorDefinition>> {
    sqlx::query_as(AssertSqlSafe(format!("SELECT {DEFINITION_COLUMNS} FROM agent_indicator_definitions d WHERE d.agent_key = $1 AND d.archived_at IS NULL ORDER BY d.name"))).bind(agent_key).fetch_all(pool).await.context("failed to list indicator definitions")
}

/// Lists only targets still selected in the agent's active analysis universe.
pub async fn list_enabled_targets(pool: &DbPool) -> Result<Vec<ScheduledIndicatorTarget>> {
    sqlx::query_as("SELECT d.agent_key, d.id AS definition_id, d.active_version_id AS version_id, target.instrument_id, timeframe.timeframe FROM agent_indicator_definitions d JOIN agent_indicator_definition_timeframes timeframe ON timeframe.indicator_definition_id = d.id JOIN agent_indicator_definition_instruments target ON target.indicator_definition_id = d.id JOIN agent_analysis_instruments selected ON selected.agent_key = d.agent_key AND selected.instrument_id = target.instrument_id JOIN hyperliquid.instruments instruments ON instruments.instrument_id = target.instrument_id WHERE d.enabled AND d.archived_at IS NULL AND instruments.active ORDER BY d.agent_key, d.id, CASE right(timeframe.timeframe, 1) WHEN 'm' THEN 60 WHEN 'h' THEN 3600 WHEN 'd' THEN 86400 END * left(timeframe.timeframe, -1)::bigint, timeframe.timeframe, target.instrument_id")
        .fetch_all(pool)
        .await
        .context("failed to list enabled indicator targets")
}
pub async fn get_definition(
    pool: &DbPool,
    agent_key: &str,
    definition_id: Uuid,
) -> Result<Option<IndicatorDefinition>> {
    sqlx::query_as(AssertSqlSafe(format!("SELECT {DEFINITION_COLUMNS} FROM agent_indicator_definitions d WHERE d.agent_key = $1 AND d.id = $2 AND d.archived_at IS NULL"))).bind(agent_key).bind(definition_id).fetch_optional(pool).await.context("failed to get indicator definition")
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
pub async fn list_latest_results_for_timeframe(
    pool: &DbPool,
    agent_key: &str,
    definition_id: Uuid,
    timeframe: &str,
    instrument_id: Option<&str>,
    limit: i64,
) -> Result<Vec<IndicatorRun>> {
    sqlx::query_as(AssertSqlSafe(format!("SELECT {RUN_COLUMNS} FROM agent_indicator_runs WHERE agent_key = $1 AND indicator_definition_id = $2 AND timeframe = $3 AND ($4::text IS NULL OR instrument_id = $4) ORDER BY scheduled_for DESC LIMIT $5"))).bind(agent_key).bind(definition_id).bind(timeframe).bind(instrument_id).bind(limit.clamp(1, 100)).fetch_all(pool).await.context("failed to list indicator results for timeframe")
}

pub async fn get_result_by_id(
    pool: &DbPool,
    agent_key: &str,
    definition_id: Uuid,
    timeframe: &str,
    instrument_id: Option<&str>,
    run_id: Uuid,
) -> Result<Option<IndicatorRun>> {
    sqlx::query_as(AssertSqlSafe(format!("SELECT {RUN_COLUMNS} FROM agent_indicator_runs WHERE agent_key = $1 AND indicator_definition_id = $2 AND timeframe = $3 AND ($4::text IS NULL OR instrument_id = $4) AND id = $5")))
        .bind(agent_key)
        .bind(definition_id)
        .bind(timeframe)
        .bind(instrument_id)
        .bind(run_id)
        .fetch_optional(pool)
        .await
        .context("failed to get exact indicator result")
}

pub async fn list_frozen_dependency_results(
    pool: &DbPool,
    analysis_run_id: i64,
    agent_key: &str,
    definition_id: Uuid,
    filter: FrozenDependencyResultFilter<'_>,
) -> Result<Vec<IndicatorRun>> {
    sqlx::query_as(AssertSqlSafe(format!("SELECT {RUN_COLUMNS_QUALIFIED} FROM harness_run_indicator_dependencies dependency JOIN harness_run_indicator_sets dependency_set ON dependency_set.analysis_run_id = dependency.analysis_run_id JOIN harness_sub_agent_runs analysis_run ON analysis_run.id = dependency.analysis_run_id JOIN agent_indicator_runs r ON r.id = dependency.indicator_run_id WHERE dependency.analysis_run_id = $1 AND analysis_run.agent_key = $2 AND dependency_set.frozen_at IS NOT NULL AND dependency.snapshot_status IN ('succeeded', 'failed', 'skipped') AND r.indicator_definition_id = $3 AND ($4::text IS NULL OR r.timeframe = $4) AND ($5::text IS NULL OR r.instrument_id = $5) AND ($6::uuid IS NULL OR r.id = $6) ORDER BY r.scheduled_for DESC LIMIT $7")))
        .bind(analysis_run_id)
        .bind(agent_key)
        .bind(definition_id)
        .bind(filter.timeframe)
        .bind(filter.instrument_id)
        .bind(filter.run_id)
        .bind(filter.limit.clamp(1, 100))
        .fetch_all(pool)
        .await
        .context("failed to list frozen Analysis indicator results")
}
pub async fn get_chart_run(
    pool: &DbPool,
    agent_key: &str,
    definition_id: Uuid,
    instrument_id: &str,
    timeframe: &str,
) -> Result<Option<IndicatorRun>> {
    sqlx::query_as(AssertSqlSafe(format!("SELECT {RUN_COLUMNS} FROM agent_indicator_runs WHERE agent_key = $1 AND indicator_definition_id = $2 AND instrument_id = $3 AND timeframe = $4 AND status = 'succeeded' ORDER BY scheduled_for DESC LIMIT 1"))).bind(agent_key).bind(definition_id).bind(instrument_id).bind(timeframe).fetch_optional(pool).await.context("failed to get indicator chart run")
}

pub async fn delete_definition(
    pool: &DbPool,
    agent_key: &str,
    definition_id: Uuid,
) -> Result<bool> {
    let result =
        sqlx::query("UPDATE agent_indicator_definitions SET enabled = false, archived_at = now(), updated_at = now() WHERE agent_key = $1 AND id = $2 AND archived_at IS NULL")
            .bind(agent_key)
            .bind(definition_id)
            .execute(pool)
            .await
            .context("failed to delete indicator definition")?;
    Ok(result.rows_affected() == 1)
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
    let inserted: Option<(Uuid,)> = sqlx::query_as("INSERT INTO agent_indicator_runs (id, agent_key, indicator_definition_id, indicator_version_id, instrument_id, timeframe, scheduled_for, status) SELECT $1, d.agent_key, d.id, $3, $4, $5, $6, 'queued' FROM agent_indicator_definitions d JOIN agent_indicator_definition_instruments i ON i.indicator_definition_id = d.id WHERE d.id = $2 AND d.agent_key = $7 AND d.enabled AND d.archived_at IS NULL AND i.instrument_id = $4 ON CONFLICT (indicator_version_id, instrument_id, timeframe, scheduled_for) DO NOTHING RETURNING id").bind(id).bind(definition_id).bind(version_id).bind(instrument_id).bind(timeframe).bind(scheduled_for).bind(agent_key).fetch_optional(pool).await.context("failed to enqueue indicator run")?;
    if inserted.is_some() {
        super::coordination::notify_queue();
    }
    Ok(inserted.map(|(id,)| id))
}
pub async fn claim_next_queued_run(
    pool: &DbPool,
    excluded_agent_keys: &[String],
) -> Result<Option<IndicatorRun>> {
    let claim_token = Uuid::new_v4();
    sqlx::query_as(AssertSqlSafe(format!("WITH next AS (SELECT id FROM agent_indicator_runs WHERE status = 'queued' AND next_attempt_at <= now() AND NOT (agent_key = ANY($1)) ORDER BY scheduled_for, created_at FOR UPDATE SKIP LOCKED LIMIT 1) UPDATE agent_indicator_runs r SET status = 'running', attempt_count = attempt_count + 1, claim_token = $2, lease_expires_at = now() + interval '60 seconds', started_at = now(), finished_at = NULL, updated_at = now() FROM next WHERE r.id = next.id RETURNING {RUN_COLUMNS_QUALIFIED}"))) .bind(excluded_agent_keys).bind(claim_token).fetch_optional(pool).await.context("failed to claim indicator run")
}

pub async fn renew_run_lease(pool: &DbPool, run_id: Uuid, claim_token: Uuid) -> Result<bool> {
    let result = sqlx::query("UPDATE agent_indicator_runs SET lease_expires_at = now() + interval '60 seconds', updated_at = now() WHERE id = $1 AND claim_token = $2 AND status = 'running'")
        .bind(run_id)
        .bind(claim_token)
        .execute(pool)
        .await
        .context("failed to renew indicator run lease")?;
    Ok(result.rows_affected() == 1)
}

pub async fn release_owned_claim(pool: &DbPool, run_id: Uuid, claim_token: Uuid) -> Result<bool> {
    let result = sqlx::query("UPDATE agent_indicator_runs SET status = 'queued', attempt_count = GREATEST(0, attempt_count - 1), claim_token = NULL, lease_expires_at = NULL, started_at = NULL, finished_at = NULL, updated_at = now() WHERE id = $1 AND claim_token = $2 AND status = 'running'")
        .bind(run_id)
        .bind(claim_token)
        .execute(pool)
        .await
        .context("failed to release indicator run claim")?;
    if result.rows_affected() == 1 {
        super::coordination::notify_queue();
    }
    Ok(result.rows_affected() == 1)
}

pub async fn next_queued_attempt_at(pool: &DbPool) -> Result<Option<DateTime<Utc>>> {
    sqlx::query_scalar("SELECT min(next_attempt_at) FROM agent_indicator_runs WHERE status = 'queued' AND next_attempt_at > now()")
        .fetch_one(pool)
        .await
        .context("failed to load next indicator retry deadline")
}

pub fn retry_delay(attempt_count: i32) -> Duration {
    let exponent = u32::try_from(attempt_count.saturating_sub(1))
        .unwrap_or(0)
        .min(5);
    Duration::seconds((1_i64 << exponent).min(30))
}

pub async fn requeue_retryable_run(
    pool: &DbPool,
    run_id: Uuid,
    claim_token: Uuid,
    attempt_count: i32,
    error_summary: &str,
    max_attempts: i32,
) -> Result<bool> {
    let result = sqlx::query("UPDATE agent_indicator_runs SET status = 'queued', diagnostics = '[]'::jsonb, error_summary = $1, next_attempt_at = now() + make_interval(secs => $2), claim_token = NULL, lease_expires_at = NULL, started_at = NULL, updated_at = now() WHERE id = $3 AND claim_token = $4 AND status = 'running' AND attempt_count < $5")
        .bind(error_summary)
        .bind(retry_delay(attempt_count).num_seconds() as f64)
        .bind(run_id)
        .bind(claim_token)
        .bind(max_attempts)
        .execute(pool)
        .await
        .context("failed to requeue retryable indicator run")?;
    Ok(result.rows_affected() == 1)
}

pub async fn recover_stale_running_runs(pool: &DbPool, max_attempts: i32) -> Result<u64> {
    let result = sqlx::query("UPDATE agent_indicator_runs SET status = CASE WHEN attempt_count < $1 THEN 'queued' ELSE 'failed' END, error_summary = CASE WHEN attempt_count < $1 THEN 'indicator execution lease expired; retrying' ELSE 'indicator execution lease expired after maximum retries' END, next_attempt_at = CASE WHEN attempt_count < $1 THEN now() + make_interval(secs => LEAST(30, (1 << LEAST(5, GREATEST(0, attempt_count - 1))))) ELSE next_attempt_at END, claim_token = NULL, lease_expires_at = NULL, started_at = NULL, finished_at = CASE WHEN attempt_count < $1 THEN NULL ELSE now() END, updated_at = now() WHERE status = 'running' AND lease_expires_at <= now()")
        .bind(max_attempts)
        .execute(pool)
        .await
        .context("failed to recover stale indicator runs")?;
    Ok(result.rows_affected())
}

pub async fn indicator_queue_health(pool: &DbPool) -> Result<IndicatorQueueHealth> {
    sqlx::query_as(
        "SELECT
             (SELECT count(*) FROM agent_indicator_runs WHERE status = 'queued' AND next_attempt_at <= now()) AS ready_count,
             (SELECT EXTRACT(EPOCH FROM (now() - min(next_attempt_at)))::float8 FROM agent_indicator_runs WHERE status = 'queued' AND next_attempt_at <= now()) AS oldest_ready_age_seconds,
             (SELECT count(*) FROM agent_indicator_runs WHERE status = 'running') AS running_count,
             (SELECT count(*) FROM agent_indicator_runs WHERE status = 'queued' AND attempt_count > 0) AS retry_count,
             (SELECT count(*) FROM harness_run_indicator_dependencies WHERE snapshot_status = 'timed_out') AS timed_out_dependency_count",
    )
    .fetch_one(pool)
    .await
    .context("failed to load indicator queue health")
}
pub async fn finish_run_succeeded(
    pool: &DbPool,
    run_id: Uuid,
    claim_token: Uuid,
    output: PersistedIndicatorOutput,
) -> Result<bool> {
    let result = sqlx::query("UPDATE agent_indicator_runs SET status = 'succeeded', candle_data = $1, plot_data = $2, visual_data = $3, latest_values = $4, diagnostics = $5, error_summary = NULL, claim_token = NULL, lease_expires_at = NULL, finished_at = now(), updated_at = now() WHERE id = $6 AND claim_token = $7 AND status = 'running'").bind(output.candle_data).bind(output.plot_data).bind(output.visual_data).bind(output.latest_values).bind(output.diagnostics).bind(run_id).bind(claim_token).execute(pool).await?;
    Ok(result.rows_affected() == 1)
}
pub async fn finish_run_failed(
    pool: &DbPool,
    run_id: Uuid,
    claim_token: Uuid,
    diagnostics: Value,
    error_summary: &str,
) -> Result<bool> {
    let result = sqlx::query("UPDATE agent_indicator_runs SET status = 'failed', diagnostics = $1, error_summary = $2, claim_token = NULL, lease_expires_at = NULL, finished_at = now(), updated_at = now() WHERE id = $3 AND claim_token = $4 AND status = 'running'").bind(diagnostics).bind(error_summary).bind(run_id).bind(claim_token).execute(pool).await?;
    Ok(result.rows_affected() == 1)
}
pub async fn cleanup_expired_runs(pool: &DbPool, now: DateTime<Utc>, limit: i64) -> Result<u64> {
    let cutoff = now - Duration::days(30);
    let result = sqlx::query("DELETE FROM agent_indicator_runs WHERE id IN (SELECT r.id FROM agent_indicator_runs r WHERE r.finished_at < $1 AND NOT EXISTS (SELECT 1 FROM harness_run_indicator_dependencies dependency WHERE dependency.indicator_run_id = r.id) AND NOT (r.status = 'succeeded' AND NOT EXISTS (SELECT 1 FROM agent_indicator_runs newer WHERE newer.indicator_definition_id = r.indicator_definition_id AND newer.instrument_id = r.instrument_id AND newer.timeframe = r.timeframe AND newer.status = 'succeeded' AND newer.scheduled_for > r.scheduled_for)) ORDER BY r.finished_at LIMIT $2)").bind(cutoff).bind(limit.clamp(1, 1000)).execute(pool).await.context("failed to clean up expired indicator runs")?;
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
            timeframes: vec!["1h".to_string()],
            enabled: true,
            instrument_ids: vec!["BTC".to_string()],
            version: version(),
        }
    }

    #[test]
    fn timeframe_normalization_sorts_trims_and_rejects_invalid_sets() {
        assert_eq!(
            normalize_timeframes(&[" 4h ".to_string(), "15m".to_string(), "1h".to_string()])
                .expect("normalize timeframes"),
            ["15m", "1h", "4h"]
        );
        assert!(normalize_timeframes(&[]).is_err());
        assert!(normalize_timeframes(&["1h".to_string(), " 1h ".to_string()]).is_err());
        assert!(normalize_timeframes(&vec!["1m".to_string(); 9]).is_err());
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
            timeframes: vec!["15m".to_string(), "4h".to_string()],
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
        let stored = get_definition(&pool, "indicator-update", created.id)
            .await
            .expect("load updated definition")
            .expect("updated definition");
        assert_eq!(stored.timeframes, ["15m", "4h"]);
        let targets = list_enabled_targets(&pool)
            .await
            .expect("list cross-product targets")
            .into_iter()
            .filter(|target| target.definition_id == created.id)
            .collect::<Vec<_>>();
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].timeframe, "15m");
        assert_eq!(targets[1].timeframe, "4h");
        let mut stale_update = update.clone();
        stale_update.timeframes = vec!["1d".to_string()];
        assert_eq!(
            update_definition_with_new_version(
                &pool,
                "indicator-update",
                created.id,
                &stale_update,
            )
            .await
            .expect("stale update"),
            IndicatorUpdateResult::VersionConflict
        );
        assert_eq!(
            get_definition(&pool, "indicator-update", created.id)
                .await
                .expect("load after conflict")
                .expect("definition after conflict")
                .timeframes,
            ["15m", "4h"]
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
        let first = claim_next_queued_run(&pool, &[])
            .await
            .expect("claim")
            .expect("first run");
        assert!(
            claim_next_queued_run(&pool, &[])
                .await
                .expect("claim second")
                .is_some()
        );
        assert!(
            finish_run_succeeded(
                &pool,
                first.id,
                first.claim_token.expect("first claim token"),
                PersistedIndicatorOutput {
                    candle_data: json!([]),
                    plot_data: json!([]),
                    visual_data: json!({"version": 1, "markers": []}),
                    latest_values: json!({}),
                    diagnostics: json!([]),
                }
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
                second.id,
                second.claim_token.expect("second claim token"),
                PersistedIndicatorOutput {
                    candle_data: json!([]),
                    plot_data: json!([]),
                    visual_data: json!({
                        "version": 1,
                        "markers": [
                            {
                                "kind": "plotshape", "bar_index": 0, "value": 1.0,
                                "title": "Buy", "text": "BUY", "style": "triangleup",
                                "location": "belowbar", "color": null, "text_color": null,
                                "size": "normal", "offset": 0
                            },
                            {
                                "kind": "plotchar", "bar_index": 0, "value": 1.0,
                                "title": "Stage", "character": "1", "text": "",
                                "location": "belowbar", "color": null, "text_color": null,
                                "size": "normal", "offset": 0
                            },
                            {
                                "kind": "plotarrow", "bar_index": 0, "value": -2.0,
                                "title": "Momentum", "color_up": null, "color_down": null,
                                "min_height": 5.0, "max_height": 100.0, "offset": 0
                            }
                        ]
                    }),
                    latest_values: json!({}),
                    diagnostics: json!([]),
                }
            )
            .await
            .expect("finish second")
        );
        let stored = list_latest_results(&pool, "indicator-runs", definition.id, 2)
            .await
            .expect("list persisted runs")
            .into_iter()
            .find(|run| run.id == second.id)
            .expect("persisted second run");
        assert_eq!(
            stored.visual_data,
            Some(json!({
                "version": 1,
                "markers": [
                    {
                        "kind": "plotshape", "bar_index": 0, "value": 1.0,
                        "title": "Buy", "text": "BUY", "style": "triangleup",
                        "location": "belowbar", "color": null, "text_color": null,
                        "size": "normal", "offset": 0
                    },
                    {
                        "kind": "plotchar", "bar_index": 0, "value": 1.0,
                        "title": "Stage", "character": "1", "text": "",
                        "location": "belowbar", "color": null, "text_color": null,
                        "size": "normal", "offset": 0
                    },
                    {
                        "kind": "plotarrow", "bar_index": 0, "value": -2.0,
                        "title": "Momentum", "color_up": null, "color_down": null,
                        "min_height": 5.0, "max_height": 100.0, "offset": 0
                    }
                ]
            }))
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
            2
        );
    }

    #[tokio::test]
    async fn retry_deadline_and_claim_token_fence_attempts() {
        let pool = test_db::pool().await;
        seed_agent(&pool, "indicator-fencing").await;
        create_definition_with_initial_version(&pool, "indicator-fencing", &definition())
            .await
            .expect("create definition");
        let first = claim_next_queued_run(&pool, &[])
            .await
            .expect("claim first attempt")
            .expect("queued run");
        let first_token = first.claim_token.expect("first claim token");
        assert!(
            requeue_retryable_run(
                &pool,
                first.id,
                first_token,
                first.attempt_count,
                "temporary upstream failure",
                3,
            )
            .await
            .expect("schedule retry")
        );
        assert!(
            claim_next_queued_run(&pool, &[])
                .await
                .expect("check retry deadline")
                .is_none()
        );
        let persisted_deadline: DateTime<Utc> =
            sqlx::query_scalar("SELECT next_attempt_at FROM agent_indicator_runs WHERE id = $1")
                .bind(first.id)
                .fetch_one(&pool)
                .await
                .expect("load persisted retry deadline");
        assert_eq!(
            next_queued_attempt_at(&pool)
                .await
                .expect("load scheduler retry deadline"),
            Some(persisted_deadline)
        );

        sqlx::query("UPDATE agent_indicator_runs SET next_attempt_at = now() WHERE id = $1")
            .bind(first.id)
            .execute(&pool)
            .await
            .expect("make retry ready");
        let second = claim_next_queued_run(&pool, &[])
            .await
            .expect("claim second attempt")
            .expect("ready retry");
        assert_ne!(second.claim_token, Some(first_token));
        assert!(
            !finish_run_failed(&pool, first.id, first_token, json!([]), "stale attempt")
                .await
                .expect("fenced stale completion")
        );
        sqlx::query("UPDATE agent_indicator_runs SET attempt_count = 3, lease_expires_at = now() - interval '1 second' WHERE id = $1")
            .bind(second.id)
            .execute(&pool)
            .await
            .expect("expire final attempt");
        assert_eq!(
            recover_stale_running_runs(&pool, 3)
                .await
                .expect("recover final attempt"),
            1
        );
        assert!(
            !finish_run_failed(
                &pool,
                second.id,
                second.claim_token.expect("second claim token"),
                json!([]),
                "expired attempt",
            )
            .await
            .expect("fence expired attempt")
        );
        let stored = list_latest_results(
            &pool,
            "indicator-fencing",
            second.indicator_definition_id,
            1,
        )
        .await
        .expect("load recovered run")
        .pop()
        .expect("recovered run");
        assert_eq!(stored.status, "failed");
        assert!(stored.claim_token.is_none());
    }

    #[tokio::test]
    async fn claim_exclusions_make_each_pass_fair_between_agents() {
        let pool = test_db::pool().await;
        seed_agent(&pool, "indicator-fair-a").await;
        seed_agent(&pool, "indicator-fair-b").await;
        create_definition_with_initial_version(&pool, "indicator-fair-a", &definition())
            .await
            .expect("create first definition");
        create_definition_with_initial_version(&pool, "indicator-fair-b", &definition())
            .await
            .expect("create second definition");

        let first = claim_next_queued_run(&pool, &[])
            .await
            .expect("first claim")
            .expect("first run");
        let second = claim_next_queued_run(&pool, std::slice::from_ref(&first.agent_key))
            .await
            .expect("second claim")
            .expect("second run");
        assert_ne!(first.agent_key, second.agent_key);
    }

    #[tokio::test]
    async fn releasing_an_owned_claim_is_fenced_and_restores_the_attempt() {
        let pool = test_db::pool().await;
        seed_agent(&pool, "indicator-release").await;
        create_definition_with_initial_version(&pool, "indicator-release", &definition())
            .await
            .expect("create definition");
        let claimed = claim_next_queued_run(&pool, &[])
            .await
            .expect("claim run")
            .expect("queued run");
        let token = claimed.claim_token.expect("claim token");
        assert!(
            release_owned_claim(&pool, claimed.id, token)
                .await
                .expect("release claim")
        );
        assert!(
            !release_owned_claim(&pool, claimed.id, token)
                .await
                .expect("fence stale release")
        );
        let restored = get_result_by_id(
            &pool,
            "indicator-release",
            claimed.indicator_definition_id,
            "1h",
            None,
            claimed.id,
        )
        .await
        .expect("load released run")
        .expect("released run");
        assert_eq!(restored.status, "queued");
        assert_eq!(restored.attempt_count, 0);
        assert!(restored.claim_token.is_none());
    }

    #[tokio::test]
    async fn successful_completion_clears_a_stale_retry_error() {
        let pool = test_db::pool().await;
        seed_agent(&pool, "indicator-clear-error").await;
        create_definition_with_initial_version(&pool, "indicator-clear-error", &definition())
            .await
            .expect("create definition");
        let claimed = claim_next_queued_run(&pool, &[])
            .await
            .expect("claim run")
            .expect("queued run");
        sqlx::query("UPDATE agent_indicator_runs SET error_summary = 'old retry' WHERE id = $1")
            .bind(claimed.id)
            .execute(&pool)
            .await
            .expect("seed stale error");
        assert!(
            finish_run_succeeded(
                &pool,
                claimed.id,
                claimed.claim_token.expect("claim token"),
                PersistedIndicatorOutput {
                    candle_data: json!([]),
                    plot_data: json!({}),
                    visual_data: json!({"version": 1, "markers": []}),
                    latest_values: json!({}),
                    diagnostics: json!([]),
                },
            )
            .await
            .expect("finish run")
        );
        let error_summary: Option<String> =
            sqlx::query_scalar("SELECT error_summary FROM agent_indicator_runs WHERE id = $1")
                .bind(claimed.id)
                .fetch_one(&pool)
                .await
                .expect("load completed run error");
        assert!(error_summary.is_none());
    }
}
