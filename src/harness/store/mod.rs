mod common;
mod jobs;
mod recovery;
mod runs;
mod workspace;

#[allow(unused_imports)]
pub use common::set_all_agent_jobs_enabled;
pub(crate) use common::{ACTIVE_STATUSES, lock_agent_coordination_tx};
#[allow(unused_imports)]
pub use jobs::{
    ClaimedCandleJobRun, claim_due_candle_job, get_agent_job, get_dispatch_job,
    get_enabled_event_job, insert_candle_job_with_model_variant, insert_default_harness_jobs,
    insert_event_job_with_model_variant, list_agent_jobs, list_due_candle_jobs, list_job_runs,
    set_candle_job_timeframe, set_job_enabled, set_job_model_with_variant, set_job_timeout,
};
#[allow(unused_imports)]
pub use recovery::recover_inactive_runs_all;
#[allow(unused_imports)]
pub use runs::{
    QueuedJobRun, agent_has_active_runs, count_agent_runs, get_run, has_prior_active_run_in_lane,
    insert_queued_event_run, insert_queued_event_run_for_automatic_dispatch,
    insert_queued_manual_run, list_active_agent_runs, list_agent_runs_page, mark_run_aborted,
    mark_run_failed, mark_run_running, mark_run_succeeded,
};
#[cfg(test)]
pub use runs::{insert_test_run, list_agent_runs};
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
