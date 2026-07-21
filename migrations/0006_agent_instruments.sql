CREATE TABLE IF NOT EXISTS agent_instruments (
    agent_key TEXT NOT NULL REFERENCES agents(agent_key) ON DELETE CASCADE,
    instrument_id TEXT NOT NULL REFERENCES hyperliquid.instruments(instrument_id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (agent_key, instrument_id)
);
