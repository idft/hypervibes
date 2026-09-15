ALTER TABLE agent_instruments RENAME TO agent_trading_instruments;

CREATE TABLE agent_analysis_instruments (
    agent_key TEXT NOT NULL REFERENCES agents(agent_key) ON DELETE CASCADE,
    instrument_id TEXT NOT NULL REFERENCES hyperliquid.instruments(instrument_id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (agent_key, instrument_id)
);

INSERT INTO agent_analysis_instruments (agent_key, instrument_id, created_at)
SELECT agent_key, instrument_id, created_at
FROM agent_trading_instruments;
