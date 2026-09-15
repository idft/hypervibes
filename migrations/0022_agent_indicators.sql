CREATE TABLE agent_indicator_definitions (
    id UUID PRIMARY KEY,
    agent_key TEXT NOT NULL REFERENCES agents(agent_key) ON DELETE CASCADE,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    timeframe TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT true,
    active_version_id UUID NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (agent_key, name)
);

CREATE TABLE agent_indicator_versions (
    id UUID PRIMARY KEY,
    indicator_definition_id UUID NOT NULL REFERENCES agent_indicator_definitions(id) ON DELETE CASCADE,
    version_number INTEGER NOT NULL CHECK (version_number > 0),
    source TEXT NOT NULL,
    source_sha256 TEXT NOT NULL,
    compiler_version TEXT NOT NULL,
    metadata JSONB NOT NULL,
    input_values JSONB NOT NULL,
    created_by_kind TEXT NOT NULL CHECK (created_by_kind IN ('operator', 'chat', 'review')),
    created_by_run_id BIGINT NULL,
    created_by_conversation_id UUID NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (indicator_definition_id, version_number),
    CHECK ((created_by_kind = 'review') = (created_by_run_id IS NOT NULL)),
    CHECK ((created_by_kind = 'chat') = (created_by_conversation_id IS NOT NULL))
);

ALTER TABLE agent_indicator_definitions
    ADD CONSTRAINT agent_indicator_definitions_active_version_fk
    FOREIGN KEY (active_version_id) REFERENCES agent_indicator_versions(id);

CREATE TABLE agent_indicator_definition_instruments (
    indicator_definition_id UUID NOT NULL REFERENCES agent_indicator_definitions(id) ON DELETE CASCADE,
    instrument_id TEXT NOT NULL REFERENCES hyperliquid.instruments(instrument_id),
    PRIMARY KEY (indicator_definition_id, instrument_id)
);

CREATE TABLE agent_indicator_runs (
    id UUID PRIMARY KEY,
    agent_key TEXT NOT NULL REFERENCES agents(agent_key) ON DELETE CASCADE,
    indicator_definition_id UUID NOT NULL REFERENCES agent_indicator_definitions(id) ON DELETE CASCADE,
    indicator_version_id UUID NOT NULL REFERENCES agent_indicator_versions(id) ON DELETE RESTRICT,
    instrument_id TEXT NOT NULL REFERENCES hyperliquid.instruments(instrument_id),
    timeframe TEXT NOT NULL,
    scheduled_for TIMESTAMPTZ NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('queued', 'running', 'succeeded', 'failed', 'skipped')),
    candle_data JSONB NULL,
    plot_data JSONB NULL,
    latest_values JSONB NULL,
    diagnostics JSONB NOT NULL DEFAULT '[]'::jsonb,
    error_summary TEXT NULL,
    started_at TIMESTAMPTZ NULL,
    finished_at TIMESTAMPTZ NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (indicator_version_id, instrument_id, timeframe, scheduled_for)
);

CREATE INDEX agent_indicator_runs_latest_idx
    ON agent_indicator_runs (agent_key, indicator_definition_id, instrument_id, timeframe, scheduled_for DESC);
CREATE INDEX agent_indicator_runs_queued_idx
    ON agent_indicator_runs (status, scheduled_for) WHERE status = 'queued';
