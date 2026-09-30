-- Rollback removes this feature's projected cycles, notes and conversation
-- journal permissions. Exchange-source fills, funding and ledger rows remain.

DROP TRIGGER remove_agent_journal_projection ON public.agents;
DROP FUNCTION hyperliquid.remove_agent_journal_projection();
DROP TRIGGER trade_fill_projection_dirty ON hyperliquid.trade_fills;
DROP FUNCTION hyperliquid.mark_trade_projection_dirty();

DROP TABLE hyperliquid.trade_projection_dirty;
DROP TABLE hyperliquid.journal_notes;
DROP TABLE hyperliquid.trade_cycle_fills;
DROP TABLE hyperliquid.trade_cycles;

DELETE FROM public.agent_conversation_tool_policies WHERE tool_group = 'journal_writes';
ALTER TABLE public.agent_conversation_tool_policies
    DROP CONSTRAINT agent_conversation_tool_policies_tool_group_check;
ALTER TABLE public.agent_conversation_tool_policies
    ADD CONSTRAINT agent_conversation_tool_policies_tool_group_check
    CHECK (tool_group IN ('orders', 'memory_writes', 'notifications'));

-- CREATE OR REPLACE cannot remove the appended activity_type and token columns;
-- recreate the view with precisely its pre-0026 shape and cash-flow semantics.
DROP VIEW hyperliquid.account_timeline;
CREATE VIEW hyperliquid.account_timeline AS
SELECT
    hash || ':' || trade_id AS event_id,
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

DROP FUNCTION hyperliquid.ledger_usdc_delta(TEXT, TEXT, NUMERIC, TEXT, NUMERIC, NUMERIC, JSONB);
