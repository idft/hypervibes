DROP INDEX IF EXISTS agents_runtime_id_idx;
DROP INDEX IF EXISTS agents_backend_kind_idx;
DROP INDEX IF EXISTS agent_runtimes_enabled_backend_kind_idx;
DROP INDEX IF EXISTS agent_runtimes_backend_kind_idx;

ALTER TABLE agents
    DROP CONSTRAINT IF EXISTS agents_backend_kind_check;

ALTER TABLE agents
    DROP COLUMN IF EXISTS runtime_config,
    DROP COLUMN IF EXISTS runtime_id,
    DROP COLUMN IF EXISTS backend_kind;

DROP TABLE IF EXISTS agent_runtimes;
