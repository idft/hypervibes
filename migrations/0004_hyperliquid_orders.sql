CREATE TABLE IF NOT EXISTS hyperliquid.orders (
    id UUID PRIMARY KEY,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    agent_key TEXT NOT NULL,
    account_address TEXT NOT NULL,
    environment TEXT NOT NULL,
    group_id UUID,
    parent_cloid TEXT,
    memory_record_ids JSONB NOT NULL DEFAULT '[]'::jsonb,
    symbol TEXT NOT NULL,
    instrument_id TEXT,
    side TEXT NOT NULL,
    order_kind TEXT NOT NULL,
    reduce_only BOOLEAN NOT NULL DEFAULT false,
    requested_price NUMERIC(38, 18),
    rounded_price NUMERIC(38, 18),
    requested_size NUMERIC(38, 18) NOT NULL,
    rounded_size NUMERIC(38, 18),
    trigger_price NUMERIC(38, 18),
    time_in_force TEXT,
    cloid TEXT NOT NULL,
    exchange_oid TEXT,
    status TEXT NOT NULL,
    status_detail TEXT,
    filled_size NUMERIC(38, 18),
    avg_fill_price NUMERIC(38, 18),
    request_payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    response_payload JSONB,
    CONSTRAINT orders_cloid_unique UNIQUE (account_address, environment, cloid),
    FOREIGN KEY (agent_key) REFERENCES agents(agent_key) ON DELETE CASCADE,
    FOREIGN KEY (instrument_id) REFERENCES hyperliquid.instruments(instrument_id),
    CHECK (environment IN ('live')),
    CHECK (side IN ('buy', 'sell')),
    CHECK (order_kind IN ('limit', 'market', 'take_profit', 'stop_loss')),
    CHECK (status IN (
        'pending_submission','submitted','resting','partially_filled',
        'filled','canceled','rejected','error','unknown'
    ))
);

CREATE INDEX IF NOT EXISTS orders_agent_created_idx ON hyperliquid.orders(agent_key, created_at DESC);
CREATE INDEX IF NOT EXISTS orders_account_status_idx ON hyperliquid.orders(account_address, environment, status);
CREATE INDEX IF NOT EXISTS orders_group_idx ON hyperliquid.orders(group_id);
CREATE INDEX IF NOT EXISTS orders_exchange_oid_idx ON hyperliquid.orders(exchange_oid);

CREATE TABLE IF NOT EXISTS hyperliquid.order_events (
    id UUID PRIMARY KEY,
    order_id UUID NOT NULL,
    account_address TEXT NOT NULL,
    environment TEXT NOT NULL,
    status TEXT NOT NULL,
    status_detail TEXT,
    filled_size NUMERIC(38, 18),
    avg_fill_price NUMERIC(38, 18),
    status_timestamp TIMESTAMPTZ NOT NULL,
    source TEXT NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    inserted_at TIMESTAMPTZ NOT NULL,
    FOREIGN KEY (order_id) REFERENCES hyperliquid.orders(id) ON DELETE CASCADE,
    CONSTRAINT order_events_dedup_unique UNIQUE (order_id, status, status_timestamp, source),
    CHECK (environment IN ('live')),
    CHECK (source IN ('http_response', 'ws_order_update', 'reconcile'))
);

CREATE INDEX IF NOT EXISTS order_events_order_time_idx ON hyperliquid.order_events(order_id, status_timestamp);
