SET search_path TO public;

DROP INDEX IF EXISTS orders_memory_record_ids_gin_idx;

ALTER TABLE hyperliquid.orders
    DROP CONSTRAINT IF EXISTS orders_attribution_source_check;

ALTER TABLE hyperliquid.orders
    DROP COLUMN IF EXISTS attribution_source;

DROP INDEX IF EXISTS memory_links_agent_type_idx;
DROP INDEX IF EXISTS memory_links_agent_target_idx;
DROP INDEX IF EXISTS memory_links_agent_source_idx;
DROP TABLE IF EXISTS memory.links;

ALTER TABLE memory.records
    DROP CONSTRAINT IF EXISTS records_id_agent_key_unique;

DROP TABLE IF EXISTS agent_strategy_prompts;
