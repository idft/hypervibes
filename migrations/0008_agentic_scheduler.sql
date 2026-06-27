-- Migration 0007 (`opencode_database_plugin`) sets
-- `search_path TO opencode, public;` and does not restore it. Pin the
-- search path back to `public` for this migration so the new tables
-- land in the same schema as the rest of the agent registry.
SET search_path TO public;

CREATE TABLE IF NOT EXISTS agentic_job_schedules (
    id BIGSERIAL PRIMARY KEY,
    agent_key TEXT NOT NULL REFERENCES agents(agent_key) ON DELETE CASCADE,
    job_key TEXT NOT NULL,
    job_kind TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT true,
    timeframe TEXT NOT NULL,
    trigger_delay_seconds INTEGER NOT NULL DEFAULT 1,
    next_run_at TIMESTAMPTZ NOT NULL,
    model_provider_id TEXT,
    model_id TEXT,
    timeout_seconds INTEGER NOT NULL,
    operator_prompt TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (agent_key, job_kind, timeframe),
    CHECK (job_kind IN ('analysis', 'trading')),
    CHECK (length(trim(timeframe)) > 0),
    CHECK (trigger_delay_seconds >= 0),
    CHECK (timeout_seconds > 0)
);

CREATE TABLE IF NOT EXISTS agentic_runs (
    id BIGSERIAL PRIMARY KEY,
    schedule_id BIGINT REFERENCES agentic_job_schedules(id) ON DELETE SET NULL,
    agent_key TEXT NOT NULL REFERENCES agents(agent_key) ON DELETE CASCADE,
    job_key TEXT NOT NULL,
    job_kind TEXT NOT NULL,
    timeframe TEXT NOT NULL,
    status TEXT NOT NULL,
    backend_run_ref TEXT,
    model_provider_id TEXT,
    model_id TEXT,
    scheduled_for TIMESTAMPTZ NOT NULL,
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    timeout_seconds INTEGER NOT NULL,
    error_summary TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (job_kind IN ('analysis', 'trading')),
    CHECK (status IN ('queued', 'running', 'succeeded', 'failed', 'aborted', 'skipped')),
    CHECK (length(trim(timeframe)) > 0),
    CHECK (timeout_seconds > 0)
);

CREATE INDEX IF NOT EXISTS agentic_job_schedules_agent_key_idx
    ON agentic_job_schedules (agent_key);

CREATE INDEX IF NOT EXISTS agentic_job_schedules_due_idx
    ON agentic_job_schedules (enabled, next_run_at);

CREATE INDEX IF NOT EXISTS agentic_runs_schedule_status_idx
    ON agentic_runs (schedule_id, status);

CREATE INDEX IF NOT EXISTS agentic_runs_agent_started_idx
    ON agentic_runs (agent_key, started_at DESC NULLS LAST, created_at DESC);

CREATE INDEX IF NOT EXISTS agentic_runs_status_idx
    ON agentic_runs (status);

CREATE INDEX IF NOT EXISTS agentic_runs_backend_run_ref_idx
    ON agentic_runs (backend_run_ref)
    WHERE backend_run_ref IS NOT NULL;
