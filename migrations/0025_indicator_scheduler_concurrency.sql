ALTER TABLE agent_indicator_runs
    ADD COLUMN next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    ADD COLUMN claim_token UUID NULL,
    ADD COLUMN lease_expires_at TIMESTAMPTZ NULL;

UPDATE agent_indicator_runs
SET status = 'queued',
    error_summary = 'indicator execution interrupted by coordination migration; retrying',
    started_at = NULL,
    updated_at = now()
WHERE status = 'running';

ALTER TABLE agent_indicator_runs
    ADD CONSTRAINT agent_indicator_runs_claim_state_check CHECK (
        (status = 'running' AND claim_token IS NOT NULL AND lease_expires_at IS NOT NULL)
        OR
        (status <> 'running' AND claim_token IS NULL AND lease_expires_at IS NULL)
    );

DROP INDEX agent_indicator_runs_queued_idx;

CREATE INDEX agent_indicator_runs_ready_idx
    ON agent_indicator_runs (next_attempt_at, scheduled_for, created_at)
    WHERE status = 'queued';

CREATE TABLE agent_indicator_definition_timeframes (
    indicator_definition_id UUID NOT NULL
        REFERENCES agent_indicator_definitions(id) ON DELETE CASCADE,
    timeframe TEXT NOT NULL CHECK (timeframe ~ '^[1-9][0-9]*[mhd]$'),
    PRIMARY KEY (indicator_definition_id, timeframe)
);

INSERT INTO agent_indicator_definition_timeframes (indicator_definition_id, timeframe)
SELECT id, timeframe
FROM agent_indicator_definitions;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM agent_indicator_definitions AS definitions
        LEFT JOIN agent_indicator_definition_timeframes AS timeframes
            ON timeframes.indicator_definition_id = definitions.id
        GROUP BY definitions.id
        HAVING count(timeframes.timeframe) <> 1
    ) THEN
        RAISE EXCEPTION 'indicator timeframe migration did not preserve exactly one timeframe per definition';
    END IF;
END $$;

ALTER TABLE agent_indicator_definitions DROP COLUMN timeframe;

ALTER TABLE agent_indicator_definitions
    ADD COLUMN archived_at TIMESTAMPTZ;

ALTER TABLE agent_indicator_definitions
    DROP CONSTRAINT agent_indicator_definitions_agent_key_name_key;

CREATE UNIQUE INDEX agent_indicator_definitions_active_name_idx
    ON agent_indicator_definitions (agent_key, name)
    WHERE archived_at IS NULL;

CREATE TABLE harness_run_indicator_sets (
    analysis_run_id BIGINT PRIMARY KEY
        REFERENCES harness_sub_agent_runs(id) ON DELETE CASCADE,
    as_of_boundary TIMESTAMPTZ NOT NULL,
    wait_deadline_at TIMESTAMPTZ NOT NULL,
    prepared_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    frozen_at TIMESTAMPTZ,
    CHECK (wait_deadline_at >= prepared_at),
    CHECK (frozen_at IS NULL OR frozen_at >= prepared_at)
);

CREATE TABLE harness_run_indicator_dependencies (
    analysis_run_id BIGINT NOT NULL
        REFERENCES harness_run_indicator_sets(analysis_run_id) ON DELETE CASCADE,
    indicator_run_id UUID NOT NULL
        REFERENCES agent_indicator_runs(id) ON DELETE RESTRICT,
    indicator_boundary TIMESTAMPTZ NOT NULL,
    snapshot_status TEXT CHECK (
        snapshot_status IN ('succeeded', 'failed', 'skipped', 'timed_out', 'missing')
    ),
    PRIMARY KEY (analysis_run_id, indicator_run_id)
);

CREATE INDEX harness_run_indicator_dependencies_run_idx
    ON harness_run_indicator_dependencies (indicator_run_id);

ALTER TABLE harness_run_workspace_artifacts
    DROP CONSTRAINT harness_run_workspace_artifacts_context_schema_version_check;
ALTER TABLE harness_run_workspace_artifacts
    ADD CONSTRAINT harness_run_workspace_artifacts_context_schema_version_check
    CHECK (context_schema_version IN (5, 6));
