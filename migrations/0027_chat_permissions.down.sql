DELETE FROM public.agent_conversation_tool_policies
WHERE tool_group IN ('indicator_writes', 'strategy_prompt_writes');

ALTER TABLE public.agent_conversation_tool_policies
    DROP CONSTRAINT agent_conversation_tool_policies_tool_group_check;
ALTER TABLE public.agent_conversation_tool_policies
    ADD CONSTRAINT agent_conversation_tool_policies_tool_group_check
    CHECK (tool_group IN ('orders', 'memory_writes', 'notifications', 'journal_writes'));
