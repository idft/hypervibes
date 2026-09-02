/// Phase 2 persistence primitives. They remain separate from the active
/// scheduler until Phase 3 materializes isolated workspaces before dispatch.
pub mod artifacts;
mod common;
mod recovery;
mod runs;
mod runtime_credentials;
mod sub_agents;
mod workspace;

#[allow(unused_imports)]
pub use common::set_all_agent_sub_agents_enabled;
pub(crate) use common::{ACTIVE_STATUSES, lock_agent_coordination_tx};
#[allow(unused_imports)]
pub use recovery::recover_inactive_runs_all;
#[allow(unused_imports)]
pub use runs::{
    QueuedRunForDispatch, QueuedSubAgentRun, agent_has_active_runs, count_agent_runs, get_run,
    has_prior_active_run_in_lane, insert_queued_event_run,
    insert_queued_event_run_for_automatic_dispatch, insert_queued_manual_run,
    list_active_agent_runs, list_agent_runs_page, list_queued_runs_for_dispatch, mark_run_aborted,
    mark_run_failed, mark_run_running, mark_run_succeeded, set_run_error_summary,
};
#[cfg(test)]
pub use runs::{insert_test_run, list_agent_runs};
pub use runtime_credentials::{
    authenticate_run_runtime_credential, issue_run_runtime_credential,
    revoke_run_runtime_credential,
};
#[allow(unused_imports)]
pub use sub_agents::{
    ClaimedCandleSubAgentRun, claim_due_candle_sub_agent, count_sub_agent_runs,
    get_agent_sub_agent, get_dispatch_sub_agent, get_enabled_sub_agent,
    insert_candle_sub_agent_with_model_variant, insert_default_harness_sub_agents,
    insert_unscheduled_sub_agent_with_model_variant, list_agent_sub_agents,
    list_due_candle_sub_agents, list_sub_agent_runs_page, set_candle_sub_agent_timeframe,
    set_sub_agent_capabilities, set_sub_agent_enabled, set_sub_agent_model_with_variant,
    set_sub_agent_notification_send_enabled, set_sub_agent_operator_prompt, set_sub_agent_timeout,
};
pub use workspace::agent_has_blocking_workspace_maintenance;
#[cfg(test)]
pub use workspace::get_latest_workspace_regenerate_task;
#[allow(unused_imports)]
pub use workspace::{
    AnalysisCodingTaskRequest, CodingTriggerMode, InsertAnalysisCodingTaskOutcome,
    InsertGlobalMaintenanceTaskOutcome, InsertWorkspaceMaintenanceTaskOutcome,
    agent_has_active_live_runs, compare_and_set_maintenance_phase, get_latest_maintenance_task,
    get_latest_provider_config_reload_task, get_next_queued_provider_config_reload_task,
    get_next_queued_workspace_regenerate_task, heartbeat_maintenance_task,
    insert_analysis_coding_task_and_run, insert_provider_config_reload_task,
    insert_workspace_regenerate_task, list_queued_maintenance_candidates,
    list_stale_running_maintenance_tasks, mark_maintenance_task_failed,
    mark_maintenance_task_running, mark_maintenance_task_succeeded,
    requeue_stale_provider_config_reload_tasks,
};

#[cfg(test)]
mod test_support;

#[cfg(test)]
mod common_tests;
#[cfg(test)]
mod runs_tests;
#[cfg(test)]
mod workspace_tests;
