CREATE SCHEMA IF NOT EXISTS agents;

CREATE TABLE IF NOT EXISTS agents.registry (
    agent_key TEXT PRIMARY KEY,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT true,
    display_name TEXT NOT NULL,
    prompt TEXT NOT NULL DEFAULT '',
    wallet_address TEXT NOT NULL,
    environment TEXT NOT NULL DEFAULT 'live',
    api_key TEXT NOT NULL UNIQUE,
    api_key_last_used_at TIMESTAMPTZ,
    hyperliquid_private_key_ciphertext BYTEA NOT NULL,
    hyperliquid_private_key_key_id TEXT NOT NULL,
    CONSTRAINT registry_wallet_environment_unique UNIQUE (wallet_address, environment),
    CHECK (environment IN ('live', 'sandbox'))
);

CREATE UNIQUE INDEX IF NOT EXISTS registry_agent_key_idx ON agents.registry (agent_key);
CREATE UNIQUE INDEX IF NOT EXISTS registry_wallet_address_idx ON agents.registry (wallet_address);
CREATE UNIQUE INDEX IF NOT EXISTS registry_api_key_idx ON agents.registry (api_key);
