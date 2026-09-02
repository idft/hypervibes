use anyhow::{Context, Result, bail};
use chrono::{Duration, Utc};
use sha2::{Digest, Sha256};
use sqlx::query_as;
use uuid::Uuid;

use crate::{
    db::DbPool,
    harness::model::{CAPABILITY_SCHEMA_VERSION, RunApiScope, run_api_scopes_for_sub_agent},
};

const RUN_CREDENTIAL_GRACE_SECONDS: i64 = 300;
const MAX_RUNTIME_TOKEN_BYTES: usize = 512;

/// A plaintext runtime token exists only while a run workspace is being
/// materialized. Its database row stores a one-way digest instead.
#[derive(Clone)]
pub struct IssuedRunRuntimeCredential {
    pub credential_id: Uuid,
    pub token: String,
}

#[derive(Debug, Clone)]
pub struct AuthenticatedRunRuntimeCredential {
    pub agent_key: String,
    pub run_id: i64,
    pub capability_schema_version: i32,
    pub api_scopes: Vec<RunApiScope>,
}

#[derive(sqlx::FromRow)]
struct RunCredentialIssueRow {
    timeout_seconds: i32,
    sub_agent_kind: String,
    capability_schema_version: i32,
    #[sqlx(json)]
    enabled_capabilities: Vec<String>,
}

#[derive(sqlx::FromRow)]
struct RunCredentialAuthRow {
    agent_key: String,
    run_id: i64,
    capability_schema_version: i32,
    api_scopes: serde_json::Value,
}

pub async fn issue_run_runtime_credential(
    pool: &DbPool,
    agent_key: &str,
    run_id: i64,
) -> Result<IssuedRunRuntimeCredential> {
    let mut tx = pool
        .begin()
        .await
        .context("failed to begin run runtime credential issue")?;
    let row: Option<RunCredentialIssueRow> = query_as(
        "SELECT runs.timeout_seconds,
                runs.sub_agent_kind,
                 artifacts.capability_snapshot AS enabled_capabilities,
                artifacts.capability_schema_version
           FROM harness_sub_agent_runs AS runs
           JOIN harness_run_workspace_artifacts AS artifacts
             ON artifacts.run_id = runs.id
          WHERE runs.id = $1
            AND runs.agent_key = $2
            AND runs.status = 'running'
            AND artifacts.workspace_status IN ('preparing', 'ready')
          FOR UPDATE OF runs, artifacts",
    )
    .bind(run_id)
    .bind(agent_key)
    .fetch_optional(&mut *tx)
    .await
    .context("failed to lock run runtime credential issue")?;
    let Some(row) = row else {
        bail!("run is not eligible for a runtime credential");
    };
    if row.capability_schema_version != CAPABILITY_SCHEMA_VERSION {
        bail!("run has an unsupported capability schema version");
    }
    let api_scopes = run_api_scopes_for_sub_agent(&row.sub_agent_kind, &row.enabled_capabilities)?;
    let api_scope_snapshot = serde_json::Value::Array(
        api_scopes
            .iter()
            .map(|scope| serde_json::Value::String(scope.as_str().to_string()))
            .collect(),
    );

    let credential_id = Uuid::new_v4();
    let token = format!("vtr_{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let timeout_seconds = i64::from(row.timeout_seconds.max(1));
    let expires_at = Utc::now() + Duration::seconds(timeout_seconds + RUN_CREDENTIAL_GRACE_SECONDS);
    sqlx::query(
        "INSERT INTO harness_run_runtime_credentials (
             run_id, agent_key, credential_id, token_hash,
             capability_schema_version, api_scopes, expires_at
         ) VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (run_id) DO UPDATE
             SET agent_key = EXCLUDED.agent_key,
                 credential_id = EXCLUDED.credential_id,
                 token_hash = EXCLUDED.token_hash,
                 capability_schema_version = EXCLUDED.capability_schema_version,
                 api_scopes = EXCLUDED.api_scopes,
                 issued_at = now(),
                 expires_at = EXCLUDED.expires_at,
                 revoked_at = NULL",
    )
    .bind(run_id)
    .bind(agent_key)
    .bind(credential_id)
    .bind(token_hash(&token))
    .bind(row.capability_schema_version)
    .bind(api_scope_snapshot)
    .bind(expires_at)
    .execute(&mut *tx)
    .await
    .context("failed to persist run runtime credential")?;
    tx.commit()
        .await
        .context("failed to commit run runtime credential")?;

    Ok(IssuedRunRuntimeCredential {
        credential_id,
        token,
    })
}

pub async fn revoke_run_runtime_credential(
    pool: &DbPool,
    agent_key: &str,
    run_id: i64,
) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE harness_run_runtime_credentials
            SET revoked_at = COALESCE(revoked_at, now())
          WHERE run_id = $1
            AND agent_key = $2",
    )
    .bind(run_id)
    .bind(agent_key)
    .execute(pool)
    .await
    .context("failed to revoke run runtime credential")?;
    Ok(result.rows_affected() > 0)
}

pub async fn authenticate_run_runtime_credential(
    pool: &DbPool,
    token: &str,
) -> Result<Option<AuthenticatedRunRuntimeCredential>> {
    if token.is_empty() || token.len() > MAX_RUNTIME_TOKEN_BYTES {
        return Ok(None);
    }
    let row: Option<RunCredentialAuthRow> = query_as(
        "SELECT credentials.agent_key,
                 credentials.run_id,
                 credentials.capability_schema_version,
                credentials.api_scopes
           FROM harness_run_runtime_credentials AS credentials
           JOIN harness_sub_agent_runs AS runs
             ON runs.id = credentials.run_id
            AND runs.agent_key = credentials.agent_key
           JOIN harness_run_workspace_artifacts AS artifacts
             ON artifacts.run_id = credentials.run_id
          WHERE credentials.token_hash = $1
            AND credentials.revoked_at IS NULL
            AND credentials.expires_at > now()
            AND runs.status IN ('queued', 'running')
            AND artifacts.workspace_status = 'ready'",
    )
    .bind(token_hash(token))
    .fetch_optional(pool)
    .await
    .context("failed to authenticate run runtime credential")?;
    row.map(|row| {
        let api_scopes = parse_api_scope_snapshot(&row.api_scopes)?;
        Ok(AuthenticatedRunRuntimeCredential {
            agent_key: row.agent_key,
            run_id: row.run_id,
            capability_schema_version: row.capability_schema_version,
            api_scopes,
        })
    })
    .transpose()
}

fn parse_api_scope_snapshot(value: &serde_json::Value) -> Result<Vec<RunApiScope>> {
    let values = value
        .as_array()
        .context("run runtime credential API scopes are not an array")?;
    let mut scopes = Vec::with_capacity(values.len());
    for value in values {
        let value = value
            .as_str()
            .context("run runtime credential API scope is not a string")?;
        let scope = RunApiScope::parse(value).ok_or_else(|| {
            anyhow::anyhow!("run runtime credential has an unsupported API scope")
        })?;
        if scopes.contains(&scope) {
            bail!("run runtime credential has a duplicate API scope");
        }
        scopes.push(scope);
    }
    Ok(scopes)
}

fn token_hash(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::token_hash;

    #[test]
    fn token_digest_is_deterministic_and_non_reversible_in_shape() {
        let digest = token_hash("vtr_example");
        assert_eq!(digest.len(), 64);
        assert_ne!(digest, "vtr_example");
        assert_eq!(digest, token_hash("vtr_example"));
    }
}
