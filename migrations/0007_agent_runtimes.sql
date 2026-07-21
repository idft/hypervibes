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
