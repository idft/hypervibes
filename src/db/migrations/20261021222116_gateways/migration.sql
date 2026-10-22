CREATE TABLE `hv_gateways` (
	`agent_id` text NOT NULL,
	`type` text NOT NULL,
	`bot_token_ciphertext` blob NOT NULL,
	`bot_token_hash` text NOT NULL,
	`key_id` text NOT NULL,
	`bot_username` text NOT NULL,
	`chat_id` integer,
	`chat_username` text,
	`updated_at` integer NOT NULL,
	CONSTRAINT `hv_gateways_agent_id_type_pk` PRIMARY KEY (`agent_id`,`type`),
	CONSTRAINT `fk_hv_gateways_agent_id_hv_agents_id_fk` FOREIGN KEY (`agent_id`) REFERENCES `hv_agents`(`id`) ON DELETE CASCADE
);
--> statement-breakpoint
CREATE UNIQUE INDEX `hv_gateways_token_hash_idx` ON `hv_gateways` (`bot_token_hash`);
