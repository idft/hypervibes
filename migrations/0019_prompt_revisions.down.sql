SET search_path TO public;

CREATE TABLE agent_strategy_prompts (
    agent_key TEXT NOT NULL REFERENCES agents(agent_key) ON DELETE CASCADE,
    prompt_kind TEXT NOT NULL,
    prompt TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (agent_key, prompt_kind),
    CHECK (prompt_kind IN ('analysis', 'market_analysis', 'trading', 'daily_review', 'analysis_coding'))
);

INSERT INTO agent_strategy_prompts (agent_key, prompt_kind, prompt, created_at, updated_at)
SELECT active.agent_key, active.prompt_kind, revisions.prompt, revisions.created_at, active.activated_at
FROM agent_strategy_prompt_active_revisions AS active
JOIN agent_strategy_prompt_revisions AS revisions ON revisions.id = active.revision_id;

DROP TABLE agent_strategy_prompt_revision_evidence;
DROP TABLE agent_strategy_prompt_active_revisions;
DROP TABLE agent_strategy_prompt_revisions;
DROP TABLE agent_strategy_prompt_revision_batches;
