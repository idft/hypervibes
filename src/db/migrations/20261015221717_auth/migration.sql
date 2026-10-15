CREATE TABLE `hv_auth_accounts` (
	`id` text PRIMARY KEY,
	`account_id` text NOT NULL,
	`provider_id` text NOT NULL,
	`user_id` text NOT NULL,
	`access_token` text,
	`refresh_token` text,
	`id_token` text,
	`access_token_expires_at` integer,
	`refresh_token_expires_at` integer,
	`scope` text,
	`password` text,
	`created_at` integer DEFAULT (cast(unixepoch('subsecond') * 1000 as integer)) NOT NULL,
	`updated_at` integer NOT NULL,
	CONSTRAINT `fk_hv_auth_accounts_user_id_hv_users_id_fk` FOREIGN KEY (`user_id`) REFERENCES `hv_users`(`id`) ON DELETE CASCADE
);
--> statement-breakpoint
CREATE TABLE `hv_auth_verifications` (
	`id` text PRIMARY KEY,
	`identifier` text NOT NULL,
	`value` text NOT NULL,
	`expires_at` integer NOT NULL,
	`created_at` integer DEFAULT (cast(unixepoch('subsecond') * 1000 as integer)) NOT NULL,
	`updated_at` integer DEFAULT (cast(unixepoch('subsecond') * 1000 as integer)) NOT NULL
);
--> statement-breakpoint
CREATE TABLE `hv_sessions` (
	`id` text PRIMARY KEY,
	`expires_at` integer NOT NULL,
	`token` text NOT NULL UNIQUE,
	`created_at` integer DEFAULT (cast(unixepoch('subsecond') * 1000 as integer)) NOT NULL,
	`updated_at` integer NOT NULL,
	`ip_address` text,
	`user_agent` text,
	`user_id` text NOT NULL,
	CONSTRAINT `fk_hv_sessions_user_id_hv_users_id_fk` FOREIGN KEY (`user_id`) REFERENCES `hv_users`(`id`) ON DELETE CASCADE
);
--> statement-breakpoint
CREATE TABLE `hv_users` (
	`id` text PRIMARY KEY,
	`name` text NOT NULL,
	`email` text NOT NULL UNIQUE,
	`email_verified` integer DEFAULT false NOT NULL,
	`image` text,
	`created_at` integer DEFAULT (cast(unixepoch('subsecond') * 1000 as integer)) NOT NULL,
	`updated_at` integer DEFAULT (cast(unixepoch('subsecond') * 1000 as integer)) NOT NULL
);
--> statement-breakpoint
CREATE TABLE `hv_wallet_addresses` (
	`id` text PRIMARY KEY,
	`user_id` text NOT NULL,
	`address` text NOT NULL,
	`chain_id` integer NOT NULL,
	`is_primary` integer DEFAULT false,
	`created_at` integer NOT NULL,
	CONSTRAINT `fk_hv_wallet_addresses_user_id_hv_users_id_fk` FOREIGN KEY (`user_id`) REFERENCES `hv_users`(`id`) ON DELETE CASCADE
);
--> statement-breakpoint
CREATE INDEX `hvAuthAccounts_userId_idx` ON `hv_auth_accounts` (`user_id`);--> statement-breakpoint
CREATE INDEX `hvAuthVerifications_identifier_idx` ON `hv_auth_verifications` (`identifier`);--> statement-breakpoint
CREATE INDEX `hvSessions_userId_idx` ON `hv_sessions` (`user_id`);--> statement-breakpoint
CREATE INDEX `hvWalletAddresses_userId_idx` ON `hv_wallet_addresses` (`user_id`);