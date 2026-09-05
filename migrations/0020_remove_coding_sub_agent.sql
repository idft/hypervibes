SET search_path TO public;

-- Remove active pointers before touching the restricted prompt-revision chain.
DELETE FROM agent_strategy_prompt_active_revisions
 WHERE (agent_key, target_sub_agent_id) IN (
     SELECT agent_key, id
       FROM harness_sub_agents
      WHERE sub_agent_kind = 'coding'
 );

CREATE TEMP TABLE coding_prompt_batches ON COMMIT DROP AS
SELECT DISTINCT batch_id
  FROM agent_strategy_prompt_revisions AS revisions
  JOIN harness_sub_agents AS jobs
    ON jobs.id = revisions.target_sub_agent_id
   AND jobs.agent_key = revisions.agent_key
 WHERE jobs.sub_agent_kind = 'coding';

-- Self-referential prompt links use ON DELETE RESTRICT.
UPDATE agent_strategy_prompt_revisions AS revisions
   SET parent_revision_id = NULL,
       rollback_of_revision_id = NULL
 WHERE (revisions.agent_key, revisions.target_sub_agent_id) IN (
     SELECT agent_key, id
       FROM harness_sub_agents
      WHERE sub_agent_kind = 'coding'
 );

DELETE FROM agent_strategy_prompt_revisions AS revisions
 WHERE (revisions.agent_key, revisions.target_sub_agent_id) IN (
     SELECT agent_key, id
       FROM harness_sub_agents
      WHERE sub_agent_kind = 'coding'
 );

DELETE FROM agent_strategy_prompt_revision_batches AS batches
 WHERE batches.id IN (SELECT batch_id FROM coding_prompt_batches)
   AND NOT EXISTS (
       SELECT 1
         FROM agent_strategy_prompt_revisions AS revisions
        WHERE revisions.batch_id = batches.id
   );

-- Existing foreign-key cascades remove Coding runs, credentials, artifacts,
-- and maintenance rows. OpenCode's mirrored session tables are not touched.
DELETE FROM harness_sub_agents
 WHERE sub_agent_kind = 'coding';

ALTER TABLE harness_run_workspace_artifacts
    DROP CONSTRAINT IF EXISTS harness_run_workspace_artifacts_context_schema_version_check;

-- V3 removes the retired quantitative package from persisted run context.
-- Convert existing V2 artifacts before enforcing the new version constraint.
UPDATE harness_run_workspace_artifacts
   SET context_schema_version = 3,
       context_snapshot = context_snapshot - 'quantitative_package'
 WHERE context_schema_version = 2;

ALTER TABLE harness_run_workspace_artifacts
    ADD CONSTRAINT harness_run_workspace_artifacts_context_schema_version_check
    CHECK (context_schema_version = 3);

DROP INDEX IF EXISTS harness_sub_agents_singleton_kind_idx;
CREATE UNIQUE INDEX harness_sub_agents_singleton_kind_idx
    ON harness_sub_agents (agent_key, sub_agent_kind)
    WHERE sub_agent_kind IN ('trading', 'review');

DROP INDEX IF EXISTS harness_coding_source_memory_once_idx;

-- Replace the historical unnamed checks with stable constraints. Workspace
-- regeneration remains valid historical data; new Coding tasks are rejected.
DO $$
DECLARE
    constraint_name TEXT;
BEGIN
    FOR constraint_name IN
        SELECT conname
          FROM pg_constraint
         WHERE conrelid = 'harness_maintenance_tasks'::regclass
           AND pg_get_constraintdef(oid) LIKE '%analysis_coding%'
    LOOP
        EXECUTE format(
            'ALTER TABLE harness_maintenance_tasks DROP CONSTRAINT %I',
            constraint_name
        );
    END LOOP;
END
$$;

ALTER TABLE harness_maintenance_tasks
    ADD CONSTRAINT harness_maintenance_tasks_task_kind_check
    CHECK (task_kind IN ('workspace_regenerate', 'provider_config_reload')),
    ADD CONSTRAINT harness_maintenance_tasks_task_shape_check
    CHECK (
        (task_kind = 'workspace_regenerate'
            AND sub_agent_id IS NULL AND run_id IS NULL)
        OR
        (task_kind = 'provider_config_reload'
            AND agent_key IS NULL AND sub_agent_id IS NULL AND run_id IS NULL)
    );
