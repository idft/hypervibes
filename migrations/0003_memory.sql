CREATE SCHEMA IF NOT EXISTS memory;

CREATE TABLE IF NOT EXISTS memory.records (
    id UUID PRIMARY KEY,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    agent_key TEXT NOT NULL REFERENCES agents.registry(agent_key) ON DELETE CASCADE,
    symbol TEXT NOT NULL,
    timeframe TEXT,
    memory_type TEXT NOT NULL,
    summary TEXT NOT NULL,
    content TEXT NOT NULL,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb
);

CREATE INDEX IF NOT EXISTS records_agent_symbol_tf_created_idx
    ON memory.records (agent_key, symbol, timeframe, created_at DESC);

CREATE INDEX IF NOT EXISTS records_agent_created_idx
    ON memory.records (agent_key, created_at DESC);
