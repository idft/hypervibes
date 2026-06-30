SET search_path TO public;

CREATE TABLE IF NOT EXISTS agentic_job_hooks (
    id BIGSERIAL PRIMARY KEY,
    agent_key TEXT NOT NULL REFERENCES agents(agent_key) ON DELETE CASCADE,
    job_key TEXT NOT NULL,
    job_kind TEXT NOT NULL,
    hook_event TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT true,
    model_provider_id TEXT,
    model_id TEXT,
    timeout_seconds INTEGER NOT NULL,
    operator_prompt TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (agent_key, job_kind, hook_event),
    CHECK (job_kind IN ('market_analysis')),
    CHECK (hook_event IN ('analysis_batch_completed')),
    CHECK (length(trim(job_key)) > 0),
    CHECK (timeout_seconds > 0)
);

CREATE INDEX IF NOT EXISTS agentic_job_hooks_agent_key_idx
    ON agentic_job_hooks (agent_key);

CREATE INDEX IF NOT EXISTS agentic_job_hooks_agent_event_enabled_idx
    ON agentic_job_hooks (agent_key, hook_event, enabled);

ALTER TABLE agentic_runs
    DROP CONSTRAINT IF EXISTS agentic_runs_schedule_id_fkey;

ALTER TABLE agentic_runs
    ADD CONSTRAINT agentic_runs_schedule_id_fkey
    FOREIGN KEY (schedule_id)
    REFERENCES agentic_job_schedules(id)
    ON DELETE CASCADE;

ALTER TABLE agentic_runs
    ADD COLUMN hook_id BIGINT;

ALTER TABLE agentic_runs
    ADD CONSTRAINT agentic_runs_hook_id_fkey
    FOREIGN KEY (hook_id)
    REFERENCES agentic_job_hooks(id)
    ON DELETE CASCADE;

ALTER TABLE agentic_runs
    ALTER COLUMN timeframe DROP NOT NULL;

ALTER TABLE agentic_runs
    DROP CONSTRAINT IF EXISTS agentic_runs_job_kind_check;

ALTER TABLE agentic_runs
    ADD CONSTRAINT agentic_runs_job_kind_check
    CHECK (job_kind IN ('analysis', 'trading', 'market_analysis'));

ALTER TABLE agentic_runs
    DROP CONSTRAINT IF EXISTS agentic_runs_timeframe_check;

ALTER TABLE agentic_runs
    ADD CONSTRAINT agentic_runs_timeframe_check
    CHECK (timeframe IS NULL OR length(trim(timeframe)) > 0);

ALTER TABLE agentic_runs
    ADD CONSTRAINT agentic_runs_exactly_one_source_check
    CHECK (
        (schedule_id IS NOT NULL AND hook_id IS NULL)
        OR
        (schedule_id IS NULL AND hook_id IS NOT NULL)
    );

CREATE INDEX IF NOT EXISTS agentic_runs_hook_status_idx
    ON agentic_runs (hook_id, status);

CREATE INDEX IF NOT EXISTS agentic_runs_agent_status_kind_idx
    ON agentic_runs (agent_key, status, job_kind);
