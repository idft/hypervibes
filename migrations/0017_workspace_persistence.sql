SET search_path TO public;

CREATE TABLE harness_run_workspace_artifacts (
    run_id BIGINT PRIMARY KEY REFERENCES harness_sub_agent_runs(id) ON DELETE CASCADE,
    context_schema_version INTEGER NOT NULL,
    context_snapshot JSONB NOT NULL,
    capability_schema_version INTEGER NOT NULL,
    capability_snapshot JSONB NOT NULL,
    workspace_status TEXT NOT NULL DEFAULT 'preparing',
    workspace_created_at TIMESTAMPTZ,
    runtime_secrets_scrubbed_at TIMESTAMPTZ,
    terminalized_at TIMESTAMPTZ,
    expires_at TIMESTAMPTZ,
    deletion_started_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ,
    observed_size_bytes BIGINT,
    observed_file_count BIGINT,
    error_summary TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT harness_run_workspace_artifacts_context_schema_version_check
        CHECK (context_schema_version = 1),
    CONSTRAINT harness_run_workspace_artifacts_context_snapshot_check
        CHECK (jsonb_typeof(context_snapshot) = 'object'),
    CONSTRAINT harness_run_workspace_artifacts_capability_schema_version_check
        CHECK (capability_schema_version = 1),
    CONSTRAINT harness_run_workspace_artifacts_capability_snapshot_check
        CHECK (jsonb_typeof(capability_snapshot) = 'array'),
    CONSTRAINT harness_run_workspace_artifacts_status_check
        CHECK (workspace_status IN ('preparing', 'ready', 'retained', 'deleting', 'deleted', 'failed')),
    CONSTRAINT harness_run_workspace_artifacts_expiration_check
        CHECK (expires_at IS NULL OR terminalized_at IS NOT NULL),
    CONSTRAINT harness_run_workspace_artifacts_deletion_check
        CHECK (deleted_at IS NULL OR workspace_status = 'deleted'),
    CONSTRAINT harness_run_workspace_artifacts_size_check
        CHECK (observed_size_bytes IS NULL OR observed_size_bytes >= 0),
    CONSTRAINT harness_run_workspace_artifacts_file_count_check
        CHECK (observed_file_count IS NULL OR observed_file_count >= 0)
);

CREATE INDEX harness_run_workspace_artifacts_expiry_idx
    ON harness_run_workspace_artifacts (expires_at, run_id)
    WHERE workspace_status = 'retained' AND deleted_at IS NULL;

CREATE OR REPLACE FUNCTION public.set_harness_run_workspace_artifacts_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER set_harness_run_workspace_artifacts_updated_at
    BEFORE UPDATE ON harness_run_workspace_artifacts
    FOR EACH ROW
    EXECUTE FUNCTION public.set_harness_run_workspace_artifacts_updated_at();

CREATE TABLE agent_conversation_workspaces (
    conversation_id UUID PRIMARY KEY REFERENCES agent_conversations(id) ON DELETE CASCADE,
    capability_schema_version INTEGER NOT NULL DEFAULT 1,
    workspace_status TEXT NOT NULL DEFAULT 'preparing',
    workspace_created_at TIMESTAMPTZ,
    runtime_secrets_scrubbed_at TIMESTAMPTZ,
    deletion_started_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ,
    observed_size_bytes BIGINT,
    observed_file_count BIGINT,
    error_summary TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT agent_conversation_workspaces_capability_schema_version_check
        CHECK (capability_schema_version = 1),
    CONSTRAINT agent_conversation_workspaces_status_check
        CHECK (workspace_status IN ('preparing', 'ready', 'deleting', 'deleted', 'failed')),
    CONSTRAINT agent_conversation_workspaces_deletion_check
        CHECK (deleted_at IS NULL OR workspace_status = 'deleted'),
    CONSTRAINT agent_conversation_workspaces_size_check
        CHECK (observed_size_bytes IS NULL OR observed_size_bytes >= 0),
    CONSTRAINT agent_conversation_workspaces_file_count_check
        CHECK (observed_file_count IS NULL OR observed_file_count >= 0)
);

CREATE OR REPLACE FUNCTION public.set_agent_conversation_workspaces_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER set_agent_conversation_workspaces_updated_at
    BEFORE UPDATE ON agent_conversation_workspaces
    FOR EACH ROW
    EXECUTE FUNCTION public.set_agent_conversation_workspaces_updated_at();

ALTER TABLE agent_conversation_tool_policies
    DROP CONSTRAINT agent_conversation_tool_policies_tool_group_check;
ALTER TABLE agent_conversation_tool_policies
    ADD CONSTRAINT agent_conversation_tool_policies_tool_group_check
    CHECK (tool_group IN ('orders', 'memory_writes', 'notifications'));

INSERT INTO agent_conversation_tool_policies (conversation_id, tool_group, policy)
SELECT id, 'notifications', 'deny'
FROM agent_conversations
ON CONFLICT (conversation_id, tool_group) DO NOTHING;

ALTER TABLE harness_sub_agent_runs
    ADD CONSTRAINT harness_sub_agent_runs_id_agent_key_unique UNIQUE (id, agent_key);

ALTER TABLE agent_conversations
    ADD CONSTRAINT agent_conversations_id_agent_key_unique UNIQUE (id, agent_key);

ALTER TABLE notifications
    ADD COLUMN source_kind TEXT,
    ADD COLUMN source_run_id BIGINT,
    ADD COLUMN source_conversation_id UUID,
    ADD COLUMN source_capability_schema_version INTEGER;

ALTER TABLE notifications
    ADD CONSTRAINT notifications_source_kind_check
    CHECK (source_kind IS NULL OR source_kind IN ('run', 'conversation')),
    ADD CONSTRAINT notifications_source_capability_schema_version_check
    CHECK (
        source_capability_schema_version IS NULL
        OR source_capability_schema_version = 1
    ),
    ADD CONSTRAINT notifications_source_provenance_check
    CHECK ((
        (source_kind IS NULL
            AND source_run_id IS NULL
            AND source_conversation_id IS NULL
            AND source_capability_schema_version IS NULL)
        OR (source_kind = 'run'
            AND source_conversation_id IS NULL
            AND source_capability_schema_version IS NOT NULL)
        OR (source_kind = 'conversation'
            AND source_run_id IS NULL
            AND source_capability_schema_version IS NOT NULL)
    ) IS TRUE),
    ADD CONSTRAINT notifications_source_run_id_fkey
    FOREIGN KEY (source_run_id, agent_key)
    REFERENCES harness_sub_agent_runs(id, agent_key)
    ON DELETE SET NULL (source_run_id),
    ADD CONSTRAINT notifications_source_conversation_id_fkey
    FOREIGN KEY (source_conversation_id, agent_key)
    REFERENCES agent_conversations(id, agent_key)
    ON DELETE SET NULL (source_conversation_id);

CREATE INDEX notifications_source_run_idx
    ON notifications (source_run_id)
    WHERE source_run_id IS NOT NULL;
CREATE INDEX notifications_source_conversation_idx
    ON notifications (source_conversation_id)
    WHERE source_conversation_id IS NOT NULL;
