SET search_path TO public;

ALTER TABLE harness_sub_agents
    ADD COLUMN enabled_capabilities JSONB NOT NULL DEFAULT '[]'::jsonb,
    ADD CONSTRAINT harness_sub_agents_enabled_capabilities_check
        CHECK (jsonb_typeof(enabled_capabilities) = 'array');

-- Existing trading schedules retain their established notification capability;
-- every other role stays denied until explicitly configured. The daily_review
-- seed below is converted to `review` by migration 0019, and its capability
-- string remains valid under the Review role.
UPDATE harness_sub_agents
   SET enabled_capabilities = CASE sub_agent_kind
        WHEN 'trading' THEN '["hypervibes:notification_send"]'::jsonb
        WHEN 'daily_review' THEN '["hypervibes:prompt_revision_submit"]'::jsonb
        ELSE '[]'::jsonb
    END;

CREATE TABLE harness_run_runtime_credentials (
    run_id BIGINT PRIMARY KEY,
    agent_key TEXT NOT NULL,
    credential_id UUID NOT NULL UNIQUE,
    token_hash TEXT NOT NULL UNIQUE,
    capability_schema_version INTEGER NOT NULL,
    api_scopes JSONB NOT NULL,
    issued_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    CONSTRAINT harness_run_runtime_credentials_capability_schema_version_check
        CHECK (capability_schema_version = 2),
    CONSTRAINT harness_run_runtime_credentials_api_scopes_check
        CHECK (jsonb_typeof(api_scopes) = 'array'),
    CONSTRAINT harness_run_runtime_credentials_expiry_check
        CHECK (expires_at > issued_at),
    CONSTRAINT harness_run_runtime_credentials_run_agent_fkey
        FOREIGN KEY (run_id, agent_key)
        REFERENCES harness_sub_agent_runs(id, agent_key)
        ON DELETE CASCADE
);
