SET search_path TO public;

CREATE TABLE agent_conversations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agent_key TEXT NOT NULL REFERENCES agents(agent_key) ON DELETE CASCADE,
    opencode_session_id TEXT NOT NULL UNIQUE,
    channel TEXT NOT NULL DEFAULT 'web',
    external_conversation_key TEXT,
    title TEXT NOT NULL,
    model_provider_id TEXT NOT NULL,
    model_id TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (length(trim(channel)) > 0),
    CHECK (length(trim(title)) > 0),
    CHECK (length(trim(model_provider_id)) > 0),
    CHECK (length(trim(model_id)) > 0)
);

CREATE INDEX agent_conversations_agent_updated_idx
    ON agent_conversations (agent_key, updated_at DESC, id DESC);

CREATE UNIQUE INDEX agent_conversations_external_source_idx
    ON agent_conversations (agent_key, channel, external_conversation_key)
    WHERE external_conversation_key IS NOT NULL;

CREATE TABLE agent_conversation_tool_policies (
    conversation_id UUID NOT NULL REFERENCES agent_conversations(id) ON DELETE CASCADE,
    tool_group TEXT NOT NULL,
    policy TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (conversation_id, tool_group),
    CHECK (tool_group IN ('orders', 'memory_writes')),
    CHECK (policy IN ('deny', 'confirm', 'allow'))
);

CREATE OR REPLACE FUNCTION public.set_agent_conversation_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER set_agent_conversations_updated_at
    BEFORE UPDATE ON agent_conversations
    FOR EACH ROW
    EXECUTE FUNCTION public.set_agent_conversation_updated_at();

CREATE TRIGGER set_agent_conversation_tool_policies_updated_at
    BEFORE UPDATE ON agent_conversation_tool_policies
    FOR EACH ROW
    EXECUTE FUNCTION public.set_agent_conversation_updated_at();

CREATE OR REPLACE FUNCTION public.notify_agent_conversation_changed()
RETURNS TRIGGER AS $$
DECLARE
    conversation_id UUID;
BEGIN
    conversation_id := CASE WHEN TG_OP = 'DELETE' THEN OLD.id ELSE NEW.id END;
    PERFORM pg_notify('agent_conversation_changed', conversation_id::text);
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION public.notify_agent_conversation_tool_policy_changed()
RETURNS TRIGGER AS $$
DECLARE
    conversation_id UUID;
BEGIN
    conversation_id := CASE
        WHEN TG_OP = 'DELETE' THEN OLD.conversation_id
        ELSE NEW.conversation_id
    END;
    PERFORM pg_notify('agent_conversation_changed', conversation_id::text);
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER notify_agent_conversation_changed
    AFTER INSERT OR UPDATE OR DELETE ON agent_conversations
    FOR EACH ROW
    EXECUTE FUNCTION public.notify_agent_conversation_changed();

CREATE TRIGGER notify_agent_conversation_tool_policy_changed
    AFTER INSERT OR UPDATE OR DELETE ON agent_conversation_tool_policies
    FOR EACH ROW
    EXECUTE FUNCTION public.notify_agent_conversation_tool_policy_changed();
