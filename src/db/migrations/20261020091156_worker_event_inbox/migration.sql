ALTER TABLE `hv_subagents` ADD `triggers` text DEFAULT '{"inboxEnabled":false,"workerDelegates":[],"researchSources":[],"fills":false,"orderChanges":false}' NOT NULL;
--> statement-breakpoint
CREATE TABLE `hv_worker_requests` (
  `id` text PRIMARY KEY,
  `agent_id` text NOT NULL REFERENCES `hv_agents` (`id`) ON DELETE RESTRICT,
  `subagent_id` text NOT NULL,
  `event_key` text NOT NULL,
  `payload` text NOT NULL,
  `created_at` integer NOT NULL,
  `run_id` text,
  `discarded_reason` text,
  FOREIGN KEY (`agent_id`, `subagent_id`) REFERENCES `hv_subagents` (`agent_id`, `id`),
  FOREIGN KEY (`agent_id`, `run_id`, `subagent_id`) REFERENCES `hv_subagent_runs` (`agent_id`, `id`, `subagent_id`)
);
--> statement-breakpoint
CREATE UNIQUE INDEX `hv_worker_requests_event_idx` ON `hv_worker_requests` (`agent_id`, `subagent_id`, `event_key`);
--> statement-breakpoint
CREATE INDEX `hv_worker_requests_pending_idx` ON `hv_worker_requests` (`subagent_id`, `created_at`) WHERE `run_id` IS NULL AND `discarded_reason` IS NULL;

--> statement-breakpoint
CREATE TABLE hv_worker_subscriptions (
  subagent_id text NOT NULL REFERENCES hv_subagents(id),
  event_type text NOT NULL,
  source_id text NOT NULL,
  activated_at integer NOT NULL,
  PRIMARY KEY (subagent_id, event_type, source_id)
);
--> statement-breakpoint
CREATE VIEW hv_enabled_worker_subscriptions AS
  SELECT s.id AS subagent_id, 'research' AS event_type, j.value AS source_id, s.updated_at AS activated_at
  FROM hv_subagents s, json_each(s.triggers, '$.researchSources') j
  WHERE s.enabled=1 AND s.archived=0 AND json_extract(s.triggers,'$.inboxEnabled')=1
  UNION ALL
  SELECT id, 'fill', '', updated_at FROM hv_subagents
  WHERE enabled=1 AND archived=0 AND json_extract(triggers,'$.inboxEnabled')=1 AND json_extract(triggers,'$.fills')=1
  UNION ALL
  SELECT id, 'order_status', '', updated_at FROM hv_subagents
  WHERE enabled=1 AND archived=0 AND json_extract(triggers,'$.inboxEnabled')=1 AND json_extract(triggers,'$.orderChanges')=1;
--> statement-breakpoint
INSERT INTO hv_worker_subscriptions SELECT * FROM hv_enabled_worker_subscriptions;
--> statement-breakpoint
CREATE TRIGGER hv_worker_subscriptions_insert AFTER INSERT ON hv_subagents BEGIN
  INSERT INTO hv_worker_subscriptions SELECT * FROM hv_enabled_worker_subscriptions WHERE subagent_id=NEW.id;
END;
--> statement-breakpoint
CREATE TRIGGER hv_worker_subscriptions_update AFTER UPDATE OF triggers, enabled, archived ON hv_subagents BEGIN
  DELETE FROM hv_worker_subscriptions WHERE subagent_id=NEW.id AND NOT EXISTS (
    SELECT 1 FROM hv_enabled_worker_subscriptions v WHERE v.subagent_id=NEW.id
      AND v.event_type=hv_worker_subscriptions.event_type AND v.source_id=hv_worker_subscriptions.source_id
  );
  INSERT INTO hv_worker_subscriptions SELECT * FROM hv_enabled_worker_subscriptions WHERE subagent_id=NEW.id
    AND NOT EXISTS (SELECT 1 FROM hv_worker_subscriptions old WHERE old.subagent_id=NEW.id
      AND old.event_type=hv_enabled_worker_subscriptions.event_type AND old.source_id=hv_enabled_worker_subscriptions.source_id);
END;
