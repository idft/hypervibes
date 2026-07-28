SET search_path TO public;

CREATE TABLE harness_jobs (
    id BIGSERIAL PRIMARY KEY,
    agent_key TEXT NOT NULL REFERENCES agents(agent_key) ON DELETE CASCADE,
    job_key TEXT NOT NULL,
    job_kind TEXT NOT NULL,
    trigger_type TEXT NOT NULL,
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
    CHECK (length(trim(job_key)) > 0),
    CHECK (timeout_seconds > 0),
    CHECK ((model_provider_id IS NULL AND model_id IS NULL) OR (length(trim(model_provider_id)) > 0 AND length(trim(model_id)) > 0)),
    CHECK (model_variant IS NULL OR (length(trim(model_variant)) > 0 AND model_provider_id IS NOT NULL AND model_id IS NOT NULL)),
    CHECK (enabled = false OR (model_provider_id IS NOT NULL AND model_id IS NOT NULL)),
    CHECK (
        (trigger_type = 'candle_closed' AND job_kind IN ('analysis', 'trading', 'daily_review') AND timeframe ~ '^[1-9][0-9]*[mhd]$' AND trigger_delay_seconds >= 0 AND next_run_at IS NOT NULL)
        OR (trigger_type = 'analysis_batch_completed' AND job_kind = 'market_analysis' AND timeframe IS NULL AND trigger_delay_seconds IS NULL AND next_run_at IS NULL)
        OR (trigger_type = 'daily_review_completed' AND job_kind = 'analysis_coding' AND timeframe IS NULL AND trigger_delay_seconds IS NULL AND next_run_at IS NULL)
    )
);

CREATE TABLE harness_runs (
    id BIGSERIAL PRIMARY KEY,
    job_id BIGINT NOT NULL REFERENCES harness_jobs(id) ON DELETE CASCADE,
    agent_key TEXT NOT NULL REFERENCES agents(agent_key) ON DELETE CASCADE,
    job_key TEXT NOT NULL,
    job_kind TEXT NOT NULL,
    trigger_type TEXT NOT NULL,
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
    CHECK (status IN ('queued', 'running', 'succeeded', 'failed', 'aborted', 'skipped')),
    CHECK (length(trim(job_key)) > 0),
    CHECK (timeout_seconds > 0),
    CHECK ((trigger_type = 'candle_closed' AND timeframe ~ '^[1-9][0-9]*[mhd]$') OR (trigger_type IN ('analysis_batch_completed', 'daily_review_completed') AND timeframe IS NULL)),
    CHECK ((model_provider_id IS NULL AND model_id IS NULL) OR (length(trim(model_provider_id)) > 0 AND length(trim(model_id)) > 0)),
    CHECK (model_variant IS NULL OR (length(trim(model_variant)) > 0 AND model_provider_id IS NOT NULL AND model_id IS NOT NULL))
);

CREATE UNIQUE INDEX harness_jobs_agent_job_key_idx ON harness_jobs (agent_key, job_key);
CREATE UNIQUE INDEX harness_jobs_one_candle_kind_timeframe_idx ON harness_jobs (agent_key, job_kind, timeframe) WHERE trigger_type = 'candle_closed';
CREATE UNIQUE INDEX harness_jobs_one_event_trigger_idx ON harness_jobs (agent_key, trigger_type) WHERE trigger_type IN ('analysis_batch_completed', 'daily_review_completed');
CREATE INDEX harness_jobs_due_idx ON harness_jobs (next_run_at, id) WHERE enabled = true AND trigger_type = 'candle_closed';
CREATE INDEX harness_jobs_agent_list_idx ON harness_jobs (agent_key, trigger_type, job_kind, timeframe, id);
CREATE INDEX harness_jobs_enabled_event_idx ON harness_jobs (agent_key, trigger_type) WHERE enabled = true AND trigger_type IN ('analysis_batch_completed', 'daily_review_completed');

CREATE OR REPLACE FUNCTION public.set_harness_jobs_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
CREATE TRIGGER set_harness_jobs_updated_at BEFORE UPDATE ON harness_jobs FOR EACH ROW EXECUTE FUNCTION public.set_harness_jobs_updated_at();

CREATE INDEX harness_runs_job_status_idx ON harness_runs (job_id, status);
CREATE INDEX harness_runs_agent_created_idx ON harness_runs (agent_key, created_at DESC, id DESC);
CREATE INDEX harness_runs_agent_status_kind_idx ON harness_runs (agent_key, status, job_kind);
CREATE INDEX harness_runs_status_idx ON harness_runs (status);
CREATE INDEX harness_runs_backend_run_ref_idx ON harness_runs (backend_run_ref) WHERE backend_run_ref IS NOT NULL;

CREATE OR REPLACE FUNCTION public.notify_harness_run_detail_changed()
RETURNS TRIGGER AS $$
BEGIN
    PERFORM pg_notify('agent_run_detail_run_changed', NEW.id::text);
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER notify_harness_runs_run_detail_changed
    AFTER INSERT OR UPDATE OF status, backend_run_ref, started_at, finished_at, error_summary
    ON harness_runs
    FOR EACH ROW
    EXECUTE FUNCTION public.notify_harness_run_detail_changed();
