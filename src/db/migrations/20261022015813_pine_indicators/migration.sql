CREATE TABLE `hv_indicator_operations` (
	`agent_id` text NOT NULL,
	`key` text NOT NULL,
	`hash` text NOT NULL,
	`result` text NOT NULL,
	CONSTRAINT `hv_indicator_operations_pk` PRIMARY KEY(`agent_id`, `key`),
	CONSTRAINT `fk_hv_indicator_operations_agent_id_hv_agents_id_fk` FOREIGN KEY (`agent_id`) REFERENCES `hv_agents`(`id`) ON DELETE CASCADE
);
--> statement-breakpoint
CREATE TABLE `hv_indicator_versions` (
	`indicator_id` text NOT NULL,
	`version` integer NOT NULL,
	`source` text NOT NULL,
	`source_hash` text NOT NULL,
	`metadata` text NOT NULL,
	`engine_version` text NOT NULL,
	`wrapper_version` integer NOT NULL,
	`created_at` integer NOT NULL,
	`actor_kind` text NOT NULL,
	`user_id` text,
	`subagent_id` text,
	`run_id` text,
	`conversation_id` text,
	`task_id` text,
	CONSTRAINT `hv_indicator_versions_pk` PRIMARY KEY(`indicator_id`, `version`),
	CONSTRAINT `fk_hv_indicator_versions_indicator_id_hv_indicators_id_fk` FOREIGN KEY (`indicator_id`) REFERENCES `hv_indicators`(`id`) ON DELETE CASCADE,
	CONSTRAINT "hv_indicator_versions_positive" CHECK("version" > 0)
);
--> statement-breakpoint
CREATE TABLE `hv_indicators` (
	`id` text PRIMARY KEY,
	`agent_id` text NOT NULL,
	`name` text NOT NULL,
	`description` text NOT NULL,
	`interpretation` text NOT NULL,
	`current_version` integer NOT NULL,
	`created_at` integer NOT NULL,
	`updated_at` integer NOT NULL,
	CONSTRAINT `fk_hv_indicators_agent_id_hv_agents_id_fk` FOREIGN KEY (`agent_id`) REFERENCES `hv_agents`(`id`) ON DELETE CASCADE,
	CONSTRAINT "hv_indicators_version_positive" CHECK("current_version" > 0)
);
--> statement-breakpoint
CREATE INDEX `hv_indicators_agent_idx` ON `hv_indicators` (`agent_id`,`updated_at`);