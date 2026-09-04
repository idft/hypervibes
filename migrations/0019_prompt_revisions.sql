SET search_path TO public;

-- Composite ownership foreign keys below require an explicit candidate key.
-- `id` remains the primary identity; `(id, agent_key)` prevents a child row
-- from pairing an otherwise valid id with a different agent.
ALTER TABLE harness_sub_agents
    ADD CONSTRAINT harness_sub_agents_id_agent_key_unique UNIQUE (id, agent_key);

-- Each sub-agent now owns its complete versioned strategy prompt. The legacy
-- operator prompt was only needed to differentiate jobs sharing one prompt.
ALTER TABLE harness_sub_agents
    DROP COLUMN operator_prompt;

-- ============================================================================
-- Prompt revisions become sub-agent targeted. A revision records the
-- `harness_sub_agents.id` it applies to (owned by the same agent) plus the
-- target's sub_agent_key for readable audit history. Batches keep provenance
-- (review runs), evidence links, and one Review batch per Review run.
-- ============================================================================

CREATE TABLE agent_strategy_prompt_revision_batches (
    id BIGSERIAL PRIMARY KEY,
    agent_key TEXT NOT NULL REFERENCES agents(agent_key) ON DELETE CASCADE,
    source_type TEXT NOT NULL,
    source_run_id BIGINT,
    rationale TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT agent_strategy_prompt_revision_batches_id_agent_key_unique UNIQUE (id, agent_key),
    CONSTRAINT agent_strategy_prompt_revision_batches_source_run_agent_fkey
        FOREIGN KEY (source_run_id, agent_key)
        REFERENCES harness_sub_agent_runs(id, agent_key)
        ON DELETE SET NULL (source_run_id),
    CHECK (source_type IN ('migration', 'manual', 'chat', 'review', 'rollback')),
    CHECK (length(trim(rationale)) <= 8192),
    CHECK ((source_type = 'review') = (source_run_id IS NOT NULL))
);

CREATE UNIQUE INDEX agent_strategy_prompt_review_batch_idx
    ON agent_strategy_prompt_revision_batches (source_run_id)
    WHERE source_type = 'review';

CREATE TABLE agent_strategy_prompt_revisions (
    id BIGSERIAL PRIMARY KEY,
    batch_id BIGINT NOT NULL,
    agent_key TEXT NOT NULL REFERENCES agents(agent_key) ON DELETE CASCADE,
    target_sub_agent_id BIGINT NOT NULL,
    target_sub_agent_key TEXT NOT NULL,
    parent_revision_id BIGINT,
    rollback_of_revision_id BIGINT,
    prompt TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT agent_strategy_prompt_revisions_batch_agent_fkey
        FOREIGN KEY (batch_id, agent_key)
        REFERENCES agent_strategy_prompt_revision_batches(id, agent_key)
        ON DELETE RESTRICT,
    CONSTRAINT agent_strategy_prompt_revisions_target_agent_fkey
        FOREIGN KEY (target_sub_agent_id, agent_key)
        REFERENCES harness_sub_agents(id, agent_key)
        ON DELETE RESTRICT,
    CHECK (length(trim(target_sub_agent_key)) > 0),
    CHECK (length(prompt) <= 65536),
    UNIQUE (batch_id, target_sub_agent_id),
    UNIQUE (id, agent_key, target_sub_agent_id),
    FOREIGN KEY (parent_revision_id, agent_key, target_sub_agent_id)
        REFERENCES agent_strategy_prompt_revisions(id, agent_key, target_sub_agent_id)
        ON DELETE RESTRICT,
    FOREIGN KEY (rollback_of_revision_id, agent_key, target_sub_agent_id)
        REFERENCES agent_strategy_prompt_revisions(id, agent_key, target_sub_agent_id)
        ON DELETE RESTRICT
);

