SET search_path TO public;

DROP TRIGGER IF EXISTS notify_agent_conversation_tool_policy_changed
    ON agent_conversation_tool_policies;
DROP TRIGGER IF EXISTS set_agent_conversation_tool_policies_updated_at
    ON agent_conversation_tool_policies;
DROP FUNCTION IF EXISTS public.notify_agent_conversation_tool_policy_changed();

DROP TRIGGER IF EXISTS notify_agent_conversation_changed ON agent_conversations;
DROP TRIGGER IF EXISTS set_agent_conversations_updated_at ON agent_conversations;
DROP FUNCTION IF EXISTS public.notify_agent_conversation_changed();
DROP FUNCTION IF EXISTS public.set_agent_conversation_updated_at();

DROP INDEX IF EXISTS agent_conversations_external_source_idx;
DROP INDEX IF EXISTS agent_conversations_agent_updated_idx;

DROP TABLE IF EXISTS agent_conversation_tool_policies;
DROP TABLE IF EXISTS agent_conversations;
