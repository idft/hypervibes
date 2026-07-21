SET search_path TO public;

CREATE TABLE IF NOT EXISTS agent_runtimes (
    id TEXT PRIMARY KEY,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    name TEXT NOT NULL UNIQUE,
    backend_kind TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT true,
    base_url TEXT,
    runtime_config JSONB NOT NULL DEFAULT '{}'::jsonb,
    CHECK (backend_kind IN ('opencode')),
    CHECK (id ~ '^[a-z0-9]([a-z0-9-]*[a-z0-9])?$')
);

CREATE TABLE IF NOT EXISTS agents (
    agent_key TEXT PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT true,
    display_name TEXT NOT NULL,
    trading_account_address TEXT,
    environment TEXT NOT NULL DEFAULT 'live',
    api_key TEXT NOT NULL UNIQUE,
    api_key_last_used_at TIMESTAMPTZ,
    backend_kind TEXT NOT NULL,
    runtime_id TEXT NOT NULL REFERENCES agent_runtimes(id),
    runtime_config JSONB NOT NULL DEFAULT '{}'::jsonb,
    lifecycle TEXT NOT NULL DEFAULT 'pending_subaccount_creation',
    CHECK (environment IN ('live', 'sandbox')),
    CHECK (backend_kind IN ('opencode')),
    CHECK (lifecycle IN ('pending_subaccount_creation', 'pending_funding', 'active')),
    CHECK (
        lifecycle = 'pending_subaccount_creation'
        OR trading_account_address IS NOT NULL
    )
);

CREATE UNIQUE INDEX IF NOT EXISTS agents_agent_key_idx ON agents (agent_key);
CREATE UNIQUE INDEX IF NOT EXISTS agents_unique_trading_account_per_environment_idx
    ON agents (trading_account_address, environment);
CREATE UNIQUE INDEX IF NOT EXISTS agents_api_key_idx ON agents (api_key);
CREATE INDEX IF NOT EXISTS agents_user_id_idx ON agents (user_id);
CREATE INDEX IF NOT EXISTS agents_backend_kind_idx ON agents (backend_kind);
CREATE INDEX IF NOT EXISTS agents_runtime_id_idx ON agents (runtime_id);
CREATE INDEX IF NOT EXISTS agents_user_id_lifecycle_idx ON agents (user_id, lifecycle);
CREATE INDEX IF NOT EXISTS agent_runtimes_backend_kind_idx ON agent_runtimes (backend_kind);
CREATE INDEX IF NOT EXISTS agent_runtimes_enabled_backend_kind_idx ON agent_runtimes (enabled, backend_kind);

ALTER TABLE hyperliquid.sync_state
    ADD CONSTRAINT sync_state_agent_account_fkey
    FOREIGN KEY (account_address, environment)
    REFERENCES agents(trading_account_address, environment)
    ON DELETE CASCADE;

ALTER TABLE hyperliquid.trade_fills
    ADD CONSTRAINT trade_fills_agent_account_fkey
    FOREIGN KEY (account_address, environment)
    REFERENCES agents(trading_account_address, environment)
    ON DELETE CASCADE;

ALTER TABLE hyperliquid.funding_events
    ADD CONSTRAINT funding_events_agent_account_fkey
    FOREIGN KEY (account_address, environment)
    REFERENCES agents(trading_account_address, environment)
    ON DELETE CASCADE;

ALTER TABLE hyperliquid.ledger_events
    ADD CONSTRAINT ledger_events_agent_account_fkey
    FOREIGN KEY (account_address, environment)
    REFERENCES agents(trading_account_address, environment)
    ON DELETE CASCADE;

ALTER TABLE hyperliquid.historical_orders
    ADD CONSTRAINT historical_orders_agent_account_fkey
    FOREIGN KEY (account_address, environment)
    REFERENCES agents(trading_account_address, environment)
    ON DELETE CASCADE;
