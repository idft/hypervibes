CREATE TABLE `hv_exchange_nonces` (
	`signer` text PRIMARY KEY,
	`nonce` integer NOT NULL
);
--> statement-breakpoint
CREATE TABLE `hv_order_actions` (
	`id` text PRIMARY KEY,
	`request_id` text NOT NULL,
	`ordinal` integer NOT NULL,
	`payload` text NOT NULL,
	`state` text NOT NULL,
	`nonce` integer,
	`expires_at` integer NOT NULL,
	`updated_at` integer NOT NULL,
	CONSTRAINT `fk_hv_order_actions_request_id_hv_order_requests_id_fk` FOREIGN KEY (`request_id`) REFERENCES `hv_order_requests`(`id`),
	CONSTRAINT "hv_order_action_state" CHECK("state" IN ('prepared', 'submitting', 'unknown', 'done', 'not_sent'))
);
--> statement-breakpoint
CREATE TABLE `hv_order_events` (
	`id` integer PRIMARY KEY AUTOINCREMENT,
	`item_id` text NOT NULL,
	`source` text NOT NULL,
	`time` integer NOT NULL,
	`data` text NOT NULL,
	CONSTRAINT `fk_hv_order_events_item_id_hv_order_items_id_fk` FOREIGN KEY (`item_id`) REFERENCES `hv_order_items`(`id`)
);
--> statement-breakpoint
CREATE TABLE `hv_order_items` (
	`id` text PRIMARY KEY,
	`action_id` text NOT NULL,
	`ordinal` integer NOT NULL,
	`symbol` text NOT NULL,
	`cloid` text,
	`oid` text,
	`active` integer NOT NULL,
	`view` text NOT NULL,
	`updated_at` integer NOT NULL,
	CONSTRAINT `fk_hv_order_items_action_id_hv_order_actions_id_fk` FOREIGN KEY (`action_id`) REFERENCES `hv_order_actions`(`id`),
	CONSTRAINT "hv_order_item_active" CHECK("active" IN (0, 1))
);
--> statement-breakpoint
CREATE TABLE `hv_order_requests` (
	`id` text PRIMARY KEY,
	`agent_id` text NOT NULL,
	`account` text NOT NULL,
	`request_key` text NOT NULL,
	`kind` text NOT NULL,
	`input` text NOT NULL,
	`created_at` integer NOT NULL,
	CONSTRAINT `fk_hv_order_requests_agent_id_hv_agents_id_fk` FOREIGN KEY (`agent_id`) REFERENCES `hv_agents`(`id`),
	CONSTRAINT "hv_order_request_kind" CHECK("kind" IN ('submit', 'cancel', 'cancel_all'))
);
--> statement-breakpoint
CREATE UNIQUE INDEX `hv_order_action_ordinal` ON `hv_order_actions` (`request_id`,`ordinal`);--> statement-breakpoint
CREATE INDEX `hv_order_actions_state` ON `hv_order_actions` (`state`,`updated_at`);--> statement-breakpoint
CREATE INDEX `hv_order_events_item` ON `hv_order_events` (`item_id`,`id`);--> statement-breakpoint
CREATE UNIQUE INDEX `hv_order_item_ordinal` ON `hv_order_items` (`action_id`,`ordinal`);--> statement-breakpoint
CREATE UNIQUE INDEX `hv_order_item_cloid` ON `hv_order_items` (`cloid`);--> statement-breakpoint
CREATE INDEX `hv_order_items_active` ON `hv_order_items` (`active`,`updated_at`);--> statement-breakpoint
CREATE INDEX `hv_order_items_oid` ON `hv_order_items` (`oid`);--> statement-breakpoint
CREATE UNIQUE INDEX `hv_order_request_key` ON `hv_order_requests` (`agent_id`,`request_key`);--> statement-breakpoint
CREATE INDEX `hv_order_requests_agent` ON `hv_order_requests` (`agent_id`,`created_at`);