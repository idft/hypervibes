SET search_path TO public;

DELETE FROM agentic_runs
 WHERE hook_id IS NOT NULL
    OR job_kind = 'market_analysis'
    OR timeframe IS NULL;

DROP INDEX IF EXISTS agentic_runs_agent_status_kind_idx;
DROP INDEX IF EXISTS agentic_runs_hook_status_idx;
DROP INDEX IF EXISTS agentic_job_hooks_agent_event_enabled_idx;
DROP INDEX IF EXISTS agentic_job_hooks_agent_key_idx;

ALTER TABLE agentic_runs
    DROP CONSTRAINT IF EXISTS agentic_runs_exactly_one_source_check;

ALTER TABLE agentic_runs
    DROP CONSTRAINT IF EXISTS agentic_runs_hook_id_fkey;

ALTER TABLE agentic_runs
    DROP COLUMN IF EXISTS hook_id;

ALTER TABLE agentic_runs
    DROP CONSTRAINT IF EXISTS agentic_runs_schedule_id_fkey;

ALTER TABLE agentic_runs
    ADD CONSTRAINT agentic_runs_schedule_id_fkey
    FOREIGN KEY (schedule_id)
    REFERENCES agentic_job_schedules(id)
    ON DELETE SET NULL;

ALTER TABLE agentic_runs
    DROP CONSTRAINT IF EXISTS agentic_runs_job_kind_check;

ALTER TABLE agentic_runs
    ADD CONSTRAINT agentic_runs_job_kind_check
    CHECK (job_kind IN ('analysis', 'trading'));

ALTER TABLE agentic_runs
    DROP CONSTRAINT IF EXISTS agentic_runs_timeframe_check;

ALTER TABLE agentic_runs
    ALTER COLUMN timeframe SET NOT NULL;

ALTER TABLE agentic_runs
    ADD CONSTRAINT agentic_runs_timeframe_check
    CHECK (length(trim(timeframe)) > 0);

DROP TABLE IF EXISTS agentic_job_hooks;
