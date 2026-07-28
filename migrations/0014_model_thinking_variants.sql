SET search_path TO public;

ALTER TABLE agent_conversations
    ADD COLUMN model_variant TEXT;
ALTER TABLE agent_conversations
    ADD CONSTRAINT agent_conversations_model_variant_check
    CHECK (model_variant IS NULL OR length(trim(model_variant)) > 0);
