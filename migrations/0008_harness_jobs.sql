SET search_path TO public;

CREATE TABLE harness_sub_agents (
    id BIGSERIAL PRIMARY KEY,
    agent_key TEXT NOT NULL REFERENCES agents(agent_key) ON DELETE CASCADE,
    sub_agent_key TEXT NOT NULL,
    sub_agent_kind TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT false,
    timeframe TEXT,
    trigger_delay_seconds INTEGER,
    next_run_at TIMESTAMPTZ,
    model_provider_id TEXT,
    model_id TEXT,
    model_variant TEXT,
    timeout_seconds INTEGER NOT NULL,
    operator_prompt TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT harness_sub_agents_key_check CHECK (length(trim(sub_agent_key)) > 0),
    CONSTRAINT harness_sub_agents_kind_check CHECK (length(trim(sub_agent_kind)) > 0),
    CONSTRAINT harness_sub_agents_timeout_check CHECK (timeout_seconds > 0),
    CONSTRAINT harness_sub_agents_model_pair_check CHECK (
        (model_provider_id IS NULL AND model_id IS NULL)
        OR (
            model_provider_id IS NOT NULL
            AND model_id IS NOT NULL
            AND length(trim(model_provider_id)) > 0
            AND length(trim(model_id)) > 0
        )
    ),
    CONSTRAINT harness_sub_agents_model_variant_check CHECK (
        model_variant IS NULL
        OR (
            model_provider_id IS NOT NULL
            AND model_id IS NOT NULL
            AND length(trim(model_variant)) > 0
        )
    ),
    CONSTRAINT harness_sub_agents_enabled_model_check CHECK (
        NOT enabled OR (model_provider_id IS NOT NULL AND model_id IS NOT NULL)
    ),
    CONSTRAINT harness_sub_agents_schedule_shape_check CHECK (
        (
            timeframe IS NULL
            AND trigger_delay_seconds IS NULL
            AND next_run_at IS NULL
        )
        OR (
            timeframe IS NOT NULL
            AND timeframe ~ '^[1-9][0-9]*[mhd]$'
            AND trigger_delay_seconds IS NOT NULL
            AND trigger_delay_seconds >= 0
            AND next_run_at IS NOT NULL
        )
    )
);

CREATE TABLE harness_sub_agent_runs (
    id BIGSERIAL PRIMARY KEY,
    sub_agent_id BIGINT NOT NULL REFERENCES harness_sub_agents(id) ON DELETE CASCADE,
    agent_key TEXT NOT NULL REFERENCES agents(agent_key) ON DELETE CASCADE,
    sub_agent_key TEXT NOT NULL,
    sub_agent_kind TEXT NOT NULL,
    timeframe TEXT,
    status TEXT NOT NULL,
    backend_run_ref TEXT,
    model_provider_id TEXT,
    model_id TEXT,
    model_variant TEXT,
    scheduled_for TIMESTAMPTZ NOT NULL,
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    timeout_seconds INTEGER NOT NULL,
    error_summary TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT harness_sub_agent_runs_status_check CHECK (status IN ('queued', 'running', 'succeeded', 'failed', 'aborted', 'skipped')),
    CONSTRAINT harness_sub_agent_runs_key_check CHECK (length(trim(sub_agent_key)) > 0),
    CONSTRAINT harness_sub_agent_runs_kind_check CHECK (length(trim(sub_agent_kind)) > 0),
    CONSTRAINT harness_sub_agent_runs_timeout_check CHECK (timeout_seconds > 0),
    CONSTRAINT harness_sub_agent_runs_timeframe_check CHECK (
        timeframe IS NULL OR timeframe ~ '^[1-9][0-9]*[mhd]$'
    ),
    CONSTRAINT harness_sub_agent_runs_model_pair_check CHECK (
        (model_provider_id IS NULL AND model_id IS NULL)
        OR (
            model_provider_id IS NOT NULL
            AND model_id IS NOT NULL
            AND length(trim(model_provider_id)) > 0
            AND length(trim(model_id)) > 0
        )
    ),
    CONSTRAINT harness_sub_agent_runs_model_variant_check CHECK (
        model_variant IS NULL
        OR (
            model_provider_id IS NOT NULL
            AND model_id IS NOT NULL
            AND length(trim(model_variant)) > 0
        )
    )
);

CREATE UNIQUE INDEX harness_sub_agents_agent_key_idx ON harness_sub_agents (agent_key, sub_agent_key);
CREATE UNIQUE INDEX harness_sub_agents_kind_timeframe_idx ON harness_sub_agents (agent_key, sub_agent_kind, timeframe) WHERE timeframe IS NOT NULL;
CREATE INDEX harness_sub_agents_due_idx ON harness_sub_agents (next_run_at, id) WHERE enabled = true AND next_run_at IS NOT NULL;
CREATE INDEX harness_sub_agents_agent_list_idx ON harness_sub_agents (agent_key, sub_agent_kind, timeframe, id);
CREATE INDEX harness_sub_agents_enabled_kind_idx ON harness_sub_agents (agent_key, sub_agent_kind) WHERE enabled = true;

CREATE OR REPLACE FUNCTION public.set_harness_sub_agents_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
CREATE TRIGGER set_harness_sub_agents_updated_at BEFORE UPDATE ON harness_sub_agents FOR EACH ROW EXECUTE FUNCTION public.set_harness_sub_agents_updated_at();

CREATE INDEX harness_sub_agent_runs_sub_agent_status_idx ON harness_sub_agent_runs (sub_agent_id, status);
CREATE INDEX harness_sub_agent_runs_agent_created_idx ON harness_sub_agent_runs (agent_key, created_at DESC, id DESC);
CREATE INDEX harness_sub_agent_runs_agent_status_kind_idx ON harness_sub_agent_runs (agent_key, status, sub_agent_kind);
CREATE INDEX harness_sub_agent_runs_status_idx ON harness_sub_agent_runs (status);
CREATE INDEX harness_sub_agent_runs_backend_run_ref_idx ON harness_sub_agent_runs (backend_run_ref) WHERE backend_run_ref IS NOT NULL;

CREATE OR REPLACE FUNCTION public.notify_harness_sub_agent_run_detail_changed()
RETURNS TRIGGER AS $$
BEGIN
    PERFORM pg_notify('agent_sub_agent_run_detail_changed', NEW.id::text);
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER notify_harness_sub_agent_runs_run_detail_changed
    AFTER INSERT OR UPDATE OF status, backend_run_ref, started_at, finished_at, error_summary
    ON harness_sub_agent_runs
    FOR EACH ROW
    EXECUTE FUNCTION public.notify_harness_sub_agent_run_detail_changed();
