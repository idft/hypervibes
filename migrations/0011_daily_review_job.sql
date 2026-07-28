SET search_path TO public;

CREATE TABLE agent_strategy_prompts (
    agent_key TEXT NOT NULL REFERENCES agents(agent_key) ON DELETE CASCADE,
    prompt_kind TEXT NOT NULL,
    prompt TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (agent_key, prompt_kind),
    CHECK (prompt_kind IN ('analysis', 'market_analysis', 'trading', 'daily_review'))
);

INSERT INTO agent_strategy_prompts (agent_key, prompt_kind, prompt)
SELECT agent_key, 'daily_review', ''
FROM agents;

ALTER TABLE memory.records
    ADD CONSTRAINT records_id_agent_key_unique UNIQUE (id, agent_key);

CREATE TABLE memory.links (
    id BIGSERIAL PRIMARY KEY,
    agent_key TEXT NOT NULL REFERENCES agents(agent_key) ON DELETE CASCADE,
    source_memory_id UUID NOT NULL,
    target_memory_id UUID NOT NULL,
    link_type TEXT NOT NULL,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (source_memory_id, agent_key)
        REFERENCES memory.records(id, agent_key)
        ON DELETE CASCADE,
    FOREIGN KEY (target_memory_id, agent_key)
        REFERENCES memory.records(id, agent_key)
        ON DELETE CASCADE,
    CHECK (length(trim(link_type)) > 0),
    CHECK (source_memory_id <> target_memory_id),
    UNIQUE (source_memory_id, target_memory_id, link_type)
);

CREATE INDEX memory_links_agent_source_idx
    ON memory.links (agent_key, source_memory_id);

CREATE INDEX memory_links_agent_target_idx
    ON memory.links (agent_key, target_memory_id);

CREATE INDEX memory_links_agent_type_idx
    ON memory.links (agent_key, link_type);

ALTER TABLE hyperliquid.orders
    ADD COLUMN attribution_source TEXT NOT NULL DEFAULT 'agent';

ALTER TABLE hyperliquid.orders
    ADD CONSTRAINT orders_attribution_source_check
    CHECK (attribution_source IN ('agent', 'manual'));

CREATE INDEX orders_memory_record_ids_gin_idx
    ON hyperliquid.orders
    USING GIN (memory_record_ids);
