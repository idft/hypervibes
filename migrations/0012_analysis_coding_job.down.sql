SET search_path TO public;

DELETE FROM agent_strategy_prompts
WHERE prompt_kind = 'analysis_coding';

ALTER TABLE agent_strategy_prompts
    DROP CONSTRAINT IF EXISTS agent_strategy_prompts_prompt_kind_check;
ALTER TABLE agent_strategy_prompts
    ADD CONSTRAINT agent_strategy_prompts_prompt_kind_check
    CHECK (prompt_kind IN ('analysis', 'market_analysis', 'trading', 'daily_review'));
