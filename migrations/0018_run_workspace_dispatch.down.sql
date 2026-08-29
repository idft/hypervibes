SET search_path TO public;

DROP INDEX IF EXISTS harness_run_runtime_credentials_active_token_idx;
DROP TABLE IF EXISTS harness_run_runtime_credentials;

ALTER TABLE harness_sub_agents
    DROP COLUMN IF EXISTS notification_send_enabled;
