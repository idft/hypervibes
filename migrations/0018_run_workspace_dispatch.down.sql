SET search_path TO public;

DROP TABLE IF EXISTS harness_run_runtime_credentials;

ALTER TABLE harness_sub_agents
    DROP CONSTRAINT IF EXISTS harness_sub_agents_enabled_capabilities_check,
    DROP COLUMN IF EXISTS enabled_capabilities;
