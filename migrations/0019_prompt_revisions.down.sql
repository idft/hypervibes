SET search_path TO public;

DROP INDEX IF EXISTS memory.memory_instrument_targets_target_idx;
DROP INDEX IF EXISTS memory.memory_records_type_idx;
DROP INDEX IF EXISTS memory.memory_records_source_run_idx;
DROP INDEX IF EXISTS memory.memory_records_owner_scope_idx;

DROP TABLE IF EXISTS memory.instrument_targets;

ALTER TABLE memory.records
    DROP CONSTRAINT IF EXISTS memory_records_source_run_agent_fkey,
    DROP COLUMN IF EXISTS source_run_id,
    DROP COLUMN IF EXISTS scope_kind;

-- Restore the legacy NOT NULL on `symbol`; the forward migration relaxed it
-- because scoped writes no longer populate the column.
ALTER TABLE memory.records
    ALTER COLUMN symbol SET NOT NULL;

-- Restore the replaced singleton index before dropping the revision tables.
CREATE UNIQUE INDEX harness_sub_agents_kind_timeframe_idx
    ON harness_sub_agents (agent_key, sub_agent_kind, timeframe)
    WHERE timeframe IS NOT NULL;

DROP TABLE IF EXISTS agent_strategy_prompt_revision_evidence;
DROP TABLE IF EXISTS agent_strategy_prompt_active_revisions;
DROP TABLE IF EXISTS agent_strategy_prompt_revisions;
DROP TABLE IF EXISTS agent_strategy_prompt_revision_batches;

DROP INDEX IF EXISTS harness_sub_agents_singleton_kind_idx;

ALTER TABLE harness_sub_agents
    DROP CONSTRAINT IF EXISTS harness_sub_agents_id_agent_key_unique;
