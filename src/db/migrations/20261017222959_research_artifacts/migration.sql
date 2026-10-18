CREATE TABLE `hv_artifact_links` (
	`parent` text NOT NULL,
	`child` text NOT NULL,
	CONSTRAINT `fk_hv_artifact_links_parent_hv_artifacts_id_fk` FOREIGN KEY (`parent`) REFERENCES `hv_artifacts`(`id`) ON DELETE CASCADE,
	CONSTRAINT `fk_hv_artifact_links_child_hv_artifacts_id_fk` FOREIGN KEY (`child`) REFERENCES `hv_artifacts`(`id`) ON DELETE CASCADE
);
--> statement-breakpoint
CREATE TABLE `hv_artifact_receipts` (
	`agent_id` text NOT NULL,
	`operation_key` text NOT NULL,
	`request_hash` text NOT NULL,
	`artifact_id` text NOT NULL,
	CONSTRAINT `fk_hv_artifact_receipts_agent_id_hv_agents_id_fk` FOREIGN KEY (`agent_id`) REFERENCES `hv_agents`(`id`)
);
--> statement-breakpoint
CREATE TABLE `hv_artifacts` (
	`id` text PRIMARY KEY,
	`agent_id` text NOT NULL,
	`title` text NOT NULL,
	`description` text NOT NULL,
	`media_type` text NOT NULL,
	`payload` blob NOT NULL,
	`size` integer NOT NULL,
	`tags` text NOT NULL,
	`instrument` text NOT NULL,
	`interval` text NOT NULL,
	`conversation_id` integer,
	`task_id` integer,
	`created_at` integer NOT NULL,
	`expires_at` integer,
	CONSTRAINT `fk_hv_artifacts_agent_id_hv_agents_id_fk` FOREIGN KEY (`agent_id`) REFERENCES `hv_agents`(`id`)
);
--> statement-breakpoint
CREATE UNIQUE INDEX `hv_artifact_links_pair_idx` ON `hv_artifact_links` (`parent`,`child`);--> statement-breakpoint
CREATE INDEX `hv_artifact_links_child_idx` ON `hv_artifact_links` (`child`);--> statement-breakpoint
CREATE UNIQUE INDEX `hv_artifact_receipts_operation_idx` ON `hv_artifact_receipts` (`agent_id`,`operation_key`);--> statement-breakpoint
CREATE INDEX `hv_artifacts_agent_idx` ON `hv_artifacts` (`agent_id`,`created_at`);