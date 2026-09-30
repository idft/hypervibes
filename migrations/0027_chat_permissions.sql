ALTER TABLE public.agent_conversation_tool_policies
    DROP CONSTRAINT agent_conversation_tool_policies_tool_group_check;
ALTER TABLE public.agent_conversation_tool_policies
    ADD CONSTRAINT agent_conversation_tool_policies_tool_group_check
    CHECK (tool_group IN ('orders', 'memory_writes', 'notifications', 'journal_writes', 'indicator_writes', 'strategy_prompt_writes'));

-- Preserve the previous confirmation requirement for existing conversations.
INSERT INTO public.agent_conversation_tool_policies (conversation_id, tool_group, policy)
SELECT conversations.id, groups.tool_group, 'confirm'
FROM public.agent_conversations AS conversations
CROSS JOIN (VALUES ('indicator_writes'), ('strategy_prompt_writes')) AS groups(tool_group)
ON CONFLICT DO NOTHING;
