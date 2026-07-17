SET search_path TO public;

-- 1. Delete every analysis_coding maintenance task before restoring
-- the narrow task-kind CHECK that excludes it.
DELETE FROM agentic_maintenance_tasks
 WHERE task_kind = 'analysis_coding';

-- 2. Delete analysis_coding runs (any hook-backed runs created by
-- this feature) before restoring the narrow run-kind CHECK.
DELETE FROM agentic_runs
 WHERE job_kind = 'analysis_coding';

-- 3. Delete analysis_coding hooks.
DELETE FROM agentic_job_hooks
 WHERE job_kind = 'analysis_coding';

-- 4. Delete analysis_coding strategy prompt rows.
DELETE FROM agent_strategy_prompts
 WHERE prompt_kind = 'analysis_coding';

-- 5. Drop new maintenance indexes and columns.
DROP INDEX IF EXISTS agentic_maintenance_tasks_status_phase_created_idx;
DROP INDEX IF EXISTS agentic_coding_source_memory_once_idx;
DROP INDEX IF EXISTS agentic_maintenance_tasks_one_active_agent_task_idx;

ALTER TABLE agentic_maintenance_tasks
    DROP CONSTRAINT IF EXISTS agentic_maintenance_tasks_coding_has_run_check;
ALTER TABLE agentic_maintenance_tasks
    DROP CONSTRAINT IF EXISTS agentic_maintenance_tasks_regenerate_no_run_check;
ALTER TABLE agentic_maintenance_tasks
    DROP CONSTRAINT IF EXISTS agentic_maintenance_tasks_attempt_count_check;
ALTER TABLE agentic_maintenance_tasks
    DROP COLUMN IF EXISTS attempt_count;
ALTER TABLE agentic_maintenance_tasks
    DROP COLUMN IF EXISTS heartbeat_at;
ALTER TABLE agentic_maintenance_tasks
    DROP COLUMN IF EXISTS source_memory_id;
ALTER TABLE agentic_maintenance_tasks
    DROP COLUMN IF EXISTS source_run_id;
ALTER TABLE agentic_maintenance_tasks
    DROP COLUMN IF EXISTS run_id;
ALTER TABLE agentic_maintenance_tasks
    DROP CONSTRAINT IF EXISTS agentic_maintenance_tasks_phase_check;
ALTER TABLE agentic_maintenance_tasks
    DROP COLUMN IF EXISTS phase;

-- 6. Restore the original per-(agent, task_kind) uniqueness index for
-- workspace regeneration.
CREATE UNIQUE INDEX IF NOT EXISTS agentic_maintenance_tasks_one_active_workspace_task_idx
    ON agentic_maintenance_tasks (agent_key, task_kind)
    WHERE status IN ('queued', 'running');

-- 7. Restore the narrow task-kind CHECK that only allows regeneration.
ALTER TABLE agentic_maintenance_tasks
    DROP CONSTRAINT IF EXISTS agentic_maintenance_tasks_task_kind_check;
ALTER TABLE agentic_maintenance_tasks
    ADD CONSTRAINT agentic_maintenance_tasks_task_kind_check
    CHECK (task_kind IN ('workspace_regenerate'));

-- 8. Restore the run-kind CHECK without analysis_coding.
ALTER TABLE agentic_runs
    DROP CONSTRAINT IF EXISTS agentic_runs_job_kind_check;
ALTER TABLE agentic_runs
    ADD CONSTRAINT agentic_runs_job_kind_check
    CHECK (job_kind IN (
        'analysis',
        'trading',
        'market_analysis',
        'daily_review'
    ));

-- 9. Restore the hook kind/event CHECKs as the two separate constraints
-- the previous migrations used. Existing market_analysis hooks satisfy
-- them since they were created under the same constraints.
ALTER TABLE agentic_job_hooks
    DROP CONSTRAINT IF EXISTS agentic_job_hooks_kind_event_pair_check;
ALTER TABLE agentic_job_hooks
    ADD CONSTRAINT agentic_job_hooks_job_kind_check
    CHECK (job_kind IN ('market_analysis'));
ALTER TABLE agentic_job_hooks
    ADD CONSTRAINT agentic_job_hooks_hook_event_check
    CHECK (hook_event IN ('analysis_batch_completed'));

-- 10. Restore the strategy prompt-kind CHECK without analysis_coding.
ALTER TABLE agent_strategy_prompts
    DROP CONSTRAINT IF EXISTS agent_strategy_prompts_prompt_kind_check;
ALTER TABLE agent_strategy_prompts
    ADD CONSTRAINT agent_strategy_prompts_prompt_kind_check
    CHECK (prompt_kind IN (
        'analysis',
        'market_analysis',
        'trading',
        'daily_review'
    ));