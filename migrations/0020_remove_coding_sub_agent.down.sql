SET search_path TO public;

-- Down migration restores schema/index admission only. Deleted Coding rows,
-- prompt source, OpenCode sessions, and workspace-volume packages cannot be
-- reconstructed here.
ALTER TABLE harness_run_workspace_artifacts
    DROP CONSTRAINT IF EXISTS harness_run_workspace_artifacts_context_schema_version_check;

UPDATE harness_run_workspace_artifacts
   SET context_schema_version = 2,
       context_snapshot = context_snapshot || jsonb_build_object('quantitative_package', NULL)
 WHERE context_schema_version = 3;

ALTER TABLE harness_run_workspace_artifacts
    ADD CONSTRAINT harness_run_workspace_artifacts_context_schema_version_check
    CHECK (context_schema_version = 2);

ALTER TABLE harness_maintenance_tasks
    DROP CONSTRAINT IF EXISTS harness_maintenance_tasks_task_kind_check,
    DROP CONSTRAINT IF EXISTS harness_maintenance_tasks_task_shape_check;

ALTER TABLE harness_maintenance_tasks
    ADD CONSTRAINT harness_maintenance_tasks_task_kind_check
    CHECK (task_kind IN ('workspace_regenerate', 'analysis_coding', 'provider_config_reload')),
    ADD CONSTRAINT harness_maintenance_tasks_task_shape_check
    CHECK (
        (task_kind = 'analysis_coding'
            AND agent_key IS NOT NULL AND sub_agent_id IS NOT NULL AND run_id IS NOT NULL)
        OR
        (task_kind = 'workspace_regenerate'
            AND sub_agent_id IS NULL AND run_id IS NULL)
        OR
        (task_kind = 'provider_config_reload'
            AND agent_key IS NULL AND sub_agent_id IS NULL AND run_id IS NULL)
    );

DROP INDEX IF EXISTS harness_sub_agents_singleton_kind_idx;
CREATE UNIQUE INDEX harness_sub_agents_singleton_kind_idx
    ON harness_sub_agents (agent_key, sub_agent_kind)
    WHERE sub_agent_kind IN ('trading', 'coding', 'review');

CREATE UNIQUE INDEX IF NOT EXISTS harness_coding_source_memory_once_idx
    ON harness_maintenance_tasks (source_memory_id)
    WHERE task_kind = 'analysis_coding' AND source_memory_id IS NOT NULL;
