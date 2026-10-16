CREATE TABLE `hv_installation` (
	`id` integer PRIMARY KEY,
	`device_id` text NOT NULL
);
--> statement-breakpoint
CREATE TABLE `hv_provider_connections` (
	`user_id` text NOT NULL,
	`provider_id` text NOT NULL,
	`auth_type` text NOT NULL,
	`ciphertext` blob NOT NULL,
	`key_id` text NOT NULL,
	`catalog` text,
	`updated_at` integer NOT NULL,
	CONSTRAINT `hv_provider_connections_pk` PRIMARY KEY(`user_id`, `provider_id`),
	CONSTRAINT `fk_hv_provider_connections_user_id_hv_users_id_fk` FOREIGN KEY (`user_id`) REFERENCES `hv_users`(`id`) ON DELETE CASCADE
);
--> statement-breakpoint
ALTER TABLE `hv_agents` ADD `initial_provider` text;--> statement-breakpoint
ALTER TABLE `hv_agents` ADD `initial_model_id` text;