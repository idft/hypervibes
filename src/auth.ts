import { drizzleAdapter } from "@better-auth/drizzle-adapter/relations-v2";
import { betterAuth } from "better-auth";
import { inArray, lt } from "drizzle-orm";
import type { ApplicationDatabase } from "./db/shared-database.js";
import { createAuthOptions } from "./auth-options.js";
import type { AppConfig } from "./config.js";
import * as schema from "./db/auth-schema.js";

export function createAuth(db: ApplicationDatabase, config: AppConfig) {
  const auth = betterAuth({
    ...createAuthOptions(config),
    // Individual statements use the shared queue. Better Auth's SQLite
    // consumeOne uses atomic DELETE RETURNING for nonce replay safety.
    // Proxy transactions must not emit an unreserved BEGIN/COMMIT sequence.
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
          await db.delete(table).where(inArray(table.id,
            db.select({ id: table.id }).from(table).where(lt(table.expiresAt, now)).limit(100),
          )).run();
        }
      }
      return response;
    },
  };
}
