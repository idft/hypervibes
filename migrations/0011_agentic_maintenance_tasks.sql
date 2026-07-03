SET search_path TO public;

CREATE TABLE IF NOT EXISTS agentic_maintenance_tasks (
    id BIGSERIAL PRIMARY KEY,
    agent_key TEXT NOT NULL REFERENCES agents(agent_key) ON DELETE CASCADE,
    task_kind TEXT NOT NULL,
    hard_reset BOOLEAN NOT NULL DEFAULT false,
    status TEXT NOT NULL,
    error_summary TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    CHECK (task_kind IN ('workspace_regenerate')),
    CHECK (status IN ('queued', 'running', 'succeeded', 'failed', 'aborted'))
);

CREATE INDEX IF NOT EXISTS agentic_maintenance_tasks_agent_created_idx
    ON agentic_maintenance_tasks (agent_key, created_at DESC);

CREATE INDEX IF NOT EXISTS agentic_maintenance_tasks_status_created_idx
    ON agentic_maintenance_tasks (status, created_at);

CREATE UNIQUE INDEX IF NOT EXISTS agentic_maintenance_tasks_one_active_workspace_task_idx
    ON agentic_maintenance_tasks (agent_key, task_kind)
    WHERE status IN ('queued', 'running');
