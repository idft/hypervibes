CREATE TABLE `hv_memories` (
	`agent_id` text NOT NULL,
	`number` integer NOT NULL,
	`kind` text NOT NULL,
	`topic` text NOT NULL,
	`instrument` text,
	`interval` text,
	`summary` text NOT NULL,
	`content` text NOT NULL,
	`payload` text NOT NULL,
	`created_at` integer NOT NULL,
	`evidence_as_of` integer,
	`valid_until` integer,
	`producer_subagent_id` text,
	`source_run_id` text,
	`conversation_id` integer,
	`tool_task_id` integer,
	`retracted_at` integer,
	`retraction_reason` text,
	CONSTRAINT `hv_memories_pk` PRIMARY KEY(`agent_id`, `number`),
	CONSTRAINT `fk_hv_memories_agent_id_hv_agents_id_fk` FOREIGN KEY (`agent_id`) REFERENCES `hv_agents`(`id`) ON DELETE RESTRICT,
	CONSTRAINT `fk_hv_memories_agent_id_producer_subagent_id_hv_subagents_agent_id_id_fk` FOREIGN KEY (`agent_id`,`producer_subagent_id`) REFERENCES `hv_subagents`(`agent_id`,`id`),
	CONSTRAINT `fk_hv_memories_agent_id_source_run_id_producer_subagent_id_hv_subagent_runs_agent_id_id_subagent_id_fk` FOREIGN KEY (`agent_id`,`source_run_id`,`producer_subagent_id`) REFERENCES `hv_subagent_runs`(`agent_id`,`id`,`subagent_id`),
	CONSTRAINT "hv_memory_number_positive" CHECK("number" BETWEEN 1 AND 9007199254740990),
	CONSTRAINT "hv_memory_kind" CHECK("kind" IN ('research','decision','review')),
	CONSTRAINT "hv_memory_text_bounds" CHECK(length("summary") BETWEEN 1 AND 500 AND length("content") BETWEEN 1 AND 16000 AND length(CAST("payload" AS BLOB)) <= 16384 AND json_valid("payload")),
	CONSTRAINT "hv_memory_research_deadline" CHECK("kind" != 'research' OR ("evidence_as_of" IS NOT NULL AND "valid_until" IS NOT NULL AND "valid_until" > "evidence_as_of" AND "instrument" IS NOT NULL AND "interval" IS NOT NULL)),
	CONSTRAINT "hv_memory_run_producer" CHECK("source_run_id" IS NULL OR "producer_subagent_id" IS NOT NULL),
	CONSTRAINT "hv_memory_retraction" CHECK(("retracted_at" IS NULL) = ("retraction_reason" IS NULL))
);
--> statement-breakpoint
CREATE TABLE `hv_memory_artifacts` (
	`agent_id` text NOT NULL,
	`memory_number` integer NOT NULL,
	`artifact_id` text NOT NULL,
	CONSTRAINT `hv_memory_artifacts_pk` PRIMARY KEY(`agent_id`, `memory_number`, `artifact_id`),
	CONSTRAINT `fk_hv_memory_artifacts_agent_id_memory_number_hv_memories_agent_id_number_fk` FOREIGN KEY (`agent_id`,`memory_number`) REFERENCES `hv_memories`(`agent_id`,`number`),
	CONSTRAINT `fk_hv_memory_artifacts_agent_id_artifact_id_hv_artifacts_agent_id_id_fk` FOREIGN KEY (`agent_id`,`artifact_id`) REFERENCES `hv_artifacts`(`agent_id`,`id`)
);
--> statement-breakpoint
CREATE TABLE `hv_memory_counters` (
	`agent_id` text PRIMARY KEY,
	`next_number` integer DEFAULT 1 NOT NULL,
	CONSTRAINT `fk_hv_memory_counters_agent_id_hv_agents_id_fk` FOREIGN KEY (`agent_id`) REFERENCES `hv_agents`(`id`) ON DELETE RESTRICT,
	CONSTRAINT "hv_memory_counter_positive" CHECK("next_number" BETWEEN 1 AND 9007199254740991)
);
--> statement-breakpoint
CREATE TABLE `hv_memory_links` (
	`agent_id` text NOT NULL,
	`source_number` integer NOT NULL,
	`target_number` integer NOT NULL,
	`relation` text NOT NULL,
	CONSTRAINT `hv_memory_links_pk` PRIMARY KEY(`agent_id`, `source_number`, `target_number`, `relation`),
	CONSTRAINT `fk_hv_memory_links_agent_id_source_number_hv_memories_agent_id_number_fk` FOREIGN KEY (`agent_id`,`source_number`) REFERENCES `hv_memories`(`agent_id`,`number`),
	CONSTRAINT `fk_hv_memory_links_agent_id_target_number_hv_memories_agent_id_number_fk` FOREIGN KEY (`agent_id`,`target_number`) REFERENCES `hv_memories`(`agent_id`,`number`),
	CONSTRAINT "hv_memory_link_backward" CHECK("target_number" < "source_number"),
	CONSTRAINT "hv_memory_link_relation" CHECK("relation" IN ('based_on','corrects','supersedes','reviews'))
);
--> statement-breakpoint
CREATE TABLE `hv_memory_operations` (
	`agent_id` text NOT NULL,
	`operation_key` text NOT NULL,
	`operation_kind` text NOT NULL,
	`request_hash` text NOT NULL,
	`result` text NOT NULL,
	`created_at` integer NOT NULL,
	CONSTRAINT `hv_memory_operations_pk` PRIMARY KEY(`agent_id`, `operation_key`),
	CONSTRAINT `fk_hv_memory_operations_agent_id_hv_agents_id_fk` FOREIGN KEY (`agent_id`) REFERENCES `hv_agents`(`id`)
);
--> statement-breakpoint
CREATE TABLE `hv_learning_revision_memories` (
	`agent_id` text NOT NULL,
	`set_id` text NOT NULL,
	`version` integer NOT NULL,
	`memory_number` integer NOT NULL,
	CONSTRAINT `hv_learning_revision_memories_pk` PRIMARY KEY(`agent_id`, `set_id`, `version`, `memory_number`),
	CONSTRAINT `fk_hv_learning_revision_memories_agent_id_set_id_version_hv_learning_revisions_agent_id_set_id_version_fk` FOREIGN KEY (`agent_id`,`set_id`,`version`) REFERENCES `hv_learning_revisions`(`agent_id`,`set_id`,`version`),
	CONSTRAINT `fk_hv_learning_revision_memories_agent_id_memory_number_hv_memories_agent_id_number_fk` FOREIGN KEY (`agent_id`,`memory_number`) REFERENCES `hv_memories`(`agent_id`,`number`)
);
--> statement-breakpoint
CREATE TABLE `hv_learning_revisions` (
	`agent_id` text NOT NULL,
	`set_id` text NOT NULL,
	`version` integer NOT NULL,
	`content` text NOT NULL,
	`operation` text NOT NULL,
	`reason` text NOT NULL,
	`created_at` integer NOT NULL,
	`actor_user_id` text,
	`producer_subagent_id` text,
	`source_run_id` text,
	`conversation_id` integer,
	`tool_task_id` integer,
	CONSTRAINT `hv_learning_revisions_pk` PRIMARY KEY(`set_id`, `version`),
	CONSTRAINT `fk_hv_learning_revisions_agent_id_set_id_hv_learning_sets_agent_id_id_fk` FOREIGN KEY (`agent_id`,`set_id`) REFERENCES `hv_learning_sets`(`agent_id`,`id`),
	CONSTRAINT `fk_hv_learning_revisions_agent_id_producer_subagent_id_hv_subagents_agent_id_id_fk` FOREIGN KEY (`agent_id`,`producer_subagent_id`) REFERENCES `hv_subagents`(`agent_id`,`id`),
	CONSTRAINT `fk_hv_learning_revisions_agent_id_source_run_id_producer_subagent_id_hv_subagent_runs_agent_id_id_subagent_id_fk` FOREIGN KEY (`agent_id`,`source_run_id`,`producer_subagent_id`) REFERENCES `hv_subagent_runs`(`agent_id`,`id`,`subagent_id`),
	CONSTRAINT "hv_learning_revision_version" CHECK("version" BETWEEN 1 AND 9007199254740990),
	CONSTRAINT "hv_learning_content_bound" CHECK(length("content") <= 8000),
	CONSTRAINT "hv_learning_operation" CHECK("operation" IN ('append','replace')),
	CONSTRAINT "hv_learning_revision_actor" CHECK(("actor_user_id" IS NOT NULL AND "source_run_id" IS NULL) OR ("actor_user_id" IS NULL AND "producer_subagent_id" IS NOT NULL))
);
--> statement-breakpoint
CREATE TABLE `hv_learning_sets` (
	`id` text PRIMARY KEY,
	`agent_id` text NOT NULL,
	`scope_kind` text NOT NULL,
	`subagent_id` text,
	`current_version` integer DEFAULT 0 NOT NULL,
	`updated_at` integer NOT NULL,
	CONSTRAINT `fk_hv_learning_sets_agent_id_hv_agents_id_fk` FOREIGN KEY (`agent_id`) REFERENCES `hv_agents`(`id`),
	CONSTRAINT `fk_hv_learning_sets_agent_id_subagent_id_hv_subagents_agent_id_id_fk` FOREIGN KEY (`agent_id`,`subagent_id`) REFERENCES `hv_subagents`(`agent_id`,`id`),
	CONSTRAINT "hv_learning_scope" CHECK(("scope_kind" = 'agent' AND "subagent_id" IS NULL) OR ("scope_kind" = 'worker' AND "subagent_id" IS NOT NULL)),
	CONSTRAINT "hv_learning_head" CHECK("current_version" BETWEEN 0 AND 9007199254740990)
);
--> statement-breakpoint
CREATE TABLE `hv_subagent_run_contexts` (
	`agent_id` text NOT NULL,
	`run_id` text PRIMARY KEY,
	`context` text NOT NULL,
	CONSTRAINT `fk_hv_subagent_run_contexts_agent_id_run_id_hv_subagent_runs_agent_id_id_fk` FOREIGN KEY (`agent_id`,`run_id`) REFERENCES `hv_subagent_runs`(`agent_id`,`id`),
	CONSTRAINT "hv_run_context_bound" CHECK(length("context") <= 120000)
);
--> statement-breakpoint
CREATE TABLE `hv_risk_settings` (
	`agent_id` text PRIMARY KEY,
	`trading_enabled` integer DEFAULT false NOT NULL,
	`allowed_instruments` text,
	`max_order_notional` text,
	`max_total_notional` text,
	`updated_at` integer NOT NULL,
	CONSTRAINT `fk_hv_risk_settings_agent_id_hv_agents_id_fk` FOREIGN KEY (`agent_id`) REFERENCES `hv_agents`(`id`) ON DELETE CASCADE,
	CONSTRAINT "hv_risk_instruments_bound" CHECK(length(coalesce("allowed_instruments", '[]')) <= 4000 AND json_valid(coalesce("allowed_instruments", '[]')))
);
--> statement-breakpoint
ALTER TABLE `hv_subagents` ADD `template_origin` text;--> statement-breakpoint
CREATE UNIQUE INDEX `hv_order_request_identity` ON `hv_order_requests` (`agent_id`,`id`);--> statement-breakpoint
CREATE UNIQUE INDEX `hv_subagent_runs_producer_idx` ON `hv_subagent_runs` (`agent_id`,`id`,`subagent_id`);--> statement-breakpoint
CREATE UNIQUE INDEX `hv_subagent_runs_identity_idx` ON `hv_subagent_runs` (`agent_id`,`id`);--> statement-breakpoint
CREATE UNIQUE INDEX `hv_subagents_identity_idx` ON `hv_subagents` (`agent_id`,`id`);--> statement-breakpoint
CREATE UNIQUE INDEX `hv_artifacts_identity_idx` ON `hv_artifacts` (`agent_id`,`id`);--> statement-breakpoint
CREATE INDEX `hv_memories_history_idx` ON `hv_memories` (`agent_id`,`created_at`,`number`);--> statement-breakpoint
CREATE INDEX `hv_memories_stream_idx` ON `hv_memories` (`agent_id`,`kind`,`instrument`,`topic`,`interval`,`producer_subagent_id`,`number`);--> statement-breakpoint
CREATE INDEX `hv_memories_run_idx` ON `hv_memories` (`agent_id`,`source_run_id`);--> statement-breakpoint
CREATE INDEX `hv_memory_artifacts_evidence_idx` ON `hv_memory_artifacts` (`agent_id`,`artifact_id`);--> statement-breakpoint
CREATE UNIQUE INDEX `hv_learning_revision_identity_idx` ON `hv_learning_revisions` (`agent_id`,`set_id`,`version`);--> statement-breakpoint
CREATE UNIQUE INDEX `hv_learning_sets_identity_idx` ON `hv_learning_sets` (`agent_id`,`id`);--> statement-breakpoint
CREATE UNIQUE INDEX `hv_learning_sets_agent_idx` ON `hv_learning_sets` (`agent_id`) WHERE "hv_learning_sets"."scope_kind" = 'agent';--> statement-breakpoint
CREATE UNIQUE INDEX `hv_learning_sets_worker_idx` ON `hv_learning_sets` (`agent_id`,`subagent_id`) WHERE "hv_learning_sets"."scope_kind" = 'worker';