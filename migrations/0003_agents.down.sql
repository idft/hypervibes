ALTER TABLE hyperliquid.sync_state
    DROP CONSTRAINT IF EXISTS sync_state_agent_account_fkey;

ALTER TABLE hyperliquid.trade_fills
    DROP CONSTRAINT IF EXISTS trade_fills_agent_account_fkey;

ALTER TABLE hyperliquid.funding_events
    DROP CONSTRAINT IF EXISTS funding_events_agent_account_fkey;

ALTER TABLE hyperliquid.ledger_events
    DROP CONSTRAINT IF EXISTS ledger_events_agent_account_fkey;

ALTER TABLE hyperliquid.historical_orders
    DROP CONSTRAINT IF EXISTS historical_orders_agent_account_fkey;

DROP TABLE IF EXISTS agents;
DROP TABLE IF EXISTS agent_runtimes;
