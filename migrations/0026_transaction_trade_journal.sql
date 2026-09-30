-- Classify source ledger events without rewriting exchange data. NULL means that
-- the event cannot yet be accounted for, not that it moved zero USDC.
CREATE FUNCTION hyperliquid.ledger_usdc_delta(
    account TEXT, kind TEXT, usdc NUMERIC, token TEXT, amount NUMERIC,
    fee NUMERIC, details JSONB
) RETURNS NUMERIC LANGUAGE SQL IMMUTABLE AS $$
    SELECT CASE
        WHEN kind IN ('deposit', 'withdraw', 'withdrawal') THEN usdc
        WHEN kind = 'send' AND upper(token) = 'USDC' AND amount >= 0
             AND details->>'user' IS NOT NULL AND details->>'destination' IS NOT NULL
             AND (lower(details->>'user') = lower(account)
                  OR lower(details->>'destination') = lower(account))
             AND (fee IS NULL OR fee = 0 OR upper(details->>'feeToken') = 'USDC')
        THEN
            CASE
                WHEN lower(details->>'user') = lower(account)
                     AND lower(details->>'destination') = lower(account) THEN 0
                WHEN lower(details->>'user') = lower(account) THEN -amount
                ELSE amount
            END
            - CASE WHEN lower(details->>'user') = lower(account)
                   THEN coalesce(fee, 0) ELSE 0 END
        ELSE NULL
    END
$$;

-- PostgreSQL requires the original columns of a replaced view to retain their
-- order and types. New identity/coverage fields are appended at the end.
CREATE OR REPLACE VIEW hyperliquid.account_timeline AS
SELECT hash || ':' || trade_id AS event_id, account_address, environment,
       event_time, event_type, source_stream, instrument_id, asset, symbol,
       (CASE WHEN upper(fee_token) = 'USDC' THEN fee ELSE NULL END)::numeric(38,18) AS fee_usdc,
       realized_pnl_usdc,
       CASE WHEN upper(fee_token) = 'USDC' AND fee IS NOT NULL
            THEN coalesce(realized_pnl_usdc, 0) - fee ELSE NULL END AS usdc_delta,
       payload, ingest_source, inserted_at, 'fill'::text AS event_category,
       CASE WHEN instrument_id IS NULL THEN 'fill (unresolved instrument)'
            ELSE 'fill ' || side END AS activity_type, fee_token AS token
