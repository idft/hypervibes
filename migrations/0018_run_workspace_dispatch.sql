SET search_path TO public;

ALTER TABLE harness_sub_agents
    ADD COLUMN notification_send_enabled BOOLEAN NOT NULL DEFAULT false;

-- Existing trading schedules retain their established notification default;
-- every other role stays denied until an explicit capability setting exists.
UPDATE harness_sub_agents
   SET notification_send_enabled = true
 WHERE sub_agent_kind = 'trading';

CREATE TABLE harness_run_runtime_credentials (
    run_id BIGINT PRIMARY KEY REFERENCES harness_sub_agent_runs(id) ON DELETE CASCADE,
    agent_key TEXT NOT NULL,
    credential_id UUID NOT NULL UNIQUE,
    token_hash TEXT NOT NULL UNIQUE,
    capability_schema_version INTEGER NOT NULL,
    api_scopes JSONB NOT NULL,
    issued_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    CONSTRAINT harness_run_runtime_credentials_capability_schema_version_check
        CHECK (capability_schema_version = 1),
    CONSTRAINT harness_run_runtime_credentials_api_scopes_check
        CHECK (jsonb_typeof(api_scopes) = 'array'),
    CONSTRAINT harness_run_runtime_credentials_expiry_check
        CHECK (expires_at > issued_at)
);

CREATE INDEX harness_run_runtime_credentials_active_token_idx
    ON harness_run_runtime_credentials (token_hash)
    WHERE revoked_at IS NULL;
