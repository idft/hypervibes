SET search_path TO public;

DROP INDEX IF EXISTS notifications_source_conversation_idx;
DROP INDEX IF EXISTS notifications_source_run_idx;

ALTER TABLE notifications
    DROP CONSTRAINT IF EXISTS notifications_source_conversation_id_fkey,
    DROP CONSTRAINT IF EXISTS notifications_source_run_id_fkey,
    DROP CONSTRAINT IF EXISTS notifications_source_provenance_check,
    DROP CONSTRAINT IF EXISTS notifications_source_capability_schema_version_check,
    DROP CONSTRAINT IF EXISTS notifications_source_kind_check,
    DROP COLUMN IF EXISTS source_capability_schema_version,
    DROP COLUMN IF EXISTS source_conversation_id,
    DROP COLUMN IF EXISTS source_run_id,
    DROP COLUMN IF EXISTS source_kind;

-- A down migration must not silently turn an operator-approved notification
-- policy into deny. Default-deny rows are reproducible on re-apply; any
-- non-default policy requires an explicit data migration instead.
DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM agent_conversation_tool_policies
        WHERE tool_group = 'notifications'
          AND policy <> 'deny'
    ) THEN
        RAISE EXCEPTION
            'cannot roll back workspace persistence with non-default conversation notification policies';
    END IF;
END;
$$;

DELETE FROM agent_conversation_tool_policies WHERE tool_group = 'notifications';
ALTER TABLE agent_conversation_tool_policies
    DROP CONSTRAINT IF EXISTS agent_conversation_tool_policies_tool_group_check;
ALTER TABLE agent_conversation_tool_policies
    ADD CONSTRAINT agent_conversation_tool_policies_tool_group_check
    CHECK (tool_group IN ('orders', 'memory_writes'));

DROP TRIGGER IF EXISTS set_agent_conversation_workspaces_updated_at
    ON agent_conversation_workspaces;
DROP FUNCTION IF EXISTS public.set_agent_conversation_workspaces_updated_at();
DROP TABLE IF EXISTS agent_conversation_workspaces;

DROP TRIGGER IF EXISTS set_harness_run_workspace_artifacts_updated_at
    ON harness_run_workspace_artifacts;
DROP FUNCTION IF EXISTS public.set_harness_run_workspace_artifacts_updated_at();
DROP INDEX IF EXISTS harness_run_workspace_artifacts_expiry_idx;
DROP TABLE IF EXISTS harness_run_workspace_artifacts;

ALTER TABLE agent_conversations
    DROP CONSTRAINT IF EXISTS agent_conversations_id_agent_key_unique;
ALTER TABLE harness_sub_agent_runs
    DROP CONSTRAINT IF EXISTS harness_sub_agent_runs_id_agent_key_unique;
