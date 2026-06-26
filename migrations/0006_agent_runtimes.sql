CREATE TABLE IF NOT EXISTS agent_runtimes (
    id TEXT PRIMARY KEY,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    name TEXT NOT NULL UNIQUE,
    backend_kind TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT true,
    base_url TEXT,
    runtime_config JSONB NOT NULL DEFAULT '{}'::jsonb,
    CHECK (backend_kind IN ('hermes', 'opencode')),
    CHECK (id ~ '^[a-z0-9]([a-z0-9-]*[a-z0-9])?$')
);

ALTER TABLE agents
    ADD COLUMN IF NOT EXISTS backend_kind TEXT NOT NULL,
    ADD COLUMN IF NOT EXISTS runtime_id TEXT NOT NULL REFERENCES agent_runtimes(id),
    ADD COLUMN IF NOT EXISTS runtime_config JSONB NOT NULL DEFAULT '{}'::jsonb;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
          FROM pg_constraint
         WHERE conname = 'agents_backend_kind_check'
    ) THEN
        ALTER TABLE agents
            ADD CONSTRAINT agents_backend_kind_check
            CHECK (backend_kind IN ('hermes', 'opencode'));
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS agent_runtimes_backend_kind_idx
    ON agent_runtimes (backend_kind);

CREATE INDEX IF NOT EXISTS agent_runtimes_enabled_backend_kind_idx
    ON agent_runtimes (enabled, backend_kind);

CREATE INDEX IF NOT EXISTS agents_backend_kind_idx
    ON agents (backend_kind);

CREATE INDEX IF NOT EXISTS agents_runtime_id_idx
    ON agents (runtime_id);

INSERT INTO agent_runtimes (
    id,
    name,
    backend_kind,
    enabled,
    base_url,
    runtime_config
) VALUES (
    'opencode-local',
    'OpenCode local',
    'opencode',
    true,
    'http://localhost:14096',
    '{}'::jsonb
) ON CONFLICT (id) DO UPDATE SET
    name = EXCLUDED.name,
    backend_kind = EXCLUDED.backend_kind,
    enabled = EXCLUDED.enabled,
    base_url = EXCLUDED.base_url,
    runtime_config = EXCLUDED.runtime_config,
    updated_at = now();
