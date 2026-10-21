CREATE TABLE `hv_chart_layouts` (
  `id` text PRIMARY KEY,
  `agent_id` text NOT NULL,
  `document` text NOT NULL,
  `revision` integer DEFAULT 0 NOT NULL,
  `history` text NOT NULL,
  `updated_at` integer NOT NULL,
  CONSTRAINT `fk_hv_chart_layouts_agent_id_hv_agents_id_fk` FOREIGN KEY (`agent_id`) REFERENCES `hv_agents`(`id`) ON DELETE CASCADE
);
--> statement-breakpoint
CREATE TABLE `hv_chart_operations` (
  `agent_id` text NOT NULL,
  `key` text NOT NULL,
  `hash` text NOT NULL,
  `result` text NOT NULL,
  CONSTRAINT `hv_chart_operations_pk` PRIMARY KEY (`agent_id`, `key`),
  CONSTRAINT `fk_hv_chart_operations_agent_id_hv_agents_id_fk` FOREIGN KEY (`agent_id`) REFERENCES `hv_agents`(`id`) ON DELETE CASCADE
);
--> statement-breakpoint
CREATE TABLE `hv_chart_snapshots` (
  `id` text NOT NULL,
  `agent_id` text NOT NULL,
  `request_hash` text NOT NULL,
  `context` text NOT NULL,
  `image` text,
  `created_at` integer NOT NULL,
  CONSTRAINT `hv_chart_snapshots_pk` PRIMARY KEY (`agent_id`, `id`),
  CONSTRAINT `fk_hv_chart_snapshots_agent_id_hv_agents_id_fk` FOREIGN KEY (`agent_id`) REFERENCES `hv_agents`(`id`) ON DELETE CASCADE
);
--> statement-breakpoint
CREATE INDEX `hv_chart_layouts_agent_idx` ON `hv_chart_layouts` (`agent_id`, `updated_at`);
--> statement-breakpoint
CREATE INDEX `hv_chart_snapshots_agent_idx` ON `hv_chart_snapshots` (`agent_id`, `created_at`);
