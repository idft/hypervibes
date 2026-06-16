CREATE SCHEMA IF NOT EXISTS hyperliquid;

CREATE TABLE IF NOT EXISTS hyperliquid.sync_state (
    account_address TEXT NOT NULL,
    environment TEXT NOT NULL,
    stream_name TEXT NOT NULL,
    last_event_time TIMESTAMPTZ,
    last_event_key TEXT,
    last_synced_at TIMESTAMPTZ,
    status TEXT NOT NULL,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    PRIMARY KEY (account_address, environment, stream_name),
    -- testnet/sandbox intentionally unsupported for now; widen this set to re-add.
    CHECK (environment IN ('live'))
);

CREATE TABLE IF NOT EXISTS hyperliquid.instruments (
    instrument_id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    market_type TEXT NOT NULL,
    base_asset TEXT NOT NULL,
    quote_asset TEXT NOT NULL,
    settlement_asset TEXT,
    asset_index INTEGER,
    price_decimals INTEGER NOT NULL,
    size_decimals INTEGER NOT NULL,
    lot_size NUMERIC(38, 18) NOT NULL,
    max_leverage INTEGER,
    is_hip3 BOOLEAN NOT NULL DEFAULT FALSE,
    active BOOLEAN NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS instruments_name_idx ON hyperliquid.instruments(name);
CREATE INDEX IF NOT EXISTS instruments_market_type_idx ON hyperliquid.instruments(market_type);

CREATE TABLE IF NOT EXISTS hyperliquid.trade_fills (
    hash TEXT NOT NULL,
    account_address TEXT NOT NULL,
    environment TEXT NOT NULL,
    event_time TIMESTAMPTZ NOT NULL,
    event_type TEXT NOT NULL DEFAULT 'fill',
    source_stream TEXT NOT NULL,
    instrument_id TEXT,
    asset TEXT,
    symbol TEXT,
    fee_usdc NUMERIC(38, 18),
    realized_pnl_usdc NUMERIC(38, 18),
    fill_time TIMESTAMPTZ NOT NULL,
    direction TEXT NOT NULL,
    side TEXT NOT NULL,
    price NUMERIC(38, 18) NOT NULL,
    size NUMERIC(38, 18) NOT NULL,
    trade_value NUMERIC(38, 18),
    order_id TEXT,
    trade_id TEXT NOT NULL,
    start_position NUMERIC(38, 18),
    fee NUMERIC(38, 18),
    fee_token TEXT,
    builder_fee NUMERIC(38, 18),
    crossed BOOLEAN,
    tx_hash TEXT,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    ingest_source TEXT NOT NULL,
    inserted_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (hash, trade_id),
    FOREIGN KEY (instrument_id) REFERENCES hyperliquid.instruments(instrument_id),
    CHECK (environment IN ('live'))
);

CREATE INDEX IF NOT EXISTS trade_fills_account_event_time_idx ON hyperliquid.trade_fills(account_address, environment, event_time DESC);
CREATE INDEX IF NOT EXISTS trade_fills_account_fill_time_idx ON hyperliquid.trade_fills(account_address, environment, fill_time DESC);
CREATE INDEX IF NOT EXISTS trade_fills_account_instrument_event_time_idx ON hyperliquid.trade_fills(account_address, environment, instrument_id, event_time DESC);
CREATE INDEX IF NOT EXISTS trade_fills_account_instrument_fill_time_idx ON hyperliquid.trade_fills(account_address, environment, instrument_id, fill_time DESC);
CREATE INDEX IF NOT EXISTS trade_fills_trade_id_idx ON hyperliquid.trade_fills(trade_id);

CREATE TABLE IF NOT EXISTS hyperliquid.funding_events (
    account_address TEXT NOT NULL,
    environment TEXT NOT NULL,
    instrument_id TEXT NOT NULL,
    event_time TIMESTAMPTZ NOT NULL,
    event_type TEXT NOT NULL DEFAULT 'funding',
    source_stream TEXT NOT NULL,
    asset TEXT,
    symbol TEXT,
    fee_usdc NUMERIC(38, 18),
    realized_pnl_usdc NUMERIC(38, 18),
    usdc NUMERIC(38, 18) NOT NULL,
    position_size NUMERIC(38, 18),
    funding_rate NUMERIC(38, 18),
    hash TEXT,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    ingest_source TEXT NOT NULL,
    inserted_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (account_address, environment, instrument_id, event_time),
    FOREIGN KEY (instrument_id) REFERENCES hyperliquid.instruments(instrument_id),
    CHECK (environment IN ('live'))
);

CREATE INDEX IF NOT EXISTS funding_events_account_event_time_idx ON hyperliquid.funding_events(account_address, environment, event_time DESC);
CREATE INDEX IF NOT EXISTS funding_events_account_instrument_event_time_idx ON hyperliquid.funding_events(account_address, environment, instrument_id, event_time DESC);

CREATE TABLE IF NOT EXISTS hyperliquid.ledger_events (
    hash TEXT PRIMARY KEY,
    account_address TEXT NOT NULL,
    environment TEXT NOT NULL,
    event_time TIMESTAMPTZ NOT NULL,
    event_type TEXT NOT NULL,
    source_stream TEXT NOT NULL,
    instrument_id TEXT,
    asset TEXT,
    symbol TEXT,
    fee_usdc NUMERIC(38, 18),
    realized_pnl_usdc NUMERIC(38, 18),
    ledger_type TEXT NOT NULL,
    usdc NUMERIC(38, 18),
    token TEXT,
    amount NUMERIC(38, 18),
    fee NUMERIC(38, 18),
    source_user TEXT,
    destination_user TEXT,
    tx_hash TEXT,
    details JSONB NOT NULL DEFAULT '{}'::jsonb,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    ingest_source TEXT NOT NULL,
    inserted_at TIMESTAMPTZ NOT NULL,
    FOREIGN KEY (instrument_id) REFERENCES hyperliquid.instruments(instrument_id),
    CHECK (environment IN ('live'))
);

CREATE INDEX IF NOT EXISTS ledger_events_account_event_time_idx ON hyperliquid.ledger_events(account_address, environment, event_time DESC);
CREATE INDEX IF NOT EXISTS ledger_events_account_ledger_type_event_time_idx ON hyperliquid.ledger_events(account_address, environment, ledger_type, event_time DESC);

CREATE TABLE IF NOT EXISTS hyperliquid.historical_orders (
    account_address TEXT NOT NULL,
    environment TEXT NOT NULL,
    order_id TEXT NOT NULL,
    event_time TIMESTAMPTZ NOT NULL,
    source_stream TEXT NOT NULL DEFAULT 'historical_orders',
    instrument_id TEXT,
    asset TEXT,
    symbol TEXT,
    order_status TEXT,
    side TEXT,
    order_type TEXT,
    price NUMERIC(38, 18),
    size NUMERIC(38, 18),
    filled_size NUMERIC(38, 18),
    reduce_only BOOLEAN,
    time_in_force TEXT,
    client_order_id TEXT,
    status_timestamp TIMESTAMPTZ,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    ingest_source TEXT NOT NULL,
    inserted_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (account_address, environment, order_id),
    FOREIGN KEY (instrument_id) REFERENCES hyperliquid.instruments(instrument_id),
    CHECK (environment IN ('live'))
);

CREATE INDEX IF NOT EXISTS historical_orders_account_event_time_idx ON hyperliquid.historical_orders(account_address, environment, event_time DESC);
CREATE INDEX IF NOT EXISTS historical_orders_account_instrument_event_time_idx ON hyperliquid.historical_orders(account_address, environment, instrument_id, event_time DESC);
CREATE INDEX IF NOT EXISTS historical_orders_client_order_id_idx ON hyperliquid.historical_orders(client_order_id);

CREATE OR REPLACE VIEW hyperliquid.account_timeline AS
SELECT
    hash AS event_id,
    account_address,
    environment,
    event_time,
    event_type,
    source_stream,
    instrument_id,
    asset,
    symbol,
    fee_usdc,
    realized_pnl_usdc,
    COALESCE(realized_pnl_usdc, 0) - COALESCE(fee_usdc, 0) AS usdc_delta,
    payload,
    ingest_source,
    inserted_at,
    'fill'::text AS event_category
FROM hyperliquid.trade_fills

UNION ALL

SELECT
    account_address || ':' || environment || ':' || instrument_id || ':' || to_char(event_time, 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS event_id,
    account_address,
    environment,
    event_time,
    event_type,
    source_stream,
    instrument_id,
    asset,
    symbol,
    fee_usdc,
    realized_pnl_usdc,
    usdc AS usdc_delta,
    payload,
    ingest_source,
    inserted_at,
    'funding'::text
FROM hyperliquid.funding_events

UNION ALL

SELECT
    hash AS event_id,
    account_address,
    environment,
    event_time,
    event_type,
    source_stream,
    instrument_id,
    asset,
    symbol,
    fee_usdc,
    realized_pnl_usdc,
    COALESCE(usdc, 0) - COALESCE(fee, 0) AS usdc_delta,
    payload,
    ingest_source,
    inserted_at,
    'ledger'::text
FROM hyperliquid.ledger_events;
