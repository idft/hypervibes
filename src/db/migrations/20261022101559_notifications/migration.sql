CREATE TABLE `hv_notification_operations` (
	`agent_id` text NOT NULL,
	`operation_key` text NOT NULL,
	`request_hash` text NOT NULL,
	`result` text NOT NULL,
	CONSTRAINT `fk_hv_notification_operations_agent_id_hv_agents_id_fk` FOREIGN KEY (`agent_id`) REFERENCES `hv_agents`(`id`) ON DELETE CASCADE
);
--> statement-breakpoint
CREATE TABLE `hv_notification_settings` (
	`agent_id` text PRIMARY KEY,
	`enabled` integer DEFAULT true NOT NULL,
	`submissions` integer DEFAULT true NOT NULL,
	`fills` integer DEFAULT true NOT NULL,
	`enabled_since` integer NOT NULL,
	`fills_since` integer NOT NULL,
	CONSTRAINT `fk_hv_notification_settings_agent_id_hv_agents_id_fk` FOREIGN KEY (`agent_id`) REFERENCES `hv_agents`(`id`) ON DELETE CASCADE
);
--> statement-breakpoint
CREATE TABLE `hv_notifications` (
	`id` text PRIMARY KEY,
	`agent_id` text NOT NULL,
	`event_key` text NOT NULL,
	`category` text NOT NULL,
	`title` text NOT NULL,
	`body` text NOT NULL,
	`severity` text NOT NULL,
	`source` text NOT NULL,
	`subagent_id` text,
	`run_id` text,
	`conversation_id` integer,
	`order_request_id` text,
	`created_at` integer NOT NULL,
	`delivery` text NOT NULL,
	`chat_id` integer,
	`bot_token_hash` text,
	`message_ids` text DEFAULT '[]' NOT NULL,
	`attempts` integer DEFAULT 0 NOT NULL,
	`next_attempt_at` integer NOT NULL,
	`sent_at` integer,
	`error` text,
	CONSTRAINT `fk_hv_notifications_agent_id_hv_agents_id_fk` FOREIGN KEY (`agent_id`) REFERENCES `hv_agents`(`id`) ON DELETE CASCADE
);
--> statement-breakpoint
CREATE UNIQUE INDEX `hv_notification_operations_key_idx` ON `hv_notification_operations` (`agent_id`,`operation_key`);--> statement-breakpoint
CREATE UNIQUE INDEX `hv_notifications_event_idx` ON `hv_notifications` (`agent_id`,`event_key`);--> statement-breakpoint
CREATE INDEX `hv_notifications_history_idx` ON `hv_notifications` (`agent_id`,`created_at`,`id`);--> statement-breakpoint
CREATE INDEX `hv_notifications_delivery_idx` ON `hv_notifications` (`delivery`,`next_attempt_at`);
--> statement-breakpoint
-- Existing accounts start observing now, never announcing historical imports.
INSERT INTO hv_notification_settings(agent_id,enabled_since,fills_since)
SELECT id, CAST(unixepoch('subsec') * 1000 AS INTEGER), CAST(unixepoch('subsec') * 1000 AS INTEGER) FROM hv_agents;