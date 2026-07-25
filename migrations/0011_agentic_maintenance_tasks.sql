SET search_path TO public;

CREATE TABLE IF NOT EXISTS agentic_maintenance_tasks (
    id BIGSERIAL PRIMARY KEY,
    agent_key TEXT REFERENCES agents(agent_key) ON DELETE CASCADE,
    task_kind TEXT NOT NULL,
    parameters JSONB NOT NULL DEFAULT '{}'::jsonb,
    status TEXT NOT NULL,
    error_summary TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    CHECK (status IN ('queued', 'running', 'succeeded', 'failed', 'aborted'))
);

CREATE INDEX IF NOT EXISTS agentic_maintenance_tasks_agent_created_idx
    ON agentic_maintenance_tasks (agent_key, created_at DESC);

CREATE INDEX IF NOT EXISTS agentic_maintenance_tasks_status_created_idx
    ON agentic_maintenance_tasks (status, created_at);

CREATE UNIQUE INDEX IF NOT EXISTS agentic_maintenance_tasks_one_active_workspace_task_idx
    ON agentic_maintenance_tasks (agent_key, task_kind)
    WHERE status IN ('queued', 'running') AND agent_key IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS agentic_maintenance_tasks_one_active_global_task_idx
    ON agentic_maintenance_tasks (task_kind)
    WHERE status IN ('queued', 'running') AND agent_key IS NULL;
