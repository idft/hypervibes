SET search_path TO public;

CREATE TABLE IF NOT EXISTS users (
    id UUID PRIMARY KEY,
    wallet_address TEXT NOT NULL UNIQUE,
    api_wallet_address TEXT UNIQUE,
    hyperliquid_private_key_ciphertext BYTEA,
    hyperliquid_private_key_key_id TEXT,
    api_wallet_approved_at TIMESTAMPTZ,
    api_wallet_expires_at TIMESTAMPTZ,
    api_wallet_expiry_checked_at TIMESTAMPTZ,
    builder_fee_tenths_of_bp SMALLINT NOT NULL DEFAULT 10,
    builder_fee_approved_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (builder_fee_tenths_of_bp BETWEEN 1 AND 100),
    CHECK (
        api_wallet_address IS NULL
        OR (
            hyperliquid_private_key_ciphertext IS NOT NULL
            AND hyperliquid_private_key_key_id IS NOT NULL
        )
    ),
    CHECK (
        api_wallet_approved_at IS NULL
        OR (
            api_wallet_address IS NOT NULL
            AND hyperliquid_private_key_ciphertext IS NOT NULL
            AND hyperliquid_private_key_key_id IS NOT NULL
        )
    )
);

CREATE TABLE IF NOT EXISTS login_challenges (
    id UUID PRIMARY KEY,
    wallet_address TEXT NOT NULL,
    nonce TEXT NOT NULL UNIQUE,
    uri TEXT NOT NULL,
    domain TEXT NOT NULL,
    chain_id BIGINT NOT NULL,
    issued_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS user_sessions (
    token_hash BYTEA PRIMARY KEY,
    csrf_hash BYTEA NOT NULL,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    issued_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    user_agent_hash BYTEA,
    ip_hash BYTEA
);

CREATE INDEX IF NOT EXISTS users_wallet_address_idx ON users (wallet_address);
CREATE INDEX IF NOT EXISTS users_api_wallet_expires_at_idx
    ON users (api_wallet_expires_at) WHERE api_wallet_expires_at IS NOT NULL;
CREATE INDEX IF NOT EXISTS user_sessions_user_id_idx ON user_sessions (user_id);
