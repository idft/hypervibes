SET search_path TO public;

ALTER TABLE agent_conversations
    DROP CONSTRAINT agent_conversations_model_variant_check;
ALTER TABLE agent_conversations
    DROP COLUMN model_variant;
