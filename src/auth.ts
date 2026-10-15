import { drizzleAdapter } from "@better-auth/drizzle-adapter/relations-v2";
import { betterAuth } from "better-auth";
import { inArray, lt } from "drizzle-orm";
import type { NodeSQLiteDatabase } from "drizzle-orm/node-sqlite";
import { createAuthOptions } from "./auth-options.js";
import type { AppConfig } from "./config.js";
import * as schema from "./db/auth-schema.js";

export function createAuth(db: NodeSQLiteDatabase, config: AppConfig) {
  const auth = betterAuth({
    ...createAuthOptions(config),
    // DatabaseSync transactions require synchronous callbacks. Better Auth's
    // SQLite consumeOne still uses atomic DELETE RETURNING for nonce replay safety.
    database: drizzleAdapter(db, { provider: "sqlite", schema, transaction: false }),
  });
  return {
    ...auth,
    async handler(request: Request) {
      const response = await auth.handler(request);
      // Bounded, request-driven cleanup after an admitted nonce request; no timer.
      if (response.ok && request.method === "POST" && /\/siwe\/(nonce|get-nonce)$/.test(new URL(request.url).pathname)) {
        const now = new Date();
        for (const table of [schema.hvSessions, schema.hvAuthVerifications]) {
          db.delete(table).where(inArray(table.id,
            db.select({ id: table.id }).from(table).where(lt(table.expiresAt, now)).limit(100),
          )).run();
        }
      }
      return response;
    },
  };
}
