CREATE TABLE `hv_agents` (
	`id` text PRIMARY KEY,
	`user_id` text NOT NULL,
	`name` text NOT NULL,
	`creation_request_id` text NOT NULL,
	`state` text DEFAULT 'provisioning' NOT NULL,
	`conversation_id` integer,
	`error` text,
	`created_at` integer NOT NULL,
	`updated_at` integer NOT NULL,
	CONSTRAINT `fk_hv_agents_user_id_hv_users_id_fk` FOREIGN KEY (`user_id`) REFERENCES `hv_users`(`id`) ON DELETE RESTRICT,
	CONSTRAINT "hv_agents_name_check" CHECK(length(trim("name")) BETWEEN 1 AND 80),
	CONSTRAINT "hv_agents_state_check" CHECK("state" IN ('provisioning', 'ready', 'failed'))
);
--> statement-breakpoint
CREATE INDEX `hv_agents_owner_idx` ON `hv_agents` (`user_id`);--> statement-breakpoint
CREATE UNIQUE INDEX `hv_agents_creation_request_idx` ON `hv_agents` (`user_id`,`creation_request_id`);