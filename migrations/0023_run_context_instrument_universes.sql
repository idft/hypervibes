UPDATE harness_run_workspace_artifacts
SET context_snapshot = jsonb_set(
    jsonb_set(
        jsonb_set(context_snapshot - 'selected_instruments', '{analysis_instruments}', context_snapshot->'selected_instruments'),
        '{trading_instruments}', context_snapshot->'selected_instruments'
    ),
    '{indicator_snapshot}', 'null'::jsonb
), context_schema_version = 5
WHERE context_schema_version = 3;

ALTER TABLE harness_run_workspace_artifacts
    DROP CONSTRAINT IF EXISTS harness_run_workspace_artifacts_context_schema_version_check;
ALTER TABLE harness_run_workspace_artifacts
    ADD CONSTRAINT harness_run_workspace_artifacts_context_schema_version_check
    CHECK (context_schema_version = 5);
