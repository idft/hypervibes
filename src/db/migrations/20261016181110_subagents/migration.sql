CREATE TABLE `hv_subagent_runs` (
	`id` text PRIMARY KEY,
	`agent_id` text NOT NULL,
	`subagent_id` text NOT NULL,
	`request_id` text NOT NULL,
	`source` text NOT NULL,
	`task` text NOT NULL,
	`config` text NOT NULL,
	`status` text DEFAULT 'queued' NOT NULL,
	`task_id` integer,
	`conversation_id` integer,
	`result` text DEFAULT '' NOT NULL,
	`created_at` integer NOT NULL,
	`finished_at` integer,
	`reported_at` integer,
	CONSTRAINT `fk_hv_subagent_runs_agent_id_hv_agents_id_fk` FOREIGN KEY (`agent_id`) REFERENCES `hv_agents`(`id`) ON DELETE RESTRICT,
	CONSTRAINT `fk_hv_subagent_runs_subagent_id_hv_subagents_id_fk` FOREIGN KEY (`subagent_id`) REFERENCES `hv_subagents`(`id`) ON DELETE RESTRICT
);
--> statement-breakpoint
CREATE TABLE `hv_subagents` (
	`id` text PRIMARY KEY,
	`agent_id` text NOT NULL,
	`kind` text NOT NULL,
	`name` text NOT NULL,
	`description` text DEFAULT '' NOT NULL,
	`instructions` text NOT NULL,
	`provider` text NOT NULL,
	`model_id` text NOT NULL,
	`thinking_level` text DEFAULT '' NOT NULL,
	`tools` text NOT NULL,
	`enabled` integer DEFAULT true NOT NULL,
	`archived` integer DEFAULT false NOT NULL,
	`delegatable` integer DEFAULT true NOT NULL,
	`persistent` integer DEFAULT false NOT NULL,
	`conversation_id` integer,
	`schedule` text DEFAULT '' NOT NULL,
	`delay_seconds` integer DEFAULT 30 NOT NULL,
	`next_run_at` integer,
	`completion` text DEFAULT 'wake' NOT NULL,
	`timeout_seconds` integer DEFAULT 900 NOT NULL,
	`created_at` integer NOT NULL,
	`updated_at` integer NOT NULL,
	CONSTRAINT `fk_hv_subagents_agent_id_hv_agents_id_fk` FOREIGN KEY (`agent_id`) REFERENCES `hv_agents`(`id`) ON DELETE RESTRICT,
	CONSTRAINT "hv_subagents_main_protected" CHECK("kind" != 'orchestrator' OR ("enabled" = 1 AND "archived" = 0 AND "schedule" = ''))
);
--> statement-breakpoint
CREATE UNIQUE INDEX `hv_subagent_runs_request_idx` ON `hv_subagent_runs` (`agent_id`,`request_id`);--> statement-breakpoint
CREATE UNIQUE INDEX `hv_subagent_runs_active_idx` ON `hv_subagent_runs` (`subagent_id`) WHERE "hv_subagent_runs"."status" IN ('queued', 'running');--> statement-breakpoint
CREATE INDEX `hv_subagent_runs_history_idx` ON `hv_subagent_runs` (`subagent_id`,`created_at`);--> statement-breakpoint
CREATE INDEX `hv_subagents_agent_idx` ON `hv_subagents` (`agent_id`);--> statement-breakpoint
CREATE UNIQUE INDEX `hv_subagents_main_idx` ON `hv_subagents` (`agent_id`) WHERE "hv_subagents"."kind" = 'orchestrator';