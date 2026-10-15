import { drizzleAdapter } from "@better-auth/drizzle-adapter/relations-v2";
import { betterAuth } from "better-auth";
import { drizzle } from "drizzle-orm/node-sqlite";
import { randomBytes } from "node:crypto";
import { createAuthOptions } from "../auth-options.js";

// CLI-only configuration. No database, environment secrets, listener or runtime.
const options = createAuthOptions({ authOrigin: "http://localhost:3003", authSecret: randomBytes(32).toString("hex") });
export const auth = betterAuth({
  ...options,
  // There is deliberately no schema yet when regenerating. Runtime validation stays enabled.
  advanced: { ...options.advanced, database: { ...options.advanced.database, validateSchema: false } },
  database: drizzleAdapter(drizzle.mock(), { provider: "sqlite", schema: {}, transaction: false }),
});
