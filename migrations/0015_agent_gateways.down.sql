SET search_path TO public;

DROP TRIGGER IF EXISTS set_agent_gateway_updated_at ON agent_gateways;
DROP FUNCTION IF EXISTS public.set_agent_gateway_updated_at();
DROP INDEX IF EXISTS idx_agent_gateways_enabled;
DROP TABLE IF EXISTS agent_gateways;