SET search_path TO public;

CREATE TABLE harness_maintenance_tasks (
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
    phase TEXT NOT NULL DEFAULT 'queued',
    run_id BIGINT UNIQUE REFERENCES harness_runs(id) ON DELETE CASCADE,
    job_id BIGINT REFERENCES harness_jobs(id) ON DELETE CASCADE,
    source_run_id BIGINT REFERENCES harness_runs(id) ON DELETE SET NULL,
    source_memory_id UUID REFERENCES memory.records(id) ON DELETE SET NULL,
    heartbeat_at TIMESTAMPTZ,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    CHECK (status IN ('queued', 'running', 'succeeded', 'failed', 'aborted')),
    CHECK (jsonb_typeof(parameters) = 'object'),
    CHECK (attempt_count >= 0),
    CHECK (task_kind IN ('workspace_regenerate', 'analysis_coding', 'provider_config_reload')),
    CHECK ((task_kind = 'analysis_coding' AND agent_key IS NOT NULL AND job_id IS NOT NULL AND run_id IS NOT NULL) OR (task_kind = 'workspace_regenerate' AND job_id IS NULL AND run_id IS NULL) OR (task_kind = 'provider_config_reload' AND agent_key IS NULL AND job_id IS NULL AND run_id IS NULL))
);

CREATE INDEX harness_maintenance_tasks_agent_created_idx ON harness_maintenance_tasks (agent_key, created_at DESC);
CREATE INDEX harness_maintenance_tasks_status_created_idx ON harness_maintenance_tasks (status, created_at);
CREATE INDEX harness_maintenance_tasks_status_phase_created_idx ON harness_maintenance_tasks (status, phase, created_at, id);
CREATE UNIQUE INDEX harness_maintenance_tasks_one_active_agent_task_idx ON harness_maintenance_tasks (agent_key) WHERE status IN ('queued', 'running') AND agent_key IS NOT NULL;
CREATE UNIQUE INDEX harness_maintenance_tasks_one_active_global_task_idx ON harness_maintenance_tasks (task_kind) WHERE status IN ('queued', 'running') AND agent_key IS NULL;
CREATE UNIQUE INDEX harness_coding_source_memory_once_idx ON harness_maintenance_tasks (source_memory_id) WHERE task_kind = 'analysis_coding' AND source_memory_id IS NOT NULL;
