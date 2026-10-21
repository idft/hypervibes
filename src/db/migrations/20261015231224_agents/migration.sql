CREATE TABLE `hv_agents` (
	`id` text PRIMARY KEY,
	`user_id` text NOT NULL,
	`name` text NOT NULL,
	`creation_request_id` text NOT NULL,
	`state` text DEFAULT 'provisioning' NOT NULL,
	`conversation_id` integer,
	`error` text,
	`trading_account_address` text,
	`created_at` integer NOT NULL,
	`updated_at` integer NOT NULL,
	CONSTRAINT `fk_hv_agents_user_id_hv_users_id_fk` FOREIGN KEY (`user_id`) REFERENCES `hv_users`(`id`) ON DELETE RESTRICT,
	CONSTRAINT "hv_agents_name_check" CHECK(length(trim("name")) BETWEEN 1 AND 80),
	CONSTRAINT "hv_agents_state_check" CHECK("state" IN ('provisioning', 'ready', 'failed', 'stopping', 'stopped'))
);
--> statement-breakpoint
CREATE INDEX `hv_agents_owner_idx` ON `hv_agents` (`user_id`);--> statement-breakpoint
CREATE UNIQUE INDEX `hv_agents_creation_request_idx` ON `hv_agents` (`user_id`,`creation_request_id`);--> statement-breakpoint
CREATE UNIQUE INDEX `hv_agents_trading_account_idx` ON `hv_agents` (`trading_account_address`);--> statement-breakpoint
CREATE TABLE `hv_hyperliquid_settings` (
	`user_id` text PRIMARY KEY,
	`api_wallet_address` text,
	`private_key_ciphertext` blob,
	`private_key_key_id` text,
	`api_wallet_approved_at` integer,
	`api_wallet_expires_at` integer,
	`api_wallet_expiry_checked_at` integer,
	`builder_fee_tenths_of_bp` integer DEFAULT 0 NOT NULL,
	`builder_fee_approved_at` integer,
	`created_at` integer NOT NULL,
	`updated_at` integer NOT NULL,
	CONSTRAINT `fk_hv_hyperliquid_settings_user_id_hv_users_id_fk` FOREIGN KEY (`user_id`) REFERENCES `hv_users`(`id`) ON DELETE RESTRICT,
	CONSTRAINT "hv_hyperliquid_settings_builder_fee_check" CHECK("builder_fee_tenths_of_bp" BETWEEN 0 AND 100)
);