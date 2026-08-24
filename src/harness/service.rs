use anyhow::{Context, Result, anyhow};
use sqlx::query_as;

use crate::{
    agents::store::get_agent,
    db::DbPool,
    harness::{
        store::{ACTIVE_STATUSES, lock_agent_coordination_tx},
        workspace_lease::WorkspaceLeaseManager,
    },
    opencode::{
        client::{DeleteSessionResult, OpenCodeClient, SessionStatusKind},
        workspace::OpenCodeWorkspaceRuntimeConfig,
    },
};

pub enum DeleteJobOutcome {
    Deleted,
    Missing,
}

/// Delete an idle job only after OpenCode has removed every mirrored terminal
/// session. OpenCode owns that cleanup, so this code never mutates its schema.
pub async fn delete_idle_job(
    pool: &DbPool,
    client: &OpenCodeClient,
    workspace_leases: &WorkspaceLeaseManager,
    base_url: &str,
    agent_key: &str,
    sub_agent_id: i64,
) -> Result<DeleteJobOutcome> {
    let agent = get_agent(pool, agent_key)
        .await?
        .ok_or_else(|| anyhow!("agent not found"))?;
    let runtime = OpenCodeWorkspaceRuntimeConfig::from_value(&agent.runtime_config)
        .ok_or_else(|| anyhow!("agent is missing OpenCode workspace metadata"))?;
    let _workspace_lease = workspace_leases.acquire_live_read(agent_key).await;
    let mut tx = pool.begin().await.context("failed to begin job deletion")?;
    lock_agent_coordination_tx(&mut tx, agent_key).await?;

    let job_exists: Option<(i64,)> =
        query_as("SELECT id FROM harness_sub_agents WHERE agent_key = $1 AND id = $2 FOR UPDATE")
            .bind(agent_key)
            .bind(sub_agent_id)
            .fetch_optional(&mut *tx)
            .await
            .context("failed to lock harness job for deletion")?;
    if job_exists.is_none() {
        tx.rollback().await?;
        return Ok(DeleteJobOutcome::Missing);
    }

    let runs: Vec<(String, Option<String>)> = query_as(
        "SELECT status, backend_run_ref
           FROM harness_sub_agent_runs
          WHERE agent_key = $1 AND sub_agent_id = $2
          FOR UPDATE",
    )
    .bind(agent_key)
    .bind(sub_agent_id)
    .fetch_all(&mut *tx)
    .await
    .context("failed to load harness job runs for deletion")?;
    if runs
        .iter()
        .any(|(status, _)| ACTIVE_STATUSES.contains(&status.as_str()))
    {
        anyhow::bail!("job has queued or running runs");
    }

    for (_, session_id) in &runs {
        let Some(session_id) = session_id.as_deref() else {
            continue;
        };
        let status = client
            .get_session_status_in_directory(
                base_url,
                session_id,
                Some(&runtime.workspace_container_path),
            )
            .await?;
        if status.as_ref().is_some_and(SessionStatusKind::is_active) {
            anyhow::bail!("job has an active OpenCode session");
        }
    }

    for (_, session_id) in &runs {
        let Some(session_id) = session_id.as_deref() else {
            continue;
        };
        match client
            .delete_session(base_url, &runtime.workspace_container_path, session_id)
            .await?
        {
            DeleteSessionResult::Deleted | DeleteSessionResult::NotFound => {}
        }
    }

    sqlx::query("DELETE FROM harness_sub_agents WHERE agent_key = $1 AND id = $2")
        .bind(agent_key)
        .bind(sub_agent_id)
        .execute(&mut *tx)
        .await
        .context("failed to delete idle harness job")?;
    tx.commit().await.context("failed to commit job deletion")?;
    Ok(DeleteJobOutcome::Deleted)
}
