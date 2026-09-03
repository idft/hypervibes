SET search_path TO public;

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
    CHECK (source_type IN ('migration', 'manual', 'chat', 'daily_review', 'rollback')),
    CHECK (length(trim(rationale)) <= 8192),
    CHECK ((source_type = 'daily_review') = (source_run_id IS NOT NULL))
);

CREATE UNIQUE INDEX agent_strategy_prompt_daily_review_batch_idx
    ON agent_strategy_prompt_revision_batches (source_run_id)
    WHERE source_type = 'daily_review';

CREATE TABLE agent_strategy_prompt_revisions (
    id BIGSERIAL PRIMARY KEY,
    batch_id BIGINT NOT NULL,
    agent_key TEXT NOT NULL REFERENCES agents(agent_key) ON DELETE CASCADE,
    prompt_kind TEXT NOT NULL,
    parent_revision_id BIGINT,
    rollback_of_revision_id BIGINT,
    prompt TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT agent_strategy_prompt_revisions_batch_agent_fkey
        FOREIGN KEY (batch_id, agent_key)
        REFERENCES agent_strategy_prompt_revision_batches(id, agent_key)
        ON DELETE RESTRICT,
    CHECK (prompt_kind IN ('analysis', 'market_analysis', 'trading', 'daily_review', 'analysis_coding')),
    CHECK (length(prompt) <= 65536),
    UNIQUE (batch_id, prompt_kind),
    UNIQUE (id, agent_key, prompt_kind),
    FOREIGN KEY (parent_revision_id, agent_key, prompt_kind)
        REFERENCES agent_strategy_prompt_revisions(id, agent_key, prompt_kind)
        ON DELETE RESTRICT,
    FOREIGN KEY (rollback_of_revision_id, agent_key, prompt_kind)
        REFERENCES agent_strategy_prompt_revisions(id, agent_key, prompt_kind)
        ON DELETE RESTRICT
);

CREATE TABLE agent_strategy_prompt_active_revisions (
    agent_key TEXT NOT NULL REFERENCES agents(agent_key) ON DELETE CASCADE,
    prompt_kind TEXT NOT NULL,
    revision_id BIGINT NOT NULL,
    activated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (agent_key, prompt_kind),
    CHECK (prompt_kind IN ('analysis', 'market_analysis', 'trading', 'daily_review', 'analysis_coding')),
    FOREIGN KEY (revision_id, agent_key, prompt_kind)
        REFERENCES agent_strategy_prompt_revisions(id, agent_key, prompt_kind)
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

DROP TABLE agent_strategy_prompts;
