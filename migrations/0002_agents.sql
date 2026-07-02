CREATE TABLE IF NOT EXISTS agents (
    agent_key TEXT PRIMARY KEY,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT true,
    display_name TEXT NOT NULL,
    analysis_prompt TEXT NOT NULL DEFAULT '',
    trading_prompt TEXT NOT NULL DEFAULT '',
    wallet_address TEXT NOT NULL,
    environment TEXT NOT NULL DEFAULT 'live',
    api_key TEXT NOT NULL UNIQUE,
    api_key_last_used_at TIMESTAMPTZ,
    hyperliquid_private_key_ciphertext BYTEA NOT NULL,
    hyperliquid_private_key_key_id TEXT NOT NULL,
    CONSTRAINT registry_wallet_environment_unique UNIQUE (wallet_address, environment),
    CHECK (environment IN ('live', 'sandbox'))
);

CREATE UNIQUE INDEX IF NOT EXISTS agents_agent_key_idx ON agents (agent_key);
CREATE UNIQUE INDEX IF NOT EXISTS agents_wallet_address_idx ON agents (wallet_address);
CREATE UNIQUE INDEX IF NOT EXISTS agents_api_key_idx ON agents (api_key);

ALTER TABLE hyperliquid.sync_state
    ADD CONSTRAINT sync_state_agent_account_fkey
    FOREIGN KEY (account_address, environment)
    REFERENCES agents(wallet_address, environment)
    ON DELETE CASCADE;

ALTER TABLE hyperliquid.trade_fills
    ADD CONSTRAINT trade_fills_agent_account_fkey
    FOREIGN KEY (account_address, environment)
    REFERENCES agents(wallet_address, environment)
    ON DELETE CASCADE;

ALTER TABLE hyperliquid.funding_events
    ADD CONSTRAINT funding_events_agent_account_fkey
    FOREIGN KEY (account_address, environment)
    REFERENCES agents(wallet_address, environment)
    ON DELETE CASCADE;

ALTER TABLE hyperliquid.ledger_events
    ADD CONSTRAINT ledger_events_agent_account_fkey
    FOREIGN KEY (account_address, environment)
    REFERENCES agents(wallet_address, environment)
    ON DELETE CASCADE;

ALTER TABLE hyperliquid.historical_orders
    ADD CONSTRAINT historical_orders_agent_account_fkey
    FOREIGN KEY (account_address, environment)
    REFERENCES agents(wallet_address, environment)
    ON DELETE CASCADE;
