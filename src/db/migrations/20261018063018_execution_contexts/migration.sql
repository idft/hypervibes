CREATE TABLE `hv_agent_execution_frames` (
	`agent_id` text NOT NULL,
	`generation_task_id` integer NOT NULL,
	`conversation_id` integer NOT NULL,
	`input_scope_key` text NOT NULL,
	`inputs` text NOT NULL,
	`run_id` text,
	`origin` text NOT NULL,
	`config` text NOT NULL,
	`created_at` integer NOT NULL,
	`expires_at` integer NOT NULL,
	CONSTRAINT `hv_agent_execution_frames_pk` PRIMARY KEY(`agent_id`, `generation_task_id`),
	CONSTRAINT `fk_hv_agent_execution_frames_agent_id_hv_agents_id_fk` FOREIGN KEY (`agent_id`) REFERENCES `hv_agents`(`id`),
	CONSTRAINT `fk_hv_agent_execution_frames_agent_id_run_id_hv_subagent_runs_agent_id_id_fk` FOREIGN KEY (`agent_id`,`run_id`) REFERENCES `hv_subagent_runs`(`agent_id`,`id`)
);
--> statement-breakpoint
CREATE TABLE `hv_agent_input_origins` (
	`agent_id` text NOT NULL,
	`request_id` text NOT NULL,
	`body_hash` text NOT NULL,
	`user_id` text NOT NULL,
	`admitted_at` integer NOT NULL,
	`expires_at` integer NOT NULL,
	CONSTRAINT `hv_agent_input_origins_pk` PRIMARY KEY(`agent_id`, `request_id`),
	CONSTRAINT `fk_hv_agent_input_origins_agent_id_hv_agents_id_fk` FOREIGN KEY (`agent_id`) REFERENCES `hv_agents`(`id`),
	CONSTRAINT "hv_input_deadline" CHECK("expires_at" > "admitted_at")
);
--> statement-breakpoint
CREATE TABLE `hv_automatic_entry_claims` (
	`agent_id` text NOT NULL,
	`slot_key` text NOT NULL,
	`request_id` text NOT NULL,
	CONSTRAINT `hv_automatic_entry_claims_pk` PRIMARY KEY(`agent_id`, `slot_key`),
	CONSTRAINT `fk_hv_automatic_entry_claims_agent_id_request_id_hv_order_requests_agent_id_id_fk` FOREIGN KEY (`agent_id`,`request_id`) REFERENCES `hv_order_requests`(`agent_id`,`id`)
);
--> statement-breakpoint
CREATE TABLE `hv_order_execution_contexts` (
	`agent_id` text NOT NULL,
	`request_id` text PRIMARY KEY,
	`decision_number` integer,
	`generation_task_id` integer NOT NULL,
	`run_id` text,
	`origin` text NOT NULL,
	`policy` text NOT NULL,
	`slot_key` text,
	CONSTRAINT `fk_hv_order_execution_contexts_agent_id_request_id_hv_order_requests_agent_id_id_fk` FOREIGN KEY (`agent_id`,`request_id`) REFERENCES `hv_order_requests`(`agent_id`,`id`),
	CONSTRAINT `fk_hv_order_execution_contexts_agent_id_decision_number_hv_memories_agent_id_number_fk` FOREIGN KEY (`agent_id`,`decision_number`) REFERENCES `hv_memories`(`agent_id`,`number`),
	CONSTRAINT `fk_hv_order_execution_contexts_agent_id_generation_task_id_hv_agent_execution_frames_agent_id_generation_task_id_fk` FOREIGN KEY (`agent_id`,`generation_task_id`) REFERENCES `hv_agent_execution_frames`(`agent_id`,`generation_task_id`)
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
ALTER TABLE `hv_subagents` ADD `workflow` text;--> statement-breakpoint
ALTER TABLE `hv_subagents` ADD `template_origin` text;--> statement-breakpoint
CREATE UNIQUE INDEX `hv_order_request_identity` ON `hv_order_requests` (`agent_id`,`id`);--> statement-breakpoint
CREATE UNIQUE INDEX `hv_subagent_runs_identity_idx` ON `hv_subagent_runs` (`agent_id`,`id`);--> statement-breakpoint
CREATE UNIQUE INDEX `hv_entry_claim_request_idx` ON `hv_automatic_entry_claims` (`agent_id`,`request_id`);