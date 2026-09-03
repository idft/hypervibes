use anyhow::{Context, Result, anyhow};
use sqlx::query_as;

use crate::{
    agents::store::get_agent,
    db::DbPool,
    harness::{
        model::SUB_AGENT_KIND_ANALYSIS_CODING,
        store::{
            ACTIVE_STATUSES, analysis_coding_task_ids_for_agent_runs, lock_agent_coordination_tx,
        },
        workspace_lease::WorkspaceLeaseManager,
    },
    opencode::client::{DeleteSessionResult, OpenCodeClient, SessionStatusKind},
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
    container_workspaces_root: &str,
    agent_key: &str,
    sub_agent_id: i64,
) -> Result<DeleteJobOutcome> {
    get_agent(pool, agent_key)
        .await?
        .ok_or_else(|| anyhow!("agent not found"))?;
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

    let runs: Vec<(i64, String, Option<String>)> = query_as(
        "SELECT id, sub_agent_kind, backend_run_ref
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
        .any(|(_, status, _)| ACTIVE_STATUSES.contains(&status.as_str()))
    {
        anyhow::bail!("job has queued or running runs");
    }

    // Each session lives in its isolated workspace directory: normal runs in
    // their run workspace, coding runs in the candidate workspace owned by
    // their maintenance task. There is no agent workspace fallback.
    let coding_task_by_run_id = analysis_coding_task_ids_for_agent_runs(pool, agent_key).await?;
    let container_root = container_workspaces_root.trim_end_matches('/');
    let mut sessions = Vec::with_capacity(runs.len());
    for (run_id, sub_agent_kind, backend_run_ref) in &runs {
        let Some(session_id) = backend_run_ref.as_deref() else {
            continue;
        };
        let directory = if sub_agent_kind == SUB_AGENT_KIND_ANALYSIS_CODING {
            let task_id = coding_task_by_run_id
                .get(run_id)
                .ok_or_else(|| anyhow!("coding run {run_id} has no owning maintenance task"))?;
            format!("{container_root}/coding/{agent_key}/{task_id}/workspace")
        } else {
            format!("{container_root}/runs/{agent_key}/{run_id}/workspace")
        };
        sessions.push((session_id, directory));
    }

    for (session_id, directory) in &sessions {
        let status = client
            .get_session_status_in_directory(base_url, session_id, Some(directory))
            .await?;
        if status.as_ref().is_some_and(SessionStatusKind::is_active) {
            anyhow::bail!("job has an active OpenCode session");
        }
    }

    for (session_id, directory) in &sessions {
        match client
            .delete_session(base_url, directory, session_id)
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
