mod common;
mod hooks;
mod recovery;
mod runs;
mod schedules;
mod workspace;

#[allow(unused_imports)]
pub use common::set_all_agent_jobs_enabled;
#[allow(unused_imports)]
pub use hooks::{
    delete_agent_hook, get_agent_hook, get_enabled_hook_for_event, get_opencode_hook_for_dispatch,
    insert_agent_hook, list_agent_hooks, list_hook_runs, set_hook_enabled, set_hook_model,
    set_hook_timeout,
};
#[allow(unused_imports)]
pub use recovery::recover_inactive_runs_all;
#[allow(unused_imports)]
pub use runs::{
    QueuedHookRun, QueuedScheduleRun, agent_has_active_runs, count_agent_runs, get_run,
    has_prior_active_run_in_lane, insert_queued_hook_run,
    insert_queued_hook_run_for_automatic_dispatch, insert_queued_run, list_active_agent_runs,
    list_agent_runs_page, mark_run_failed, mark_run_running, mark_run_succeeded,
};
#[cfg(test)]
pub use runs::{insert_test_run, list_agent_runs, mark_run_aborted};
#[allow(unused_imports)]
pub use schedules::{
    ClaimedScheduleRun, claim_due_schedule, delete_agent_schedule, get_agent_schedule,
    get_opencode_schedule_for_dispatch, insert_agent_schedule, insert_default_opencode_schedules,
    list_agent_schedules, list_due_opencode_schedules, list_schedule_runs, set_schedule_enabled,
    set_schedule_model, set_schedule_timeframe, set_schedule_timeout,
};
#[cfg(test)]
pub use workspace::agent_has_blocking_workspace_maintenance;
#[cfg(test)]
pub use workspace::get_latest_workspace_regenerate_task;
#[allow(unused_imports)]
pub use workspace::{
    AnalysisCodingTaskRequest, CodingTriggerMode, InsertAnalysisCodingTaskOutcome,
    InsertWorkspaceMaintenanceTaskOutcome, agent_has_active_live_runs,
    compare_and_set_maintenance_phase, get_latest_maintenance_task,
    get_next_queued_workspace_regenerate_task, heartbeat_maintenance_task,
    insert_analysis_coding_task_and_run, insert_workspace_regenerate_task,
    list_queued_maintenance_candidates, list_stale_running_maintenance_tasks,
    mark_maintenance_task_failed, mark_maintenance_task_running, mark_maintenance_task_succeeded,
};

#[cfg(test)]
mod test_support;

#[cfg(test)]
mod common_tests;
#[cfg(test)]
mod hooks_tests;
#[cfg(test)]
mod runs_tests;
#[cfg(test)]
mod schedules_tests;
#[cfg(test)]
mod workspace_tests;
