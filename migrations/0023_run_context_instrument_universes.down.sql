UPDATE harness_run_workspace_artifacts
SET context_snapshot = jsonb_set(context_snapshot - 'analysis_instruments' - 'trading_instruments' - 'indicator_snapshot', '{selected_instruments}', context_snapshot->'trading_instruments'),
    context_schema_version = 3
WHERE context_schema_version = 5;

ALTER TABLE harness_run_workspace_artifacts
    DROP CONSTRAINT IF EXISTS harness_run_workspace_artifacts_context_schema_version_check;
ALTER TABLE harness_run_workspace_artifacts
    ADD CONSTRAINT harness_run_workspace_artifacts_context_schema_version_check
    CHECK (context_schema_version = 3);
