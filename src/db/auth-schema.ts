import { defineRelationsPart, sql } from "drizzle-orm";
import { sqliteTable, text, integer, index } from "drizzle-orm/sqlite-core";

export const hvUsers = sqliteTable("hv_users", {
  id: text("id").primaryKey(),
  name: text("name").notNull(),
  email: text("email").notNull().unique(),
  emailVerified: integer("email_verified", { mode: "boolean" })
    .default(false)
    .notNull(),
  image: text("image"),
  createdAt: integer("created_at", { mode: "timestamp_ms" })
    .default(sql`(cast(unixepoch('subsecond') * 1000 as integer))`)
    .notNull(),
  updatedAt: integer("updated_at", { mode: "timestamp_ms" })
    .default(sql`(cast(unixepoch('subsecond') * 1000 as integer))`)
    .$onUpdate(() => /* @__PURE__ */ new Date())
    .notNull(),
});

export const hvSessions = sqliteTable(
  "hv_sessions",
  {
    id: text("id").primaryKey(),
    expiresAt: integer("expires_at", { mode: "timestamp_ms" }).notNull(),
    token: text("token").notNull().unique(),
    createdAt: integer("created_at", { mode: "timestamp_ms" })
      .default(sql`(cast(unixepoch('subsecond') * 1000 as integer))`)
      .notNull(),
    updatedAt: integer("updated_at", { mode: "timestamp_ms" })
      .$onUpdate(() => /* @__PURE__ */ new Date())
      .notNull(),
    ipAddress: text("ip_address"),
    userAgent: text("user_agent"),
    userId: text("user_id")
      .notNull()
      .references(() => hvUsers.id, { onDelete: "cascade" }),
  },
  (table) => [index("hvSessions_userId_idx").on(table.userId)],
);

export const hvAuthAccounts = sqliteTable(
  "hv_auth_accounts",
  {
    id: text("id").primaryKey(),
    accountId: text("account_id").notNull(),
    providerId: text("provider_id").notNull(),
    userId: text("user_id")
      .notNull()
      .references(() => hvUsers.id, { onDelete: "cascade" }),
    accessToken: text("access_token"),
    refreshToken: text("refresh_token"),
    idToken: text("id_token"),
    accessTokenExpiresAt: integer("access_token_expires_at", {
      mode: "timestamp_ms",
    }),
    refreshTokenExpiresAt: integer("refresh_token_expires_at", {
      mode: "timestamp_ms",
    }),
    scope: text("scope"),
    password: text("password"),
    createdAt: integer("created_at", { mode: "timestamp_ms" })
      .default(sql`(cast(unixepoch('subsecond') * 1000 as integer))`)
      .notNull(),
    updatedAt: integer("updated_at", { mode: "timestamp_ms" })
      .$onUpdate(() => /* @__PURE__ */ new Date())
      .notNull(),
  },
  (table) => [index("hvAuthAccounts_userId_idx").on(table.userId)],
);

export const hvAuthVerifications = sqliteTable(
  "hv_auth_verifications",
  {
    id: text("id").primaryKey(),
    identifier: text("identifier").notNull(),
    value: text("value").notNull(),
    expiresAt: integer("expires_at", { mode: "timestamp_ms" }).notNull(),
    createdAt: integer("created_at", { mode: "timestamp_ms" })
      .default(sql`(cast(unixepoch('subsecond') * 1000 as integer))`)
      .notNull(),
    updatedAt: integer("updated_at", { mode: "timestamp_ms" })
      .default(sql`(cast(unixepoch('subsecond') * 1000 as integer))`)
      .$onUpdate(() => /* @__PURE__ */ new Date())
      .notNull(),
  },
  (table) => [index("hvAuthVerifications_identifier_idx").on(table.identifier)],
);

export const hvWalletAddresses = sqliteTable(
  "hv_wallet_addresses",
  {
    id: text("id").primaryKey(),
    userId: text("user_id")
      .notNull()
      .references(() => hvUsers.id, { onDelete: "cascade" }),
    address: text("address").notNull(),
    chainId: integer("chain_id").notNull(),
    isPrimary: integer("is_primary", { mode: "boolean" }).default(false),
    createdAt: integer("created_at", { mode: "timestamp_ms" }).notNull(),
  },
  (table) => [index("hvWalletAddresses_userId_idx").on(table.userId)],
);

export const authRelations = defineRelationsPart(
  {
    hvUsers,
    hvSessions,
    hvAuthAccounts,
    hvAuthVerifications,
    hvWalletAddresses,
  },
  (r) => ({
    hvUsers: {
      hvSessions: r.many.hvSessions({
        from: r.hvUsers.id,
        to: r.hvSessions.userId,
      }),
      hvAuthAccounts: r.many.hvAuthAccounts({
        from: r.hvUsers.id,
        to: r.hvAuthAccounts.userId,
      }),
      hvWalletAddresses: r.many.hvWalletAddresses({
        from: r.hvUsers.id,
        to: r.hvWalletAddresses.userId,
      }),
    },
    hvSessions: {
      hvUsers: r.one.hvUsers({
        from: r.hvSessions.userId,
        to: r.hvUsers.id,
      }),
    },
    hvAuthAccounts: {
      hvUsers: r.one.hvUsers({
        from: r.hvAuthAccounts.userId,
        to: r.hvUsers.id,
      }),
    },
    hvWalletAddresses: {
      hvUsers: r.one.hvUsers({
        from: r.hvWalletAddresses.userId,
        to: r.hvUsers.id,
      }),
    },
  }),
);
