SET search_path TO public;

ALTER TABLE agentic_job_schedules
    DROP CONSTRAINT agentic_job_schedules_model_variant_check;
ALTER TABLE agentic_job_schedules
    DROP COLUMN model_variant;

ALTER TABLE agentic_job_hooks
    DROP CONSTRAINT agentic_job_hooks_model_variant_check;
ALTER TABLE agentic_job_hooks
    DROP COLUMN model_variant;

ALTER TABLE agentic_runs
    DROP CONSTRAINT agentic_runs_model_variant_check;
ALTER TABLE agentic_runs
    DROP COLUMN model_variant;

ALTER TABLE agent_conversations
    DROP CONSTRAINT agent_conversations_model_variant_check;
ALTER TABLE agent_conversations
    DROP COLUMN model_variant;
