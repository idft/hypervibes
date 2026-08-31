SET search_path TO public;

ALTER TABLE agents
    ADD COLUMN daily_review_prompt_improvement_enabled BOOLEAN NOT NULL DEFAULT true;

CREATE TABLE agent_strategy_prompt_revision_batches (
    id BIGSERIAL PRIMARY KEY,
    agent_key TEXT NOT NULL REFERENCES agents(agent_key) ON DELETE CASCADE,
    source_type TEXT NOT NULL,
    source_run_id BIGINT REFERENCES harness_sub_agent_runs(id) ON DELETE SET NULL,
    rationale TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (source_type IN ('migration', 'manual', 'chat', 'daily_review', 'rollback')),
    CHECK (length(trim(rationale)) <= 8192),
    CHECK ((source_type = 'daily_review') = (source_run_id IS NOT NULL))
);

CREATE UNIQUE INDEX agent_strategy_prompt_daily_review_batch_idx
    ON agent_strategy_prompt_revision_batches (source_run_id)
    WHERE source_type = 'daily_review';

CREATE TABLE agent_strategy_prompt_revisions (
    id BIGSERIAL PRIMARY KEY,
    batch_id BIGINT NOT NULL REFERENCES agent_strategy_prompt_revision_batches(id) ON DELETE RESTRICT,
    agent_key TEXT NOT NULL REFERENCES agents(agent_key) ON DELETE CASCADE,
    prompt_kind TEXT NOT NULL,
    parent_revision_id BIGINT REFERENCES agent_strategy_prompt_revisions(id) ON DELETE RESTRICT,
    rollback_of_revision_id BIGINT REFERENCES agent_strategy_prompt_revisions(id) ON DELETE RESTRICT,
    prompt TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (prompt_kind IN ('analysis', 'market_analysis', 'trading', 'daily_review', 'analysis_coding')),
    CHECK (length(prompt) <= 65536),
    UNIQUE (batch_id, prompt_kind)
);

CREATE TABLE agent_strategy_prompt_active_revisions (
    agent_key TEXT NOT NULL REFERENCES agents(agent_key) ON DELETE CASCADE,
    prompt_kind TEXT NOT NULL,
    revision_id BIGINT NOT NULL REFERENCES agent_strategy_prompt_revisions(id) ON DELETE RESTRICT,
    activated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (agent_key, prompt_kind),
    CHECK (prompt_kind IN ('analysis', 'market_analysis', 'trading', 'daily_review', 'analysis_coding'))
);

CREATE TABLE agent_strategy_prompt_revision_evidence (
    revision_batch_id BIGINT NOT NULL REFERENCES agent_strategy_prompt_revision_batches(id) ON DELETE CASCADE,
    memory_id UUID NOT NULL,
    agent_key TEXT NOT NULL REFERENCES agents(agent_key) ON DELETE CASCADE,
    PRIMARY KEY (revision_batch_id, memory_id),
    FOREIGN KEY (memory_id, agent_key)
        REFERENCES memory.records(id, agent_key)
        ON DELETE RESTRICT
);

WITH batches AS (
    INSERT INTO agent_strategy_prompt_revision_batches (agent_key, source_type, rationale)
    SELECT DISTINCT agent_key, 'migration', 'Migrated active strategy prompt'
    FROM agent_strategy_prompts
    RETURNING id, agent_key
), revisions AS (
    INSERT INTO agent_strategy_prompt_revisions (batch_id, agent_key, prompt_kind, prompt, created_at)
    SELECT batches.id, prompts.agent_key, prompts.prompt_kind, prompts.prompt, prompts.updated_at
    FROM agent_strategy_prompts prompts
    JOIN batches ON batches.agent_key = prompts.agent_key
    RETURNING id, agent_key, prompt_kind
)
INSERT INTO agent_strategy_prompt_active_revisions (agent_key, prompt_kind, revision_id)
SELECT agent_key, prompt_kind, id FROM revisions;
