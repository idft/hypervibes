DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM harness_run_workspace_artifacts WHERE context_schema_version = 6
    ) THEN
        RAISE EXCEPTION 'cannot remove run context V6 while V6 artifacts exist';
    END IF;
END $$;

ALTER TABLE harness_run_workspace_artifacts
    DROP CONSTRAINT harness_run_workspace_artifacts_context_schema_version_check;
ALTER TABLE harness_run_workspace_artifacts
    ADD CONSTRAINT harness_run_workspace_artifacts_context_schema_version_check
    CHECK (context_schema_version = 5);

DROP TABLE harness_run_indicator_dependencies;
DROP TABLE harness_run_indicator_sets;

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM agent_indicator_definitions WHERE archived_at IS NOT NULL) THEN
        RAISE EXCEPTION 'cannot remove indicator archival while archived definitions exist';
    END IF;
END $$;

DROP INDEX agent_indicator_definitions_active_name_idx;
ALTER TABLE agent_indicator_definitions
    ADD CONSTRAINT agent_indicator_definitions_agent_key_name_key UNIQUE (agent_key, name);
ALTER TABLE agent_indicator_definitions DROP COLUMN archived_at;

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
        RAISE EXCEPTION 'cannot collapse indicator definitions unless each has exactly one timeframe';
    END IF;
END $$;

ALTER TABLE agent_indicator_definitions ADD COLUMN timeframe TEXT;

UPDATE agent_indicator_definitions AS definitions
SET timeframe = timeframes.timeframe
FROM agent_indicator_definition_timeframes AS timeframes
WHERE timeframes.indicator_definition_id = definitions.id;

ALTER TABLE agent_indicator_definitions ALTER COLUMN timeframe SET NOT NULL;
DROP TABLE agent_indicator_definition_timeframes;

DROP INDEX agent_indicator_runs_ready_idx;

ALTER TABLE agent_indicator_runs
    DROP CONSTRAINT agent_indicator_runs_claim_state_check,
    DROP COLUMN lease_expires_at,
    DROP COLUMN claim_token,
    DROP COLUMN next_attempt_at;

CREATE INDEX agent_indicator_runs_queued_idx
    ON agent_indicator_runs (status, scheduled_for) WHERE status = 'queued';
