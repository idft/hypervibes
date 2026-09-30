CREATE TABLE public.agent_chat_policy_defaults (
    agent_key TEXT NOT NULL REFERENCES public.agents(agent_key) ON DELETE CASCADE,
    tool_group TEXT NOT NULL,
    policy TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (agent_key, tool_group),
    CHECK (tool_group IN ('orders', 'memory_writes', 'notifications', 'journal_writes', 'indicator_writes', 'strategy_prompt_writes')),
    CHECK (policy IN ('deny', 'confirm', 'allow'))
);

CREATE TRIGGER set_agent_chat_policy_defaults_updated_at
    BEFORE UPDATE ON public.agent_chat_policy_defaults
    FOR EACH ROW
    EXECUTE FUNCTION public.set_agent_conversation_updated_at();

INSERT INTO public.agent_chat_policy_defaults (agent_key, tool_group, policy)
SELECT agents.agent_key, defaults.tool_group, defaults.policy
FROM public.agents AS agents
CROSS JOIN (VALUES
    ('orders', 'confirm'),
    ('memory_writes', 'confirm'),
    ('notifications', 'deny'),
    ('journal_writes', 'deny'),
    ('indicator_writes', 'confirm'),
    ('strategy_prompt_writes', 'confirm')
) AS defaults(tool_group, policy);
