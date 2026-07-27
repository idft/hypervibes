SET search_path TO public;

ALTER TABLE agentic_job_schedules
    ADD COLUMN model_variant TEXT;
ALTER TABLE agentic_job_schedules
    ADD CONSTRAINT agentic_job_schedules_model_variant_check
    CHECK (model_variant IS NULL OR length(trim(model_variant)) > 0);

ALTER TABLE agentic_job_hooks
    ADD COLUMN model_variant TEXT;
ALTER TABLE agentic_job_hooks
    ADD CONSTRAINT agentic_job_hooks_model_variant_check
    CHECK (model_variant IS NULL OR length(trim(model_variant)) > 0);

ALTER TABLE agentic_runs
    ADD COLUMN model_variant TEXT;
ALTER TABLE agentic_runs
    ADD CONSTRAINT agentic_runs_model_variant_check
    CHECK (model_variant IS NULL OR length(trim(model_variant)) > 0);

ALTER TABLE agent_conversations
    ADD COLUMN model_variant TEXT;
ALTER TABLE agent_conversations
    ADD CONSTRAINT agent_conversations_model_variant_check
    CHECK (model_variant IS NULL OR length(trim(model_variant)) > 0);