CREATE TABLE agent_strategy_prompt_active_revisions (
    agent_key TEXT NOT NULL REFERENCES agents(agent_key) ON DELETE CASCADE,
    target_sub_agent_id BIGINT NOT NULL,
    revision_id BIGINT NOT NULL,
    activated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (agent_key, target_sub_agent_id),
    FOREIGN KEY (target_sub_agent_id, agent_key)
        REFERENCES harness_sub_agents(id, agent_key)
        ON DELETE CASCADE,
    FOREIGN KEY (revision_id, agent_key, target_sub_agent_id)
        REFERENCES agent_strategy_prompt_revisions(id, agent_key, target_sub_agent_id)
        ON DELETE RESTRICT
);

CREATE TABLE agent_strategy_prompt_revision_evidence (
    revision_batch_id BIGINT NOT NULL,
    memory_id UUID NOT NULL,
    agent_key TEXT NOT NULL REFERENCES agents(agent_key) ON DELETE CASCADE,
    PRIMARY KEY (revision_batch_id, memory_id),
    FOREIGN KEY (revision_batch_id, agent_key)
        REFERENCES agent_strategy_prompt_revision_batches(id, agent_key)
        ON DELETE CASCADE,
    FOREIGN KEY (memory_id, agent_key)
        REFERENCES memory.records(id, agent_key)
        ON DELETE RESTRICT
);

-- ============================================================================
-- Replace the `(agent_key, sub_agent_kind, timeframe)` uniqueness from 0008
-- with a partial singleton uniqueness for the durable singletons while
-- allowing multiple Analysis jobs that share a schedule/timeframe.
-- ============================================================================
DROP INDEX IF EXISTS harness_sub_agents_kind_timeframe_idx;

CREATE UNIQUE INDEX harness_sub_agents_singleton_kind_idx
    ON harness_sub_agents (agent_key, sub_agent_kind)
    WHERE sub_agent_kind IN ('trading', 'coding', 'review');

-- ============================================================================
-- Memory scope model: add an explicit `scope_kind` plus a separate
-- target-instrument table. The legacy `symbol` column remains untouched.
-- ============================================================================

ALTER TABLE memory.records
    ADD COLUMN scope_kind TEXT NOT NULL DEFAULT 'agent'
        CONSTRAINT memory_records_scope_kind_check
            CHECK (scope_kind IN ('agent', 'instruments')),
    ADD COLUMN source_run_id BIGINT,
    ADD CONSTRAINT memory_records_source_run_agent_fkey
        FOREIGN KEY (source_run_id, agent_key)
        REFERENCES harness_sub_agent_runs(id, agent_key)
        ON DELETE SET NULL (source_run_id);

CREATE TABLE memory.instrument_targets (
    memory_id UUID NOT NULL,
    agent_key TEXT NOT NULL,
    instrument_id TEXT NOT NULL REFERENCES hyperliquid.instruments(instrument_id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (memory_id, instrument_id),
    FOREIGN KEY (memory_id, agent_key)
        REFERENCES memory.records(id, agent_key)
        ON DELETE CASCADE
);

-- PostgreSQL CHECK constraints cannot reference another table. The API/store
-- validates that instrument scope has one or more selected canonical targets
-- in the same write transaction; the target table itself owns referential
-- integrity and historical target retention.

-- The legacy `symbol` column is no longer written by the application; scope
-- now lives in `scope_kind` plus `memory.instrument_targets`. Existing rows
-- keep their historical values, but the column becomes nullable so new
-- writes do not have to fabricate a symbol.
ALTER TABLE memory.records
    ALTER COLUMN symbol DROP NOT NULL;

CREATE INDEX memory_records_owner_scope_idx
    ON memory.records (agent_key, scope_kind, created_at DESC, id DESC);

CREATE INDEX memory_records_source_run_idx
    ON memory.records (agent_key, source_run_id)
    WHERE source_run_id IS NOT NULL;

-- Replace the index left by pre-finalization migration drafts with the
-- definitive lookup index.
DROP INDEX IF EXISTS memory.memory_records_type_idx;

CREATE INDEX memory_records_type_idx
    ON memory.records (agent_key, memory_type, created_at DESC);

CREATE INDEX memory_instrument_targets_target_idx
    ON memory.instrument_targets (agent_key, instrument_id, memory_id);