FROM hyperliquid.trade_fills
UNION ALL
SELECT account_address || ':' || environment || ':' || instrument_id || ':' ||
       to_char(event_time, 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"'),
       account_address, environment, event_time, event_type, source_stream,
       instrument_id, asset, symbol, fee_usdc, realized_pnl_usdc, usdc,
       payload, ingest_source, inserted_at, 'funding'::text, 'funding'::text, 'USDC'::text
FROM hyperliquid.funding_events
UNION ALL
SELECT hash, account_address, environment, event_time, event_type, source_stream,
       instrument_id, asset, symbol,
       (CASE WHEN upper(details->>'feeToken') = 'USDC' THEN fee ELSE NULL END)::numeric(38,18),
       realized_pnl_usdc,
       hyperliquid.ledger_usdc_delta(account_address, ledger_type, usdc, token, amount, fee, details),
       payload, ingest_source, inserted_at, 'ledger'::text,
       CASE WHEN ledger_type = 'send' AND lower(details->>'user') = lower(account_address)
                 AND lower(details->>'destination') <> lower(account_address) THEN 'send outbound'
            WHEN ledger_type = 'send' AND lower(details->>'destination') = lower(account_address)
                 AND lower(details->>'user') <> lower(account_address) THEN 'send inbound'
            WHEN ledger_type = 'send' AND lower(details->>'user') = lower(account_address)
                 AND lower(details->>'destination') = lower(account_address) THEN 'send internal'
            ELSE ledger_type END,
       token
FROM hyperliquid.ledger_events;

CREATE TABLE hyperliquid.trade_cycles (
    id UUID PRIMARY KEY,
    account_address TEXT NOT NULL,
    environment TEXT NOT NULL CHECK (environment IN ('live')),
    instrument_id TEXT NOT NULL REFERENCES hyperliquid.instruments(instrument_id),
    opening_hash TEXT NOT NULL,
    opening_trade_id TEXT NOT NULL,
    opening_segment TEXT NOT NULL CHECK (opening_segment IN ('entry', 'flip_entry')),
    direction TEXT NOT NULL CHECK (direction IN ('long', 'short', 'unknown')),
    status TEXT NOT NULL CHECK (status IN ('open', 'closed', 'incomplete', 'superseded')),
    opened_at TIMESTAMPTZ NOT NULL,
    last_activity_at TIMESTAMPTZ NOT NULL,
    closed_at TIMESTAMPTZ,
    remaining_size NUMERIC(38,18) NOT NULL,
    entry_size NUMERIC(38,18) NOT NULL,
    exit_size NUMERIC(38,18) NOT NULL,
    entry_notional NUMERIC(38,18) NOT NULL,
    exit_notional NUMERIC(38,18) NOT NULL,
    gross_pnl NUMERIC(38,18) NOT NULL,
    usdc_fees NUMERIC(38,18) NOT NULL,
    net_pnl NUMERIC(38,18) NOT NULL,
    fee_coverage BOOLEAN NOT NULL DEFAULT TRUE,
    coverage_reason TEXT,
    revision BIGINT NOT NULL DEFAULT 1,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (account_address, environment, instrument_id, opening_hash, opening_trade_id, opening_segment, direction)
);
CREATE INDEX trade_cycles_account_time_idx ON hyperliquid.trade_cycles(account_address, environment, last_activity_at DESC, id);
CREATE INDEX trade_cycles_instrument_idx ON hyperliquid.trade_cycles(account_address, environment, instrument_id, opened_at);

CREATE TABLE hyperliquid.trade_cycle_fills (
    cycle_id UUID NOT NULL REFERENCES hyperliquid.trade_cycles(id),
    hash TEXT NOT NULL,
    trade_id TEXT NOT NULL,
    segment TEXT NOT NULL CHECK (segment IN ('entry', 'exit', 'flip_entry')),
    quantity NUMERIC(38,18) NOT NULL CHECK (quantity > 0),
    fee_usdc NUMERIC(38,18),
    realized_pnl_usdc NUMERIC(38,18) NOT NULL,
    PRIMARY KEY (cycle_id, hash, trade_id, segment),
    FOREIGN KEY (hash, trade_id) REFERENCES hyperliquid.trade_fills(hash, trade_id)
);
CREATE INDEX trade_cycle_fills_source_idx ON hyperliquid.trade_cycle_fills(hash, trade_id);

CREATE TABLE hyperliquid.journal_notes (
    id UUID PRIMARY KEY,
    account_address TEXT NOT NULL,
    environment TEXT NOT NULL CHECK (environment IN ('live')),
    target_kind TEXT NOT NULL CHECK (target_kind IN ('trade', 'fill', 'funding', 'ledger')),
    trade_id UUID REFERENCES hyperliquid.trade_cycles(id),
    event_id TEXT,
    author_kind TEXT NOT NULL CHECK (author_kind IN ('human', 'agent')),
    author_id TEXT NOT NULL CHECK (length(author_id) > 0),
    source_run_id BIGINT,
    source_conversation_id UUID,
    body TEXT NOT NULL CHECK (length(btrim(body)) > 0 AND length(body) <= 4000),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((target_kind = 'trade' AND trade_id IS NOT NULL AND event_id IS NULL)
        OR (target_kind <> 'trade' AND trade_id IS NULL AND event_id IS NOT NULL))
);
CREATE INDEX journal_notes_trade_idx ON hyperliquid.journal_notes(account_address, environment, trade_id, created_at);
CREATE INDEX journal_notes_event_idx ON hyperliquid.journal_notes(account_address, environment, target_kind, event_id, created_at);
CREATE INDEX journal_notes_recent_idx ON hyperliquid.journal_notes(account_address, environment, created_at DESC);

-- A fill commit and a failed subsequent projection rebuild cannot lose work.
ALTER TABLE public.agent_conversation_tool_policies
    DROP CONSTRAINT agent_conversation_tool_policies_tool_group_check;
ALTER TABLE public.agent_conversation_tool_policies
    ADD CONSTRAINT agent_conversation_tool_policies_tool_group_check
    CHECK (tool_group IN ('orders', 'memory_writes', 'notifications', 'journal_writes'));
INSERT INTO public.agent_conversation_tool_policies(conversation_id, tool_group, policy)
SELECT id, 'journal_writes', 'deny' FROM public.agent_conversations ON CONFLICT DO NOTHING;

CREATE TABLE hyperliquid.trade_projection_dirty (
    account_address TEXT NOT NULL,
    environment TEXT NOT NULL,
    instrument_id TEXT NOT NULL,
    changed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (account_address, environment, instrument_id)
);
CREATE FUNCTION hyperliquid.mark_trade_projection_dirty() RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'UPDATE' AND NEW IS NOT DISTINCT FROM OLD THEN
        RETURN NEW;
    END IF;
    IF NEW.instrument_id IS NOT NULL THEN
        INSERT INTO hyperliquid.trade_projection_dirty(account_address, environment, instrument_id)
        VALUES (NEW.account_address, NEW.environment, NEW.instrument_id)
        ON CONFLICT (account_address, environment, instrument_id)
        DO UPDATE SET changed_at = now();
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER trade_fill_projection_dirty AFTER INSERT OR UPDATE ON hyperliquid.trade_fills
FOR EACH ROW EXECUTE FUNCTION hyperliquid.mark_trade_projection_dirty();
INSERT INTO hyperliquid.trade_projection_dirty(account_address, environment, instrument_id)
SELECT DISTINCT account_address, environment, instrument_id FROM hyperliquid.trade_fills
WHERE instrument_id IS NOT NULL ON CONFLICT DO NOTHING;

-- Agent deletion already removes its locally synced exchange history. Dispose
-- of projections first so source fill FK restrictions do not block deletion.
CREATE FUNCTION hyperliquid.remove_agent_journal_projection() RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
    DELETE FROM hyperliquid.journal_notes WHERE account_address = OLD.trading_account_address
        AND environment = OLD.environment;
    DELETE FROM hyperliquid.trade_cycle_fills WHERE cycle_id IN
        (SELECT id FROM hyperliquid.trade_cycles WHERE account_address = OLD.trading_account_address
         AND environment = OLD.environment);
    DELETE FROM hyperliquid.trade_cycles WHERE account_address = OLD.trading_account_address
        AND environment = OLD.environment;
    DELETE FROM hyperliquid.trade_projection_dirty WHERE account_address = OLD.trading_account_address
        AND environment = OLD.environment;
    RETURN OLD;
END $$;
CREATE TRIGGER remove_agent_journal_projection BEFORE DELETE ON public.agents
FOR EACH ROW EXECUTE FUNCTION hyperliquid.remove_agent_journal_projection();
