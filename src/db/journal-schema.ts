import { sql } from "drizzle-orm";
import { check, foreignKey, index, integer, primaryKey, sqliteTable, text } from "drizzle-orm/sqlite-core";
import { hvUsers } from "./auth-schema.js";

export const hvJournalAccounts = sqliteTable("hv_journal_accounts", {
  account: text("account").primaryKey(),
  revision: integer("revision").notNull().default(0),
  projectedRevision: integer("projected_revision").notNull().default(-1),
  historyLimited: integer("history_limited").notNull().default(0),
}, (t) => [check("hv_journal_history_limited", sql`${t.historyLimited} IN (0, 1)`)]);
// Application-owned source records and deterministic projections. All financial
// values are canonical decimal strings in TEXT JSON; SQL never sums/casts them.
// Superseded trade entries remain valid note targets after a late-history rebuild.
export const hvJournalEntries = sqliteTable("hv_journal_entries", {
  account: text("account").notNull().references(() => hvJournalAccounts.account),
  id: text("id").notNull(), kind: text("kind").notNull(), time: integer("time").notNull(),
  data: text("data").notNull(), balance: text("balance"), active: integer("active").notNull().default(1),
}, (t) => [primaryKey({ columns: [t.account, t.id] }),
  check("hv_journal_entry_kind", sql`${t.kind} IN ('fill', 'funding', 'ledger', 'trade')`),
  check("hv_journal_entry_active", sql`${t.active} IN (0, 1)`),
  index("hv_journal_entries_page").on(t.account, t.kind, t.active, t.time, t.id),
  index("hv_journal_entries_timeline").on(t.account, t.time, t.id),
]);
export const hvJournalSync = sqliteTable("hv_journal_sync", {
  account: text("account").notNull().references(() => hvJournalAccounts.account), stream: text("stream").notNull(),
  cursor: integer("cursor").notNull().default(0), updatedAt: integer("updated_at"), error: text("error"),
}, (t) => [primaryKey({ columns: [t.account, t.stream] }), check("hv_journal_sync_stream", sql`${t.stream} IN ('fill', 'funding', 'ledger')`)]);
export const hvJournalNotes = sqliteTable("hv_journal_notes", {
  id: text("id").primaryKey(), account: text("account").notNull(), target: text("target").notNull(),
  authorId: text("author_id").notNull().references(() => hvUsers.id),
  body: text("body").notNull(), createdAt: integer("created_at").notNull(),
}, (t) => [foreignKey({ columns: [t.account, t.target], foreignColumns: [hvJournalEntries.account, hvJournalEntries.id] }),
  check("hv_journal_note_body", sql`length(trim(${t.body})) > 0 AND length(${t.body}) <= 4000`),
  index("hv_journal_notes_target").on(t.account, t.target, t.createdAt, t.id),
]);
