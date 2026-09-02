SET search_path TO public;

ALTER TABLE harness_sub_agents
    ADD COLUMN enabled_capabilities JSONB NOT NULL DEFAULT '[]'::jsonb,
    ADD CONSTRAINT harness_sub_agents_enabled_capabilities_check
        CHECK (jsonb_typeof(enabled_capabilities) = 'array');

UPDATE harness_sub_agents
   SET enabled_capabilities = CASE
       WHEN notification_send_enabled THEN '["hypervibes:notification_send"]'::jsonb
       ELSE '[]'::jsonb
   END;
