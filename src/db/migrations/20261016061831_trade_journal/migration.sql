CREATE TABLE `hv_journal_accounts` (
	`account` text PRIMARY KEY,
	`revision` integer DEFAULT 0 NOT NULL,
	`projected_revision` integer DEFAULT -1 NOT NULL,
	`history_limited` integer DEFAULT 0 NOT NULL,
	CONSTRAINT "hv_journal_history_limited" CHECK("history_limited" IN (0, 1))
);
--> statement-breakpoint
CREATE TABLE `hv_journal_entries` (
	`account` text NOT NULL,
	`id` text NOT NULL,
	`kind` text NOT NULL,
	`time` integer NOT NULL,
	`data` text NOT NULL,
	`balance` text,
	`active` integer DEFAULT 1 NOT NULL,
	CONSTRAINT `hv_journal_entries_pk` PRIMARY KEY(`account`, `id`),
	CONSTRAINT `fk_hv_journal_entries_account_hv_journal_accounts_account_fk` FOREIGN KEY (`account`) REFERENCES `hv_journal_accounts`(`account`),
	CONSTRAINT "hv_journal_entry_kind" CHECK("kind" IN ('fill', 'funding', 'ledger', 'trade')),
	CONSTRAINT "hv_journal_entry_active" CHECK("active" IN (0, 1))
);
--> statement-breakpoint
CREATE TABLE `hv_journal_notes` (
	`id` text PRIMARY KEY,
	`account` text NOT NULL,
	`target` text NOT NULL,
	`author_id` text NOT NULL,
	`body` text NOT NULL,
	`created_at` integer NOT NULL,
	CONSTRAINT `fk_hv_journal_notes_author_id_hv_users_id_fk` FOREIGN KEY (`author_id`) REFERENCES `hv_users`(`id`),
	CONSTRAINT `fk_hv_journal_notes_account_target_hv_journal_entries_account_id_fk` FOREIGN KEY (`account`,`target`) REFERENCES `hv_journal_entries`(`account`,`id`),
	CONSTRAINT "hv_journal_note_body" CHECK(length(trim("body")) > 0 AND length("body") <= 4000)
);
--> statement-breakpoint
CREATE TABLE `hv_journal_sync` (
	`account` text NOT NULL,
	`stream` text NOT NULL,
	`cursor` integer DEFAULT 0 NOT NULL,
	`updated_at` integer,
	`error` text,
	CONSTRAINT `hv_journal_sync_pk` PRIMARY KEY(`account`, `stream`),
	CONSTRAINT `fk_hv_journal_sync_account_hv_journal_accounts_account_fk` FOREIGN KEY (`account`) REFERENCES `hv_journal_accounts`(`account`),
	CONSTRAINT "hv_journal_sync_stream" CHECK("stream" IN ('fill', 'funding', 'ledger'))
);
--> statement-breakpoint
CREATE INDEX `hv_journal_entries_page` ON `hv_journal_entries` (`account`,`kind`,`active`,`time`,`id`);--> statement-breakpoint
CREATE INDEX `hv_journal_entries_timeline` ON `hv_journal_entries` (`account`,`time`,`id`);--> statement-breakpoint
CREATE INDEX `hv_journal_notes_target` ON `hv_journal_notes` (`account`,`target`,`created_at`,`id`);