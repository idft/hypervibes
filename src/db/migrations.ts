import { drizzle } from "drizzle-orm/node-sqlite";
import { migrate } from "drizzle-orm/node-sqlite/migrator";
import type { DatabaseSync } from "node:sqlite";
import { fileURLToPath } from "node:url";

/** Startup-only native access, before the application's shared SQL queue is used. */
export function migrateApplication(connection: DatabaseSync, migrationsFolder = fileURLToPath(new URL("./migrations/", import.meta.url))) {
  migrate(drizzle({ client: connection }), { migrationsFolder, migrationsTable: "hv_schema_migrations" });
}
