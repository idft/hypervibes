SET search_path TO public;

CREATE TABLE agent_gateways (
    agent_key TEXT NOT NULL REFERENCES agents(agent_key) ON DELETE CASCADE,
    gateway_type TEXT NOT NULL CHECK (gateway_type IN ('telegram')),
    enabled BOOLEAN NOT NULL DEFAULT FALSE,
    config JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (agent_key, gateway_type)
);

CREATE INDEX idx_agent_gateways_enabled
    ON agent_gateways (gateway_type)
    WHERE enabled = TRUE;

CREATE OR REPLACE FUNCTION public.set_agent_gateway_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER set_agent_gateway_updated_at
    BEFORE UPDATE ON agent_gateways
    FOR EACH ROW
    EXECUTE FUNCTION public.set_agent_gateway_updated_at();