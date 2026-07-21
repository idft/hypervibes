DROP INDEX IF EXISTS agentic_runs_backend_run_ref_idx;
DROP INDEX IF EXISTS agentic_runs_status_idx;
DROP INDEX IF EXISTS agentic_runs_agent_started_idx;
DROP INDEX IF EXISTS agentic_runs_schedule_status_idx;
DROP INDEX IF EXISTS agentic_job_schedules_due_idx;
DROP INDEX IF EXISTS agentic_job_schedules_agent_key_idx;

DROP TABLE IF EXISTS agentic_runs;
DROP TABLE IF EXISTS agentic_job_schedules;
